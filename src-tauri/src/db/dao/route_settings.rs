/// 路由设置 DAO。
///
/// 对应设计文档 §2 数据模型 `route_settings` 表。
/// 每条路由（claude-code / codex）一行，存储选择策略与故障转移参数。
use rusqlite::{params, Connection};
use std::sync::Mutex;
use time::OffsetDateTime;

/// 路由设置行。
#[derive(Debug, Clone)]
pub struct RouteSettingsRow {
    pub id: String,
    pub label: String,
    pub strategy: String,
    pub protocol_type: String,
    pub failover_enabled: bool,
    pub max_switches: i64,
    pub same_account_retries: i64,
    pub cooldown_multiplier: f64,
    pub updated_at: String,
}

fn now_iso() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}

fn row_to_settings(row: &rusqlite::Row<'_>) -> rusqlite::Result<RouteSettingsRow> {
    Ok(RouteSettingsRow {
        id: row.get("id")?,
        label: row.get("label")?,
        strategy: row.get("strategy")?,
        protocol_type: row.get("protocol_type")?,
        failover_enabled: row.get::<_, i64>("failover_enabled")? != 0,
        max_switches: row.get("max_switches")?,
        same_account_retries: row.get("same_account_retries")?,
        cooldown_multiplier: row.get("cooldown_multiplier")?,
        updated_at: row.get("updated_at")?,
    })
}

/// 获取指定路由的设置。
pub fn get(db: &Mutex<Connection>, id: &str) -> Result<Option<RouteSettingsRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM route_settings WHERE id = ?1")
        .map_err(|e| format!("查询路由设置失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_settings)
        .map_err(|e| format!("读取路由设置失败: {}", e))?;
    rows.next()
        .transpose()
        .map_err(|e| format!("路由设置行解析失败: {}", e))
}

/// 列出所有路由设置。
pub fn list_all(db: &Mutex<Connection>) -> Result<Vec<RouteSettingsRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM route_settings ORDER BY id")
        .map_err(|e| format!("查询路由设置列表失败: {}", e))?;
    let rows = stmt
        .query_map([], row_to_settings)
        .map_err(|e| format!("读取路由设置列表失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("路由设置行解析失败: {}", e))?);
    }
    Ok(out)
}

/// 插入或更新一条路由设置（全字段 upsert）。
pub fn upsert(
    db: &Mutex<Connection>,
    id: &str,
    label: &str,
    strategy: &str,
    protocol_type: &str,
    failover_enabled: bool,
    max_switches: i64,
    same_account_retries: i64,
    cooldown_multiplier: f64,
) -> Result<(), String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute(
        "INSERT INTO route_settings (id, label, strategy, protocol_type, failover_enabled, max_switches, same_account_retries, cooldown_multiplier, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
           label=excluded.label,
           strategy=excluded.strategy,
           protocol_type=excluded.protocol_type,
           failover_enabled=excluded.failover_enabled,
           max_switches=excluded.max_switches,
           same_account_retries=excluded.same_account_retries,
           cooldown_multiplier=excluded.cooldown_multiplier,
           updated_at=excluded.updated_at",
        params![
            id,
            label,
            strategy,
            protocol_type,
            failover_enabled as i64,
            max_switches,
            same_account_retries,
            cooldown_multiplier,
            now,
        ],
    )
    .map_err(|e| format!("更新路由设置失败: {}", e))?;
    Ok(())
}

/// 部分字段更新路由设置（只传 Some 的字段）。
pub fn upsert_partial(
    db: &Mutex<Connection>,
    id: &str,
    label: Option<&str>,
    strategy: Option<&str>,
    protocol_type: Option<&str>,
    failover_enabled: Option<bool>,
    max_switches: Option<i64>,
    same_account_retries: Option<i64>,
    cooldown_multiplier: Option<f64>,
) -> Result<(), String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;

    let mut sets: Vec<String> = Vec::new();
    let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(v) = label {
        sets.push("label = ?".to_string());
        param_values.push(Box::new(v.to_string()));
    }
    if let Some(v) = strategy {
        sets.push("strategy = ?".to_string());
        param_values.push(Box::new(v.to_string()));
    }
    if let Some(v) = protocol_type {
        sets.push("protocol_type = ?".to_string());
        param_values.push(Box::new(v.to_string()));
    }
    if let Some(v) = failover_enabled {
        sets.push("failover_enabled = ?".to_string());
        param_values.push(Box::new(if v { 1i64 } else { 0i64 }));
    }
    if let Some(v) = max_switches {
        sets.push("max_switches = ?".to_string());
        param_values.push(Box::new(v));
    }
    if let Some(v) = same_account_retries {
        sets.push("same_account_retries = ?".to_string());
        param_values.push(Box::new(v));
    }
    if let Some(v) = cooldown_multiplier {
        sets.push("cooldown_multiplier = ?".to_string());
        param_values.push(Box::new(v));
    }

    if sets.is_empty() {
        return Ok(());
    }

    sets.push("updated_at = ?".to_string());
    param_values.push(Box::new(now));
    param_values.push(Box::new(id.to_string()));

    let sql = format!(
        "UPDATE route_settings SET {} WHERE id = ?",
        sets.join(", ")
    );
    let param_refs: Vec<&dyn rusqlite::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    db.execute(&sql, param_refs.as_slice())
        .map_err(|e| format!("更新路由设置失败: {}", e))?;

    Ok(())
}
