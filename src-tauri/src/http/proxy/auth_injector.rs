/// 认证注入器。
///
/// 根据端点的认证模式为上游请求注入认证 Header。
/// 支持 API Key 模式（解密 → 注入 Bearer / x-api-key）和 OAuth Codex 模式（通过 oauth_refresh 获取 token）。
/// 注入前清除请求中原有的 Authorization 和 x-api-key 头。
use std::path::Path;
use std::sync::Mutex;

use axum::http::{HeaderMap, HeaderValue};
use rusqlite::Connection;

use crate::db::dao::endpoints::EndpointRow;
use crate::http::proxy::error::{ProxyError, ProxyErrorKind};
use crate::services::codex_oauth::CodexOAuthService;
use crate::services::crypto::CryptoService;

/// 认证注入结果。
#[derive(Debug, Clone)]
pub struct AuthResult {
    /// 使用的认证模式（"apikey" / "oauth_codex"）。
    pub auth_type: String,
    /// 注入时使用的端点 ID。
    pub endpoint_id: String,
}

/// 为端点注入认证 Header。
///
/// - `endpoint`：目标端点。
/// - `crypto`：加密服务（API Key 解密需要）。
/// - `codex_oauth`：CodeX OAuth 服务（OAuth 模式需要）。
/// - `data_dir`：数据目录路径。
/// - `db`：数据库连接（OAuth refresh 时需要）。
/// - `headers`：请求 Header，会被操作（清除原有认证头 + 注入新认证头）。
pub async fn inject_auth(
    endpoint: &EndpointRow,
    crypto: Option<&CryptoService>,
    codex_oauth: &CodexOAuthService,
    data_dir: &Path,
    db: &Mutex<Connection>,
    headers: &mut HeaderMap,
) -> Result<AuthResult, ProxyError> {
    // 清除原有的认证头
    headers.remove(axum::http::header::AUTHORIZATION);
    headers.remove("x-api-key");

    match endpoint.auth_mode.as_str() {
        "apikey" => inject_apikey(endpoint, crypto, headers),
        "oauth_codex" => inject_oauth(endpoint, crypto, codex_oauth, data_dir, db, headers).await,
        other => Err(ProxyError::new(
            ProxyErrorKind::LocalError,
            format!("不支持的认证模式: {}", other),
        )),
    }
}

/// API Key 认证注入。
fn inject_apikey(
    endpoint: &EndpointRow,
    crypto: Option<&CryptoService>,
    headers: &mut HeaderMap,
) -> Result<AuthResult, ProxyError> {
    let crypto = crypto.ok_or_else(|| {
        ProxyError::new(
            ProxyErrorKind::LocalError,
            "加密服务不可用，无法解密 API Key",
        )
    })?;

    let encrypted = endpoint.api_key_encrypted.as_ref().ok_or_else(|| {
        ProxyError::new(
            ProxyErrorKind::LocalError,
            format!("端点 '{}' 缺少 api_key_encrypted", endpoint.name),
        )
    })?;

    let plaintext = crypto
        .decrypt(encrypted, endpoint.id.as_bytes())
        .map_err(|e| {
            ProxyError::new(
                ProxyErrorKind::LocalError,
                format!("解密 API Key 失败: {}", e),
            )
        })?;

    let json: serde_json::Value = serde_json::from_slice(&plaintext).map_err(|e| {
        ProxyError::new(
            ProxyErrorKind::LocalError,
            format!("解析 API Key JSON 失败: {}", e),
        )
    })?;

    let api_key = json
        .get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ProxyError::new(
                ProxyErrorKind::LocalError,
                "API Key 格式无效：缺少 api_key 字段".to_string(),
            )
        })?;

    // 根据协议类型选择 Header
    if endpoint.protocol_type == crate::http::proxy::constants::PROTOCOL_ANTHROPIC {
        // Anthropic 使用 x-api-key
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(api_key).map_err(|e| {
                ProxyError::new(ProxyErrorKind::LocalError, format!("无效 Header 值: {}", e))
            })?,
        );
    } else {
        // OpenAI 系列使用 Bearer token
        let bearer = format!("Bearer {}", api_key);
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&bearer).map_err(|e| {
                ProxyError::new(ProxyErrorKind::LocalError, format!("无效 Header 值: {}", e))
            })?,
        );
    }

    Ok(AuthResult {
        auth_type: "apikey".to_string(),
        endpoint_id: endpoint.id.clone(),
    })
}

/// OAuth Codex 认证注入。
async fn inject_oauth(
    endpoint: &EndpointRow,
    crypto: Option<&CryptoService>,
    _codex_oauth: &CodexOAuthService,
    _data_dir: &Path,
    db: &Mutex<Connection>,
    headers: &mut HeaderMap,
) -> Result<AuthResult, ProxyError> {
    let crypto = crypto.ok_or_else(|| {
        ProxyError::new(
            ProxyErrorKind::LocalError,
            "加密服务不可用，无法处理 OAuth 凭据",
        )
    })?;

    let token = crate::http::proxy::oauth_refresh::ensure_valid_token(endpoint, crypto, db).await?;

    let bearer = format!("Bearer {}", token);
    headers.insert(
        axum::http::header::AUTHORIZATION,
        HeaderValue::from_str(&bearer).map_err(|e| {
            ProxyError::new(ProxyErrorKind::LocalError, format!("无效 Header 值: {}", e))
        })?,
    );

    Ok(AuthResult {
        auth_type: "oauth_codex".to_string(),
        endpoint_id: endpoint.id.clone(),
    })
}
