//! Skills 安装来源（cc-skills 阶段 C）：GitHub repo 安装与本地 zip 导入。
//!
//! 两条路径都落到临时区安全解包 → 定位含 `SKILL.md` 的目录 → 复用 `land_skill`
//! 写入 SSOT 并按启用状态投影。GitHub 安装额外写 `skill_repos` 元数据用于后续更新检查。
//! 网络访问仅在此显式调用时发生。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::db::dao::skills::{self, NewSkillRepo};

use super::download::{self, GithubSource};
use super::{land_skill, remove_dir_if_under, sanitize_directory, ImportReport, LandInput};

/// GitHub 安装入参。
#[derive(Debug, Clone)]
pub struct InstallRepoInput {
    pub repo: String,
    pub branch: Option<String>,
    pub subdir: Option<String>,
    pub directory: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    pub enabled_gemini: bool,
    pub enabled_opencode: bool,
    pub enabled_hermes: bool,
}

/// 本地 zip 导入入参。
#[derive(Debug, Clone)]
pub struct ImportZipInput {
    pub zip_path: PathBuf,
    pub subdir: Option<String>,
    pub directory: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    pub enabled_gemini: bool,
    pub enabled_opencode: bool,
    pub enabled_hermes: bool,
}

/// 临时解包根：`app_data/skills_tmp/<uuid>`，用完即删。
fn tmp_root(data_dir: &Path) -> PathBuf {
    data_dir
        .join("skills_tmp")
        .join(uuid::Uuid::new_v4().to_string())
}

/// 从解包目录推导 skill 目录名：显式 directory 优先，否则用 subdir 末段或 repo 名。
fn derive_directory(
    explicit: Option<&str>,
    subdir: Option<&str>,
    fallback: &str,
) -> Result<String, String> {
    if let Some(d) = explicit {
        if !d.trim().is_empty() {
            return sanitize_directory(d);
        }
    }
    if let Some(sub) = subdir {
        if let Some(last) = Path::new(sub.trim())
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => s.to_str(),
                _ => None,
            })
            .next_back()
        {
            return sanitize_directory(last);
        }
    }
    sanitize_directory(fallback)
}

/// 从 GitHub 安装 skill。异步下载后同步解包/落地/写 DB。
pub async fn install_repo(
    db: &Mutex<Connection>,
    data_dir: &Path,
    token: Option<String>,
    input: InstallRepoInput,
) -> Result<ImportReport, String> {
    let (owner, name) = GithubSource::parse_repo(&input.repo)?;
    let source = GithubSource {
        owner: owner.clone(),
        name: name.clone(),
        branch: input.branch.clone(),
        subdir: input.subdir.clone(),
    };

    let client = download::http_client()?;
    let bytes = download::download_repo_tarball(&client, &source, token.as_deref()).await?;

    let tmp = tmp_root(data_dir);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("创建临时解包目录失败: {}", e))?;

    // 落地或失败都清理临时区。
    let result = install_repo_from_bytes(db, data_dir, &tmp, &bytes, &source, &input);
    let _ = std::fs::remove_dir_all(&tmp);
    let report = result?;

    // 记录 repo 来源用于后续更新检查；失败不回滚已安装的 skill，仅告警。
    let _ = skills::create_repo(
        db,
        NewSkillRepo {
            id: uuid::Uuid::new_v4().to_string(),
            source: "github".to_string(),
            repo_owner: Some(owner),
            repo_name: Some(name),
            repo_url: Some(source.repo_url()),
            branch: input.branch.clone(),
            subdir: input.subdir.clone(),
            last_known_hash: Some(report.skill.content_hash.clone()),
            update_status: Some("up_to_date".to_string()),
        },
    );

    Ok(report)
}

fn install_repo_from_bytes(
    db: &Mutex<Connection>,
    data_dir: &Path,
    tmp: &Path,
    bytes: &[u8],
    source: &GithubSource,
    input: &InstallRepoInput,
) -> Result<ImportReport, String> {
    let base = download::unpack_tarball_to(bytes, tmp)?;
    let skill_dir = download::locate_skill_dir(&base, source.subdir.as_deref())?;

    let directory = derive_directory(
        input.directory.as_deref(),
        input.subdir.as_deref(),
        &source.name,
    )?;
    let name = input
        .name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| directory.clone());

    land_skill(
        db,
        data_dir,
        LandInput {
            source: &skill_dir,
            directory,
            name,
            description: input.description.clone(),
            source_type: "github".to_string(),
            source_url: Some(source.repo_url()),
            repo_owner: Some(source.owner.clone()),
            repo_name: Some(source.name.clone()),
            repo_branch: source.branch.clone(),
            repo_subdir: source.subdir.clone(),
            readme_url: None,
            enabled_claude: input.enabled_claude,
            enabled_codex: input.enabled_codex,
            enabled_gemini: input.enabled_gemini,
            enabled_opencode: input.enabled_opencode,
            enabled_hermes: input.enabled_hermes,
        },
    )
}

/// 从本地 zip 导入 skill（不联网）。
pub fn import_zip(
    db: &Mutex<Connection>,
    data_dir: &Path,
    input: ImportZipInput,
) -> Result<ImportReport, String> {
    let bytes = std::fs::read(&input.zip_path)
        .map_err(|e| format!("读取 zip 文件失败 {}: {}", input.zip_path.display(), e))?;

    let tmp = tmp_root(data_dir);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("创建临时解包目录失败: {}", e))?;

    let result = import_zip_from_bytes(db, data_dir, &tmp, &bytes, &input);
    let _ = std::fs::remove_dir_all(&tmp);
    result
}

fn import_zip_from_bytes(
    db: &Mutex<Connection>,
    data_dir: &Path,
    tmp: &Path,
    bytes: &[u8],
    input: &ImportZipInput,
) -> Result<ImportReport, String> {
    let base = download::unpack_zip_to(bytes, tmp)?;
    let skill_dir = download::locate_skill_dir(&base, input.subdir.as_deref())?;

    let fallback = input
        .zip_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("skill");
    let directory = derive_directory(input.directory.as_deref(), input.subdir.as_deref(), fallback)?;
    let name = input
        .name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| directory.clone());

    land_skill(
        db,
        data_dir,
        LandInput {
            source: &skill_dir,
            directory,
            name,
            description: input.description.clone(),
            source_type: "zip".to_string(),
            source_url: Some(input.zip_path.to_string_lossy().to_string()),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            repo_subdir: input.subdir.clone(),
            readme_url: None,
            enabled_claude: input.enabled_claude,
            enabled_codex: input.enabled_codex,
            enabled_gemini: input.enabled_gemini,
            enabled_opencode: input.enabled_opencode,
            enabled_hermes: input.enabled_hermes,
        },
    )
}

/// 供 update 模块复用：下载 GitHub tarball 并解包定位 skill 目录到 `tmp`。
/// 返回定位到的 skill 目录（位于 `tmp` 下）。
pub(super) async fn fetch_repo_skill_dir(
    tmp: &Path,
    source: &GithubSource,
    token: Option<&str>,
) -> Result<PathBuf, String> {
    let client = download::http_client()?;
    let bytes = download::download_repo_tarball(&client, source, token).await?;
    let base = download::unpack_tarball_to(&bytes, tmp)?;
    download::locate_skill_dir(&base, source.subdir.as_deref())
}

/// 清理可能残留的临时解包根（尽力而为）。
pub(super) fn cleanup_tmp(tmp: &Path, data_dir: &Path) {
    let tmp_parent = data_dir.join("skills_tmp");
    if tmp.starts_with(&tmp_parent) {
        let _ = remove_dir_if_under(tmp, &tmp_parent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_directory_prefers_explicit() {
        assert_eq!(
            derive_directory(Some("my-skill"), Some("skills/foo"), "repo").unwrap(),
            "my-skill"
        );
    }

    #[test]
    fn derive_directory_falls_back_to_subdir_last_segment() {
        assert_eq!(
            derive_directory(None, Some("skills/foo"), "repo").unwrap(),
            "foo"
        );
    }

    #[test]
    fn derive_directory_falls_back_to_repo_name() {
        assert_eq!(derive_directory(None, None, "repo").unwrap(), "repo");
    }

    #[test]
    fn derive_directory_rejects_bad_names() {
        assert!(derive_directory(Some("../x"), None, "repo").is_err());
    }
}
