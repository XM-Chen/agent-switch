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
];

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

        db.execute_batch(migration.sql).map_err(|e| {
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
