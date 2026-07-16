use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use crate::config::get_home_dir;

pub(crate) const CODEX_STATE_DB_FILENAME: &str = "state_5.sqlite";
const CODEX_SQLITE_HOME_ENV: &str = "CODEX_SQLITE_HOME";

pub(crate) fn codex_state_db_paths(config_dir: &Path, config_text: &str) -> Vec<PathBuf> {
    let mut paths = vec![config_dir.join(CODEX_STATE_DB_FILENAME)];
    let sqlite_home = sqlite_home_from_codex_config(config_text).or_else(sqlite_home_from_env);
    if let Some(path) = sqlite_home.map(|path| path.join(CODEX_STATE_DB_FILENAME)) {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

fn sqlite_home_from_codex_config(config_text: &str) -> Option<PathBuf> {
    let doc = config_text.parse::<DocumentMut>().ok()?;
    resolve_user_path(doc.get("sqlite_home")?.as_str()?.trim())
}

fn sqlite_home_from_env() -> Option<PathBuf> {
    resolve_user_path(std::env::var(CODEX_SQLITE_HOME_ENV).ok()?.trim())
}

fn resolve_user_path(raw: &str) -> Option<PathBuf> {
    if raw.is_empty() {
        return None;
    }
    if raw == "~" {
        return Some(get_home_dir());
    }
    raw.strip_prefix("~/")
        .or_else(|| raw.strip_prefix("~\\"))
        .map(|rest| get_home_dir().join(rest))
        .or_else(|| Some(PathBuf::from(raw)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn includes_config_sqlite_home() {
        let temp = tempdir().expect("tempdir");
        let sqlite_home = temp.path().join("sqlite-home");
        let config_text = format!("sqlite_home = '{}'\n", sqlite_home.display());
        assert_eq!(
            codex_state_db_paths(temp.path(), &config_text),
            vec![
                temp.path().join(CODEX_STATE_DB_FILENAME),
                sqlite_home.join(CODEX_STATE_DB_FILENAME),
            ]
        );
    }
}
