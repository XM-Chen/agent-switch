// `prompts` 表 DAO：Claude Code CLAUDE.md 单激活清单（cc-prompts）。
//
// content 存明文提示词（写 live 原样投影到 `~/.claude/CLAUDE.md`），description 仅供
// UI 展示。enabled_claude 保留 `_claude` 后缀为将来多应用扩展留位（与 mcp_servers 对齐）。
#![allow(dead_code)]

use rusqlite::{params, Connection};
use std::sync::Mutex;

use super::now_iso;

/// Prompt 行（数据库原始表示）。
#[derive(Debug, Clone)]
pub struct PromptRow {
    pub id: String,
    pub name: String,
    /// 明文提示词内容。
    pub content: String,
    pub description: Option<String>,
    pub enabled_claude: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建 Prompt 的输入。
#[derive(Debug, Clone)]
pub struct NewPrompt {
    pub id: String,
    pub name: String,
    pub content: String,
    pub description: Option<String>,
    pub enabled_claude: bool,
}

/// 更新 Prompt 的输入（部分字段）。
///
/// 嵌套 `Option`：外层 `Some` 表示「更新该字段」，内层区分「更新为 NULL」。
#[derive(Debug, Clone, Default)]
pub struct PromptUpdate {
    pub name: Option<String>,
    pub content: Option<String>,
    pub description: Option<Option<String>>,
    /// 启用开关不由 update 改：启用走 `set_enabled_exclusive`。
    pub enabled_claude: Option<bool>,
}

fn row_to_prompt(row: &rusqlite::Row<'_>) -> rusqlite::Result<PromptRow> {
    Ok(PromptRow {
        id: row.get("id")?,
        name: row.get("name")?,
        content: row.get("content")?,
        description: row.get("description")?,
        enabled_claude: row.get::<_, i64>("enabled_claude")? != 0,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

/// 列出全部 Prompt，按 name 升序、再按创建时间。
pub fn list(db: &Mutex<Connection>) -> Result<Vec<PromptRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM prompts ORDER BY name ASC, created_at ASC")
        .map_err(|e| format!("查询 prompts 失败: {}", e))?;
    let rows = stmt
        .query_map([], row_to_prompt)
        .map_err(|e| format!("读取 prompts 失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("prompts 行解析失败: {}", e))?);
    }
    Ok(out)
}

/// 获取单个 Prompt。
pub fn get(db: &Mutex<Connection>, id: &str) -> Result<Option<PromptRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM prompts WHERE id = ?1")
        .map_err(|e| format!("查询 prompts 失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_prompt)
        .map_err(|e| format!("读取 prompts 失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| format!("prompts 行解析失败: {}", e))?))
    } else {
        Ok(None)
    }
}

/// 获取当前已启用 prompt（单激活查询，至多一份）。
pub fn get_enabled_claude(db: &Mutex<Connection>) -> Result<Option<PromptRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM prompts WHERE enabled_claude = 1 LIMIT 1")
        .map_err(|e| format!("查询启用 prompt 失败: {}", e))?;
    let mut rows = stmt
        .query_map([], row_to_prompt)
        .map_err(|e| format!("读取启用 prompt 失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| format!("prompts 行解析失败: {}", e))?))
    } else {
        Ok(None)
    }
}

/// 创建 Prompt（返回新建行）。
pub fn create(db: &Mutex<Connection>, new: NewPrompt) -> Result<PromptRow, String> {
    let now = now_iso()?;
    {
        let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        db.execute(
            "INSERT INTO prompts
             (id, name, content, description, enabled_claude, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                new.id,
                new.name,
                new.content,
                new.description,
                new.enabled_claude as i64,
                now,
            ],
        )
        .map_err(|e| format!("创建 prompt 失败: {}", e))?;
    }
    get(db, &new.id)?.ok_or_else(|| "创建后无法读取 prompt".to_string())
}

/// 更新 Prompt 部分字段。
pub fn update(db: &Mutex<Connection>, id: &str, upd: PromptUpdate) -> Result<(), String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;

    let mut sets: Vec<String> = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(name) = upd.name {
        sets.push("name = ?".into());
        params_vec.push(Box::new(name));
    }
    if let Some(content) = upd.content {
        sets.push("content = ?".into());
        params_vec.push(Box::new(content));
    }
    if let Some(description) = upd.description {
        sets.push("description = ?".into());
        params_vec.push(Box::new(description));
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
    let sql = format!("UPDATE prompts SET {} WHERE id = ?", sets.join(", "));
    db.execute(&sql, rusqlite::params_from_iter(params_vec))
        .map_err(|e| format!("更新 prompt 失败: {}", e))?;
    Ok(())
}

/// 删除 Prompt。
pub fn delete(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute("DELETE FROM prompts WHERE id = ?1", params![id])
        .map_err(|e| format!("删除 prompt 失败: {}", e))?;
    Ok(())
}

/// 单激活：同一事务内把目标置 `enabled_claude=1`、其余全部置 0。
///
/// 保证至多一份激活（应用层启用走此入口）。与 mcp 全量投影不同——prompts 是单激活模型。
pub fn set_enabled_exclusive(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let mut db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let now = now_iso()?;
    let tx = db
        .transaction()
        .map_err(|e| format!("开启事务失败: {}", e))?;
    tx.execute("UPDATE prompts SET enabled_claude = 0", [])
        .map_err(|e| format!("清空启用态失败: {}", e))?;
    let changed = tx
        .execute(
            "UPDATE prompts SET enabled_claude = 1, updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )
        .map_err(|e| format!("置目标启用失败: {}", e))?;
    if changed == 0 {
        return Err(format!("prompt '{}' 不存在，无法启用", id));
    }
    tx.commit().map_err(|e| format!("提交事务失败: {}", e))?;
    Ok(())
}

/// 清空所有启用态（disable 最后一个时用）。
pub fn clear_enabled(db: &Mutex<Connection>) -> Result<(), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute("UPDATE prompts SET enabled_claude = 0", [])
        .map_err(|e| format!("清空启用态失败: {}", e))?;
    Ok(())
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

    fn new_prompt(id: &str, content: &str, enabled: bool) -> NewPrompt {
        NewPrompt {
            id: id.to_string(),
            name: format!("p-{}", id),
            content: content.to_string(),
            description: Some("desc".to_string()),
            enabled_claude: enabled,
        }
    }

    #[test]
    fn create_and_get_roundtrip() {
        let db = setup();
        let row = create(&db, new_prompt("a", "# A\n", false)).unwrap();
        assert_eq!(row.id, "a");
        assert_eq!(row.name, "p-a");
        assert!(!row.enabled_claude);
        assert_eq!(get(&db, "a").unwrap().unwrap().content, "# A\n");
    }

    #[test]
    fn get_enabled_claude_returns_at_most_one() {
        let db = setup();
        assert!(get_enabled_claude(&db).unwrap().is_none());
        create(&db, new_prompt("a", "A", true)).unwrap();
        let r = get_enabled_claude(&db).unwrap().unwrap();
        assert_eq!(r.id, "a");
    }

    #[test]
    fn update_partial_fields() {
        let db = setup();
        create(&db, new_prompt("a", "old", false)).unwrap();
        update(
            &db,
            "a",
            PromptUpdate {
                name: Some("renamed".to_string()),
                content: Some("new".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let row = get(&db, "a").unwrap().unwrap();
        assert_eq!(row.name, "renamed");
        assert_eq!(row.content, "new");
    }

    #[test]
    fn delete_removes_row() {
        let db = setup();
        create(&db, new_prompt("a", "A", false)).unwrap();
        delete(&db, "a").unwrap();
        assert!(get(&db, "a").unwrap().is_none());
    }

    #[test]
    fn set_enabled_exclusive_atomic_single_active() {
        let db = setup();
        create(&db, new_prompt("a", "A", true)).unwrap();
        create(&db, new_prompt("b", "B", false)).unwrap();
        // 启用 B：A 应被同事务清零，至多一份激活。
        set_enabled_exclusive(&db, "b").unwrap();
        assert!(!get(&db, "a").unwrap().unwrap().enabled_claude);
        assert!(get(&db, "b").unwrap().unwrap().enabled_claude);
        // 启用回 A：B 清零。
        set_enabled_exclusive(&db, "a").unwrap();
        assert!(get(&db, "a").unwrap().unwrap().enabled_claude);
        assert!(!get(&db, "b").unwrap().unwrap().enabled_claude);
    }

    #[test]
    fn set_enabled_exclusive_missing_id_errors() {
        let db = setup();
        let err = set_enabled_exclusive(&db, "ghost").unwrap_err();
        assert!(err.contains("不存在"), "{}", err);
    }

    #[test]
    fn clear_enabled_resets_all() {
        let db = setup();
        create(&db, new_prompt("a", "A", true)).unwrap();
        clear_enabled(&db).unwrap();
        assert!(get_enabled_claude(&db).unwrap().is_none());
    }
}
