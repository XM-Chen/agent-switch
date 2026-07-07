// `mcp_servers` 表 DAO：Claude Code MCP 服务器全局清单（cc-mcp）。
//
// server_config 存纯 MCP 规范（command/args/env 或 type/url/headers），写 live 时
// 原样投影（不再剥离富信息）。description/homepage/docs/tags 仅供 UI 展示。
// enabled_claude 保留 `_claude` 后缀为将来多应用扩展留位（本任务仅 Claude Code）。
#![allow(dead_code)]

use rusqlite::{params, Connection};
use std::sync::Mutex;

use super::now_iso;

/// MCP 服务器行（数据库原始表示）。
#[derive(Debug, Clone)]
pub struct McpServerRow {
    pub id: String,
    pub name: String,
    /// JSON：纯 MCP 规范（command/args/env 或 type/url/headers 等）。
    pub server_config: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub docs: Option<String>,
    /// JSON 数组文本（如 `["filesystem","git"]`），仅供 UI 展示。
    pub tags: String,
    pub enabled_claude: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建 MCP 服务器的输入。
#[derive(Debug, Clone)]
pub struct NewMcpServer {
    pub id: String,
    pub name: String,
    pub server_config: String,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub docs: Option<String>,
    pub tags: String,
    pub enabled_claude: bool,
}

/// 更新 MCP 服务器的输入（部分字段）。
///
/// 嵌套 `Option`：外层 `Some` 表示「更新该字段」，内层区分「更新为 NULL」。
#[derive(Debug, Clone, Default)]
pub struct McpServerUpdate {
    pub name: Option<String>,
    pub server_config: Option<String>,
    pub description: Option<Option<String>>,
    pub homepage: Option<Option<String>>,
    pub docs: Option<Option<String>>,
    pub tags: Option<String>,
    pub enabled_claude: Option<bool>,
}

fn row_to_server(row: &rusqlite::Row<'_>) -> rusqlite::Result<McpServerRow> {
    Ok(McpServerRow {
        id: row.get("id")?,
        name: row.get("name")?,
        server_config: row.get("server_config")?,
        description: row.get("description")?,
        homepage: row.get("homepage")?,
        docs: row.get("docs")?,
        tags: row.get("tags")?,
        enabled_claude: row.get::<_, i64>("enabled_claude")? != 0,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// 列出全部 MCP 服务器，按 name 升序、再按创建时间。
pub fn list(db: &Mutex<Connection>) -> Result<Vec<McpServerRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM mcp_servers ORDER BY name ASC, created_at ASC")
        .map_err(|e| format!("查询 mcp_servers 失败: {}", e))?;
    let rows = stmt
        .query_map([], row_to_server)
        .map_err(|e| format!("读取 mcp_servers 失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("mcp_servers 行解析失败: {}", e))?);
    }
    Ok(out)
}

/// 获取单个 MCP 服务器。
pub fn get(db: &Mutex<Connection>, id: &str) -> Result<Option<McpServerRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM mcp_servers WHERE id = ?1")
        .map_err(|e| format!("查询 mcp_servers 失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_server)
        .map_err(|e| format!("读取 mcp_servers 失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(
            r.map_err(|e| format!("mcp_servers 行解析失败: {}", e))?,
        ))
    } else {
        Ok(None)
    }
}

/// 列出所有 enabled_claude=1 的 MCP 服务器（全量投影同步用）。
pub fn list_enabled_claude(db: &Mutex<Connection>) -> Result<Vec<McpServerRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM mcp_servers WHERE enabled_claude = 1 ORDER BY name ASC")
        .map_err(|e| format!("查询启用 mcp_servers 失败: {}", e))?;
    let rows = stmt
        .query_map([], row_to_server)
        .map_err(|e| format!("读取启用 mcp_servers 失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("mcp_servers 行解析失败: {}", e))?);
    }
    Ok(out)
}

/// 创建 MCP 服务器（返回新建行）。
pub fn create(db: &Mutex<Connection>, new: NewMcpServer) -> Result<McpServerRow, String> {
    let now = now_iso()?;
    {
        let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        db.execute(
            "INSERT INTO mcp_servers
             (id, name, server_config, description, homepage, docs, tags, enabled_claude, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                new.id,
                new.name,
                new.server_config,
                new.description,
                new.homepage,
                new.docs,
                new.tags,
                new.enabled_claude as i64,
                now,
            ],
        )
        .map_err(|e| format!("创建 mcp_server 失败: {}", e))?;
    }
    get(db, &new.id)?.ok_or_else(|| "创建后无法读取 mcp_server".to_string())
}

/// 更新 MCP 服务器部分字段。
pub fn update(db: &Mutex<Connection>, id: &str, upd: McpServerUpdate) -> Result<(), String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;

    let mut sets: Vec<String> = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(name) = upd.name {
        sets.push("name = ?".into());
        params_vec.push(Box::new(name));
    }
    if let Some(server_config) = upd.server_config {
        sets.push("server_config = ?".into());
        params_vec.push(Box::new(server_config));
    }
    if let Some(description) = upd.description {
        sets.push("description = ?".into());
        params_vec.push(Box::new(description));
    }
    if let Some(homepage) = upd.homepage {
        sets.push("homepage = ?".into());
        params_vec.push(Box::new(homepage));
    }
    if let Some(docs) = upd.docs {
        sets.push("docs = ?".into());
        params_vec.push(Box::new(docs));
    }
    if let Some(tags) = upd.tags {
        sets.push("tags = ?".into());
        params_vec.push(Box::new(tags));
    }
    if let Some(enabled) = upd.enabled_claude {
        sets.push("enabled_claude = ?".into());
        params_vec.push(Box::new(enabled as i64));
    }

    if sets.is_empty() {
        return Ok(());
    }

    sets.push("updated_at = ?".into());
    params_vec.push(Box::new(now));

    params_vec.push(Box::new(id.to_string()));
    let sql = format!("UPDATE mcp_servers SET {} WHERE id = ?", sets.join(", "));
    db.execute(&sql, rusqlite::params_from_iter(params_vec))
        .map_err(|e| format!("更新 mcp_server 失败: {}", e))?;
    Ok(())
}

/// 删除 MCP 服务器。
pub fn delete(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute("DELETE FROM mcp_servers WHERE id = ?1", params![id])
        .map_err(|e| format!("删除 mcp_server 失败: {}", e))?;
    Ok(())
}

/// 反向导入用：upsert 单个 MCP 服务器。
///
/// 已存在同 id → 更新规范（`server_config`）并置 `enabled_claude=true`（不覆盖富信息列）。
/// 不存在 → 以 `id` 为 name 新建，`enabled_claude=true`。
pub fn upsert(db: &Mutex<Connection>, new: NewMcpServer) -> Result<McpServerRow, String> {
    let now = now_iso()?;
    {
        let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        let changed = db
            .execute(
                "UPDATE mcp_servers
                 SET server_config = ?2, enabled_claude = 1, updated_at = ?3
                 WHERE id = ?1",
                params![new.id, new.server_config, now],
            )
            .map_err(|e| format!("更新 mcp_server 失败: {}", e))?;
        if changed == 0 {
            db.execute(
                "INSERT INTO mcp_servers
                 (id, name, server_config, description, homepage, docs, tags, enabled_claude, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)",
                params![
                    new.id,
                    new.name,
                    new.server_config,
                    new.description,
                    new.homepage,
                    new.docs,
                    new.tags,
                    now,
                ],
            )
            .map_err(|e| format!("创建 mcp_server 失败: {}", e))?;
        }
    }
    get(db, &new.id)?.ok_or_else(|| "upsert 后无法读取 mcp_server".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn setup() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("创建内存数据库失败");
        let db = Mutex::new(conn);
        run_migrations(&db).expect("迁移失败");
        db
    }

    fn new_server(id: &str, config: &str, enabled: bool) -> NewMcpServer {
        NewMcpServer {
            id: id.to_string(),
            name: format!("srv-{}", id),
            server_config: config.to_string(),
            description: Some("desc".to_string()),
            homepage: None,
            docs: None,
            tags: "[]".to_string(),
            enabled_claude: enabled,
        }
    }

    #[test]
    fn create_and_get_roundtrip() {
        let db = setup();
        let row = create(&db, new_server("a", r#"{"command":"npx"}"#, false)).unwrap();
        assert_eq!(row.id, "a");
        assert_eq!(row.name, "srv-a");
        assert!(!row.enabled_claude);
        assert_eq!(row.tags, "[]");
        assert_eq!(
            get(&db, "a").unwrap().unwrap().server_config,
            r#"{"command":"npx"}"#
        );
    }

    #[test]
    fn list_enabled_filters_only_enabled() {
        let db = setup();
        create(&db, new_server("a", r#"{"command":"x"}"#, true)).unwrap();
        create(&db, new_server("b", r#"{"command":"y"}"#, false)).unwrap();
        create(&db, new_server("c", r#"{"command":"z"}"#, true)).unwrap();
        let enabled = list_enabled_claude(&db).unwrap();
        let ids: Vec<_> = enabled.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"]);
    }

    #[test]
    fn update_partial_fields() {
        let db = setup();
        create(&db, new_server("a", r#"{"command":"x"}"#, false)).unwrap();
        update(
            &db,
            "a",
            McpServerUpdate {
                enabled_claude: Some(true),
                name: Some("renamed".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let row = get(&db, "a").unwrap().unwrap();
        assert!(row.enabled_claude);
        assert_eq!(row.name, "renamed");
    }

    #[test]
    fn delete_removes_row() {
        let db = setup();
        create(&db, new_server("a", r#"{"command":"x"}"#, false)).unwrap();
        delete(&db, "a").unwrap();
        assert!(get(&db, "a").unwrap().is_none());
    }

    #[test]
    fn upsert_existing_only_updates_config_and_enables() {
        let db = setup();
        create(&db, new_server("a", r#"{"command":"old"}"#, false)).unwrap();
        // 已存在：upsert 只改 server_config + enabled，不动富信息。
        let updated = upsert(
            &db,
            NewMcpServer {
                id: "a".to_string(),
                name: "ignored".to_string(), // 已存在时 name 不改
                server_config: r#"{"command":"new"}"#.to_string(),
                description: Some("ignored".to_string()), // 已存在时不覆盖
                homepage: None,
                docs: None,
                tags: "[]".to_string(),
                enabled_claude: false, // 参数被忽略，恒置 true
            },
        )
        .unwrap();
        assert_eq!(updated.server_config, r#"{"command":"new"}"#);
        assert!(updated.enabled_claude);
        assert_eq!(updated.name, "srv-a", "已存在时 name 不覆盖");
        assert_eq!(updated.description.as_deref(), Some("desc"), "富信息不覆盖");
    }

    #[test]
    fn upsert_new_creates_with_id_as_name() {
        let db = setup();
        let row = upsert(&db, new_server("fresh", r#"{"command":"z"}"#, false)).unwrap();
        assert_eq!(row.id, "fresh");
        assert_eq!(row.name, "srv-fresh");
        assert!(row.enabled_claude, "新建默认启用");
    }
}
