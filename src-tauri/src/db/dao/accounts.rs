use rusqlite::{params, Connection};
use std::sync::Mutex;

use super::now_iso;

/// 账号行（数据库原始表示，含加密 BLOB）。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AccountRow {
    pub id: String,
    pub name: String,
    pub account_type: String,
    pub platform: String,
    pub status: String,
    pub credentials_encrypted: Option<Vec<u8>>,
    pub extra_json: Option<String>,
    pub priority: i64,
    pub last_login_at: Option<String>,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建账号的输入。
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub id: String,
    pub name: String,
    pub account_type: String,
    pub platform: String,
    pub credentials_encrypted: Option<Vec<u8>>,
    pub extra_json: Option<String>,
    pub priority: i64,
}

/// 更新账号的输入（部分字段）。
/// 嵌套 Option 用于区分"不更新"与"更新为 NULL"。
#[derive(Debug, Clone, Default)]
pub struct AccountUpdate {
    pub name: Option<String>,
    pub status: Option<String>,
    pub credentials_encrypted: Option<Option<Vec<u8>>>,
    pub extra_json: Option<Option<String>>,
    pub priority: Option<i64>,
    pub last_login_at: Option<Option<String>>,
    pub last_error: Option<Option<String>>,
    pub last_error_at: Option<Option<String>>,
}

fn row_to_account(row: &rusqlite::Row<'_>) -> rusqlite::Result<AccountRow> {
    Ok(AccountRow {
        id: row.get("id")?,
        name: row.get("name")?,
        account_type: row.get("account_type")?,
        platform: row.get("platform")?,
        status: row.get("status")?,
        credentials_encrypted: row.get("credentials_encrypted")?,
        extra_json: row.get("extra_json")?,
        priority: row.get("priority")?,
        last_login_at: row.get("last_login_at")?,
        last_error: row.get("last_error")?,
        last_error_at: row.get("last_error_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn list(db: &Mutex<Connection>) -> Result<Vec<AccountRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM accounts ORDER BY priority ASC, created_at ASC")
        .map_err(|e| format!("查询账号失败: {}", e))?;
    let rows = stmt
        .query_map([], row_to_account)
        .map_err(|e| format!("读取账号失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("账号行解析失败: {}", e))?);
    }
    Ok(out)
}

pub fn get(db: &Mutex<Connection>, id: &str) -> Result<Option<AccountRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM accounts WHERE id = ?1")
        .map_err(|e| format!("查询账号失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_account)
        .map_err(|e| format!("读取账号失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| format!("账号行解析失败: {}", e))?))
    } else {
        Ok(None)
    }
}

pub fn create(db: &Mutex<Connection>, new: NewAccount) -> Result<AccountRow, String> {
    let now = now_iso()?;
    {
        let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        db.execute(
            "INSERT INTO accounts (id, name, account_type, platform, status, credentials_encrypted, extra_json, priority, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, ?7, ?8, ?8)",
            params![
                new.id,
                new.name,
                new.account_type,
                new.platform,
                new.credentials_encrypted,
                new.extra_json,
                new.priority,
                now,
            ],
        )
        .map_err(|e| format!("创建账号失败: {}", e))?;
    }
    // 重新查询刚插入的行。
    get(db, &new.id)?.ok_or_else(|| "创建后无法读取账号".to_string())
}

pub fn update(db: &Mutex<Connection>, id: &str, update: AccountUpdate) -> Result<(), String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;

    // 动态构建 UPDATE，只更新提供的字段。
    let mut sets: Vec<String> = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(v) = &update.name {
        sets.push("name = ?".to_string());
        params_vec.push(Box::new(v.clone()));
    }
    if let Some(v) = &update.status {
        sets.push("status = ?".to_string());
        params_vec.push(Box::new(v.clone()));
    }
    if let Some(opt) = &update.credentials_encrypted {
        sets.push("credentials_encrypted = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(opt) = &update.extra_json {
        sets.push("extra_json = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(v) = &update.priority {
        sets.push("priority = ?".to_string());
        params_vec.push(Box::new(*v));
    }
    if let Some(opt) = &update.last_login_at {
        sets.push("last_login_at = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(opt) = &update.last_error {
        sets.push("last_error = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }
    if let Some(opt) = &update.last_error_at {
        sets.push("last_error_at = ?".to_string());
        params_vec.push(Box::new(opt.clone()));
    }

    if sets.is_empty() {
        return Ok(());
    }

    sets.push("updated_at = ?".to_string());
    params_vec.push(Box::new(now));
    params_vec.push(Box::new(id.to_string()));

    let sql = format!("UPDATE accounts SET {} WHERE id = ?", sets.join(", "));
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    db.execute(&sql, param_refs.as_slice())
        .map_err(|e| format!("更新账号失败: {}", e))?;
    Ok(())
}

pub fn delete(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute("DELETE FROM accounts WHERE id = ?1", params![id])
        .map_err(|e| format!("删除账号失败: {}", e))?;
    Ok(())
}
