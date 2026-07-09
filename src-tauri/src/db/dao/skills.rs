// `skills` / `skill_repos` 表 DAO：cc-skills 已安装清单与来源记录。
#![allow(dead_code)]

use rusqlite::{params, Connection};
use std::sync::Mutex;

use super::now_iso;

#[derive(Debug, Clone)]
pub struct SkillRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub directory: String,
    pub source_type: String,
    pub source_url: Option<String>,
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub repo_branch: Option<String>,
    pub repo_subdir: Option<String>,
    pub readme_url: Option<String>,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    pub enabled_gemini: bool,
    pub enabled_opencode: bool,
    pub enabled_hermes: bool,
    pub installed_at: String,
    pub updated_at: String,
    pub content_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct NewSkill {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub directory: String,
    pub source_type: String,
    pub source_url: Option<String>,
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub repo_branch: Option<String>,
    pub repo_subdir: Option<String>,
    pub readme_url: Option<String>,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    pub enabled_gemini: bool,
    pub enabled_opencode: bool,
    pub enabled_hermes: bool,
    pub content_hash: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkillUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub source_url: Option<Option<String>>,
    pub repo_owner: Option<Option<String>>,
    pub repo_name: Option<Option<String>>,
    pub repo_branch: Option<Option<String>>,
    pub repo_subdir: Option<Option<String>>,
    pub readme_url: Option<Option<String>>,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SkillRepoRow {
    pub id: String,
    pub source: String,
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub repo_url: Option<String>,
    pub branch: Option<String>,
    pub subdir: Option<String>,
    pub last_checked_at: Option<String>,
    pub last_known_hash: Option<String>,
    pub update_status: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewSkillRepo {
    pub id: String,
    pub source: String,
    pub repo_owner: Option<String>,
    pub repo_name: Option<String>,
    pub repo_url: Option<String>,
    pub branch: Option<String>,
    pub subdir: Option<String>,
    pub last_known_hash: Option<String>,
    pub update_status: Option<String>,
}

fn row_to_skill(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillRow> {
    Ok(SkillRow {
        id: row.get("id")?,
        name: row.get("name")?,
        description: row.get("description")?,
        directory: row.get("directory")?,
        source_type: row.get("source_type")?,
        source_url: row.get("source_url")?,
        repo_owner: row.get("repo_owner")?,
        repo_name: row.get("repo_name")?,
        repo_branch: row.get("repo_branch")?,
        repo_subdir: row.get("repo_subdir")?,
        readme_url: row.get("readme_url")?,
        enabled_claude: row.get::<_, i64>("enabled_claude")? != 0,
        enabled_codex: row.get::<_, i64>("enabled_codex")? != 0,
        enabled_gemini: row.get::<_, i64>("enabled_gemini")? != 0,
        enabled_opencode: row.get::<_, i64>("enabled_opencode")? != 0,
        enabled_hermes: row.get::<_, i64>("enabled_hermes")? != 0,
        installed_at: row.get("installed_at")?,
        updated_at: row.get("updated_at")?,
        content_hash: row.get("content_hash")?,
        created_at: row.get("created_at")?,
    })
}

fn row_to_repo(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillRepoRow> {
    Ok(SkillRepoRow {
        id: row.get("id")?,
        source: row.get("source")?,
        repo_owner: row.get("repo_owner")?,
        repo_name: row.get("repo_name")?,
        repo_url: row.get("repo_url")?,
        branch: row.get("branch")?,
        subdir: row.get("subdir")?,
        last_checked_at: row.get("last_checked_at")?,
        last_known_hash: row.get("last_known_hash")?,
        update_status: row.get("update_status")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn enabled_column(app: &str) -> Option<&'static str> {
    match app {
        "claude" | "claude-code" => Some("enabled_claude"),
        "codex" => Some("enabled_codex"),
        "gemini" => Some("enabled_gemini"),
        "opencode" => Some("enabled_opencode"),
        "hermes" => Some("enabled_hermes"),
        _ => None,
    }
}

pub fn list(db: &Mutex<Connection>) -> Result<Vec<SkillRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM skills ORDER BY name ASC, created_at ASC")
        .map_err(|e| format!("查询 skills 失败: {}", e))?;
    let rows = stmt
        .query_map([], row_to_skill)
        .map_err(|e| format!("读取 skills 失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("skills 行解析失败: {}", e))?);
    }
    Ok(out)
}

pub fn get(db: &Mutex<Connection>, id: &str) -> Result<Option<SkillRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM skills WHERE id = ?1")
        .map_err(|e| format!("查询 skills 失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_skill)
        .map_err(|e| format!("读取 skills 失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| format!("skills 行解析失败: {}", e))?))
    } else {
        Ok(None)
    }
}

pub fn get_by_directory(
    db: &Mutex<Connection>,
    directory: &str,
) -> Result<Option<SkillRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM skills WHERE directory = ?1")
        .map_err(|e| format!("查询 skills 失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![directory], row_to_skill)
        .map_err(|e| format!("读取 skills 失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(r.map_err(|e| format!("skills 行解析失败: {}", e))?))
    } else {
        Ok(None)
    }
}

pub fn list_enabled(db: &Mutex<Connection>, app: &str) -> Result<Vec<SkillRow>, String> {
    let column = enabled_column(app).ok_or_else(|| format!("不支持的 app: {}", app))?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let sql = format!(
        "SELECT * FROM skills WHERE {} = 1 ORDER BY name ASC",
        column
    );
    let mut stmt = db
        .prepare(&sql)
        .map_err(|e| format!("查询启用 skills 失败: {}", e))?;
    let rows = stmt
        .query_map([], row_to_skill)
        .map_err(|e| format!("读取启用 skills 失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("skills 行解析失败: {}", e))?);
    }
    Ok(out)
}

pub fn create(db: &Mutex<Connection>, new: NewSkill) -> Result<SkillRow, String> {
    let now = now_iso()?;
    {
        let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        db.execute(
            "INSERT INTO skills
             (id, name, description, directory, source_type, source_url, repo_owner, repo_name,
              repo_branch, repo_subdir, readme_url, enabled_claude, enabled_codex, enabled_gemini,
              enabled_opencode, enabled_hermes, installed_at, updated_at, content_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?17, ?18, ?17)",
            params![
                new.id,
                new.name,
                new.description,
                new.directory,
                new.source_type,
                new.source_url,
                new.repo_owner,
                new.repo_name,
                new.repo_branch,
                new.repo_subdir,
                new.readme_url,
                new.enabled_claude as i64,
                new.enabled_codex as i64,
                new.enabled_gemini as i64,
                new.enabled_opencode as i64,
                new.enabled_hermes as i64,
                now,
                new.content_hash,
            ],
        )
        .map_err(|e| format!("创建 skill 失败: {}", e))?;
    }
    get(db, &new.id)?.ok_or_else(|| "创建后无法读取 skill".to_string())
}

pub fn update(db: &Mutex<Connection>, id: &str, upd: SkillUpdate) -> Result<(), String> {
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut sets: Vec<String> = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(name) = upd.name {
        sets.push("name = ?".into());
        params_vec.push(Box::new(name));
    }
    if let Some(description) = upd.description {
        sets.push("description = ?".into());
        params_vec.push(Box::new(description));
    }
    if let Some(source_url) = upd.source_url {
        sets.push("source_url = ?".into());
        params_vec.push(Box::new(source_url));
    }
    if let Some(repo_owner) = upd.repo_owner {
        sets.push("repo_owner = ?".into());
        params_vec.push(Box::new(repo_owner));
    }
    if let Some(repo_name) = upd.repo_name {
        sets.push("repo_name = ?".into());
        params_vec.push(Box::new(repo_name));
    }
    if let Some(repo_branch) = upd.repo_branch {
        sets.push("repo_branch = ?".into());
        params_vec.push(Box::new(repo_branch));
    }
    if let Some(repo_subdir) = upd.repo_subdir {
        sets.push("repo_subdir = ?".into());
        params_vec.push(Box::new(repo_subdir));
    }
    if let Some(readme_url) = upd.readme_url {
        sets.push("readme_url = ?".into());
        params_vec.push(Box::new(readme_url));
    }
    if let Some(content_hash) = upd.content_hash {
        sets.push("content_hash = ?".into());
        params_vec.push(Box::new(content_hash));
    }

    if sets.is_empty() {
        return Ok(());
    }

    sets.push("updated_at = ?".into());
    params_vec.push(Box::new(now));
    params_vec.push(Box::new(id.to_string()));
    let sql = format!("UPDATE skills SET {} WHERE id = ?", sets.join(", "));
    db.execute(&sql, rusqlite::params_from_iter(params_vec))
        .map_err(|e| format!("更新 skill 失败: {}", e))?;
    Ok(())
}

pub fn set_enabled(
    db: &Mutex<Connection>,
    id: &str,
    app: &str,
    enabled: bool,
) -> Result<(), String> {
    let column = enabled_column(app).ok_or_else(|| format!("不支持的 app: {}", app))?;
    let now = now_iso()?;
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let sql = format!(
        "UPDATE skills SET {} = ?1, updated_at = ?2 WHERE id = ?3",
        column
    );
    let changed = db
        .execute(&sql, params![enabled as i64, now, id])
        .map_err(|e| format!("更新 skill 启用态失败: {}", e))?;
    if changed == 0 {
        return Err(format!("skill '{}' 不存在", id));
    }
    Ok(())
}

pub fn delete(db: &Mutex<Connection>, id: &str) -> Result<(), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute("DELETE FROM skills WHERE id = ?1", params![id])
        .map_err(|e| format!("删除 skill 失败: {}", e))?;
    Ok(())
}

pub fn create_repo(db: &Mutex<Connection>, new: NewSkillRepo) -> Result<SkillRepoRow, String> {
    let now = now_iso()?;
    {
        let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
        db.execute(
            "INSERT INTO skill_repos
             (id, source, repo_owner, repo_name, repo_url, branch, subdir, last_known_hash, update_status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                new.id,
                new.source,
                new.repo_owner,
                new.repo_name,
                new.repo_url,
                new.branch,
                new.subdir,
                new.last_known_hash,
                new.update_status,
                now,
            ],
        )
        .map_err(|e| format!("创建 skill_repo 失败: {}", e))?;
    }
    get_repo(db, &new.id)?.ok_or_else(|| "创建后无法读取 skill_repo".to_string())
}

pub fn get_repo(db: &Mutex<Connection>, id: &str) -> Result<Option<SkillRepoRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM skill_repos WHERE id = ?1")
        .map_err(|e| format!("查询 skill_repos 失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_repo)
        .map_err(|e| format!("读取 skill_repos 失败: {}", e))?;
    if let Some(r) = rows.next() {
        Ok(Some(
            r.map_err(|e| format!("skill_repos 行解析失败: {}", e))?,
        ))
    } else {
        Ok(None)
    }
}

pub fn list_repos(db: &Mutex<Connection>) -> Result<Vec<SkillRepoRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM skill_repos ORDER BY created_at DESC")
        .map_err(|e| format!("查询 skill_repos 失败: {}", e))?;
    let rows = stmt
        .query_map([], row_to_repo)
        .map_err(|e| format!("读取 skill_repos 失败: {}", e))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("skill_repos 行解析失败: {}", e))?);
    }
    Ok(out)
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

    fn new_skill(id: &str, directory: &str) -> NewSkill {
        NewSkill {
            id: id.to_string(),
            name: format!("skill-{}", id),
            description: Some("desc".to_string()),
            directory: directory.to_string(),
            source_type: "local_dir".to_string(),
            source_url: None,
            repo_owner: None,
            repo_name: None,
            repo_branch: None,
            repo_subdir: None,
            readme_url: None,
            enabled_claude: false,
            enabled_codex: false,
            enabled_gemini: false,
            enabled_opencode: false,
            enabled_hermes: false,
            content_hash: "abc".to_string(),
        }
    }

    #[test]
    fn create_get_and_list_roundtrip() {
        let db = setup();
        let row = create(&db, new_skill("a", "a-skill")).unwrap();
        assert_eq!(row.id, "a");
        assert_eq!(row.directory, "a-skill");
        assert_eq!(get(&db, "a").unwrap().unwrap().name, "skill-a");
        assert_eq!(get_by_directory(&db, "a-skill").unwrap().unwrap().id, "a");
        assert_eq!(list(&db).unwrap().len(), 1);
    }

    #[test]
    fn set_enabled_is_guarded_by_known_app_columns() {
        let db = setup();
        create(&db, new_skill("a", "a-skill")).unwrap();
        set_enabled(&db, "a", "claude", true).unwrap();
        set_enabled(&db, "a", "codex", true).unwrap();
        let row = get(&db, "a").unwrap().unwrap();
        assert!(row.enabled_claude);
        assert!(row.enabled_codex);
        assert!(set_enabled(&db, "a", "bad_app", true).is_err());
    }

    #[test]
    fn list_enabled_filters_by_app() {
        let db = setup();
        create(&db, new_skill("a", "a-skill")).unwrap();
        create(&db, new_skill("b", "b-skill")).unwrap();
        set_enabled(&db, "b", "gemini", true).unwrap();
        let enabled = list_enabled(&db, "gemini").unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].id, "b");
    }

    #[test]
    fn repos_roundtrip() {
        let db = setup();
        let row = create_repo(
            &db,
            NewSkillRepo {
                id: "r1".to_string(),
                source: "github".to_string(),
                repo_owner: Some("owner".to_string()),
                repo_name: Some("repo".to_string()),
                repo_url: Some("https://github.com/owner/repo".to_string()),
                branch: Some("main".to_string()),
                subdir: None,
                last_known_hash: None,
                update_status: None,
            },
        )
        .unwrap();
        assert_eq!(row.source, "github");
        assert_eq!(list_repos(&db).unwrap().len(), 1);
    }
}
