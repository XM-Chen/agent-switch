/// 模型级锁 DAO。
///
/// 对应设计文档 §2 数据模型 `model_locks` 表（可选方案，也可用内存集替代）。
/// 锁定键为 `{endpoint_id}_{model}`，用于限制特定端点的特定模型在冷却期内不被选中。
use rusqlite::{params, Connection};
use std::sync::Mutex;
use time::OffsetDateTime;

/// 模型锁行。
#[derive(Debug, Clone)]
pub struct ModelLockRow {
    pub id: String,
    pub endpoint_id: String,
    pub model_name: String,
    pub locked_until: String,
    pub lock_reason: Option<String>,
    pub created_at: String,
}

fn now_iso() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}

/// 设置模型级锁。
///
/// 生成 id = `{endpoint_id}_{model}`，存在则覆盖。
pub fn set_lock(
    db: &Mutex<Connection>,
    endpoint_id: &str,
    model_name: &str,
    locked_until: &str,
    lock_reason: Option<&str>,
) -> Result<(), String> {
    let now = now_iso()?;
    let id = format!("{}_{}", endpoint_id, model_name);
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute(
        "INSERT INTO model_locks (id, endpoint_id, model_name, locked_until, lock_reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
           locked_until=excluded.locked_until,
           lock_reason=excluded.lock_reason,
           created_at=excluded.created_at",
        params![id, endpoint_id, model_name, locked_until, lock_reason, now],
    )
    .map_err(|e| format!("设置模型锁失败: {}", e))?;
    Ok(())
}

/// 获取活跃的模型锁。
///
/// 返回 `locked_until > now` 的锁记录（即尚未过期的）。
pub fn get_active_lock(
    db: &Mutex<Connection>,
    endpoint_id: &str,
    model_name: &str,
) -> Result<Option<ModelLockRow>, String> {
    let now = now_iso()?;
    let id = format!("{}_{}", endpoint_id, model_name);
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM model_locks WHERE id = ?1 AND locked_until > ?2")
        .map_err(|e| format!("查询模型锁失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id, now], |row| {
            Ok(ModelLockRow {
                id: row.get("id")?,
                endpoint_id: row.get("endpoint_id")?,
                model_name: row.get("model_name")?,
                locked_until: row.get("locked_until")?,
                lock_reason: row.get("lock_reason")?,
                created_at: row.get("created_at")?,
            })
        })
        .map_err(|e| format!("读取模型锁失败: {}", e))?;
    rows.next()
        .transpose()
        .map_err(|e| format!("模型锁行解析失败: {}", e))
}

/// 清除所有已过期的模型锁。
pub fn clear_expired(db: &Mutex<Connection>) -> Result<u64, String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let count = db
        .execute(
            "DELETE FROM model_locks WHERE locked_until <= ?1",
            params![now],
        )
        .map_err(|e| format!("清除过期模型锁失败: {}", e))?;
    Ok(count as u64)
}
