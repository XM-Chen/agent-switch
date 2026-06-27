use rusqlite::Connection;
use std::sync::Mutex;
use time::OffsetDateTime;

/// A single migration entry.
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

/// Migration list: add new migrations at the end.
/// DO NOT reorder or remove entries once they have been deployed.
const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "create_schema_migrations",
    sql: "CREATE TABLE IF NOT EXISTS schema_migrations (
        version INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        applied_at TEXT NOT NULL
    );",
}];

/// Ensure the migration tracking table exists, then run pending migrations.
pub fn run_migrations(conn: &Mutex<Connection>) -> Result<(), String> {
    let db = conn.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;

    // Create schema_migrations if it does not exist
    db.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );",
        [],
    )
    .map_err(|e| format!("无法创建 schema_migrations 表: {}", e))?;

    // Read already-applied versions
    let mut stmt = db
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .map_err(|e| format!("无法查询 schema_migrations: {}", e))?;

    let applied: Vec<i64> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("读取迁移记录失败: {}", e))?
        .filter_map(|r| r.ok())
        .collect();

    drop(stmt);

    // Apply pending migrations inside a single transaction
    let pending: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|m| !applied.contains(&m.version))
        .collect();

    if pending.is_empty() {
        tracing::info!("数据库迁移：无待执行迁移");
        return Ok(());
    }

    for migration in &pending {
        tracing::info!("执行迁移 v{}: {}", migration.version, migration.name);

        db.execute(migration.sql, []).map_err(|e| {
            format!(
                "迁移 v{} ({}) 失败: {}",
                migration.version, migration.name, e
            )
        })?;

        let now = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Iso8601::DEFAULT)
            .map_err(|e| format!("时间格式化失败: {}", e))?;

        db.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![migration.version, migration.name, now],
        )
        .map_err(|e| format!("迁移记录写入失败 v{}: {}", migration.version, e))?;
    }

    tracing::info!("数据库迁移：{} 项迁移已执行", pending.len());
    Ok(())
}
