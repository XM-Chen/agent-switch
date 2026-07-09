//! Skills 更新检查与更新执行（cc-skills 阶段 C）。
//!
//! `check_updates` 对 GitHub 来源 skill 拉取远端内容 hash，与本地 `skills.content_hash` 比对。
//! `update` 下载新版 → 备份旧版 → 替换 SSOT → 更新 DB hash → 重新 sync；任一步失败会从备份恢复。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;
use serde::Serialize;

use crate::db::dao::skills::{self, SkillUpdate};

use super::backup;
use super::download::GithubSource;
use super::install::{cleanup_tmp, fetch_repo_skill_dir};
use super::{copy_dir_atomic, hash_directory, ssot_root, sync_all, BackupEntry, SyncReport};

#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheckItem {
    pub skill_id: String,
    pub directory: String,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub subdir: Option<String>,
    pub local_hash: String,
    pub remote_hash: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckUpdatesReport {
    pub checked: Vec<UpdateCheckItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateItemReport {
    pub skill_id: String,
    pub directory: String,
    pub updated: bool,
    pub old_hash: String,
    pub new_hash: Option<String>,
    pub backup: Option<BackupEntry>,
    pub sync: Vec<SyncReport>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateReport {
    pub items: Vec<UpdateItemReport>,
}

fn tmp_root(data_dir: &Path) -> PathBuf {
    data_dir
        .join("skills_tmp")
        .join(uuid::Uuid::new_v4().to_string())
}

fn github_source_for(row: &crate::db::dao::skills::SkillRow) -> Result<GithubSource, String> {
    if row.source_type != "github" {
        return Err(format!("skill '{}' 不是 GitHub 来源", row.id));
    }
    let owner = row
        .repo_owner
        .clone()
        .ok_or_else(|| format!("skill '{}' 缺少 repo_owner", row.id))?;
    let name = row
        .repo_name
        .clone()
        .ok_or_else(|| format!("skill '{}' 缺少 repo_name", row.id))?;
    Ok(GithubSource {
        owner,
        name,
        branch: row.repo_branch.clone(),
        subdir: row.repo_subdir.clone(),
    })
}

/// 检查一个或全部 GitHub 来源 skill 的更新状态。
pub async fn check_updates(
    db: &Mutex<Connection>,
    data_dir: &Path,
    token: Option<String>,
    ids: Option<Vec<String>>,
) -> Result<CheckUpdatesReport, String> {
    let rows = match ids {
        Some(ids) if !ids.is_empty() => ids
            .into_iter()
            .map(|id| {
                skills::get(db, &id)?
                    .ok_or_else(|| format!("skill '{}' 不存在", id))
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => skills::list(db)?
            .into_iter()
            .filter(|r| r.source_type == "github")
            .collect(),
    };

    let mut checked = Vec::new();
    for row in rows {
        let repo_label = match (&row.repo_owner, &row.repo_name) {
            (Some(owner), Some(name)) => Some(format!("{}/{}", owner, name)),
            _ => None,
        };
        match github_source_for(&row) {
            Ok(source) => {
                let tmp = tmp_root(data_dir);
                std::fs::create_dir_all(&tmp)
                    .map_err(|e| format!("创建临时解包目录失败: {}", e))?;
                let result = fetch_repo_skill_dir(&tmp, &source, token.as_deref()).await;
                let item = match result {
                    Ok(remote_dir) => match hash_directory(&remote_dir) {
                        Ok(remote_hash) => {
                            let status = if remote_hash == row.content_hash {
                                "up_to_date"
                            } else {
                                "update_available"
                            }
                            .to_string();
                            update_repo_check_for_row(db, &row, Some(&remote_hash), Some(&status))?;
                            UpdateCheckItem {
                                skill_id: row.id.clone(),
                                directory: row.directory.clone(),
                                repo: repo_label,
                                branch: row.repo_branch.clone(),
                                subdir: row.repo_subdir.clone(),
                                local_hash: row.content_hash.clone(),
                                remote_hash: Some(remote_hash),
                                status,
                                error: None,
                            }
                        }
                        Err(e) => UpdateCheckItem {
                            skill_id: row.id.clone(),
                            directory: row.directory.clone(),
                            repo: repo_label,
                            branch: row.repo_branch.clone(),
                            subdir: row.repo_subdir.clone(),
                            local_hash: row.content_hash.clone(),
                            remote_hash: None,
                            status: "error".to_string(),
                            error: Some(e),
                        },
                    },
                    Err(e) => {
                        let _ = update_repo_check_for_row(db, &row, None, Some("error"));
                        UpdateCheckItem {
                            skill_id: row.id.clone(),
                            directory: row.directory.clone(),
                            repo: repo_label,
                            branch: row.repo_branch.clone(),
                            subdir: row.repo_subdir.clone(),
                            local_hash: row.content_hash.clone(),
                            remote_hash: None,
                            status: "error".to_string(),
                            error: Some(e),
                        }
                    }
                };
                cleanup_tmp(&tmp, data_dir);
                checked.push(item);
            }
            Err(e) => checked.push(UpdateCheckItem {
                skill_id: row.id.clone(),
                directory: row.directory.clone(),
                repo: repo_label,
                branch: row.repo_branch.clone(),
                subdir: row.repo_subdir.clone(),
                local_hash: row.content_hash.clone(),
                remote_hash: None,
                status: "error".to_string(),
                error: Some(e),
            }),
        }
    }

    Ok(CheckUpdatesReport { checked })
}

fn update_repo_check_for_row(
    db: &Mutex<Connection>,
    row: &crate::db::dao::skills::SkillRow,
    hash: Option<&str>,
    status: Option<&str>,
) -> Result<(), String> {
    if let (Some(owner), Some(name)) = (&row.repo_owner, &row.repo_name) {
        if let Some(repo) = skills::find_repo_for(db, owner, name, row.repo_subdir.as_deref())? {
            skills::update_repo_check(db, &repo.id, hash, status)?;
        }
    }
    Ok(())
}

/// 更新一个或多个 GitHub 来源 skill。`ids` 为空时更新所有 GitHub 来源 skill。
pub async fn update(
    db: &Mutex<Connection>,
    data_dir: &Path,
    token: Option<String>,
    ids: Option<Vec<String>>,
) -> Result<UpdateReport, String> {
    let rows = match ids {
        Some(ids) if !ids.is_empty() => ids
            .into_iter()
            .map(|id| {
                skills::get(db, &id)?
                    .ok_or_else(|| format!("skill '{}' 不存在", id))
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => skills::list(db)?
            .into_iter()
            .filter(|r| r.source_type == "github")
            .collect(),
    };

    let mut items = Vec::new();
    for row in rows {
        items.push(update_one(db, data_dir, token.as_deref(), &row).await);
    }
    Ok(UpdateReport { items })
}

async fn update_one(
    db: &Mutex<Connection>,
    data_dir: &Path,
    token: Option<&str>,
    row: &crate::db::dao::skills::SkillRow,
) -> UpdateItemReport {
    let old_hash = row.content_hash.clone();
    let mut report = UpdateItemReport {
        skill_id: row.id.clone(),
        directory: row.directory.clone(),
        updated: false,
        old_hash: old_hash.clone(),
        new_hash: None,
        backup: None,
        sync: Vec::new(),
        error: None,
    };

    let source = match github_source_for(row) {
        Ok(source) => source,
        Err(e) => {
            report.error = Some(e);
            return report;
        }
    };

    let tmp = tmp_root(data_dir);
    if let Err(e) = std::fs::create_dir_all(&tmp) {
        report.error = Some(format!("创建临时解包目录失败: {}", e));
        return report;
    }

    // 阶段 1（只读，未触碰 SSOT）：下载 + 解包 + 算 hash。
    // 任何失败都不得回滚/删除本地 skill——本地内容仍是健康的。
    let fetched = async {
        let remote_dir = fetch_repo_skill_dir(&tmp, &source, token).await?;
        let new_hash = hash_directory(&remote_dir)?;
        Ok::<(PathBuf, String), String>((remote_dir, new_hash))
    }
    .await;

    let (remote_dir, new_hash) = match fetched {
        Ok(v) => v,
        Err(e) => {
            cleanup_tmp(&tmp, data_dir);
            report.error = Some(e);
            return report;
        }
    };

    // 无变化：更新 repo 检查状态，直接返回，不备份不改 SSOT。
    if new_hash == old_hash {
        cleanup_tmp(&tmp, data_dir);
        report.new_hash = Some(new_hash.clone());
        let _ = update_repo_check_for_row(db, row, Some(&new_hash), Some("up_to_date"));
        return report;
    }

    // 阶段 2：本次更新前先备份当前版本（用于本次失败回滚）。
    let backup = match backup::backup_only(data_dir, row) {
        Ok(b) => b,
        Err(e) => {
            cleanup_tmp(&tmp, data_dir);
            report.error = Some(format!("更新前备份失败，已中止更新: {}", e));
            return report;
        }
    };

    // 阶段 3（改写 SSOT）：替换内容 → 更新 DB hash → 重新投影。
    // 任一步失败都从“本次备份”恢复，不使用其它备份。
    let apply = (|| {
        let ssot_root_dir = ssot_root(data_dir);
        let ssot = ssot_root_dir.join(&row.directory);
        copy_dir_atomic(&remote_dir, &ssot, &ssot_root_dir, None)?;
        skills::update(
            db,
            &row.id,
            SkillUpdate {
                content_hash: Some(new_hash.clone()),
                ..Default::default()
            },
        )?;
        let sync = sync_all(db, data_dir)?;
        update_repo_check_for_row(db, row, Some(&new_hash), Some("up_to_date"))?;
        Ok::<Vec<SyncReport>, String>(sync)
    })();

    cleanup_tmp(&tmp, data_dir);

    match apply {
        Ok(sync) => {
            report.updated = true;
            report.new_hash = Some(new_hash);
            report.backup = Some(backup);
            report.sync = sync;
        }
        Err(e) => {
            // 明确从本次备份恢复 SSOT 与 DB 行。
            let restore_err = backup::restore(db, data_dir, &backup.directory, &backup.timestamp)
                .err()
                .map(|re| format!("；回滚也失败: {}", re))
                .unwrap_or_default();
            report.backup = Some(backup);
            report.error = Some(format!("更新失败已回滚: {}{}", e, restore_err));
        }
    }

    report
}
