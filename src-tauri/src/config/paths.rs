use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Resolve the OS-specific application data directory.
///
/// Prefer the OS application data directory. During early initialization or in
/// constrained environments where the OS resolver is unavailable, fall back to a
/// stable user/temp location; never use the current working directory as data
/// root, because that makes the SQLite DB location depend on launch CWD.
pub fn app_data_dir() -> PathBuf {
    let fallback = dirs_or_fallback();
    fallback.join(super::APP_DIR_NAME)
}

fn dirs_or_fallback() -> PathBuf {
    dirs_or_fallback_with(
        dirs::data_dir(),
        |key| std::env::var_os(key),
        std::env::temp_dir(),
    )
}

fn dirs_or_fallback_with<F>(data_dir: Option<PathBuf>, get_env: F, temp_dir: PathBuf) -> PathBuf
where
    F: Fn(&str) -> Option<OsString>,
{
    if let Some(dir) = data_dir {
        return dir;
    }

    if let Some(appdata) = get_env("APPDATA") {
        return PathBuf::from(appdata);
    }
    if let Some(local_appdata) = get_env("LOCALAPPDATA") {
        return PathBuf::from(local_appdata);
    }
    if let Some(home) = get_env("HOME") {
        return PathBuf::from(home).join(".local").join("share");
    }
    if let Some(userprofile) = get_env("USERPROFILE") {
        return PathBuf::from(userprofile);
    }

    temp_dir.join("agent-switch-data")
}

/// Path to the SQLite database file within the app data directory.
pub fn db_path(data_dir: &Path) -> PathBuf {
    data_dir.join("agent-switch.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_never_returns_current_directory_when_env_missing() {
        let dir = dirs_or_fallback_with(None, |_| None, PathBuf::from("/tmp"));
        assert_eq!(dir, PathBuf::from("/tmp").join("agent-switch-data"));
        assert_ne!(dir, PathBuf::from("."));
    }

    #[test]
    fn home_fallback_uses_local_share_not_home_root() {
        let dir = dirs_or_fallback_with(
            None,
            |key| (key == "HOME").then(|| OsString::from("/home/user")),
            PathBuf::from("/tmp"),
        );
        assert_eq!(dir, PathBuf::from("/home/user/.local/share"));
    }

    #[test]
    fn data_dir_has_highest_priority() {
        let dir = dirs_or_fallback_with(
            Some(PathBuf::from("/data")),
            |_| Some(OsString::from("/ignored")),
            PathBuf::from("/tmp"),
        );
        assert_eq!(dir, PathBuf::from("/data"));
    }
}
