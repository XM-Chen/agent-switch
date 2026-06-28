use rusqlite::{params, Connection};
use std::sync::Mutex;
use time::OffsetDateTime;

/// 端点模型行。
#[derive(Debug, Clone)]
pub struct EndpointModelRow {
    pub id: String,
    pub endpoint_id: String,
    pub model_name: String,
    pub display_name: String,
    pub source: String,
    pub capabilities: Option<String>,
    pub context_window: Option<i64>,
    pub is_available: bool,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewEndpointModel {
    pub id: String,
    pub endpoint_id: String,
    pub model_name: String,
    pub display_name: String,
    pub source: String,
    pub capabilities: Option<String>,
    pub context_window: Option<i64>,
    pub last_seen_at: Option<String>,
}

fn now_iso() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}

fn row_to_model(row: &rusqlite::Row<'_>) -> rusqlite::Result<EndpointModelRow> {
    Ok(EndpointModelRow {
        id: row.get("id")?,
        endpoint_id: row.get("endpoint_id")?,
        model_name: row.get("model_name")?,
        display_name: row.get("display_name")?,
        source: row.get("source")?,
        capabilities: row.get("capabilities")?,
        context_window: row.get("context_window")?,
        is_available: row.get::<_, i64>("is_available")? != 0,
        last_seen_at: row.get("last_seen_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn list(
    db: &Mutex<Connection>,
    endpoint_id: Option<&str>,
    source: Option<&str>,
    capability: Option<&str>,
) -> Result<Vec<EndpointModelRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut sql = "SELECT * FROM endpoint_models WHERE 1=1".to_string();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(eid) = endpoint_id {
        params_vec.push(Box::new(eid.to_string()));
        sql.push_str(&format!(" AND endpoint_id = ?{}", params_vec.len()));
    }
    if let Some(src) = source {
        params_vec.push(Box::new(src.to_string()));
        sql.push_str(&format!(" AND source = ?{}", params_vec.len()));
    }
    if let Some(cap) = capability {
        params_vec.push(Box::new(format!("%{}%", cap)));
        sql.push_str(&format!(" AND capabilities LIKE ?{}", params_vec.len()));
    }
    sql.push_str(" ORDER BY endpoint_id, model_name");

    let mut stmt = db
        .prepare(&sql)
        .map_err(|e| format!("查询模型失败: {}", e))?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), row_to_model)
        .map_err(|e| format!("读取模型失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("模型行解析失败: {}", e))?);
    }
    Ok(out)
}

pub fn get(db: &Mutex<Connection>, id: &str) -> Result<Option<EndpointModelRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM endpoint_models WHERE id = ?1")
        .map_err(|e| format!("查询模型失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_model)
        .map_err(|e| format!("读取模型失败: {}", e))?;
    rows.next()
        .transpose()
        .map_err(|e| format!("模型行解析失败: {}", e))
}

pub fn create(db: &Mutex<Connection>, new: NewEndpointModel) -> Result<EndpointModelRow, String> {
    let now = now_iso()?;
    {
        let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        db.execute(
            "INSERT INTO endpoint_models (id, endpoint_id, model_name, display_name, source, capabilities, context_window, is_available, last_seen_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?9)",
            params![
                new.id, new.endpoint_id, new.model_name, new.display_name,
                new.source, new.capabilities, new.context_window, new.last_seen_at, now,
            ],
        )
        .map_err(|e| format!("创建模型失败: {}", e))?;
    }
    get(db, &new.id)?.ok_or_else(|| "创建后无法读取模型".to_string())
}

pub fn upsert_synced(
    db: &Mutex<Connection>,
    new: NewEndpointModel,
) -> Result<EndpointModelRow, String> {
    let now = now_iso()?;
    {
        let conn = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        conn.execute(
            "INSERT INTO endpoint_models (id, endpoint_id, model_name, display_name, source, capabilities, context_window, is_available, last_seen_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'synced', ?5, ?6, 1, ?7, ?8, ?8)
             ON CONFLICT(endpoint_id, model_name) DO UPDATE SET
               display_name=excluded.display_name,
               capabilities=excluded.capabilities,
               context_window=excluded.context_window,
               is_available=1,
               last_seen_at=excluded.last_seen_at,
               updated_at=excluded.updated_at",
            params![
                new.id, new.endpoint_id, new.model_name, new.display_name,
                new.capabilities, new.context_window, new.last_seen_at, now,
            ],
        )
        .map_err(|e| format!("upsert 模型失败: {}", e))?;
    }
    // 重新查询刚 upsert 的行。
    let conn = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = conn
        .prepare("SELECT * FROM endpoint_models WHERE endpoint_id = ?1 AND model_name = ?2")
        .map_err(|e| format!("查询失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![new.endpoint_id, new.model_name], row_to_model)
        .map_err(|e| format!("读取失败: {}", e))?;
    rows.next()
        .transpose()
        .map_err(|e| format!("解析失败: {}", e))?
        .ok_or_else(|| "upsert 后无法读取模型".to_string())
}

pub fn mark_unavailable_except(
    db: &Mutex<Connection>,
    endpoint_id: &str,
    sync_time: &str,
) -> Result<usize, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let count = db
        .execute(
            "UPDATE endpoint_models SET is_available=0, updated_at=?1
             WHERE endpoint_id=?2 AND source='synced' AND (last_seen_at IS NULL OR last_seen_at < ?3)",
            params![sync_time, endpoint_id, sync_time],
        )
        .map_err(|e| format!("标记模型不可用失败: {}", e))?;
    Ok(count)
}

pub fn mark_alias_invalid_for_model(
    db: &Mutex<Connection>,
    endpoint_id: &str,
    model_name: &str,
) -> Result<usize, String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let count = db
        .execute(
            "UPDATE model_aliases SET enabled=0, invalid_reason=?1, updated_at=?2
             WHERE target_endpoint_id=?3 AND target_model_name=?4 AND enabled=1",
            params![
                format!(
                    "关联模型 '{}' 已从端点 '{}' 中删除",
                    model_name, endpoint_id
                ),
                now,
                endpoint_id,
                model_name,
            ],
        )
        .map_err(|e| format!("标记别名失效失败: {}", e))?;
    Ok(count)
}

pub fn delete(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute("DELETE FROM endpoint_models WHERE id = ?1", params![id])
        .map_err(|e| format!("删除模型失败: {}", e))?;
    Ok(())
}

/// 检查指定端点是否至少有一个可用模型具备给定能力。
///
/// SQL: SELECT COUNT(*) FROM endpoint_models
///       WHERE endpoint_id=? AND is_available=1
///         AND capabilities LIKE '%' || cap || '%'
pub fn has_capable_model(
    db: &Mutex<Connection>,
    endpoint_id: &str,
    capability: &str,
) -> Result<bool, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM endpoint_models WHERE endpoint_id = ?1 AND is_available = 1 AND capabilities LIKE '%' || ?2 || '%'",
            params![endpoint_id, capability],
            |row| row.get(0),
        )
        .map_err(|e| format!("查询端点能力模型失败: {}", e))?;
    Ok(count > 0)
}

/// 查询某端点具备指定能力的模型列表。
pub fn list_capable(
    db: &Mutex<Connection>,
    endpoint_id: &str,
    capability: &str,
) -> Result<Vec<EndpointModelRow>, String> {
    list(db, Some(endpoint_id), None, Some(capability))
}
