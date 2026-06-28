/// Data Access Object module.
///
/// DAO 只做 SQL，不做加密。加密在 `services/crypto.rs` 层完成；
/// DAO 原样存取 `credentials_encrypted` / `api_key_encrypted` BLOB。
pub mod accounts;
pub mod app_metadata;
pub mod endpoint_models;
pub mod endpoints;
pub mod model_aliases;
pub mod model_locks;
pub mod request_logs;
pub mod route_settings;
pub mod tool_takeover;
