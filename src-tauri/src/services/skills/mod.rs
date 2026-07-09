//! Skills 管理（cc-skills）。
//!
//! 覆盖：app data SSOT、本地目录/zip 导入、GitHub 安装、copy 投影、多 app 启用/禁用、
//! 状态与冲突报告、卸载/备份恢复、更新检查与批量更新、未托管扫描、skills.sh/GitHub 发现。
//! 所有网络访问仅在用户显式触发时发生；下载走临时区安全解包后再进入 SSOT。

mod backup;
mod discovery;
pub mod download;
mod install;
mod update;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::db::dao::skills::{self, NewSkill, SkillRow};

pub(crate) const ENTRY_FILE: &str = "SKILL.md";
pub(crate) const MARKER_FILE: &str = ".agent-switch-skill.json";

// 对外（HTTP API 层）暴露的阶段 C 能力。
pub use self::backup::{
    list_backups, restore, uninstall, BackupEntry, RestoreReport, UninstallReport,
};
pub use self::discovery::{scan_unmanaged, search, ScanUnmanagedReport, SearchReport};
pub use self::install::{import_zip, install_repo, ImportZipInput, InstallRepoInput};
pub use self::update::{check_updates, update, CheckUpdatesReport, UpdateReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillApp {
    Claude,
    Codex,
    Gemini,
    OpenCode,
    Hermes,
}

impl SkillApp {
    pub fn all() -> [Self; 5] {
        [
            Self::Claude,
            Self::Codex,
            Self::Gemini,
            Self::OpenCode,
            Self::Hermes,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
            Self::Hermes => "hermes",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini",
            Self::OpenCode => "OpenCode",
            Self::Hermes => "Hermes",
        }
    }

    fn enabled(self, row: &SkillRow) -> bool {
        match self {
            Self::Claude => row.enabled_claude,
            Self::Codex => row.enabled_codex,
            Self::Gemini => row.enabled_gemini,
            Self::OpenCode => row.enabled_opencode,
            Self::Hermes => row.enabled_hermes,
        }
    }

    pub(crate) fn target_dirs(self) -> Result<(PathBuf, PathBuf), String> {
        let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
        let (config_root, target_root) = match self {
            Self::Claude => {
                let root = home.join(".claude");
                (root.clone(), root.join("skills"))
            }
            Self::Codex => {
                let root = home.join(".codex");
                (root.clone(), root.join("skills"))
            }
            Self::Gemini => {
                let root = home.join(".gemini");
                (root.clone(), root.join("skills"))
            }
            Self::OpenCode => {
                let root = dirs::config_dir()
                    .unwrap_or_else(|| home.join(".config"))
                    .join("opencode");
                (root.clone(), root.join("skills"))
            }
            Self::Hermes => {
                let root = home.join(".hermes");
                (root.clone(), root.join("skills"))
            }
        };
        Ok((config_root, target_root))
    }
}

pub fn parse_app(app: &str) -> Option<SkillApp> {
    match app {
        "claude" | "claude-code" => Some(SkillApp::Claude),
        "codex" => Some(SkillApp::Codex),
        "gemini" => Some(SkillApp::Gemini),
        "opencode" => Some(SkillApp::OpenCode),
        "hermes" => Some(SkillApp::Hermes),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct ImportDirInput {
    pub source_path: PathBuf,
    pub directory: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    pub enabled_gemini: bool,
    pub enabled_opencode: bool,
    pub enabled_hermes: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub skill: SkillSummary,
    pub sync: Vec<SyncReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillSummary {
    pub id: String,
    pub name: String,
    pub directory: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillStatus {
    pub ssot_path: String,
    pub ssot_exists: bool,
    pub apps: Vec<AppStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppStatus {
    pub app: String,
    pub label: String,
    pub config_root: String,
    pub target_root: String,
    pub config_root_exists: bool,
    pub target_root_exists: bool,
    pub enabled_count: usize,
    pub managed_count: usize,
    pub conflicts: Vec<SkillConflict>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncReport {
    pub app: String,
    pub label: String,
    pub target_root: String,
    pub target_root_exists: bool,
    pub projected: usize,
    pub removed: usize,
    pub skipped_missing_root: usize,
    pub conflicts: Vec<SkillConflict>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillConflict {
    pub app: String,
    pub skill_id: String,
    pub directory: String,
    pub target_path: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
struct Marker<'a> {
    managed_by: &'static str,
    skill_id: &'a str,
    directory: &'a str,
    content_hash: &'a str,
}

pub fn ssot_root(data_dir: &Path) -> PathBuf {
    data_dir.join("skills")
}

pub fn import_dir(
    db: &Mutex<Connection>,
    data_dir: &Path,
    input: ImportDirInput,
) -> Result<ImportReport, String> {
    let source = input
        .source_path
        .canonicalize()
        .map_err(|e| format!("读取源目录失败: {}", e))?;
    validate_skill_dir(&source)?;

    let directory = match input.directory {
        Some(d) if !d.trim().is_empty() => sanitize_directory(&d)?,
        _ => source
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| "无法从源路径推导 skill 目录名".to_string())
            .and_then(sanitize_directory)?,
    };

    let name = input
        .name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| directory.clone());

    land_skill(
        db,
        data_dir,
        LandInput {
            source: &source,
            directory,
            name,
            description: input.description,
            source_type: "local_dir".to_string(),
            source_url: Some(source.to_string_lossy().to_string()),
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            repo_subdir: None,
            readme_url: None,
            enabled_claude: input.enabled_claude,
            enabled_codex: input.enabled_codex,
            enabled_gemini: input.enabled_gemini,
            enabled_opencode: input.enabled_opencode,
            enabled_hermes: input.enabled_hermes,
        },
    )
}

/// 落地一个已备好的 skill 源目录到 SSOT，写 DB 并按启用状态投影。
///
/// `source` 必须已通过 `validate_skill_dir`（含 `SKILL.md`、无符号链接）。
/// 供 `import_dir` / `install::install_repo` / `install::import_zip` 复用。
pub(crate) struct LandInput<'a> {
    pub source: &'a Path,
    pub directory: String,
    pub name: String,
    pub description: Option<String>,
    pub source_type: String,
    pub source_url: Option<String>,
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub repo_branch: Option<String>,
    pub repo_subdir: Option<String>,
    pub readme_url: Option<String>,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    pub enabled_gemini: bool,
    pub enabled_opencode: bool,
    pub enabled_hermes: bool,
}

pub(crate) fn land_skill(
    db: &Mutex<Connection>,
    data_dir: &Path,
    input: LandInput<'_>,
) -> Result<ImportReport, String> {
    validate_skill_dir(input.source)?;
    let directory = sanitize_directory(&input.directory)?;

    if skills::get_by_directory(db, &directory)?.is_some() {
        return Err(format!("skill 目录 '{}' 已存在", directory));
    }

    let root = ssot_root(data_dir);
    std::fs::create_dir_all(&root).map_err(|e| format!("创建 SSOT 目录失败: {}", e))?;
    let dest = root.join(&directory);
    ensure_child_path(&root, &dest)?;
    if dest.exists() {
        return Err(format!("SSOT 目录已存在: {}", dest.display()));
    }

    copy_dir_atomic(input.source, &dest, &root, None)?;
    let content_hash = hash_directory(&dest)?;
    let id = uuid::Uuid::new_v4().to_string();

    let create_result = skills::create(
        db,
        NewSkill {
            id: id.clone(),
            name: input.name,
            description: input.description,
            directory: directory.clone(),
            source_type: input.source_type,
            source_url: input.source_url,
            repo_owner: input.repo_owner,
            repo_name: input.repo_name,
            repo_branch: input.repo_branch,
            repo_subdir: input.repo_subdir,
            readme_url: input.readme_url,
            enabled_claude: input.enabled_claude,
            enabled_codex: input.enabled_codex,
            enabled_gemini: input.enabled_gemini,
            enabled_opencode: input.enabled_opencode,
            enabled_hermes: input.enabled_hermes,
            content_hash: content_hash.clone(),
        },
    );

    let row = match create_result {
        Ok(row) => row,
        Err(e) => {
            let _ = remove_dir_if_under(&dest, &root);
            return Err(e);
        }
    };

    let mut sync = Vec::new();
    for app in SkillApp::all() {
        if app.enabled(&row) {
            sync.push(sync_app(db, data_dir, app)?);
        }
    }

    Ok(ImportReport {
        skill: SkillSummary {
            id,
            name: row.name,
            directory,
            content_hash,
        },
        sync,
    })
}

pub fn set_enabled(
    db: &Mutex<Connection>,
    data_dir: &Path,
    id: &str,
    app: SkillApp,
    enabled: bool,
) -> Result<SyncReport, String> {
    let row = skills::get(db, id)?.ok_or_else(|| format!("skill '{}' 不存在", id))?;
    if enabled {
        preflight_conflict(app, &row)?;
    }
    skills::set_enabled(db, id, app.as_str(), enabled)?;
    sync_app(db, data_dir, app)
}

pub fn sync_all(db: &Mutex<Connection>, data_dir: &Path) -> Result<Vec<SyncReport>, String> {
    let mut out = Vec::new();
    for app in SkillApp::all() {
        out.push(sync_app(db, data_dir, app)?);
    }
    Ok(out)
}

pub fn sync_one(
    db: &Mutex<Connection>,
    data_dir: &Path,
    app: SkillApp,
) -> Result<SyncReport, String> {
    sync_app(db, data_dir, app)
}

pub fn status(db: &Mutex<Connection>, data_dir: &Path) -> Result<SkillStatus, String> {
    let root = ssot_root(data_dir);
    let mut apps = Vec::new();
    for app in SkillApp::all() {
        apps.push(app_status(db, app)?);
    }
    Ok(SkillStatus {
        ssot_path: root.to_string_lossy().to_string(),
        ssot_exists: root.exists(),
        apps,
    })
}

fn app_status(db: &Mutex<Connection>, app: SkillApp) -> Result<AppStatus, String> {
    let enabled = skills::list_enabled(db, app.as_str())?;
    let (config_root, target_root) = app.target_dirs()?;
    let managed_count = count_managed(&target_root);
    let conflicts = enabled
        .iter()
        .filter_map(|row| conflict_for(app, row).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(AppStatus {
        app: app.as_str().to_string(),
        label: app.label().to_string(),
        config_root: config_root.to_string_lossy().to_string(),
        target_root: target_root.to_string_lossy().to_string(),
        config_root_exists: config_root.exists(),
        target_root_exists: target_root.exists(),
        enabled_count: enabled.len(),
        managed_count,
        conflicts,
    })
}

fn sync_app(db: &Mutex<Connection>, data_dir: &Path, app: SkillApp) -> Result<SyncReport, String> {
    let enabled = skills::list_enabled(db, app.as_str())?;
    let (config_root, target_root) = app.target_dirs()?;
    if !config_root.exists() {
        return Ok(SyncReport {
            app: app.as_str().to_string(),
            label: app.label().to_string(),
            target_root: target_root.to_string_lossy().to_string(),
            target_root_exists: false,
            projected: 0,
            removed: 0,
            skipped_missing_root: enabled.len(),
            conflicts: Vec::new(),
            warnings: vec![format!(
                "{} 配置目录不存在，已保存启用状态但未投影",
                app.label()
            )],
        });
    }

    if !enabled.is_empty() {
        std::fs::create_dir_all(&target_root)
            .map_err(|e| format!("创建 skills 目标目录失败: {}", e))?;
    }

    let mut report = SyncReport {
        app: app.as_str().to_string(),
        label: app.label().to_string(),
        target_root: target_root.to_string_lossy().to_string(),
        target_root_exists: target_root.exists(),
        projected: 0,
        removed: 0,
        skipped_missing_root: 0,
        conflicts: Vec::new(),
        warnings: vec!["当前版本使用 copy 投影；symlink/auto 后续补齐".to_string()],
    };

    let enabled_dirs: BTreeSet<String> = enabled.iter().map(|s| s.directory.clone()).collect();
    if target_root.exists() {
        for entry in std::fs::read_dir(&target_root)
            .map_err(|e| format!("读取目标 skills 目录失败: {}", e))?
        {
            let entry = entry.map_err(|e| format!("读取目标 skills 项失败: {}", e))?;
            let path = entry.path();
            if !path.is_dir() || !is_managed_projection(&path) {
                continue;
            }
            let Some(dir) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !enabled_dirs.contains(dir) {
                remove_dir_if_under(&path, &target_root)?;
                report.removed += 1;
            }
        }
    }

    let root = ssot_root(data_dir);
    for row in enabled {
        let source = root.join(&row.directory);
        ensure_child_path(&root, &source)?;
        if !source.join(ENTRY_FILE).is_file() {
            report
                .warnings
                .push(format!("{} 缺少 {}，跳过投影", row.directory, ENTRY_FILE));
            continue;
        }

        if let Some(conflict) = conflict_for(app, &row)? {
            report.conflicts.push(conflict);
            continue;
        }

        let target = target_root.join(&row.directory);
        ensure_child_path(&target_root, &target)?;
        copy_dir_atomic(&source, &target, &target_root, Some(&row))?;
        report.projected += 1;
    }

    report.target_root_exists = target_root.exists();
    Ok(report)
}

fn preflight_conflict(app: SkillApp, row: &SkillRow) -> Result<(), String> {
    if let Some(conflict) = conflict_for(app, row)? {
        return Err(format!(
            "目标已有非托管 skill，拒绝覆盖: {} ({})",
            conflict.target_path, conflict.reason
        ));
    }
    Ok(())
}

fn conflict_for(app: SkillApp, row: &SkillRow) -> Result<Option<SkillConflict>, String> {
    let (config_root, target_root) = app.target_dirs()?;
    if !config_root.exists() {
        return Ok(None);
    }
    let target = target_root.join(&row.directory);
    if target.exists() && !is_managed_projection(&target) {
        return Ok(Some(SkillConflict {
            app: app.as_str().to_string(),
            skill_id: row.id.clone(),
            directory: row.directory.clone(),
            target_path: target.to_string_lossy().to_string(),
            reason: "同名目录/文件不是 agent-switch 托管项".to_string(),
        }));
    }
    Ok(None)
}

pub(crate) fn validate_skill_dir(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err("源路径不是目录".to_string());
    }
    if !path.join(ENTRY_FILE).is_file() {
        return Err(format!("skill 目录必须包含 {}", ENTRY_FILE));
    }
    reject_symlinks(path)
}

pub(crate)fn sanitize_directory(input: impl AsRef<str>) -> Result<String, String> {
    let name = input.as_ref().trim();
    if name.is_empty() || name == "." || name == ".." {
        return Err("skill 目录名不能为空或特殊路径".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err("skill 目录名不能包含路径分隔符".to_string());
    }
    Ok(name.to_string())
}

fn reject_symlinks(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("读取路径元数据失败 {}: {}", path.display(), e))?;
    if meta.file_type().is_symlink() {
        return Err(format!("拒绝导入符号链接: {}", path.display()));
    }
    if meta.is_dir() {
        for entry in std::fs::read_dir(path)
            .map_err(|e| format!("读取目录失败 {}: {}", path.display(), e))?
        {
            let entry = entry.map_err(|e| format!("读取目录项失败: {}", e))?;
            reject_symlinks(&entry.path())?;
        }
    }
    Ok(())
}

pub(crate) fn hash_directory(path: &Path) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hash_path(path, path, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_path(base: &Path, path: &Path, hasher: &mut Sha256) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("读取路径元数据失败 {}: {}", path.display(), e))?;
    if meta.file_type().is_symlink() {
        return Err(format!("拒绝计算符号链接 hash: {}", path.display()));
    }
    let rel = path.strip_prefix(base).unwrap_or(path);
    hasher.update(rel.to_string_lossy().as_bytes());
    if meta.is_file() {
        let bytes =
            std::fs::read(path).map_err(|e| format!("读取文件失败 {}: {}", path.display(), e))?;
        hasher.update(&bytes);
        return Ok(());
    }
    let mut entries = std::fs::read_dir(path)
        .map_err(|e| format!("读取目录失败 {}: {}", path.display(), e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取目录项失败: {}", e))?;
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        hash_path(base, &entry.path(), hasher)?;
    }
    Ok(())
}

pub(crate) fn copy_dir_atomic(
    source: &Path,
    target: &Path,
    allowed_root: &Path,
    marker: Option<&SkillRow>,
) -> Result<(), String> {
    ensure_child_path(allowed_root, target)?;
    reject_symlinks(source)?;
    let tmp = allowed_root.join(format!(
        ".{}.tmp-{}",
        target
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("skill"),
        uuid::Uuid::new_v4()
    ));
    ensure_child_path(allowed_root, &tmp)?;
    if tmp.exists() {
        remove_dir_if_under(&tmp, allowed_root)?;
    }
    copy_dir_recursive(source, &tmp)?;
    if let Some(row) = marker {
        let marker = Marker {
            managed_by: "agent-switch",
            skill_id: &row.id,
            directory: &row.directory,
            content_hash: &row.content_hash,
        };
        let bytes =
            serde_json::to_vec_pretty(&marker).map_err(|e| format!("序列化托管标记失败: {}", e))?;
        std::fs::write(tmp.join(MARKER_FILE), bytes)
            .map_err(|e| format!("写入托管标记失败: {}", e))?;
    }
    if target.exists() {
        if !is_managed_projection(target) && marker.is_some() {
            let _ = remove_dir_if_under(&tmp, allowed_root);
            return Err(format!("目标不是托管项，拒绝覆盖: {}", target.display()));
        }
        remove_dir_if_under(target, allowed_root)?;
    }
    std::fs::rename(&tmp, target).map_err(|e| {
        format!(
            "投影目录重命名失败 {} -> {}: {}",
            tmp.display(),
            target.display(),
            e
        )
    })?;
    Ok(())
}

/// 供 backup 子模块复用的目录递归复制（拒绝符号链接）。
pub(crate) fn copy_dir_recursive_pub(source: &Path, target: &Path) -> Result<(), String> {
    copy_dir_recursive(source, target)
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(source)
        .map_err(|e| format!("读取源路径失败 {}: {}", source.display(), e))?;
    if meta.file_type().is_symlink() {
        return Err(format!("拒绝复制符号链接: {}", source.display()));
    }
    if meta.is_file() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
        }
        std::fs::copy(source, target).map_err(|e| format!("复制文件失败: {}", e))?;
        return Ok(());
    }
    std::fs::create_dir_all(target).map_err(|e| format!("创建目录失败: {}", e))?;
    for entry in std::fs::read_dir(source)
        .map_err(|e| format!("读取源目录失败 {}: {}", source.display(), e))?
    {
        let entry = entry.map_err(|e| format!("读取源目录项失败: {}", e))?;
        let name = entry.file_name();
        copy_dir_recursive(&entry.path(), &target.join(name))?;
    }
    Ok(())
}

pub(crate) fn is_managed_projection(path: &Path) -> bool {
    path.join(MARKER_FILE).is_file()
}

fn count_managed(root: &Path) -> usize {
    std::fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir() && is_managed_projection(&e.path()))
        .count()
}

pub(crate)fn ensure_child_path(root: &Path, child: &Path) -> Result<(), String> {
    if child.starts_with(root) {
        Ok(())
    } else {
        Err(format!(
            "路径越界，拒绝操作: {} 不在 {} 下",
            child.display(),
            root.display()
        ))
    }
}

pub(crate)fn remove_dir_if_under(path: &Path, root: &Path) -> Result<(), String> {
    ensure_child_path(root, path)?;
    if path.exists() {
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("删除目录失败 {}: {}", path.display(), e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn setup_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        let db = Mutex::new(conn);
        run_migrations(&db).expect("迁移失败");
        db
    }

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "as-skills-test-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        dir
    }

    fn make_skill_dir(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(ENTRY_FILE), format!("# {}\n", name)).unwrap();
        dir
    }

    #[test]
    fn import_dir_copies_to_ssot_and_records_hash() {
        let db = setup_db();
        let data = unique_dir("data");
        let source_root = unique_dir("source");
        let source = make_skill_dir(&source_root, "demo-skill");

        let report = import_dir(
            &db,
            &data,
            ImportDirInput {
                source_path: source,
                directory: None,
                name: None,
                description: None,
                enabled_claude: false,
                enabled_codex: false,
                enabled_gemini: false,
                enabled_opencode: false,
                enabled_hermes: false,
            },
        )
        .unwrap();

        assert_eq!(report.skill.directory, "demo-skill");
        assert!(ssot_root(&data)
            .join("demo-skill")
            .join(ENTRY_FILE)
            .is_file());
        let row = skills::get_by_directory(&db, "demo-skill")
            .unwrap()
            .unwrap();
        assert_eq!(row.content_hash, report.skill.content_hash);
    }

    #[test]
    fn import_rejects_missing_entry_file() {
        let db = setup_db();
        let data = unique_dir("data-no-entry");
        let source = unique_dir("bad-source").join("bad");
        std::fs::create_dir_all(&source).unwrap();

        let err = import_dir(
            &db,
            &data,
            ImportDirInput {
                source_path: source,
                directory: None,
                name: None,
                description: None,
                enabled_claude: false,
                enabled_codex: false,
                enabled_gemini: false,
                enabled_opencode: false,
                enabled_hermes: false,
            },
        )
        .unwrap_err();
        assert!(err.contains(ENTRY_FILE), "{}", err);
    }

    #[test]
    fn directory_name_rejects_path_traversal() {
        assert!(sanitize_directory("../x").is_err());
        assert!(sanitize_directory("a\\b").is_err());
        assert!(sanitize_directory("ok-skill").is_ok());
    }

    #[test]
    fn copy_projection_refuses_unmanaged_target() {
        let data = unique_dir("copy-conflict-data");
        let source_root = unique_dir("copy-conflict-source");
        let source = make_skill_dir(&source_root, "skill-a");
        let live_root = unique_dir("copy-conflict-live");
        let target = live_root.join("skill-a");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join(ENTRY_FILE), "# user\n").unwrap();

        let row = SkillRow {
            id: "s1".to_string(),
            name: "skill-a".to_string(),
            description: None,
            directory: "skill-a".to_string(),
            source_type: "local_dir".to_string(),
            source_url: None,
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            repo_subdir: None,
            readme_url: None,
            enabled_claude: true,
            enabled_codex: false,
            enabled_gemini: false,
            enabled_opencode: false,
            enabled_hermes: false,
            installed_at: "now".to_string(),
            updated_at: "now".to_string(),
            content_hash: hash_directory(&source).unwrap(),
            created_at: "now".to_string(),
        };
        let err = copy_dir_atomic(&source, &target, &live_root, Some(&row)).unwrap_err();
        assert!(err.contains("拒绝覆盖"), "{}", err);
        assert_eq!(
            std::fs::read_to_string(target.join(ENTRY_FILE)).unwrap(),
            "# user\n"
        );
        assert!(data.exists());
    }

    #[test]
    fn managed_projection_can_be_replaced_and_marked() {
        let source_root = unique_dir("copy-managed-source");
        let source = make_skill_dir(&source_root, "skill-a");
        let live_root = unique_dir("copy-managed-live");
        let target = live_root.join("skill-a");
        let hash = hash_directory(&source).unwrap();
        let row = SkillRow {
            id: "s1".to_string(),
            name: "skill-a".to_string(),
            description: None,
            directory: "skill-a".to_string(),
            source_type: "local_dir".to_string(),
            source_url: None,
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            repo_subdir: None,
            readme_url: None,
            enabled_claude: true,
            enabled_codex: false,
            enabled_gemini: false,
            enabled_opencode: false,
            enabled_hermes: false,
            installed_at: "now".to_string(),
            updated_at: "now".to_string(),
            content_hash: hash,
            created_at: "now".to_string(),
        };

        copy_dir_atomic(&source, &target, &live_root, Some(&row)).unwrap();
        assert!(target.join(MARKER_FILE).is_file());
        std::fs::write(source.join(ENTRY_FILE), "# changed\n").unwrap();
        copy_dir_atomic(&source, &target, &live_root, Some(&row)).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join(ENTRY_FILE)).unwrap(),
            "# changed\n"
        );
    }
}
