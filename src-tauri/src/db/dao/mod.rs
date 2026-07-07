/// Data Access Object module.
///
/// DAO 只做 SQL，不做加密。加密在 `services/crypto.rs` 层完成；
/// DAO 原样存取 `credentials_encrypted` / `api_key_encrypted` BLOB。
pub mod accounts;
pub mod app_metadata;
pub mod endpoint_models;
pub mod endpoints;
pub mod mcp_servers;
pub mod model_aliases;
pub mod model_locks;
pub mod providers;
pub mod request_logs;
pub mod route_settings;
pub mod tool_takeover;

/// 共享 ISO8601 时间戳生成器（UTC）。供各 DAO 与 service 复用，避免重复定义。
pub(crate) fn now_iso() -> Result<String, String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}
