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
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "create_schema_migrations",
        sql: "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );",
    },
    Migration {
        version: 2,
        name: "create_accounts_and_endpoints",
        sql: "CREATE TABLE IF NOT EXISTS accounts (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            account_type TEXT NOT NULL,
            platform TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            credentials_encrypted BLOB,
            extra_json TEXT,
            priority INTEGER NOT NULL DEFAULT 0,
            last_login_at TEXT,
            last_error TEXT,
            last_error_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS endpoints (
            id TEXT PRIMARY KEY,
            account_id TEXT,
            name TEXT NOT NULL,
            base_url TEXT NOT NULL,
            protocol_type TEXT NOT NULL,
            api_key_encrypted BLOB,
            auth_mode TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            priority INTEGER NOT NULL DEFAULT 0,
            cooldown_until TEXT,
            last_success_at TEXT,
            last_failure_at TEXT,
            last_error_kind TEXT,
            extra_json TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_endpoints_account_id ON endpoints(account_id);
        CREATE INDEX IF NOT EXISTS idx_endpoints_enabled_priority ON endpoints(enabled, priority);",
    },
    Migration {
        version: 3,
        name: "create_models_and_aliases",
        sql: "CREATE TABLE IF NOT EXISTS app_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS endpoint_models (
            id TEXT PRIMARY KEY,
            endpoint_id TEXT NOT NULL REFERENCES endpoints(id) ON DELETE CASCADE,
            model_name TEXT NOT NULL,
            display_name TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'synced',
            capabilities TEXT,
            context_window INTEGER,
            is_available INTEGER NOT NULL DEFAULT 1,
            last_seen_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            UNIQUE(endpoint_id, model_name)
        );
        CREATE INDEX IF NOT EXISTS idx_models_endpoint ON endpoint_models(endpoint_id);
        CREATE INDEX IF NOT EXISTS idx_models_source ON endpoint_models(source);
        CREATE TABLE IF NOT EXISTS model_aliases (
            id TEXT PRIMARY KEY,
            scope_type TEXT NOT NULL,
            scope_id TEXT,
            alias_name TEXT NOT NULL,
            target_endpoint_id TEXT REFERENCES endpoints(id) ON DELETE SET NULL,
            target_model_name TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 0,
            enabled INTEGER NOT NULL DEFAULT 1,
            invalid_reason TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_aliases_scope ON model_aliases(scope_type, scope_id);
        CREATE INDEX IF NOT EXISTS idx_aliases_name ON model_aliases(alias_name);",
    },
    Migration {
        version: 4,
        name: "create_tool_takeover",
        sql: "CREATE TABLE IF NOT EXISTS tool_takeover (
            tool TEXT PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 0,
            last_applied_at TEXT,
            last_target TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tool_takeover_backups (
            id TEXT PRIMARY KEY,
            tool TEXT NOT NULL,
            original_path TEXT NOT NULL,
            backup_path TEXT NOT NULL,
            original_existed INTEGER NOT NULL DEFAULT 1,
            takeover_target TEXT,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tool_backups ON tool_takeover_backups(tool, created_at);",
    },
    Migration {
        version: 5,
        name: "create_route_settings_request_logs_model_locks",
        sql: "CREATE TABLE IF NOT EXISTS route_settings (
            id          TEXT PRIMARY KEY,
            label       TEXT NOT NULL,
            strategy    TEXT NOT NULL DEFAULT 'fill-first',
            protocol_type TEXT NOT NULL,
            failover_enabled INTEGER NOT NULL DEFAULT 1,
            max_switches INTEGER NOT NULL DEFAULT 10,
            same_account_retries INTEGER NOT NULL DEFAULT 3,
            cooldown_multiplier REAL NOT NULL DEFAULT 1.0,
            updated_at  TEXT NOT NULL
        );
        INSERT OR IGNORE INTO route_settings (id, label, strategy, protocol_type, updated_at)
        VALUES ('claude-code', 'Claude Code', 'fill-first', 'anthropic', datetime('now'));
        INSERT OR IGNORE INTO route_settings (id, label, strategy, protocol_type, updated_at)
        VALUES ('codex', 'Codex', 'fill-first', 'openai-responses', datetime('now'));
        CREATE TABLE IF NOT EXISTS request_logs (
            id                TEXT PRIMARY KEY,
            request_id        TEXT NOT NULL,
            tool              TEXT,
            inbound_endpoint  TEXT NOT NULL,
            requested_model   TEXT,
            resolved_alias    TEXT,
            resolved_scope    TEXT,
            target_endpoint_id TEXT,
            upstream_model    TEXT,
            upstream_endpoint TEXT,
            protocol_from     TEXT,
            protocol_to       TEXT,
            status            INTEGER,
            error_kind        TEXT,
            fallback_chain    TEXT,
            stream            INTEGER NOT NULL DEFAULT 0,
            duration_ms       INTEGER,
            first_token_ms    INTEGER,
            input_tokens      INTEGER,
            output_tokens     INTEGER,
            cache_creation_tokens INTEGER,
            cache_read_tokens     INTEGER,
            request_body_hash TEXT,
            created_at        TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_request_logs_tool ON request_logs(tool);
        CREATE INDEX IF NOT EXISTS idx_request_logs_status ON request_logs(status);
        CREATE INDEX IF NOT EXISTS idx_request_logs_created ON request_logs(created_at DESC);
        CREATE TABLE IF NOT EXISTS model_locks (
            id          TEXT PRIMARY KEY,
            endpoint_id TEXT NOT NULL,
            model_name  TEXT NOT NULL,
            locked_until TEXT NOT NULL,
            lock_reason TEXT,
            created_at  TEXT NOT NULL,
            UNIQUE(endpoint_id, model_name)
        );",
    },
    Migration {
        version: 6,
        name: "add_v1_route_and_media_log_fields",
        sql: "INSERT OR IGNORE INTO route_settings (id, label, strategy, protocol_type, failover_enabled, max_switches, same_account_retries, cooldown_multiplier, updated_at)
        VALUES ('v1', 'OpenAI v1', 'fill-first', 'openai-compatible', 1, 10, 3, 1.0, datetime('now'));
        ALTER TABLE request_logs ADD COLUMN media_type TEXT;
        ALTER TABLE request_logs ADD COLUMN content_length INTEGER;
        ALTER TABLE request_logs ADD COLUMN body_sha256_hash TEXT;",
    },
];

/// Ensure the migration tracking table exists, then run pending migrations.
pub fn run_migrations(conn: &Mutex<Connection>) -> Result<(), String> {
    let mut db = conn.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;

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

    // Apply pending migrations
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

        // 将 DDL + schema_migrations 记录包裹在单个事务中，确保原子性。
        // 若 DDL 成功但 INSERT 失败（磁盘满/crash），事务整体回滚，
        // 下次启动可安全重试，不会出现 "duplicate column" 等非幂等错误。
        let tx = db
            .transaction()
            .map_err(|e| format!("迁移 v{} 开启事务失败: {}", migration.version, e))?;

        tx.execute_batch(migration.sql).map_err(|e| {
            format!(
                "迁移 v{} ({}) 失败: {}",
                migration.version, migration.name, e
            )
        })?;

        let now = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Iso8601::DEFAULT)
            .map_err(|e| format!("时间格式化失败: {}", e))?;

        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![migration.version, migration.name, now],
        )
        .map_err(|e| format!("迁移记录写入失败 v{}: {}", migration.version, e))?;

        tx.commit()
            .map_err(|e| format!("迁移 v{} 提交事务失败: {}", migration.version, e))?;
    }

    tracing::info!("数据库迁移：{} 项迁移已执行", pending.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全新数据库必须能按数组顺序跑完全部迁移，且不得因依赖表缺失而失败。
    ///
    /// 防止 v6（依赖 v5 创建的 route_settings / request_logs）排在 v5 之前
    /// 导致全新机器首次启动崩溃。
    #[test]
    fn fresh_db_runs_all_migrations_in_order() {
        let conn = Connection::open_in_memory().expect("无法创建内存数据库");
        let db = Mutex::new(conn);

        run_migrations(&db).expect("全新数据库迁移应成功完成");

        // v6 依赖的两张表必须在迁移后存在
        let db = db.lock().unwrap();
        let mut stmt = db
            .prepare("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('route_settings','request_logs')")
            .unwrap();
        let table_count: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(
            table_count, 2,
            "route_settings 与 request_logs 表应均已创建"
        );

        // v6 的 ALTER COLUMN 应已落在 request_logs 上
        let mut stmt = db.prepare("PRAGMA table_info(request_logs)").unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            columns.iter().any(|c| c == "media_type"),
            "request_logs.media_type 应存在"
        );
        assert!(
            columns.iter().any(|c| c == "body_sha256_hash"),
            "request_logs.body_sha256_hash 应存在"
        );

        // 所有迁移版本均已记录
        let mut stmt = db
            .prepare("SELECT count(*) FROM schema_migrations")
            .unwrap();
        let applied: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
        assert_eq!(applied, MIGRATIONS.len() as i64, "全部迁移应已记录");
    }

    /// 迁移数组版本号应单调递增，防止顺序错乱导致的依赖缺失。
    #[test]
    fn migration_versions_are_ascending() {
        let mut prev: i64 = 0;
        for m in MIGRATIONS {
            assert!(
                m.version > prev,
                "迁移版本号应单调递增，但 v{} 出现在 v{} 之后",
                m.version,
                prev
            );
            prev = m.version;
        }
    }
}
