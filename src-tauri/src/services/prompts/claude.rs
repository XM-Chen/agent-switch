//! Claude Code Prompts 同步（cc-prompts，仅 Claude Code）。
//!
//! 写 `~/.claude/CLAUDE.md`：单激活投影 DB 里 `enabled_claude=1` 的 prompt 内容
//! （至多一份）。启用前做**回填保护**捕获 live 手改（避免用户直接改 CLAUDE.md 的内容
//! 被覆盖丢失）。移植 ccs `services/prompt.rs` enable_prompt 回填模型，按 agent-switch
//! 风格改写（`Result<_, String>`，复用 `tool_takeover::atomic_write`）。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::db::dao::now_iso;
use crate::db::dao::prompts::{self, NewPrompt};
use crate::services::tool_takeover::atomic_write;

/// `~/.claude/CLAUDE.md` 路径（固定 home 下，不做 override_dir）。
pub fn claude_md_path() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|h| h.join(".claude").join("CLAUDE.md"))
        .ok_or_else(|| "无法获取用户主目录".to_string())
}

/// Claude 是否已安装：CLAUDE.md 存在，或其父目录 `~/.claude` 存在。
///
/// 父目录由 path 自身 parent 推导（`~/.claude/CLAUDE.md` 的 parent = `~/.claude`），
/// 生产语义正确、测试用临时目录时天然隔离（不看真实 home，hermetic）。
fn should_sync_for_path(path: &Path) -> bool {
    if path.exists() {
        return true;
    }
    // CLAUDE.md 的 parent 就是 .claude 目录（与 MCP 的「兄弟目录」不同，这里是父目录）。
    path.parent().map(|p| p.exists()).unwrap_or(false)
}

// ── 读写 live `~/.claude/CLAUDE.md` ──────────────────────────────────────────

fn read_live_content(path: &Path) -> String {
    if !path.exists() {
        return String::new();
    }
    std::fs::read_to_string(path).unwrap_or_default()
}

fn write_live_content(path: &Path, content: &str) -> Result<(), String> {
    atomic_write(path, content.as_bytes())
}

// ── 单激活 + 回填保护 + 投影 ──────────────────────────────────────────────────

/// 启用目标 prompt：回填保护 + 单激活 + 投影写入 live。
///
/// 回填保护两分支（对齐 ccs `enable_prompt`）：
/// - 若 DB 有已启用 prompt → 把 live 内容回填进「当前已启用」那份再切换。
/// - 若 DB 无已启用 prompt 且 live 内容未在任何 prompt 中出现 → 建一份 `backup-<ts>`
///   （`enabled_claude=0`）保存 live 原文（避免重复备份：内容已存在则跳过）。
///
/// 回填后 `set_enabled_exclusive(target_id)`（同事务：目标置 1、其余置 0），再投影写 live。
/// Claude 未安装（`~/.claude` 不存在）时投影 no-op 不建文件。
pub fn enable_prompt(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let path = claude_md_path()?;
    enable_prompt_at(db, id, &path)
}

/// `enable_prompt` 的核心（接收显式 path，便于测试）。
fn enable_prompt_at(db: &Mutex<Connection>, id: &str, path: &Path) -> Result<(), String> {
    // 1. 回填保护：读 live content（存在且非空 trim 时）。
    let live = read_live_content(path);
    if !live.trim().is_empty() {
        let enabled = prompts::get_enabled_claude(db)?;
        match enabled {
            Some(current) => {
                // 分支 A：有已启用 prompt → 把 live 内容回填进该项。
                prompts::update(
                    db,
                    &current.id,
                    prompts::PromptUpdate {
                        content: Some(live.clone()),
                        ..Default::default()
                    },
                )?;
            }
            None => {
                // 分支 B：无已启用 prompt → 检查 live 内容是否已在某 prompt 中存在。
                let all = prompts::list(db)?;
                let already_exists = all.iter().any(|p| p.content == live);
                if !already_exists {
                    // live 内容未在任何 prompt 中 → 建一份 backup。
                    let ts = now_iso()?;
                    let safe_ts = ts.replace(':', "-");
                    let backup_name = format!("原始提示词 {}", safe_ts);
                    prompts::create(
                        db,
                        NewPrompt {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: backup_name,
                            content: live.clone(),
                            description: Some("从现有配置自动备份".to_string()),
                            enabled_claude: false,
                        },
                    )?;
                }
            }
        }
    }

    // 2. 单激活：目标置 1、其余置 0（同事务，DAO 已保证）。
    prompts::set_enabled_exclusive(db, id)?;

    // 3. 投影：Claude 已安装时把目标 content 写入 live。
    if should_sync_for_path(path) {
        let target = prompts::get(db, id)?.ok_or_else(|| format!("prompt '{}' 不存在", id))?;
        write_live_content(path, &target.content)?;
    }
    Ok(())
}

/// 禁用目标 prompt：置 `enabled_claude=0`；若已无任何启用项且 Claude 已安装 → 清空 live。
pub fn disable_prompt(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let path = claude_md_path()?;
    disable_prompt_at(db, id, &path)
}

/// `disable_prompt` 的核心（接收显式 path，便于测试）。
fn disable_prompt_at(db: &Mutex<Connection>, id: &str, path: &Path) -> Result<(), String> {
    // 置目标为 0（不检查是否真的是启用态，幂等）。
    prompts::update(
        db,
        id,
        prompts::PromptUpdate {
            enabled_claude: Some(false),
            ..Default::default()
        },
    )?;

    // 若已无任何启用项 → 清空 live（写空串，对齐 ccs；文件存在时）。
    let enabled = prompts::get_enabled_claude(db)?;
    if enabled.is_none() && should_sync_for_path(path) {
        write_live_content(path, "")?;
    }
    Ok(())
}

/// 删除 prompt：`enabled_claude=1` 的拒删（对齐 ccs）。
pub fn delete_prompt(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let row = prompts::get(db, id)?.ok_or_else(|| format!("prompt '{}' 不存在", id))?;
    if row.enabled_claude {
        return Err("不能删除已启用的 prompt，请先禁用".to_string());
    }
    prompts::delete(db, id)
}

// ── 反向导入 + 首次自动导入 ────────────────────────────────────────────────────

/// 从 live `~/.claude/CLAUDE.md` 反向导入到 DB。
///
/// live 非空 → 导入为一份新 prompt（`enabled_claude=0`，name 带时间戳）。
/// 空/不存在 → no-op（report 标注）。
pub fn import_from_claude(db: &Mutex<Connection>) -> Result<ImportReport, String> {
    let path = claude_md_path()?;
    import_from_path(db, &path)
}

/// `import_from_claude` 的核心（接收显式 path，便于测试）。
fn import_from_path(db: &Mutex<Connection>, path: &Path) -> Result<ImportReport, String> {
    let content = read_live_content(path);
    if content.trim().is_empty() {
        return Ok(ImportReport::default());
    }
    let ts = now_iso()?;
    let safe_ts = ts.replace(':', "-");
    let name = format!("导入的提示词 {}", safe_ts);
    prompts::create(
        db,
        NewPrompt {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            content,
            description: Some("从现有配置文件导入".to_string()),
            enabled_claude: false,
        },
    )?;
    Ok(ImportReport { imported: 1 })
}

/// 首次启动自动导入（幂等：DB 非空跳过）。
///
/// DB 已有任意 prompt → 返回 0 跳过；DB 空且 live `CLAUDE.md` 非空 → 导入为一份
/// `enabled_claude=1`（首次自动启用，对齐 ccs `import_from_file_on_first_launch`）。
pub fn import_on_first_launch(db: &Mutex<Connection>) -> Result<usize, String> {
    let path = claude_md_path()?;
    import_on_first_launch_at(db, &path)
}

/// `import_on_first_launch` 的核心（接收显式 path，便于测试）。
fn import_on_first_launch_at(db: &Mutex<Connection>, path: &Path) -> Result<usize, String> {
    // 幂等：DB 非空 → 跳过。
    let all = prompts::list(db)?;
    if !all.is_empty() {
        return Ok(0);
    }
    // DB 空 + live 非空 → 导入并启用。
    let content = read_live_content(path);
    if content.trim().is_empty() {
        return Ok(0);
    }
    let ts = now_iso()?;
    let safe_ts = ts.replace(':', "-");
    let name = format!("导入的提示词 {}", safe_ts);
    prompts::create(
        db,
        NewPrompt {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            content,
            description: Some("首次启动自动导入".to_string()),
            enabled_claude: true, // 首次自动启用
        },
    )?;
    Ok(1)
}

/// live `~/.claude/CLAUDE.md` 的 Prompts 状态摘要。
#[derive(Debug, Clone, serde::Serialize)]
pub struct PromptStatus {
    pub config_path: String,
    pub config_exists: bool,
    pub active_prompt_id: Option<String>,
}

pub fn get_status(db: &Mutex<Connection>) -> Result<PromptStatus, String> {
    let path = claude_md_path()?;
    let enabled = prompts::get_enabled_claude(db)?;
    Ok(PromptStatus {
        config_path: path.to_string_lossy().to_string(),
        config_exists: path.exists(),
        active_prompt_id: enabled.map(|p| p.id),
    })
}

/// 反向导入结果。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ImportReport {
    pub imported: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn setup_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        let db = Mutex::new(conn);
        run_migrations(&db).expect("迁移失败");
        db
    }

    fn unique_dir(tag: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "as-prompts-test-{}-{}-{}",
            tag,
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        dir
    }

    fn make_prompt(db: &Mutex<Connection>, id: &str, content: &str, enabled: bool) {
        prompts::create(
            db,
            NewPrompt {
                id: id.to_string(),
                name: format!("p-{}", id),
                content: content.to_string(),
                description: None,
                enabled_claude: enabled,
            },
        )
        .unwrap();
    }

    // ── 单激活投影 ─────────────────────────────────────────────────────────────

    #[test]
    fn enable_projects_and_switches() {
        let db = setup_db();
        let dir = unique_dir("enable");
        let path = dir.join("CLAUDE.md");
        // 模拟 Claude 已安装：建 .claude 目录（path 的 parent）。
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        make_prompt(&db, "a", "# A\n", false);
        make_prompt(&db, "b", "# B\n", false);

        // 启用 A → live == A.content。
        enable_prompt_at(&db, "a", &path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# A\n");
        assert!(prompts::get(&db, "a").unwrap().unwrap().enabled_claude);

        // 启用 B → live == B.content 且 A.enabled=false。
        enable_prompt_at(&db, "b", &path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# B\n");
        assert!(!prompts::get(&db, "a").unwrap().unwrap().enabled_claude);
        assert!(prompts::get(&db, "b").unwrap().unwrap().enabled_claude);
    }

    // ── 回填保护（有已启用项分支）────────────────────────────────────────────────

    #[test]
    fn enable_backfills_live_into_current_enabled() {
        let db = setup_db();
        let dir = unique_dir("backfill-current");
        let path = dir.join("CLAUDE.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        make_prompt(&db, "a", "# A\n", true);
        make_prompt(&db, "b", "# B\n", false);

        // 写 A → live，用户手改。
        enable_prompt_at(&db, "a", &path).unwrap();
        std::fs::write(&path, "# A modified\n").unwrap();

        // 启用 B（prev=A）→ 回填捕获 A 的手改。
        enable_prompt_at(&db, "b", &path).unwrap();
        let a_row = prompts::get(&db, "a").unwrap().unwrap();
        assert_eq!(a_row.content, "# A modified\n", "回填应捕获 live 手改");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# B\n");
    }

    // ── 回填保护（无已启用项备份分支）────────────────────────────────────────────

    #[test]
    fn enable_creates_backup_when_no_enabled_and_content_not_in_db() {
        let db = setup_db();
        let dir = unique_dir("backfill-backup");
        let path = dir.join("CLAUDE.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // 预置 live 有用户既有内容，DB 无已启用项且内容未在任何 prompt 中。
        std::fs::write(&path, "# user existing\n").unwrap();
        make_prompt(&db, "a", "# A\n", false);

        enable_prompt_at(&db, "a", &path).unwrap();

        // 应新建一份 backup prompt（enabled=false）保存 live 原文。
        let all = prompts::list(&db).unwrap();
        let backup = all.iter().find(|p| p.content == "# user existing\n");
        assert!(backup.is_some(), "应建 backup 保存 live 原文");
        assert!(backup
            .map(|p| !p.enabled_claude && p.name.contains("原始提示词"))
            .unwrap_or(false));
    }

    #[test]
    fn enable_skips_backup_when_content_already_exists() {
        let db = setup_db();
        let dir = unique_dir("backfill-dup");
        let path = dir.join("CLAUDE.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // DB 已有内容 = live 原文，DB 无已启用项。
        std::fs::write(&path, "# X\n").unwrap();
        make_prompt(&db, "x", "# X\n", false);
        make_prompt(&db, "a", "# A\n", false);

        enable_prompt_at(&db, "a", &path).unwrap();

        // 不应重复建 backup（内容已在 x 中存在）。
        let all = prompts::list(&db).unwrap();
        assert_eq!(all.len(), 2, "不应重复备份已存在的内容");
    }

    // ── 禁用清空 ──────────────────────────────────────────────────────────────

    #[test]
    fn disable_clears_live_when_last_enabled() {
        let db = setup_db();
        let dir = unique_dir("disable");
        let path = dir.join("CLAUDE.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        make_prompt(&db, "a", "# A\n", true);
        enable_prompt_at(&db, "a", &path).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().is_empty());

        // 禁用唯一激活项 → live 清空。
        disable_prompt_at(&db, "a", &path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    }

    // ── 删除保护 ──────────────────────────────────────────────────────────────

    #[test]
    fn delete_rejects_enabled_prompt() {
        let db = setup_db();
        make_prompt(&db, "a", "# A\n", true);
        let err = delete_prompt(&db, "a").unwrap_err();
        assert!(err.contains("已启用"), "{}", err);
    }

    #[test]
    fn delete_allows_disabled_prompt() {
        let db = setup_db();
        make_prompt(&db, "a", "# A\n", false);
        delete_prompt(&db, "a").unwrap();
        assert!(prompts::get(&db, "a").unwrap().is_none());
    }

    // ── 反向导入 ──────────────────────────────────────────────────────────────

    #[test]
    fn import_picks_up_live_content() {
        let db = setup_db();
        let dir = unique_dir("import");
        let path = dir.join("CLAUDE.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# live\n").unwrap();

        let report = import_from_path(&db, &path).unwrap();
        assert_eq!(report.imported, 1);
        let all = prompts::list(&db).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].content, "# live\n");
        assert!(!all[0].enabled_claude, "导入的默认禁用");
    }

    #[test]
    fn import_missing_file_is_noop() {
        let db = setup_db();
        let dir = unique_dir("import-missing");
        let path = dir.join("CLAUDE.md");
        let report = import_from_path(&db, &path).unwrap();
        assert_eq!(report.imported, 0);
    }

    // ── 首次自动导入幂等 ──────────────────────────────────────────────────────

    #[test]
    fn import_on_first_launch_when_db_empty_and_live_nonempty() {
        let db = setup_db();
        let dir = unique_dir("first");
        let path = dir.join("CLAUDE.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# first\n").unwrap();

        let n = import_on_first_launch_at(&db, &path).unwrap();
        assert_eq!(n, 1);
        let all = prompts::list(&db).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].content, "# first\n");
        assert!(all[0].enabled_claude, "首次自动启用");
    }

    #[test]
    fn import_on_first_launch_skips_when_db_nonempty() {
        let db = setup_db();
        let dir = unique_dir("first-skip");
        let path = dir.join("CLAUDE.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# first\n").unwrap();
        make_prompt(&db, "x", "# X\n", false);

        let n = import_on_first_launch_at(&db, &path).unwrap();
        assert_eq!(n, 0, "DB 非空时应跳过");
        assert_eq!(prompts::list(&db).unwrap().len(), 1);
    }

    // ── 未安装跳过 ────────────────────────────────────────────────────────────

    #[test]
    fn enable_skips_when_no_claude_install() {
        let db = setup_db();
        // 用一个不存在的父目录做 path，模拟 ~/.claude 未安装：CLAUDE.md 与其父都不存在。
        let path = std::env::temp_dir().join(format!(
            "as-prompts-no-install-{}-nonexistent/CLAUDE.md",
            std::process::id()
        ));
        make_prompt(&db, "a", "# A\n", false);
        enable_prompt_at(&db, "a", &path).unwrap();
        assert!(!path.exists(), "Claude 未安装时不应凭空建文件");
    }

    #[test]
    fn disable_skips_when_no_claude_install() {
        let db = setup_db();
        let path = std::env::temp_dir().join(format!(
            "as-prompts-no-install-disable-{}-nonexistent/CLAUDE.md",
            std::process::id()
        ));
        make_prompt(&db, "a", "# A\n", true);
        disable_prompt_at(&db, "a", &path).unwrap();
        assert!(!path.exists(), "Claude 未安装时 disable 不应建文件");
    }
}
