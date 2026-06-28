use rusqlite::{params, Connection};
use std::sync::Mutex;
use time::OffsetDateTime;

/// app_metadata key-value 表的读写，用于简单设置项（如刷新开关）。
pub fn get(db: &Mutex<Connection>, key: &str) -> Result<Option<String>, String> {
    let conn = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = conn
        .prepare("SELECT value FROM app_metadata WHERE key = ?1")
        .map_err(|e| format!("查询设置失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![key], |row| row.get::<_, String>(0))
        .map_err(|e| format!("读取设置失败: {}", e))?;
    rows.next()
        .transpose()
        .map_err(|e| format!("解析失败: {}", e))
}

pub fn set(db: &Mutex<Connection>, key: &str, value: &str) -> Result<(), String> {
    let now = OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))?;
    let conn = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    conn.execute(
        "INSERT INTO app_metadata (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at",
        params![key, value, now],
    )
    .map_err(|e| format!("写入设置失败: {}", e))?;
    Ok(())
}
