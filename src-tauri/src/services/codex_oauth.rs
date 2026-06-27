use axum::{routing::get, Router};
use base64::Engine;
use rand::Rng;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::app_state::AppState;
use crate::db::dao::accounts;

/// Codex OAuth 元数据，参考 9router / cpa。
const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
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
            let (account_id, email, plan_type) = token_resp
                .id_token
                .as_ref()
                .map(|jwt| parse_jwt_fields(jwt))
                .unwrap_or((None, None, None));

            let credentials = CodexCredentials {
                access_token: token_resp.access_token,
                refresh_token: token_resp.refresh_token,
                id_token: token_resp.id_token,
                expires_at: None,
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

            let account_id_str = account_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            match app_state.crypto.as_ref() {
                Some(crypto) => {
                    let encrypted = match crypto.encrypt(&json, account_id_str.as_bytes()) {
                        Ok(e) => e,
                        Err(e) => {
                            cleanup_session(&app_state, &callback_shutdown).await;
                            return (
                                StatusCode::INTERNAL_SERVER_ERROR,
                                Json(serde_json::json!({
                                    "error": "encrypt_failed",
                                    "message": e
                                })),
                            );
                        }
                    };

                    let new_account = accounts::NewAccount {
                        id: account_id_str.clone(),
                        name: email.clone().unwrap_or_else(|| "Codex 账号".to_string()),
                        account_type: "oauth_codex".to_string(),
                        platform: "openai_codex".to_string(),
                        credentials_encrypted: Some(encrypted),
                        extra_json: None,
                        priority: 0,
                    };
                    if let Err(e) = accounts::create(&app_state.db, new_account) {
                        cleanup_session(&app_state, &callback_shutdown).await;
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(serde_json::json!({
                                "error": "db_save_failed",
                                "message": e
                            })),
                        );
                    }
                }
                None => {
                    cleanup_session(&app_state, &callback_shutdown).await;
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(serde_json::json!({
                            "error": "crypto_unavailable",
                            "message": "加密服务不可用，无法保存凭据"
                        })),
                    );
                }
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

/// Token 交换响应。
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    #[allow(dead_code)]
    expires_in: Option<u64>,
}

/// 用授权码交换 token。
async fn exchange_code_for_token(code: &str, code_verifier: &str) -> Result<TokenResponse, String> {
    let client = Client::new();
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
fn parse_jwt_fields(jwt: &str) -> (Option<String>, Option<String>, Option<String>) {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() < 2 {
        return (None, None, None);
    }
    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(p) => p,
        Err(_) => return (None, None, None),
    };
    let value: serde_json::Value = match serde_json::from_slice(&payload) {
        Ok(v) => v,
        Err(_) => return (None, None, None),
    };

    // 9router 提取 email / chatgptAccountId / chatgptPlanType；
    // cpa 提取 accountID / email。综合两者。
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

    (account_id, email, plan_type)
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
