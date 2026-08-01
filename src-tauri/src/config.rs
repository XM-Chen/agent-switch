#[cfg(test)]
use serde::Deserialize;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::AppError;

/// 获取用户主目录，带测试隔离覆盖。
///
/// Windows 上必须使用系统用户目录，不能直接信任可能被 Git/MSYS 注入的 `HOME`。
/// 测试可通过 `AGENT_SWITCH_TEST_HOME` 隔离 Agent Switch 自有数据。
pub fn get_home_dir() -> PathBuf {
    if let Ok(home) = std::env::var("AGENT_SWITCH_TEST_HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    dirs::home_dir().unwrap_or_else(|| {
        log::warn!("无法获取用户主目录，回退到当前目录");
        PathBuf::from(".")
    })
}

/// 获取 Agent Switch 自有配置目录 (`~/.agent-switch`)。
pub fn get_app_config_dir() -> PathBuf {
    if let Some(custom) = crate::app_store::get_app_config_dir_override() {
        return custom;
    }

    let default_dir = get_home_dir().join(".agent-switch");

    // 兼容 v3.10.3 曾错误信任 HOME 的数据库位置。这里只探测 Agent Switch
    // 自有目录，绝不读取或探测任何客户端配置。
    #[cfg(windows)]
    {
        let default_db = default_dir.join("agent-switch.db");
        if !default_db.exists() {
            if let Ok(home_env) = std::env::var("HOME") {
                let trimmed = home_env.trim();
                if !trimmed.is_empty() {
                    let legacy_dir = PathBuf::from(trimmed).join(".agent-switch");
                    if legacy_dir.join("agent-switch.db").exists() {
                        log::info!(
                            "Detected v3.10.3 legacy database at {}, using it instead of {}",
                            legacy_dir.display(),
                            default_dir.display()
                        );
                        return legacy_dir;
                    }
                }
            }
        }
    }

    default_dir
}

/// 读取指定的 Agent Switch 数据文件。
///
/// 调用方负责确保路径属于应用自有数据边界；生产命令图不再导出通用读取入口。
#[cfg(test)]
pub(crate) fn read_json_file<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T, AppError> {
    if !path.exists() {
        return Err(AppError::Config(format!("文件不存在: {}", path.display())));
    }

    let content = fs::read_to_string(path).map_err(|error| AppError::io(path, error))?;
    serde_json::from_str(&content).map_err(|error| AppError::json(path, error))
}

/// 原子写入 Agent Switch 自有数据：先写临时文件，再替换目标。
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| AppError::io(parent, error))?;
    }

    let parent = path
        .parent()
        .ok_or_else(|| AppError::Config("无效的路径".to_string()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Config("无效的文件名".to_string()))?
        .to_string_lossy();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = parent.join(format!("{file_name}.tmp.{timestamp}"));

    {
        let mut file =
            fs::File::create(&temp_path).map_err(|error| AppError::io(&temp_path, error))?;
        file.write_all(data)
            .map_err(|error| AppError::io(&temp_path, error))?;
        file.flush()
            .map_err(|error| AppError::io(&temp_path, error))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(path) {
            let _ = fs::set_permissions(
                &temp_path,
                fs::Permissions::from_mode(metadata.permissions().mode()),
            );
        }
    }

    #[cfg(windows)]
    {
        if path.exists() {
            let _ = fs::remove_file(path);
        }
        fs::rename(&temp_path, path).map_err(|error| AppError::IoContext {
            context: format!(
                "原子替换失败: {} -> {}",
                temp_path.display(),
                path.display()
            ),
            source: error,
        })?;
    }

    #[cfg(not(windows))]
    {
        fs::rename(&temp_path, path).map_err(|error| AppError::IoContext {
            context: format!(
                "原子替换失败: {} -> {}",
                temp_path.display(),
                path.display()
            ),
            source: error,
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_agent_switch_file() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("gateway.json");

        atomic_write(&path, br#"{"version":1}"#).expect("initial write");
        atomic_write(&path, br#"{"version":2}"#).expect("replacement write");

        let value: serde_json::Value = read_json_file(&path).expect("read JSON");
        assert_eq!(value["version"], 2);
    }
}
