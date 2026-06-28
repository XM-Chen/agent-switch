use rusqlite::{params, Connection};
use std::sync::Mutex;
use time::OffsetDateTime;

/// 模型别名行。
#[derive(Debug, Clone)]
pub struct ModelAliasRow {
    pub id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub alias_name: String,
    pub target_endpoint_id: Option<String>,
    pub target_model_name: String,
    pub priority: i64,
    pub enabled: bool,
    pub invalid_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewModelAlias {
    pub id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub alias_name: String,
    pub target_endpoint_id: Option<String>,
    pub target_model_name: String,
    pub priority: i64,
}

fn now_iso() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}

fn row_to_alias(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelAliasRow> {
    Ok(ModelAliasRow {
        id: row.get("id")?,
        scope_type: row.get("scope_type")?,
        scope_id: row.get("scope_id")?,
        alias_name: row.get("alias_name")?,
        target_endpoint_id: row.get("target_endpoint_id")?,
        target_model_name: row.get("target_model_name")?,
        priority: row.get("priority")?,
        enabled: row.get::<_, i64>("enabled")? != 0,
        invalid_reason: row.get("invalid_reason")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn list(
    db: &Mutex<Connection>,
    scope_type: Option<&str>,
    scope_id: Option<&str>,
) -> Result<Vec<ModelAliasRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut sql = "SELECT * FROM model_aliases WHERE 1=1".to_string();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(st) = scope_type {
        params_vec.push(Box::new(st.to_string()));
        sql.push_str(&format!(" AND scope_type = ?{}", params_vec.len()));
    }
    if let Some(si) = scope_id {
        params_vec.push(Box::new(si.to_string()));
        sql.push_str(&format!(" AND scope_id = ?{}", params_vec.len()));
    }
    sql.push_str(" ORDER BY scope_type, scope_id, alias_name, priority");

    let mut stmt = db
        .prepare(&sql)
        .map_err(|e| format!("查询别名失败: {}", e))?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), row_to_alias)
        .map_err(|e| format!("读取别名失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("别名行解析失败: {}", e))?);
    }
    Ok(out)
}

pub fn get(db: &Mutex<Connection>, id: &str) -> Result<Option<ModelAliasRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM model_aliases WHERE id = ?1")
        .map_err(|e| format!("查询别名失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_alias)
        .map_err(|e| format!("读取别名失败: {}", e))?;
    rows.next()
        .transpose()
        .map_err(|e| format!("别名行解析失败: {}", e))
}

pub fn create(db: &Mutex<Connection>, new: NewModelAlias) -> Result<ModelAliasRow, String> {
    let now = now_iso()?;
    {
        let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        db.execute(
            "INSERT INTO model_aliases (id, scope_type, scope_id, alias_name, target_endpoint_id, target_model_name, priority, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)",
            params![
                new.id, new.scope_type, new.scope_id, new.alias_name,
                new.target_endpoint_id, new.target_model_name, new.priority, now,
            ],
        )
        .map_err(|e| format!("创建别名失败: {}", e))?;
    }
    get(db, &new.id)?.ok_or_else(|| "创建后无法读取别名".to_string())
}

pub fn delete(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute("DELETE FROM model_aliases WHERE id = ?1", params![id])
        .map_err(|e| format!("删除别名失败: {}", e))?;
    Ok(())
}
