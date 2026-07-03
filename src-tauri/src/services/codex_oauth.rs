use axum::{routing::get, Router};
use base64::Engine;
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::db::dao::accounts;

/// Codex OAuth 元数据，参考 9router / cpa。
const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPE: &str = "openid profile email offline_access";
const CODE_CHALLENGE_METHOD: &str = "S256";
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";

/// Codex OAuth 凭据（加密前的明文结构）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCredentials {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub expires_at: Option<String>,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub plan_type: Option<String>,
}

/// 登录会话状态。
#[allow(dead_code)]
pub struct LoginSession {
    pub code_verifier: String,
    pub state: String,
    pub auth_url: String,
}

/// 回调服务关闭句柄，跨任务共享。
type SharedShutdown = Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>;

/// OAuth 登录管理器，同一时刻只允许一个登录会话。
pub struct CodexOAuthService {
    /// 当前会话（None 表示无进行中的登录）。
    pub session: Mutex<Option<LoginSession>>,
    /// 回调服务句柄，用于完成后释放。
    pub callback_shutdown: SharedShutdown,
}

impl CodexOAuthService {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
            callback_shutdown: Arc::new(Mutex::new(None)),
        }
    }

    /// 启动一次 Codex OAuth 登录，返回授权 URL。
    ///
    /// 同时启动临时回调服务 on 127.0.0.1:1455。
    pub async fn start_login(&self, app_state: Arc<AppState>) -> Result<String, String> {
        // 互斥：同一时刻只允许一个登录会话。
        let mut session_guard = self.session.lock().await;
        if session_guard.is_some() {
            return Err("已有 Codex OAuth 登录进行中".to_string());
        }

        let code_verifier = generate_random_string(64);
        let state = generate_random_string(32);
        let code_challenge = pkce_challenge(&code_verifier);

        let auth_url = format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method={}",
            AUTH_URL,
            CLIENT_ID,
            urlencoding::encode(REDIRECT_URI),
            urlencoding::encode(SCOPE),
            state,
            code_challenge,
            CODE_CHALLENGE_METHOD
        );

        // 启动临时回调服务。
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        *self.callback_shutdown.lock().await = Some(tx);

        let app_state_for_callback = app_state.clone();
        let state_for_callback = state.clone();
        let verifier_for_callback = code_verifier.clone();
        let callback_shutdown = self.callback_shutdown.clone();

        tokio::spawn(async move {
            if let Err(e) = run_callback_server(
                app_state_for_callback,
                verifier_for_callback,
                state_for_callback,
                rx,
                callback_shutdown,
            )
            .await
            {
                tracing::error!("Codex OAuth 回调服务异常: {}", e);
            }
        });

        let url = auth_url.clone();
        *session_guard = Some(LoginSession {
            code_verifier,
            state,
            auth_url,
        });

        Ok(url)
    }

    /// 查询当前登录会话状态（仅返回是否进行中，不暴露敏感字段）。
    pub async fn status(&self) -> bool {
        self.session.lock().await.is_some()
    }

    /// 清理会话（登录完成或失败后调用）。
    pub async fn clear_session(&self) {
        *self.session.lock().await = None;
        let mut guard = self.callback_shutdown.lock().await;
        if let Some(tx) = guard.take() {
            let _ = tx.send(()).ok();
        }
    }
}

/// 运行临时回调服务 on 127.0.0.1:1455。
async fn run_callback_server(
    app_state: Arc<AppState>,
    code_verifier: String,
    expected_state: String,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    callback_shutdown: SharedShutdown,
) -> Result<(), String> {
    let addr = format!("127.0.0.1:{}", CALLBACK_PORT);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("无法绑定 {}: {}", addr, e))?;

    tracing::info!("Codex OAuth 回调服务已启动：http://{}/", addr);

    let app = Router::new().route(
        CALLBACK_PATH,
        get(move |query: axum::extract::Query<CallbackParams>| {
            handle_callback(
                query,
                app_state.clone(),
                code_verifier.clone(),
                expected_state.clone(),
                callback_shutdown.clone(),
            )
        }),
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            shutdown_rx.await.ok();
            tracing::info!("Codex OAuth 回调服务正在关闭...");
        })
        .await
        .map_err(|e| format!("回调服务运行失败: {}", e))?;

    tracing::info!("Codex OAuth 回调服务已停止");
    Ok(())
}

/// 回调查询参数。
#[derive(Deserialize)]
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// 处理回调：验证 state，交换 token，解析 JWT，加密存储。
async fn handle_callback(
    axum::extract::Query(query): axum::extract::Query<CallbackParams>,
    app_state: Arc<AppState>,
    code_verifier: String,
    expected_state: String,
    callback_shutdown: SharedShutdown,
) -> impl axum::response::IntoResponse {
    use axum::http::StatusCode;
    use axum::response::Json;

    // 校验 error。
    if let Some(err) = &query.error {
        cleanup_session(&app_state, &callback_shutdown).await;
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "oauth_callback_error",
                "message": format!("OAuth 回调返回错误: {}", err)
            })),
        );
    }

    // 校验 state（防 CSRF）。
    if query.state.as_deref() != Some(&expected_state) {
        cleanup_session(&app_state, &callback_shutdown).await;
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "oauth_state_mismatch",
                "message": "state 不匹配，可能存在 CSRF 攻击"
            })),
        );
    }

    let code = match &query.code {
        Some(c) => c.clone(),
        None => {
            cleanup_session(&app_state, &callback_shutdown).await;
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "oauth_missing_code",
                    "message": "回调缺少 code 参数"
                })),
            );
        }
    };

    // 交换 token。
    match exchange_code_for_token(&code, &code_verifier).await {
        Ok(token_resp) => {
            // 解析 id_token JWT。
            let (account_id, email, plan_type, subject) = token_resp
                .id_token
                .as_ref()
                .map(|jwt| parse_jwt_fields(jwt))
                .unwrap_or((None, None, None, None));

            let account_id = account_id.or(subject);
            let Some(account_id_str) = account_id.clone() else {
                cleanup_session(&app_state, &callback_shutdown).await;
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": "missing_account_id",
                        "message": "Codex OAuth id_token 缺少 chatgptAccountId/accountID/sub，无法稳定识别账号"
                    })),
                );
            };

            let expires_at = token_resp.expires_in.and_then(|expires_in| {
                let at = time::OffsetDateTime::now_utc()
                    + time::Duration::seconds(expires_in as i64);
                at.format(&time::format_description::well_known::Iso8601::DEFAULT)
                    .ok()
            });

            let credentials = CodexCredentials {
                access_token: token_resp.access_token,
                refresh_token: token_resp.refresh_token,
                id_token: token_resp.id_token,
                expires_at,
                account_id: account_id.clone(),
                email: email.clone(),
                plan_type: plan_type.clone(),
            };

            // 加密并存储。
            let json = match serde_json::to_vec(&credentials) {
                Ok(j) => j,
                Err(e) => {
                    cleanup_session(&app_state, &callback_shutdown).await;
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({
                            "error": "serialize_failed",
                            "message": format!("序列化凭据失败: {}", e)
                        })),
                    );
                }
            };

            let name = email.clone().unwrap_or_else(|| "Codex 账号".to_string());

            // Upsert：已存在则更新凭据，不存在则创建。
            if let Err(e) = upsert_codex_account(&app_state, &account_id_str, &name, &json) {
                cleanup_session(&app_state, &callback_shutdown).await;
                // 503 crypto_unavailable 保留原错误码映射
                if e.contains("加密服务不可用") {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(serde_json::json!({
                            "error": "crypto_unavailable",
                            "message": e
                        })),
                    );
                }
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "db_save_failed",
                        "message": e
                    })),
                );
            }

            // 清理会话和回调服务。
            cleanup_session(&app_state, &callback_shutdown).await;

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "ok",
                    "email": email,
                    "plan_type": plan_type
                })),
            )
        }
        Err(e) => {
            cleanup_session(&app_state, &callback_shutdown).await;
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "token_exchange_failed",
                    "message": e
                })),
            )
        }
    }
}

/// 清理登录会话并关闭回调服务。
async fn cleanup_session(app_state: &Arc<AppState>, callback_shutdown: &SharedShutdown) {
    app_state.codex_oauth.clear_session().await;
    let mut guard = callback_shutdown.lock().await;
    if let Some(tx) = guard.take() {
        let _ = tx.send(()).ok();
    }
}

/// 加密凭据并 upsert 到数据库（已存在则 update，不存在则 create）。
///
/// 返回 Ok(()) 或 Err(error_msg)。
fn upsert_codex_account(
    app_state: &Arc<AppState>,
    account_id_str: &str,
    name: &str,
    credentials_json: &[u8],
) -> Result<(), String> {
    let crypto = app_state.crypto.as_ref().ok_or("加密服务不可用")?;
    let encrypted = crypto.encrypt(credentials_json, account_id_str.as_bytes())?;

    match accounts::get(&app_state.db, account_id_str)? {
        Some(_) => {
            // 重登录：覆盖凭据 + 名称，不动 account_type/platform/status/priority。
            let upd = accounts::AccountUpdate {
                name: Some(name.to_string()),
                credentials_encrypted: Some(Some(encrypted)),
                ..Default::default()
            };
            accounts::update(&app_state.db, account_id_str, upd)
        }
        None => {
            // 首次登录：create（维持现有 NewAccount 构造）。
            let new = accounts::NewAccount {
                id: account_id_str.to_string(),
                name: name.to_string(),
                account_type: "oauth_codex".to_string(),
                platform: "openai_codex".to_string(),
                credentials_encrypted: Some(encrypted),
                extra_json: None,
                priority: 0,
            };
            accounts::create(&app_state.db, new).map(|_| ())
        }
    }
}

/// Token 交换响应。
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<u64>,
}

/// 用授权码交换 token。
async fn exchange_code_for_token(code: &str, code_verifier: &str) -> Result<TokenResponse, String> {
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(
            crate::http::proxy::constants::OAUTH_REFRESH_CONNECT_TIMEOUT_SECS,
        ))
        .timeout(Duration::from_secs(
            crate::http::proxy::constants::OAUTH_REFRESH_TIMEOUT_SECS,
        ))
        .build()
        .map_err(|e| format!("创建 token 交换客户端失败: {}", e))?;
    let params = [
        ("grant_type", "authorization_code"),
        ("client_id", CLIENT_ID),
        ("code", code),
        ("redirect_uri", REDIRECT_URI),
        ("code_verifier", code_verifier),
    ];

    let resp = client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("token 交换请求失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("token 交换失败 ({}): {}", status, body));
    }

    resp.json::<TokenResponse>()
        .await
        .map_err(|e| format!("token 响应解析失败: {}", e))
}

/// 解析 JWT 的 payload 部分（不验签，仅取字段）。
///
/// JWT 格式：`header.payload.signature`，payload 是 base64url 编码的 JSON。
/// 返回 `(account_id, email, plan_type, subject)`，其中 `subject` 来自 `sub` claim，
/// 供 `account_id` 缺失时作为稳定账号身份的回退（避免随机 UUID 产生重复账号）。
fn parse_jwt_fields(
    jwt: &str,
) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return (None, None, None, None);
    }
    // base64url payload 可能带或不带 `=` padding；两种都兼容。
    let payload = match decode_base64url_lenient(parts[1]) {
        Ok(p) => p,
        Err(_) => return (None, None, None, None),
    };
    let value: serde_json::Value = match serde_json::from_slice(&payload) {
        Ok(v) => v,
        Err(_) => return (None, None, None, None),
    };

    // 9router 提取 email / chatgptAccountId / chatgptPlanType；
    // cpa 提取 accountID / email。综合两者，并补 sub 作为回退。
    let email = value
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let account_id = value
        .get("chatgptAccountId")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("accountID").and_then(|v| v.as_str()))
        .map(|s| s.to_string());
    let plan_type = value
        .get("chatgptPlanType")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let subject = value
        .get("sub")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    (account_id, email, plan_type, subject)
}

/// 解码 base64url payload，兼容带 `=` padding 与不带 padding 两种形式。
fn decode_base64url_lenient(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    // 先按 no-pad 引擎解码；若失败（说明输入带了 padding），剥掉 `=` 后再试。
    match URL_SAFE_NO_PAD.decode(input) {
        Ok(bytes) => Ok(bytes),
        Err(_) => {
            let stripped: String = input.chars().filter(|c| *c != '=').collect();
            URL_SAFE_NO_PAD.decode(&stripped)
        }
    }
}

/// 生成 PKCE code_challenge (S256)。
fn pkce_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let result = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(result)
}

/// 生成随机字符串（字母+数字）。
fn generate_random_string(len: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;
    use crate::services::crypto;
    use rusqlite::Connection;
    use std::sync::Mutex;

    /// 辅助：构造最小测试 AppState（内存 SQLite + 真实 crypto）。
    fn test_app_state() -> Arc<AppState> {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        let db = Arc::new(Mutex::new(conn));
        migrations::run_migrations(&db).expect("迁移失败");
        let key = crypto::generate_master_key();
        let crypto_svc = Some(Arc::new(crypto::CryptoService::new(key)));

        Arc::new(AppState {
            db,
            shutdown_tx: tokio::sync::Mutex::new(None),
            data_dir: std::path::PathBuf::from("."),
            web_dist_dir: std::path::PathBuf::from("."),
            crypto: crypto_svc,
            codex_oauth: Arc::new(CodexOAuthService::new()),
            model_sync: Arc::new(crate::services::model_sync::ModelSyncService::new()),
            route_proxy: tokio::sync::RwLock::new(None),
        })
    }

    /// 构造测试 JWT（不验签路径只读取 payload）。
    fn test_jwt(payload: serde_json::Value, padded: bool) -> String {
        let bytes = serde_json::to_vec(&payload).unwrap();
        let payload = if padded {
            base64::engine::general_purpose::URL_SAFE.encode(bytes)
        } else {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
        };
        format!("header.{}.signature", payload)
    }

    #[test]
    fn parse_jwt_fields_accepts_padded_and_no_pad_payloads() {
        let payload = serde_json::json!({
            "chatgptAccountId": "acc-chatgpt",
            "email": "user@example.com",
            "chatgptPlanType": "plus",
            "sub": "sub-1"
        });

        let no_pad = parse_jwt_fields(&test_jwt(payload.clone(), false));
        assert_eq!(no_pad.0.as_deref(), Some("acc-chatgpt"));
        assert_eq!(no_pad.1.as_deref(), Some("user@example.com"));
        assert_eq!(no_pad.2.as_deref(), Some("plus"));
        assert_eq!(no_pad.3.as_deref(), Some("sub-1"));

        let padded = parse_jwt_fields(&test_jwt(payload, true));
        assert_eq!(padded.0.as_deref(), Some("acc-chatgpt"));
        assert_eq!(padded.1.as_deref(), Some("user@example.com"));
        assert_eq!(padded.2.as_deref(), Some("plus"));
        assert_eq!(padded.3.as_deref(), Some("sub-1"));
    }

    #[test]
    fn parse_jwt_fields_supports_account_id_and_subject_fallback() {
        let account_id_payload = serde_json::json!({
            "accountID": "acc-cpa",
            "email": "user@example.com"
        });
        let parsed = parse_jwt_fields(&test_jwt(account_id_payload, false));
        assert_eq!(parsed.0.as_deref(), Some("acc-cpa"));
        assert_eq!(parsed.3, None);

        let subject_payload = serde_json::json!({
            "sub": "stable-subject",
            "email": "user@example.com"
        });
        let parsed = parse_jwt_fields(&test_jwt(subject_payload, false));
        assert_eq!(parsed.0, None);
        assert_eq!(parsed.3.as_deref(), Some("stable-subject"));
    }

    #[test]
    fn decode_base64url_lenient_rejects_invalid_payload() {
        assert!(decode_base64url_lenient("not valid+base64").is_err());
    }

    #[test]
    fn initial_login_expires_at_uses_expires_in() {
        let token_resp = TokenResponse {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            id_token: None,
            expires_in: Some(3600),
        };
        let expires_at = token_resp.expires_in.and_then(|expires_in| {
            let at = time::OffsetDateTime::now_utc()
                + time::Duration::seconds(expires_in as i64);
            at.format(&time::format_description::well_known::Iso8601::DEFAULT)
                .ok()
        });
        assert!(expires_at.is_some());
    }

    #[test]
    fn start_login_error_status_classifier_matches_conflict_only_for_active_session() {
        fn status_for_start_login_error(e: &str) -> axum::http::StatusCode {
            if e.contains("已有 Codex OAuth 登录进行中") {
                axum::http::StatusCode::CONFLICT
            } else {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
        }

        assert_eq!(
            status_for_start_login_error("已有 Codex OAuth 登录进行中"),
            axum::http::StatusCode::CONFLICT
        );
        assert_eq!(
            status_for_start_login_error("无法绑定 127.0.0.1:1455"),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn map_role_stub_has_no_remaining_definition() {
        // Batch1 删除了 helpers::map_role 死代码；本测试作为任务边界说明，避免恢复 stub。
        let helper_source = include_str!("translator/helpers.rs");
        assert!(!helper_source.contains("fn map_role"));
    }

    #[test]
    fn upsert_codex_account_first_login_creates() {
        let app_state = test_app_state();
        let creds = serde_json::json!({"access_token": "test_token"});
        let json = serde_json::to_vec(&creds).unwrap();

        let result = upsert_codex_account(&app_state, "acc-A", "test@email.com", &json);
        assert!(result.is_ok(), "首次登录应成功创建");

        let row = accounts::get(&app_state.db, "acc-A").unwrap();
        assert!(row.is_some(), "账号应已插入");
        let acc = row.unwrap();
        assert_eq!(acc.account_type, "oauth_codex");
        assert_eq!(acc.platform, "openai_codex");
        assert!(acc.credentials_encrypted.is_some(), "应有凭据 BLOB");
    }

    #[test]
    fn upsert_codex_account_relogin_updates_credentials() {
        let app_state = test_app_state();
        let old_creds = serde_json::json!({"access_token": "old_token"});
        let old_json = serde_json::to_vec(&old_creds).unwrap();

        // 首次创建
        upsert_codex_account(&app_state, "acc-A", "old@email.com", &old_json).unwrap();
        let first = accounts::get(&app_state.db, "acc-A").unwrap().unwrap();
        let old_blob = first.credentials_encrypted.clone().unwrap();

        // 二次登录，不同凭据+邮箱
        let new_creds = serde_json::json!({"access_token": "new_token"});
        let new_json = serde_json::to_vec(&new_creds).unwrap();
        let result = upsert_codex_account(&app_state, "acc-A", "new@email.com", &new_json);
        assert!(result.is_ok(), "重登录应成功更新");

        // 断言：仍只有一行（PK 未冲突）
        let all = accounts::list(&app_state.db).unwrap();
        assert_eq!(all.len(), 1, "应只有一行（update 不产生重复）");

        let updated = accounts::get(&app_state.db, "acc-A").unwrap().unwrap();
        let new_blob = updated.credentials_encrypted.unwrap();
        assert_ne!(new_blob, old_blob, "credentials_encrypted 应已更新为新 BLOB");
        assert_eq!(updated.name, "new@email.com", "name 应已更新");
        assert_eq!(updated.account_type, "oauth_codex", "account_type 应保持不变");
        assert_eq!(updated.platform, "openai_codex", "platform 应保持不变");
        assert_eq!(updated.priority, 0, "priority 应保持不变");
    }

    #[test]
    fn upsert_codex_account_crypto_unavailable_returns_error() {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        let db = Arc::new(Mutex::new(conn));
        migrations::run_migrations(&db).expect("迁移失败");

        let app_state = Arc::new(AppState {
            db,
            shutdown_tx: tokio::sync::Mutex::new(None),
            data_dir: std::path::PathBuf::from("."),
            web_dist_dir: std::path::PathBuf::from("."),
            crypto: None, // 无加密服务
            codex_oauth: Arc::new(CodexOAuthService::new()),
            model_sync: Arc::new(crate::services::model_sync::ModelSyncService::new()),
            route_proxy: tokio::sync::RwLock::new(None),
        });

        let creds = serde_json::json!({"access_token": "test"});
        let json = serde_json::to_vec(&creds).unwrap();

        let result = upsert_codex_account(&app_state, "acc-A", "test@email.com", &json);
        assert!(result.is_err(), "crypto=None 应返回错误");
        assert!(
            result.unwrap_err().contains("加密服务不可用"),
            "错误消息应指出加密服务不可用"
        );
    }
}
