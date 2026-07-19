//! C2b 四模块的 `SnapshotModuleAdapter` 实现（Claude Desktop / OpenCode / OpenClaw / Hermes）。
//!
//! 复用 C1 的版本化 manifest 与 `capture_snapshot_once` / `restore_snapshot_and_release`
//! / `abandon_snapshot_ownership`；本文件只提供各模块的 **目标适配器**（capture 目标、
//! 事务式 restore），不改 trait 或 manifest 格式（冻结）。
//!
//! 快照策略（见 C2b design §6 / matrix §5）：
//! - **Claude Desktop**：多文件 `file_bytes`，稳定 target id = `normal_config` / `threep_config`
//!   / `profile` / `meta`；恢复走 `restore_snapshots` 语义（写入前 snapshot 现状用于补偿回滚）。
//! - **OpenCode**：单 `file_bytes` = `opencode.json`（凭据所在）；**从不触碰 `opencode.db`**。
//! - **OpenClaw**：单 `file_bytes` = `openclaw.json`（JSON5，逐字节保注释/格式）。
//! - **Hermes**：单 `file_bytes` = `config.yaml`（逐字节，避免 section replacement 语义漂移）。
//!
//! 四模块均无 legacy（无版本）快照：`decode_stored_snapshot` 只对 claude/codex/gemini 放行
//! 旧格式，故 `restore_legacy` 对四模块不可达；仍显式返回错误以防误接入。

#![allow(dead_code)]

use crate::app_config::AppType;
use crate::config::{atomic_write, delete_file};
use crate::error::AppError;
use crate::proxy::snapshot::{
    LegacySnapshot, SnapshotManifest, SnapshotModuleAdapter, SnapshotTarget,
};
use std::path::{Path, PathBuf};

/// 读取单个文件为快照目标：存在则逐字节记录，不存在则记 `existed=false`。
fn capture_file_target(id: &'static str, path: &Path) -> Result<SnapshotTarget, String> {
    if path.exists() {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("读取快照目标 {} 失败: {e}", path.display()))?;
        Ok(SnapshotTarget::file_bytes(id, Some(&bytes)))
    } else {
        Ok(SnapshotTarget::file_bytes(id, None))
    }
}

fn validate_manifest_targets(
    manifest: &SnapshotManifest,
    app_type: &AppType,
    expected_ids: &[&str],
) -> Result<(), String> {
    manifest.validate()?;
    if manifest.app_type != app_type.as_str() {
        return Err(format!(
            "快照 app_type 不匹配：期望 {}，实际 {}",
            app_type.as_str(),
            manifest.app_type
        ));
    }
    if manifest.targets.len() != expected_ids.len()
        || manifest
            .targets
            .iter()
            .any(|target| !expected_ids.contains(&target.id()))
    {
        return Err(format!("{} 快照包含不支持的目标集合", app_type.as_str()));
    }
    for id in expected_ids {
        let target = manifest
            .targets
            .iter()
            .find(|target| target.id() == *id)
            .ok_or_else(|| format!("快照缺少目标 {id}"))?;
        target.file_payload()?;
    }
    Ok(())
}

/// 按快照目标恢复单个文件：存在则逐字节写回，不存在则删除 AGS 创建的目标。
fn restore_file_target(path: &Path, target: &SnapshotTarget) -> Result<(), AppError> {
    match target.file_payload().map_err(AppError::Message)? {
        Some(bytes) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
            }
            atomic_write(path, &bytes)
        }
        None => delete_file(path),
    }
}

/// 单文件模块（OpenCode / OpenClaw / Hermes）的通用 file_bytes 适配器。
///
/// 恢复是原子的单文件操作，无需多目标补偿；失败时 manifest 与接管状态由
/// `restore_snapshot_and_release` 保留（它只有在本方法返回 Ok 后才清所有权）。
struct SingleFileAdapter {
    app: AppType,
    target_id: &'static str,
    path: PathBuf,
}

impl SnapshotModuleAdapter for SingleFileAdapter {
    fn app_type(&self) -> AppType {
        self.app.clone()
    }

    fn capture_targets(&self) -> Result<Vec<SnapshotTarget>, String> {
        Ok(vec![capture_file_target(self.target_id, &self.path)?])
    }

    fn restore_manifest_transactional(&self, manifest: &SnapshotManifest) -> Result<(), String> {
        // 在任何写入前完整校验：目标集合、app_type、payload 合法。否则忽略未知目标后
        // 误清所有权，或恢复到半写状态。
        validate_manifest_targets(manifest, &self.app, &[self.target_id])?;
        let target = manifest
            .targets
            .iter()
            .find(|t| t.id() == self.target_id)
            .ok_or_else(|| format!("{} 快照缺少目标 {}", self.app.as_str(), self.target_id))?;
        restore_file_target(&self.path, target)
            .map_err(|e| format!("恢复 {} 失败: {e}", self.path.display()))
    }

    fn restore_legacy(&self, _legacy: &LegacySnapshot) -> Result<(), String> {
        Err(format!(
            "{} 不支持旧版无版本快照（应为版本化 manifest）",
            self.app.as_str()
        ))
    }
}

/// Claude Desktop 多文件适配器：四个稳定 target id，事务式恢复。
///
/// 恢复前先 snapshot 全部目标当前字节，任一写入失败则用该 snapshot 补偿回滚已写目标，
/// 保证 R10「多文件事务回滚」；补偿后返回原始错误，manifest 与接管状态被保留。
struct ClaudeDesktopSnapshotAdapter {
    targets: Vec<(&'static str, PathBuf)>,
}

impl ClaudeDesktopSnapshotAdapter {
    fn resolve() -> Result<Self, String> {
        let targets = crate::claude_desktop_config::snapshot_target_paths()
            .map_err(|e| format!("解析 Claude Desktop 快照目标失败: {e}"))?;
        Ok(Self { targets })
    }
}

impl SnapshotModuleAdapter for ClaudeDesktopSnapshotAdapter {
    fn app_type(&self) -> AppType {
        AppType::ClaudeDesktop
    }

    fn capture_targets(&self) -> Result<Vec<SnapshotTarget>, String> {
        self.targets
            .iter()
            .map(|(id, path)| capture_file_target(id, path))
            .collect()
    }

    fn restore_manifest_transactional(&self, manifest: &SnapshotManifest) -> Result<(), String> {
        // 1. 在任何写入前完整校验四个稳定 target id 与 file_bytes payload。
        validate_manifest_targets(
            manifest,
            &AppType::ClaudeDesktop,
            &["normal_config", "threep_config", "profile", "meta"],
        )?;

        // 2. 建立恢复计划，保证后续只访问已校验目标。
        let restore_plan = self
            .targets
            .iter()
            .map(|(id, path)| {
                let target = manifest
                    .targets
                    .iter()
                    .find(|target| target.id() == *id)
                    .expect("目标集合已完成预检");
                (id, path, target)
            })
            .collect::<Vec<_>>();

        // 3. 先 snapshot 当前字节，作为补偿回滚来源。
        let rollback: Vec<(PathBuf, Option<Vec<u8>>)> = self
            .targets
            .iter()
            .map(|(_, path)| {
                let current = if path.exists() {
                    Some(
                        std::fs::read(path)
                            .map_err(|e| format!("读取回滚快照 {} 失败: {e}", path.display()))?,
                    )
                } else {
                    None
                };
                Ok((path.clone(), current))
            })
            .collect::<Result<_, String>>()?;

        // 4. 逐目标恢复；任一失败用 rollback 补偿已写目标后返回原始错误。
        for (id, path, target) in restore_plan {
            if let Err(err) = restore_file_target(path, target) {
                let restore_err = format!("恢复 Claude Desktop 目标 {id} 失败: {err}");
                if let Err(compensate_err) = compensate_rollback(&rollback) {
                    return Err(format!("{restore_err}；补偿回滚亦失败: {compensate_err}"));
                }
                return Err(restore_err);
            }
        }
        Ok(())
    }

    fn restore_legacy(&self, _legacy: &LegacySnapshot) -> Result<(), String> {
        Err("Claude Desktop 不支持旧版无版本快照（应为版本化 manifest）".to_string())
    }
}

/// 用回滚 snapshot 把目标恢复到 restore 开始前的字节状态。
fn compensate_rollback(rollback: &[(PathBuf, Option<Vec<u8>>)]) -> Result<(), String> {
    for (path, content) in rollback {
        let result = match content {
            Some(bytes) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| AppError::io(parent, e))
                        .and_then(|_| atomic_write(path, bytes))
                } else {
                    atomic_write(path, bytes)
                }
            }
            None => delete_file(path),
        };
        result.map_err(|e| format!("补偿回滚 {} 失败: {e}", path.display()))?;
    }
    Ok(())
}

/// 为四模块解析对应的 snapshot adapter；三现有模块（Claude/Codex/Gemini）由 C2a 拥有，
/// 不在此处理，返回 None 以便调用方回退到 legacy JSON 备份路径。
pub fn snapshot_adapter_for(
    app_type: &AppType,
) -> Result<Option<Box<dyn SnapshotModuleAdapter>>, String> {
    Ok(match app_type {
        AppType::ClaudeDesktop => Some(Box::new(ClaudeDesktopSnapshotAdapter::resolve()?)),
        AppType::OpenCode => Some(Box::new(SingleFileAdapter {
            app: AppType::OpenCode,
            target_id: "opencode.json",
            path: crate::opencode_config::get_opencode_config_path(),
        })),
        AppType::OpenClaw => Some(Box::new(SingleFileAdapter {
            app: AppType::OpenClaw,
            target_id: "openclaw.json",
            path: crate::openclaw_config::get_openclaw_config_path(),
        })),
        AppType::Hermes => Some(Box::new(SingleFileAdapter {
            app: AppType::Hermes,
            target_id: "config.yaml",
            path: crate::hermes_config::get_hermes_config_path(),
        })),
        // 三现有模块归 C2a；C2b 不接管它们的 snapshot adapter。
        AppType::Claude | AppType::Codex | AppType::Gemini => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("ags-c2b-snap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_bytes(path: &PathBuf, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(bytes).unwrap();
    }

    #[test]
    fn single_file_captures_and_restores_non_utf8_bytes() {
        let dir = temp_dir();
        let path = dir.join("openclaw.json");
        // 非 UTF-8 字节，验证逐字节 round-trip（file_bytes 用 base64）。
        let original = [0u8, 159, 146, 150, b'{', b'}'];
        write_bytes(&path, &original);

        let adapter = SingleFileAdapter {
            app: AppType::OpenClaw,
            target_id: "openclaw.json",
            path: path.clone(),
        };
        let manifest =
            SnapshotManifest::new(&AppType::OpenClaw, adapter.capture_targets().unwrap()).unwrap();

        // 接管期间文件被改写。
        write_bytes(&path, b"proxy-managed");
        adapter.restore_manifest_transactional(&manifest).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), original.to_vec());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn single_file_missing_target_is_deleted_on_restore() {
        let dir = temp_dir();
        let path = dir.join("config.yaml");
        // capture 时文件不存在（existed=false）。
        let adapter = SingleFileAdapter {
            app: AppType::Hermes,
            target_id: "config.yaml",
            path: path.clone(),
        };
        let manifest =
            SnapshotManifest::new(&AppType::Hermes, adapter.capture_targets().unwrap()).unwrap();

        // 接管创建了文件；恢复必须删除 AGS 创建的目标。
        write_bytes(&path, b"created-by-ags");
        adapter.restore_manifest_transactional(&manifest).unwrap();

        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn claude_desktop_multi_target_restores_all_and_deletes_created() {
        let dir = temp_dir();
        let normal = dir.join("normal.json");
        let threep = dir.join("threep.json");
        let profile = dir.join("profile.json");
        let meta = dir.join("meta.json");

        // 初始：normal/threep 存在，profile/meta 不存在。
        write_bytes(&normal, b"normal-original");
        write_bytes(&threep, b"threep-original");

        let adapter = ClaudeDesktopSnapshotAdapter {
            targets: vec![
                ("normal_config", normal.clone()),
                ("threep_config", threep.clone()),
                ("profile", profile.clone()),
                ("meta", meta.clone()),
            ],
        };
        let manifest =
            SnapshotManifest::new(&AppType::ClaudeDesktop, adapter.capture_targets().unwrap())
                .unwrap();

        // 接管：改写 normal/threep，创建 profile/meta。
        write_bytes(&normal, b"gateway");
        write_bytes(&threep, b"gateway");
        write_bytes(&profile, b"gateway-profile");
        write_bytes(&meta, b"gateway-meta");

        adapter.restore_manifest_transactional(&manifest).unwrap();

        assert_eq!(std::fs::read(&normal).unwrap(), b"normal-original");
        assert_eq!(std::fs::read(&threep).unwrap(), b"threep-original");
        assert!(!profile.exists());
        assert!(!meta.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn claude_desktop_invalid_manifest_fails_before_any_target_is_written() {
        let dir = temp_dir();
        let normal = dir.join("normal.json");
        let threep = dir.join("threep.json");
        let profile = dir.join("profile.json");
        let meta = dir.join("meta.json");
        for path in [&normal, &threep, &profile, &meta] {
            write_bytes(path, b"managed");
        }

        let adapter = ClaudeDesktopSnapshotAdapter {
            targets: vec![
                ("normal_config", normal.clone()),
                ("threep_config", threep.clone()),
                ("profile", profile.clone()),
                ("meta", meta.clone()),
            ],
        };
        // 缺 meta：若未预检，前三个目标可能先被恢复后才报错。
        let malformed = SnapshotManifest::new(
            &AppType::ClaudeDesktop,
            vec![
                SnapshotTarget::file_bytes("normal_config", Some(b"original")),
                SnapshotTarget::file_bytes("threep_config", Some(b"original")),
                SnapshotTarget::file_bytes("profile", None),
            ],
        )
        .unwrap();

        assert!(adapter.restore_manifest_transactional(&malformed).is_err());
        for path in [&normal, &threep, &profile, &meta] {
            assert_eq!(std::fs::read(path).unwrap(), b"managed");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn legacy_restore_is_rejected_for_new_modules() {
        let adapter = SingleFileAdapter {
            app: AppType::OpenCode,
            target_id: "opencode.json",
            path: PathBuf::from("unused"),
        };
        let legacy = LegacySnapshot {
            app_type: "opencode".to_string(),
            original_config: "{}".to_string(),
        };
        assert!(adapter.restore_legacy(&legacy).is_err());
    }
}
