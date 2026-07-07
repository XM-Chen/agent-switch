pub mod accounts;
pub mod aliases;
pub mod auth;
pub mod common_config;
pub mod endpoints;
pub mod logs;
pub mod models;
pub mod providers;
pub mod routes;
pub mod settings;
pub mod tests;
pub mod tools;
pub mod v1_models;

use axum::http::StatusCode;

use crate::app_state::AppState;

/// 加密 API Key 凭据，返回 BLOB。
///
/// `api_key=None` → 返回 `None`（不写入凭据）。`api_key=Some(k)` → JSON `{"api_key": k}` 加密为 BLOB。
/// crypto 不可用时返回 503，序列化/加密失败返回 500。
pub(crate) fn encrypt_api_key(
    state: &AppState,
    id: &str,
    api_key: Option<&str>,
) -> Result<Option<Vec<u8>>, (StatusCode, String)> {
    let key = match api_key {
        None => None,
        Some(k) => {
            let crypto = state.crypto.as_ref().ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "系统凭据管理器不可用，无法保存凭据".to_string(),
            ))?;
            let json = serde_json::json!({ "api_key": k });
            let json_bytes = serde_json::to_vec(&json).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("序列化凭据失败: {}", e),
                )
            })?;
            Some(crypto.encrypt(&json_bytes, id.as_bytes()).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("加密失败: {}", e),
                )
            })?)
        }
    };
    Ok(key)
}
