/// Data Access Object module.
///
/// DAO 只做 SQL，不做加密。加密在 `services/crypto.rs` 层完成；
/// DAO 原样存取 `credentials_encrypted` / `api_key_encrypted` BLOB。
pub mod accounts;
pub mod endpoints;
