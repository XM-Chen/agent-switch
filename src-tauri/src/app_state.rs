use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

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
}
