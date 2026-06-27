use std::path::{Path, PathBuf};

/// Resolve the OS-specific application data directory.
///
/// Returns `~/.agent-switch` as a fallback when the Tauri resolver
/// is not available (e.g. during early initialization).
pub fn app_data_dir() -> PathBuf {
    // Prefer the Tauri path resolver (called after app handle is available).
    // This function provides a compile-safe default for early use.
    let fallback = dirs_or_fallback();
    fallback.join(super::APP_DIR_NAME)
}

fn dirs_or_fallback() -> PathBuf {
    // $HOME/.local/share on Linux, ~/Library/Application Support on macOS,
    // %APPDATA% on Windows (via CSIDL_APPDATA / FOLDERID_RoamingAppData).
    dirs::data_dir().unwrap_or_else(|| {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
    })
}

/// Path to the SQLite database file within the app data directory.
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("agent-switch.db")
}
