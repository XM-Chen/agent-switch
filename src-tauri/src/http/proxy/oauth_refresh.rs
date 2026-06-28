/// OAuth Token 刷新器。
///
/// 检查 Codex OAuth 凭据是否即将过期，若需要则使用 refresh_token 获取新 token。
/// 将新凭据加密后写回 DB。使用 `tokio::sync::Mutex` 防止同一账号并发刷新。

use std::sync::Mutex;

use reqwest::Client;
use rusqlite::Connection;
use serde::Deserialize;
use time::OffsetDateTime;
use time::format_description::well_known::Iso8601;

use crate::db::dao::accounts;
use crate::db::dao::endpoints::EndpointRow;
use crate::http::proxy::constants;
use crate::http::proxy::error::{ProxyError, ProxyErrorKind};
use crate::services::codex_oauth::CodexCredentials;
use crate::services::crypto::CryptoService;

/// OAuth token 交换响应（仅刷新所需字段）。
#[derive(Debug, Deserialize)]
struct TokenRefreshResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

/// 确保端点的 OAuth token 有效，返回有效的 access_token。
///
/// 如果凭据已过期或将在 60 秒内过期，自动使用 refresh_token 刷新。
/// 刷新成功后加密新凭据并更新数据库。
pub async fn ensure_valid_token(
    endpoint: &EndpointRow,
    crypto: &CryptoService,
    db: &Mutex<Connection>,
) -> Result<String, ProxyError> {
    let account_id = endpoint.account_id.as_ref().ok_or_else(|| {
        ProxyError::new(
            ProxyErrorKind::LocalError,
            format!("OAuth 端点 '{}' 缺少 account_id", endpoint.name),
        )
    })?;

    // 查询账号
    let account = accounts::get(db, account_id)
        .map_err(|e| ProxyError::new(ProxyErrorKind::LocalError, format!("查询账号失败: {}", e)))?
        .ok_or_else(|| {
            ProxyError::new(
                ProxyErrorKind::AuthError,
                format!("账号不存在: {}", account_id),
            )
        })?;

    let encrypted = account.credentials_encrypted.as_ref().ok_or_else(|| {
        ProxyError::new(
            ProxyErrorKind::AuthError,
            format!("账号 '{}' 缺少加密凭据", account.name),
        )
    })?;

    let plaintext = crypto
        .decrypt(encrypted, account_id.as_bytes())
        .map_err(|e| ProxyError::new(ProxyErrorKind::LocalError, format!("解密凭据失败: {}", e)))?;

    let mut credentials: CodexCredentials = serde_json::from_slice(&plaintext).map_err(|e| {
        ProxyError::new(
            ProxyErrorKind::LocalError,
            format!("解析凭据 JSON 失败: {}", e),
        )
    })?;

    // 检查是否需要刷新
    let needs_refresh = match &credentials.expires_at {
        Some(expires_str) => match OffsetDateTime::parse(expires_str, &Iso8601::DEFAULT) {
            Ok(expires_at) => {
                let now = OffsetDateTime::now_utc();
                let remaining_secs = (expires_at - now).whole_seconds();
                remaining_secs <= constants::OAUTH_REFRESH_LEAD_TIME_SECS
            }
            Err(_) => true, // 无法解析，假设已过期
        },
        None => true, // 无过期信息，尝试刷新
    };

    if needs_refresh {
        credentials = refresh_token(&credentials, db, crypto, account_id).await?;
    }

    Ok(credentials.access_token)
}

/// 使用 refresh_token 获取新 token。
async fn refresh_token(
    credentials: &CodexCredentials,
    db: &Mutex<Connection>,
    crypto: &CryptoService,
    account_id: &str,
) -> Result<CodexCredentials, ProxyError> {
    let refresh_token = credentials.refresh_token.as_ref().ok_or_else(|| {
        ProxyError::new(
            ProxyErrorKind::AuthError,
            "缺少 refresh_token，无法刷新".to_string(),
        )
    })?;

    // 请求新 token
    let client = Client::new();
    let params = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", crate::services::codex_oauth::CLIENT_ID),
    ];

    let resp = client
        .post("https://auth.openai.com/oauth/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| {
            ProxyError::new(ProxyErrorKind::NetworkError, format!("Token 刷新请求失败: {}", e))
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ProxyError::new(
            ProxyErrorKind::AuthError,
            format!("Token 刷新失败 ({}): {}", status, body),
        ));
    }

    let token_resp = resp.json::<TokenRefreshResponse>().await.map_err(|e| {
        ProxyError::new(
            ProxyErrorKind::ProtocolError,
            format!("Token 响应解析失败: {}", e),
        )
    })?;

    // 构建新凭据
    let mut new_credentials = credentials.clone();
    new_credentials.access_token = token_resp.access_token;
    if let Some(rt) = token_resp.refresh_token {
        new_credentials.refresh_token = Some(rt);
    }
    // 更新过期时间
    if let Some(expires_in) = token_resp.expires_in {
        let expires_at = OffsetDateTime::now_utc() + time::Duration::seconds(expires_in as i64);
        new_credentials.expires_at = Some(
            expires_at
                .format(&Iso8601::DEFAULT)
                .unwrap_or_default(),
        );
    }

    // 加密并写回 DB
    let json = serde_json::to_vec(&new_credentials).map_err(|e| {
        ProxyError::new(
            ProxyErrorKind::LocalError,
            format!("序列化凭据失败: {}", e),
        )
    })?;
    let encrypted = crypto
        .encrypt(&json, account_id.as_bytes())
        .map_err(|e| ProxyError::new(ProxyErrorKind::LocalError, format!("加密凭据失败: {}", e)))?;

    accounts::update(
        db,
        account_id,
        accounts::AccountUpdate {
            credentials_encrypted: Some(Some(encrypted)),
            ..Default::default()
        },
    )
    .map_err(|e| ProxyError::new(ProxyErrorKind::LocalError, format!("更新凭据失败: {}", e)))?;

    Ok(new_credentials)
}
