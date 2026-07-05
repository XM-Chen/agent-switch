use std::path::{Path, PathBuf};

/// Resolve the OS-specific application data directory.
///
/// Prefer the OS application data directory; fall back to a stable temp location.
/// Never use the current working directory as data root — that makes the SQLite
/// DB location depend on launch CWD.
pub fn app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("agent-switch-data"))
        .join(super::APP_DIR_NAME)
}

/// Path to the SQLite database file within the app data directory.
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("agent-switch.db")
}
