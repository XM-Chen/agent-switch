//! Skills 卸载、备份与恢复（cc-skills 阶段 C）。
//!
//! - `uninstall`：卸载前强制备份 SSOT 目录 + DB 行快照，随后删除 live 投影、SSOT 与 DB 记录。
//! - `list_backups`：列出某 skill 或全部备份。
//! - `restore`：从备份恢复 SSOT 内容与 DB 行，并按恢复后的启用状态重新投影。
//!
//! 备份根：`app_data/skills_backups/<directory>/<timestamp>/`，内含 `skill/`（SSOT 副本）
//! 与 `skill.json`（DB 行快照）。所有删除都限定在 SSOT / live skills / 备份根内。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;
use serde::Serialize;

use crate::db::dao::skills::{self, NewSkill, SkillRow};

use super::{
    copy_dir_recursive_pub, ensure_child_path, remove_dir_if_under, ssot_root, sync_all, SyncReport,
};

/// 备份根目录。
fn backups_root(data_dir: &Path) -> PathBuf {
    data_dir.join("skills_backups")
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupEntry {
    pub directory: String,
    pub timestamp: String,
    pub path: String,
    pub has_snapshot: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallReport {
    pub id: String,
    pub directory: String,
    pub backup: BackupEntry,
    pub sync: Vec<SyncReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreReport {
    pub directory: String,
    pub restored_from: String,
    pub sync: Vec<SyncReport>,
}

/// DB 行快照，用于备份/恢复时还原 skills 表记录。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SkillSnapshot {
    id: String,
    name: String,
    description: Option<String>,
    directory: String,
    source_type: String,
    source_url: Option<String>,
    repo_owner: Option<String>,
    repo_name: Option<String>,
    repo_branch: Option<String>,
    repo_subdir: Option<String>,
    readme_url: Option<String>,
    enabled_claude: bool,
    enabled_codex: bool,
    enabled_gemini: bool,
    enabled_opencode: bool,
    enabled_hermes: bool,
    content_hash: String,
}

impl SkillSnapshot {
    fn from_row(r: &SkillRow) -> Self {
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            description: r.description.clone(),
            directory: r.directory.clone(),
            source_type: r.source_type.clone(),
            source_url: r.source_url.clone(),
            repo_owner: r.repo_owner.clone(),
            repo_name: r.repo_name.clone(),
            repo_branch: r.repo_branch.clone(),
            repo_subdir: r.repo_subdir.clone(),
            readme_url: r.readme_url.clone(),
            enabled_claude: r.enabled_claude,
            enabled_codex: r.enabled_codex,
            enabled_gemini: r.enabled_gemini,
            enabled_opencode: r.enabled_opencode,
            enabled_hermes: r.enabled_hermes,
            content_hash: r.content_hash.clone(),
        }
    }
}

fn timestamp() -> String {
    // 文件名安全的紧凑时间戳。
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{}", now)
}

/// 备份一个 skill 的 SSOT 内容与 DB 行；返回备份条目。
fn backup_skill(data_dir: &Path, row: &SkillRow) -> Result<BackupEntry, String> {
    let ssot = ssot_root(data_dir).join(&row.directory);
    let root = backups_root(data_dir);
    let ts = timestamp();
    let dest = root.join(&row.directory).join(&ts);
    std::fs::create_dir_all(&dest).map_err(|e| format!("创建备份目录失败: {}", e))?;
    ensure_child_path(&root, &dest)?;

    // 备份 SSOT 目录内容（若存在）。
    if ssot.is_dir() {
        let skill_dest = dest.join("skill");
        copy_dir_recursive_pub(&ssot, &skill_dest)?;
    }

    // 备份 DB 行快照。
    let snapshot = SkillSnapshot::from_row(row);
    let bytes =
        serde_json::to_vec_pretty(&snapshot).map_err(|e| format!("序列化 skill 快照失败: {}", e))?;
    std::fs::write(dest.join("skill.json"), bytes)
        .map_err(|e| format!("写入 skill 快照失败: {}", e))?;

    Ok(BackupEntry {
        directory: row.directory.clone(),
        timestamp: ts,
        path: dest.to_string_lossy().to_string(),
        has_snapshot: true,
    })
}

/// 卸载 skill：先备份，再删 live 投影、SSOT 与 DB 记录。
pub fn uninstall(
    db: &Mutex<Connection>,
    data_dir: &Path,
    id: &str,
) -> Result<UninstallReport, String> {
    let row = skills::get(db, id)?.ok_or_else(|| format!("skill '{}' 不存在", id))?;

    // 1. 备份。
    let backup = backup_skill(data_dir, &row)?;

    // 2. 删 DB 记录（先删记录，使后续 sync_all 将其从各 app live 目录移除）。
    skills::delete(db, id)?;

    // 3. sync_all 会移除不再属于任何启用记录的托管投影。
    let sync = sync_all(db, data_dir)?;

    // 4. 删 SSOT 目录。
    let ssot_root_dir = ssot_root(data_dir);
    let ssot = ssot_root_dir.join(&row.directory);
    remove_dir_if_under(&ssot, &ssot_root_dir)?;

    Ok(UninstallReport {
        id: row.id,
        directory: row.directory,
        backup,
        sync,
    })
}

/// 列出备份：指定 directory 时只列该 skill，否则列全部。
pub fn list_backups(data_dir: &Path, directory: Option<&str>) -> Result<Vec<BackupEntry>, String> {
    let root = backups_root(data_dir);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let dirs: Vec<PathBuf> = match directory {
        Some(d) => vec![root.join(d)],
        None => std::fs::read_dir(&root)
            .map_err(|e| format!("读取备份根失败: {}", e))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
    };
    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }
        let Some(dir_name) = dir.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        for entry in std::fs::read_dir(&dir).map_err(|e| format!("读取备份目录失败: {}", e))? {
            let entry = entry.map_err(|e| format!("读取备份项失败: {}", e))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(ts) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            out.push(BackupEntry {
                directory: dir_name.to_string(),
                timestamp: ts.to_string(),
                path: path.to_string_lossy().to_string(),
                has_snapshot: path.join("skill.json").is_file(),
            });
        }
    }
    // 时间戳倒序（新→旧）。
    out.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(out)
}

/// 从备份恢复 skill：还原 SSOT 内容与 DB 行，并重新投影。
pub fn restore(
    db: &Mutex<Connection>,
    data_dir: &Path,
    directory: &str,
    timestamp: &str,
) -> Result<RestoreReport, String> {
    let root = backups_root(data_dir);
    let backup_dir = root.join(directory).join(timestamp);
    ensure_child_path(&root, &backup_dir)?;
    if !backup_dir.is_dir() {
        return Err(format!("备份不存在: {}/{}", directory, timestamp));
    }

    let snapshot_path = backup_dir.join("skill.json");
    if !snapshot_path.is_file() {
        return Err("备份缺少 skill.json 快照，无法恢复".to_string());
    }
    let bytes = std::fs::read(&snapshot_path).map_err(|e| format!("读取备份快照失败: {}", e))?;
    let snapshot: SkillSnapshot =
        serde_json::from_slice(&bytes).map_err(|e| format!("解析备份快照失败: {}", e))?;

    // 1. 恢复 SSOT 内容。
    let ssot_root_dir = ssot_root(data_dir);
    std::fs::create_dir_all(&ssot_root_dir).map_err(|e| format!("创建 SSOT 根失败: {}", e))?;
    let ssot = ssot_root_dir.join(&snapshot.directory);
    ensure_child_path(&ssot_root_dir, &ssot)?;
    let backup_skill_dir = backup_dir.join("skill");
    if !backup_skill_dir.is_dir() {
        return Err("备份缺少 skill 目录内容，无法恢复".to_string());
    }
    // 先清理已存在的 SSOT 目录（限定在 SSOT 根内）。
    remove_dir_if_under(&ssot, &ssot_root_dir)?;
    copy_dir_recursive_pub(&backup_skill_dir, &ssot)?;

    // 2. 恢复 DB 行：若已存在同 directory 则先删，再按快照重建。
    if let Some(existing) = skills::get_by_directory(db, &snapshot.directory)? {
        skills::delete(db, &existing.id)?;
    }
    let restored = skills::create(
        db,
        NewSkill {
            id: snapshot.id.clone(),
            name: snapshot.name.clone(),
            description: snapshot.description.clone(),
            directory: snapshot.directory.clone(),
            source_type: snapshot.source_type.clone(),
            source_url: snapshot.source_url.clone(),
            repo_owner: snapshot.repo_owner.clone(),
            repo_name: snapshot.repo_name.clone(),
            repo_branch: snapshot.repo_branch.clone(),
            repo_subdir: snapshot.repo_subdir.clone(),
            readme_url: snapshot.readme_url.clone(),
            enabled_claude: snapshot.enabled_claude,
            enabled_codex: snapshot.enabled_codex,
            enabled_gemini: snapshot.enabled_gemini,
            enabled_opencode: snapshot.enabled_opencode,
            enabled_hermes: snapshot.enabled_hermes,
            content_hash: snapshot.content_hash.clone(),
        },
    );
    // 若因唯一约束外的原因失败，回滚 SSOT 恢复。
    if let Err(e) = restored {
        let _ = remove_dir_if_under(&ssot, &ssot_root_dir);
        return Err(e);
    }

    // 3. 重新投影。
    let sync = sync_all(db, data_dir)?;

    Ok(RestoreReport {
        directory: snapshot.directory,
        restored_from: backup_dir.to_string_lossy().to_string(),
        sync,
    })
}

/// 供 update 模块复用：备份当前 skill（不删除），返回备份条目。
pub(super) fn backup_only(data_dir: &Path, row: &SkillRow) -> Result<BackupEntry, String> {
    backup_skill(data_dir, row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn setup_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("db");
        let db = Mutex::new(conn);
        run_migrations(&db).expect("migrate");
        db
    }

    fn unique_dir(tag: &str) -> PathBuf {
        static C: AtomicU64 = AtomicU64::new(0);
        let n = C.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "as-skills-bk-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_skill(db: &Mutex<Connection>, data_dir: &Path, directory: &str) -> String {
        let ssot = ssot_root(data_dir).join(directory);
        std::fs::create_dir_all(&ssot).unwrap();
        std::fs::write(ssot.join("SKILL.md"), format!("# {}\n", directory)).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        skills::create(
            db,
            NewSkill {
                id: id.clone(),
                name: directory.to_string(),
                description: None,
                directory: directory.to_string(),
                source_type: "local_dir".to_string(),
                source_url: None,
                repo_owner: None,
                repo_name: None,
                repo_branch: None,
                repo_subdir: None,
                readme_url: None,
                enabled_claude: false,
                enabled_codex: false,
                enabled_gemini: false,
                enabled_opencode: false,
                enabled_hermes: false,
                content_hash: "hash".to_string(),
            },
        )
        .unwrap();
        id
    }

    #[test]
    fn uninstall_backs_up_and_removes() {
        let db = setup_db();
        let data = unique_dir("data");
        let id = seed_skill(&db, &data, "demo");

        let report = uninstall(&db, &data, &id).unwrap();
        assert_eq!(report.directory, "demo");
        assert!(report.backup.has_snapshot);
        // SSOT 已删。
        assert!(!ssot_root(&data).join("demo").exists());
        // DB 记录已删。
        assert!(skills::get(&db, &id).unwrap().is_none());
        // 备份存在。
        let backups = list_backups(&data, Some("demo")).unwrap();
        assert_eq!(backups.len(), 1);
    }

    #[test]
    fn restore_recreates_ssot_and_db() {
        let db = setup_db();
        let data = unique_dir("data");
        let id = seed_skill(&db, &data, "demo");
        let report = uninstall(&db, &data, &id).unwrap();
        let ts = report.backup.timestamp.clone();

        let restored = restore(&db, &data, "demo", &ts).unwrap();
        assert_eq!(restored.directory, "demo");
        assert!(ssot_root(&data).join("demo").join("SKILL.md").is_file());
        assert!(skills::get_by_directory(&db, "demo").unwrap().is_some());
    }

    #[test]
    fn restore_missing_backup_errors() {
        let db = setup_db();
        let data = unique_dir("data");
        assert!(restore(&db, &data, "nope", "123").is_err());
    }
}
