use rusqlite::{params, Connection};
use std::sync::Mutex;
use time::OffsetDateTime;

/// 端点行（数据库原始表示，含加密 BLOB）。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct EndpointRow {
    pub id: String,
    pub account_id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub protocol_type: String,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub auth_mode: String,
    pub enabled: bool,
    pub priority: i64,
    pub cooldown_until: Option<String>,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_error_kind: Option<String>,
    pub extra_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建端点的输入。
#[derive(Debug, Clone)]
pub struct NewEndpoint {
    pub id: String,
    pub account_id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub protocol_type: String,
    pub api_key_encrypted: Option<Vec<u8>>,
    pub auth_mode: String,
    pub priority: i64,
    pub extra_json: Option<String>,
}

/// 更新端点的输入（部分字段）。
#[derive(Debug, Clone, Default)]
pub struct EndpointUpdate {
    pub account_id: Option<Option<String>>,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub protocol_type: Option<String>,
    pub api_key_encrypted: Option<Option<Vec<u8>>>,
    pub auth_mode: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
    pub cooldown_until: Option<Option<String>>,
    pub last_success_at: Option<Option<String>>,
    pub last_failure_at: Option<Option<String>>,
    pub last_error_kind: Option<Option<String>>,
    pub extra_json: Option<Option<String>>,
}

fn now_iso() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}

fn row_to_endpoint(row: &rusqlite::Row<'_>) -> rusqlite::Result<EndpointRow> {
    Ok(EndpointRow {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        name: row.get("name")?,
        base_url: row.get("base_url")?,
        protocol_type: row.get("protocol_type")?,
        api_key_encrypted: row.get("api_key_encrypted")?,
        auth_mode: row.get("auth_mode")?,
        enabled: row.get::<_, i64>("enabled")? != 0,
        priority: row.get("priority")?,
        cooldown_until: row.get("cooldown_until")?,
        last_success_at: row.get("last_success_at")?,
        last_failure_at: row.get("last_failure_at")?,
        last_error_kind: row.get("last_error_kind")?,
        extra_json: row.get("extra_json")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn list(db: &Mutex<Connection>) -> Result<Vec<EndpointRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM endpoints ORDER BY priority ASC, created_at ASC")
        .map_err(|e| format!("查询端点失败: {}", e))?;
    let rows = stmt
        .query_map([], row_to_endpoint)
        .map_err(|e| format!("读取端点失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("端点行解析失败: {}", e))?);
    }
    Ok(out)
}

pub fn list_enabled(db: &Mutex<Connection>) -> Result<Vec<EndpointRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM endpoints WHERE enabled = 1 ORDER BY priority ASC, created_at ASC")
        .map_err(|e| format!("查询启用端点失败: {}", e))?;
    let rows = stmt
        .query_map([], row_to_endpoint)
        .map_err(|e| format!("读取端点失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("端点行解析失败: {}", e))?);
    }
    Ok(out)
}

pub fn get(db: &Mutex<Connection>, id: &str) -> Result<Option<EndpointRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM endpoints WHERE id = ?1")
        .map_err(|e| format!("查询端点失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_endpoint)
        .map_err(|e| format!("读取端点失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| format!("端点行解析失败: {}", e))?))
    } else {
        Ok(None)
    }
}

pub fn create(db: &Mutex<Connection>, new: NewEndpoint) -> Result<EndpointRow, String> {
    let now = now_iso()?;
    {
        let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        db.execute(
            "INSERT INTO endpoints (id, account_id, name, base_url, protocol_type, api_key_encrypted, auth_mode, enabled, priority, extra_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10, ?10)",
            params![
                new.id,
                new.account_id,
                new.name,
                new.base_url,
                new.protocol_type,
                new.api_key_encrypted,
                new.auth_mode,
                new.priority,
                new.extra_json,
                now,
            ],
        )
        .map_err(|e| format!("创建端点失败: {}", e))?;
    }
    get(db, &new.id)?.ok_or_else(|| "创建后无法读取端点".to_string())
}

pub fn update(db: &Mutex<Connection>, id: &str, update: EndpointUpdate) -> Result<(), String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;

    let mut sets: Vec<String> = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(opt) = &update.account_id {
        sets.push("account_id = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(v) = &update.name {
        sets.push("name = ?".to_string());
        params_vec.push(Box::new(v.clone()));
    }
    if let Some(v) = &update.base_url {
        sets.push("base_url = ?".to_string());
        params_vec.push(Box::new(v.clone()));
    }
    if let Some(v) = &update.protocol_type {
        sets.push("protocol_type = ?".to_string());
        params_vec.push(Box::new(v.clone()));
    }
    if let Some(opt) = &update.api_key_encrypted {
        sets.push("api_key_encrypted = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(v) = &update.auth_mode {
        sets.push("auth_mode = ?".to_string());
        params_vec.push(Box::new(v.clone()));
    }
    if let Some(v) = &update.enabled {
        sets.push("enabled = ?".to_string());
        params_vec.push(Box::new(if *v { 1i64 } else { 0i64 }));
    }
    if let Some(v) = &update.priority {
        sets.push("priority = ?".to_string());
        params_vec.push(Box::new(*v));
    }
    if let Some(opt) = &update.cooldown_until {
        sets.push("cooldown_until = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(opt) = &update.last_success_at {
        sets.push("last_success_at = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(opt) = &update.last_failure_at {
        sets.push("last_failure_at = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(opt) = &update.last_error_kind {
        sets.push("last_error_kind = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(opt) = &update.extra_json {
        sets.push("extra_json = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }

    if sets.is_empty() {
        return Ok(());
    }

    sets.push("updated_at = ?".to_string());
    params_vec.push(Box::new(now));
    params_vec.push(Box::new(id.to_string()));

    let sql = format!("UPDATE endpoints SET {} WHERE id = ?", sets.join(", "));
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    db.execute(&sql, param_refs.as_slice())
        .map_err(|e| format!("更新端点失败: {}", e))?;
    Ok(())
}

pub fn delete(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute("DELETE FROM endpoints WHERE id = ?1", params![id])
        .map_err(|e| format!("删除端点失败: {}", e))?;
    Ok(())
}
