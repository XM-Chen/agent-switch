use rusqlite::{params, Connection};
use std::sync::Mutex;

/// 工具接管状态行。
#[derive(Debug, Clone)]
pub struct ToolTakeoverRow {
    pub enabled: bool,
    /// 'proxy' | 'direct'（v8 新增）。
    pub mode: String,
    /// direct 模式下激活的 provider id（v8 新增）。
    pub active_provider_id: Option<String>,
    pub last_applied_at: Option<String>,
    pub last_target: Option<String>,
    pub last_error: Option<String>,
}

/// 工具接管备份记录行。
#[derive(Debug, Clone)]
pub struct ToolBackupRow {
    pub id: String,
    pub original_path: String,
    pub backup_path: String,
    pub original_existed: bool,
    pub takeover_target: Option<String>,
    pub created_at: String,
}

/// 获取指定工具的接管状态。
pub fn get_state(db: &Mutex<Connection>, tool: &str) -> Result<Option<ToolTakeoverRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT enabled, mode, active_provider_id, last_applied_at, last_target, last_error FROM tool_takeover WHERE tool = ?1")
        .map_err(|e| format!("查询接管状态失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![tool], |row| {
            Ok(ToolTakeoverRow {
                enabled: row.get::<_, i64>("enabled")? != 0,
                mode: row.get("mode")?,
                active_provider_id: row.get("active_provider_id")?,
                last_applied_at: row.get("last_applied_at")?,
                last_target: row.get("last_target")?,
                last_error: row.get("last_error")?,
            })
        })
        .map_err(|e| format!("读取接管状态失败: {}", e))?;
    rows.next()
        .transpose()
        .map_err(|e| format!("接管状态行解析失败: {}", e))
}

/// 插入或更新工具接管状态。
///
/// `mode` / `active_provider_id` 为 v8 双模式字段；旧调用方传 "proxy" / None 即可保持原行为。
///
/// 参数数量对应接管状态行的完整字段集，写整行时逐一显式传入比包一层结构体更直观。
#[allow(clippy::too_many_arguments)]
pub fn upsert_state(
    db: &Mutex<Connection>,
    tool: &str,
    enabled: bool,
    mode: &str,
    active_provider_id: Option<&str>,
    last_applied_at: Option<&str>,
    last_target: Option<&str>,
    last_error: Option<&str>,
) -> Result<(), String> {
    let now = iso_now()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute(
        "INSERT INTO tool_takeover (tool, enabled, mode, active_provider_id, last_applied_at, last_target, last_error, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(tool) DO UPDATE SET
           enabled=excluded.enabled,
           mode=excluded.mode,
           active_provider_id=excluded.active_provider_id,
           last_applied_at=excluded.last_applied_at,
           last_target=excluded.last_target,
           last_error=excluded.last_error,
           updated_at=excluded.updated_at",
        params![
            tool,
            enabled as i64,
            mode,
            active_provider_id,
            last_applied_at,
            last_target,
            last_error,
            now
        ],
    )
    .map_err(|e| format!("更新接管状态失败: {}", e))?;
    Ok(())
}

/// 仅切换模式与激活 provider，不动 enabled（用于 direct↔proxy 切换）。
/// 子任务 3 的切换 API 接线前暂未被引用。
#[allow(dead_code)]
pub fn set_mode(
    db: &Mutex<Connection>,
    tool: &str,
    mode: &str,
    active_provider_id: Option<&str>,
) -> Result<(), String> {
    let now = iso_now()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute(
        "UPDATE tool_takeover SET mode=?1, active_provider_id=?2, updated_at=?3 WHERE tool=?4",
        params![mode, active_provider_id, now, tool],
    )
    .map_err(|e| format!("更新接管模式失败: {}", e))?;
    Ok(())
}

/// 仅更新启停状态(关闭路径)。
pub fn set_enabled(db: &Mutex<Connection>, tool: &str, enabled: bool) -> Result<(), String> {
    let now = iso_now()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute(
        "UPDATE tool_takeover SET enabled=?1, updated_at=?2 WHERE tool=?3",
        params![enabled as i64, now, tool],
    )
    .map_err(|e| format!("更新开关状态失败: {}", e))?;
    Ok(())
}

/// 插入一条备份记录。
pub fn insert_backup(
    db: &Mutex<Connection>,
    id: &str,
    tool: &str,
    original_path: &str,
    backup_path: &str,
    original_existed: bool,
    takeover_target: Option<&str>,
) -> Result<(), String> {
    let now = iso_now()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute(
        "INSERT INTO tool_takeover_backups (id, tool, original_path, backup_path, original_existed, takeover_target, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, tool, original_path, backup_path, original_existed as i64, takeover_target, now],
    )
    .map_err(|e| format!("插入备份记录失败: {}", e))?;
    Ok(())
}

/// 列出指定工具的所有备份记录。
pub fn list_backups(db: &Mutex<Connection>, tool: &str) -> Result<Vec<ToolBackupRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare(
            "SELECT id, original_path, backup_path, original_existed, takeover_target, created_at FROM tool_takeover_backups WHERE tool = ?1 ORDER BY created_at DESC",
        )
        .map_err(|e| format!("查询备份记录失败: {}", e))?;
    let rows = stmt
        .query_map(params![tool], |row| {
            Ok(ToolBackupRow {
                id: row.get("id")?,
                original_path: row.get("original_path")?,
                backup_path: row.get("backup_path")?,
                original_existed: row.get::<_, i64>("original_existed")? != 0,
                takeover_target: row.get("takeover_target")?,
                created_at: row.get("created_at")?,
            })
        })
        .map_err(|e| format!("读取备份记录失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("备份记录行解析失败: {}", e))?);
    }
    Ok(out)
}

fn iso_now() -> Result<String, String> {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn setup() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("无法创建内存数据库");
        let db = Mutex::new(conn);
        run_migrations(&db).expect("迁移应成功");
        db
    }

    #[test]
    fn upsert_defaults_mode_proxy() {
        let db = setup();
        upsert_state(&db, "claude-code", true, "proxy", None, None, None, None).unwrap();
        let row = get_state(&db, "claude-code").unwrap().unwrap();
        assert!(row.enabled);
        assert_eq!(row.mode, "proxy");
        assert!(row.active_provider_id.is_none());
    }

    #[test]
    fn upsert_persists_direct_mode_and_provider() {
        let db = setup();
        upsert_state(
            &db,
            "codex",
            true,
            "direct",
            Some("prov-1"),
            None,
            None,
            None,
        )
        .unwrap();
        let row = get_state(&db, "codex").unwrap().unwrap();
        assert_eq!(row.mode, "direct");
        assert_eq!(row.active_provider_id.as_deref(), Some("prov-1"));
    }

    #[test]
    fn set_mode_changes_only_mode_and_provider() {
        let db = setup();
        upsert_state(&db, "claude-code", true, "proxy", None, None, None, None).unwrap();
        set_mode(&db, "claude-code", "direct", Some("prov-2")).unwrap();
        let row = get_state(&db, "claude-code").unwrap().unwrap();
        assert_eq!(row.mode, "direct");
        assert_eq!(row.active_provider_id.as_deref(), Some("prov-2"));
        // enabled 不应被 set_mode 改动
        assert!(row.enabled);
    }

    #[test]
    fn set_enabled_preserves_mode() {
        let db = setup();
        upsert_state(
            &db,
            "claude-code",
            true,
            "direct",
            Some("prov-1"),
            None,
            None,
            None,
        )
        .unwrap();
        set_enabled(&db, "claude-code", false).unwrap();
        let row = get_state(&db, "claude-code").unwrap().unwrap();
        assert!(!row.enabled);
        assert_eq!(row.mode, "direct", "set_enabled 不应清空 mode");
        assert_eq!(row.active_provider_id.as_deref(), Some("prov-1"));
    }
}
