use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

use crate::services::codex_oauth::CodexOAuthService;
use crate::services::crypto::CryptoService;
use crate::services::model_sync::ModelSyncService;

/// Shared application state accessible across Tauri commands, HTTP handlers, and services.
pub struct AppState {
    /// Thread-safe database connection
    pub db: Arc<Mutex<Connection>>,
    /// Sender for graceful HTTP server shutdown
    pub shutdown_tx: tokio::sync::Mutex<Option<oneshot::Sender<()>>>,
    /// Application data directory path
    pub data_dir: PathBuf,
    /// Built Web UI assets directory served at `/` by Axum.
    pub web_dist_dir: PathBuf,
    /// 凭据加密服务（Keychain 不可用时为 None）。
    pub crypto: Option<Arc<CryptoService>>,
    /// Codex OAuth 登录管理器。
    pub codex_oauth: Arc<CodexOAuthService>,
    /// 模型刷新服务。
    pub model_sync: Arc<ModelSyncService>,
}
