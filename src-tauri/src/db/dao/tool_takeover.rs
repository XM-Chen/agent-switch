use rusqlite::{params, Connection};
use std::sync::Mutex;

/// 工具接管状态行。
#[derive(Debug, Clone)]
pub struct ToolTakeoverRow {
    pub enabled: bool,
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
        .prepare("SELECT enabled, last_applied_at, last_target, last_error FROM tool_takeover WHERE tool = ?1")
        .map_err(|e| format!("查询接管状态失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![tool], |row| {
            Ok(ToolTakeoverRow {
                enabled: row.get::<_, i64>("enabled")? != 0,
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
pub fn upsert_state(
    db: &Mutex<Connection>,
    tool: &str,
    enabled: bool,
    last_applied_at: Option<&str>,
    last_target: Option<&str>,
    last_error: Option<&str>,
) -> Result<(), String> {
    let now = iso_now()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute(
        "INSERT INTO tool_takeover (tool, enabled, last_applied_at, last_target, last_error, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(tool) DO UPDATE SET
           enabled=excluded.enabled,
           last_applied_at=excluded.last_applied_at,
           last_target=excluded.last_target,
           last_error=excluded.last_error,
           updated_at=excluded.updated_at",
        params![tool, enabled as i64, last_applied_at, last_target, last_error, now],
    )
    .map_err(|e| format!("更新接管状态失败: {}", e))?;
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
