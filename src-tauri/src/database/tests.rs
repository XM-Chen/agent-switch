//! 数据库模块测试
//!
//! 包含 Schema 迁移和基本功能的测试。

use super::*;
use rusqlite::{params, Connection};
use serde_json::json;
use tempfile::NamedTempFile;

const LEGACY_SCHEMA_SQL: &str = r#"
    CREATE TABLE providers (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        settings_config TEXT NOT NULL,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE provider_endpoints (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        provider_id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        url TEXT NOT NULL
    );
    CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        server_config TEXT NOT NULL
    );
    CREATE TABLE prompts (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        content TEXT NOT NULL,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE skills (
        key TEXT PRIMARY KEY,
        installed BOOLEAN NOT NULL DEFAULT 0
    );
    CREATE TABLE skill_repos (
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        PRIMARY KEY (owner, name)
    );
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT
    );
"#;

// v3.8.x（schema v1）的真实表结构快照：用于验证从 v3.8.* 升级到当前版本的迁移链路
// 参考：tag v3.8.3 的 src-tauri/src/database/schema.rs
const V3_8_SCHEMA_V1_SQL: &str = r#"
    CREATE TABLE providers (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        settings_config TEXT NOT NULL,
        website_url TEXT,
        category TEXT,
        created_at INTEGER,
        sort_index INTEGER,
        notes TEXT,
        icon TEXT,
        icon_color TEXT,
        meta TEXT NOT NULL DEFAULT '{}',
        is_current BOOLEAN NOT NULL DEFAULT 0,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE provider_endpoints (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        provider_id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        url TEXT NOT NULL,
        added_at INTEGER,
        FOREIGN KEY (provider_id, app_type) REFERENCES providers(id, app_type) ON DELETE CASCADE
    );
    CREATE TABLE mcp_servers (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        server_config TEXT NOT NULL,
        description TEXT,
        homepage TEXT,
        docs TEXT,
        tags TEXT NOT NULL DEFAULT '[]',
        enabled_claude BOOLEAN NOT NULL DEFAULT 0,
        enabled_codex BOOLEAN NOT NULL DEFAULT 0,
        enabled_gemini BOOLEAN NOT NULL DEFAULT 0
    );
    CREATE TABLE prompts (
        id TEXT NOT NULL,
        app_type TEXT NOT NULL,
        name TEXT NOT NULL,
        content TEXT NOT NULL,
        description TEXT,
        enabled BOOLEAN NOT NULL DEFAULT 1,
        created_at INTEGER,
        updated_at INTEGER,
        PRIMARY KEY (id, app_type)
    );
    CREATE TABLE skills (
        key TEXT PRIMARY KEY,
        installed BOOLEAN NOT NULL DEFAULT 0,
        installed_at INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE skill_repos (
        owner TEXT NOT NULL,
        name TEXT NOT NULL,
        branch TEXT NOT NULL DEFAULT 'main',
        enabled BOOLEAN NOT NULL DEFAULT 1,
        PRIMARY KEY (owner, name)
    );
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT
    );
"#;

#[derive(Debug)]
struct ColumnInfo {
    r#type: String,
    notnull: i64,
    default: Option<String>,
}

fn get_column_info(conn: &Connection, table: &str, column: &str) -> ColumnInfo {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\");"))
        .expect("prepare pragma");
    let mut rows = stmt.query([]).expect("query pragma");
    while let Some(row) = rows.next().expect("read row") {
        let column_name: String = row.get(1).expect("name");
        if column_name.eq_ignore_ascii_case(column) {
            return ColumnInfo {
                r#type: row.get::<_, String>(2).expect("type"),
                notnull: row.get::<_, i64>(3).expect("notnull"),
                default: row.get::<_, Option<String>>(4).ok().flatten(),
            };
        }
    }
    panic!("column {table}.{column} not found");
}

fn normalize_default(default: &Option<String>) -> Option<String> {
    default
        .as_ref()
        .map(|s| s.trim_matches('\'').trim_matches('"').to_string())
}

#[test]
fn schema_migration_sets_user_version_when_missing() {
    let conn = Connection::open_in_memory().expect("open memory db");

    Database::create_tables_on_conn(&conn).expect("create tables");
    assert_eq!(
        Database::get_user_version(&conn).expect("read version before"),
        0
    );

    Database::apply_schema_migrations_on_conn(&conn).expect("apply migration");

    assert_eq!(
        Database::get_user_version(&conn).expect("read version after"),
        SCHEMA_VERSION
    );
}

#[test]
fn schema_migration_rejects_future_version() {
    let conn = Connection::open_in_memory().expect("open memory db");
    Database::create_tables_on_conn(&conn).expect("create tables");
    Database::set_user_version(&conn, SCHEMA_VERSION + 1).expect("set future version");

    let err =
        Database::apply_schema_migrations_on_conn(&conn).expect_err("should reject higher version");
    assert!(
        err.to_string().contains("数据库版本过新"),
        "unexpected error: {err}"
    );
}

#[test]
fn schema_migration_adds_missing_columns_for_providers() {
    let conn = Connection::open_in_memory().expect("open memory db");

    // 创建旧版 providers 表，缺少新增列
    conn.execute_batch(LEGACY_SCHEMA_SQL)
        .expect("seed old schema");

    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    // 验证关键新增列已补齐
    for (table, column) in [
        ("providers", "meta"),
        ("providers", "is_current"),
        ("provider_endpoints", "added_at"),
        ("mcp_servers", "enabled_gemini"),
        ("prompts", "updated_at"),
        ("skills", "installed_at"),
        ("skill_repos", "enabled"),
    ] {
        assert!(
            Database::has_column(&conn, table, column).expect("check column"),
            "{table}.{column} should exist after migration"
        );
    }

    // 验证 meta 列约束保持一致
    let meta = get_column_info(&conn, "providers", "meta");
    assert_eq!(meta.notnull, 1, "meta should be NOT NULL");
    assert_eq!(
        normalize_default(&meta.default).as_deref(),
        Some("{}"),
        "meta default should be '{{}}'"
    );

    assert_eq!(
        Database::get_user_version(&conn).expect("version after migration"),
        SCHEMA_VERSION
    );
}

#[test]
fn schema_migration_aligns_column_defaults_and_types() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(LEGACY_SCHEMA_SQL)
        .expect("seed old schema");

    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    let is_current = get_column_info(&conn, "providers", "is_current");
    assert_eq!(is_current.r#type, "BOOLEAN");
    assert_eq!(is_current.notnull, 1);
    assert_eq!(normalize_default(&is_current.default).as_deref(), Some("0"));

    let tags = get_column_info(&conn, "mcp_servers", "tags");
    assert_eq!(tags.r#type, "TEXT");
    assert_eq!(tags.notnull, 1);
    assert_eq!(normalize_default(&tags.default).as_deref(), Some("[]"));

    let enabled = get_column_info(&conn, "prompts", "enabled");
    assert_eq!(enabled.r#type, "BOOLEAN");
    assert_eq!(enabled.notnull, 1);
    assert_eq!(normalize_default(&enabled.default).as_deref(), Some("1"));

    let installed_at = get_column_info(&conn, "skills", "installed_at");
    assert_eq!(installed_at.r#type, "INTEGER");
    assert_eq!(installed_at.notnull, 1);
    assert_eq!(
        normalize_default(&installed_at.default).as_deref(),
        Some("0")
    );

    let branch = get_column_info(&conn, "skill_repos", "branch");
    assert_eq!(branch.r#type, "TEXT");
    assert_eq!(normalize_default(&branch.default).as_deref(), Some("main"));

    let skill_repo_enabled = get_column_info(&conn, "skill_repos", "enabled");
    assert_eq!(skill_repo_enabled.r#type, "BOOLEAN");
    assert_eq!(skill_repo_enabled.notnull, 1);
    assert_eq!(
        normalize_default(&skill_repo_enabled.default).as_deref(),
        Some("1")
    );
}

#[test]
fn schema_create_tables_include_pricing_model_columns() {
    let conn = Connection::open_in_memory().expect("open memory db");
    Database::create_tables_on_conn(&conn).expect("create tables");

    let multiplier = get_column_info(&conn, "proxy_config", "default_cost_multiplier");
    assert_eq!(multiplier.r#type, "TEXT");
    assert_eq!(multiplier.notnull, 1);
    assert_eq!(normalize_default(&multiplier.default).as_deref(), Some("1"));

    let pricing_source = get_column_info(&conn, "proxy_config", "pricing_model_source");
    assert_eq!(pricing_source.r#type, "TEXT");
    assert_eq!(pricing_source.notnull, 1);
    assert_eq!(
        normalize_default(&pricing_source.default).as_deref(),
        Some("response")
    );

    let request_model = get_column_info(&conn, "proxy_request_logs", "request_model");
    assert_eq!(request_model.r#type, "TEXT");
    assert_eq!(request_model.notnull, 0);
}

#[test]
fn schema_migration_v4_adds_pricing_model_columns() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(
        r#"
        CREATE TABLE providers (
            id TEXT NOT NULL,
            app_type TEXT NOT NULL,
            name TEXT NOT NULL,
            settings_config TEXT NOT NULL DEFAULT '{}',
            meta TEXT NOT NULL DEFAULT '{}',
            PRIMARY KEY (id, app_type)
        );
        CREATE TABLE proxy_config (app_type TEXT PRIMARY KEY);
        CREATE TABLE proxy_request_logs (request_id TEXT PRIMARY KEY, model TEXT NOT NULL);
        CREATE TABLE mcp_servers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            server_config TEXT NOT NULL,
            enabled_claude INTEGER NOT NULL DEFAULT 0,
            enabled_codex INTEGER NOT NULL DEFAULT 0,
            enabled_gemini INTEGER NOT NULL DEFAULT 0,
            enabled_opencode INTEGER NOT NULL DEFAULT 0
        );
        "#,
    )
    .expect("seed v4 schema");

    Database::set_user_version(&conn, 4).expect("set user_version=4");
    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    let multiplier = get_column_info(&conn, "proxy_config", "default_cost_multiplier");
    assert_eq!(multiplier.r#type, "TEXT");
    assert_eq!(multiplier.notnull, 1);
    assert_eq!(normalize_default(&multiplier.default).as_deref(), Some("1"));

    let pricing_source = get_column_info(&conn, "proxy_config", "pricing_model_source");
    assert_eq!(pricing_source.r#type, "TEXT");
    assert_eq!(pricing_source.notnull, 1);
    assert_eq!(
        normalize_default(&pricing_source.default).as_deref(),
        Some("response")
    );

    let request_model = get_column_info(&conn, "proxy_request_logs", "request_model");
    assert_eq!(request_model.r#type, "TEXT");
    assert_eq!(request_model.notnull, 0);

    assert_eq!(
        Database::get_user_version(&conn).expect("version after migration"),
        SCHEMA_VERSION
    );
}

#[test]
fn migration_v10_to_v11_rebuilds_rollups_with_request_model_dimension() {
    let conn = Connection::open_in_memory().expect("open memory db");

    // 模拟 v10 形状的 rollup 表（主键不含 request_model）+ 一行历史聚合数据，
    // 以及 v10 形状的明细表（无 pricing_model 列）
    conn.execute_batch(
        r#"
        CREATE TABLE proxy_request_logs (
            request_id TEXT PRIMARY KEY,
            model TEXT NOT NULL,
            request_model TEXT
        );
        CREATE TABLE usage_daily_rollups (
            date TEXT NOT NULL,
            app_type TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            model TEXT NOT NULL,
            request_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost_usd TEXT NOT NULL DEFAULT '0',
            avg_latency_ms INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (date, app_type, provider_id, model)
        );
        INSERT INTO usage_daily_rollups
            (date, app_type, provider_id, model, request_count, success_count,
             input_tokens, output_tokens, total_cost_usd, avg_latency_ms)
        VALUES ('2026-05-01', 'claude', 'p1', 'kimi-k2', 7, 7, 1000, 500, '0.07', 120);
        "#,
    )
    .expect("seed v10 rollup table");

    Database::set_user_version(&conn, 10).expect("set user_version=10");
    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    // 新列存在且 NOT NULL DEFAULT ''
    let request_model = get_column_info(&conn, "usage_daily_rollups", "request_model");
    assert_eq!(request_model.r#type, "TEXT");
    assert_eq!(request_model.notnull, 1);
    let rollup_pricing_model = get_column_info(&conn, "usage_daily_rollups", "pricing_model");
    assert_eq!(rollup_pricing_model.r#type, "TEXT");
    assert_eq!(rollup_pricing_model.notnull, 1);

    // 明细表补上 pricing_model 列（可空，历史行 NULL）
    let pricing_model = get_column_info(&conn, "proxy_request_logs", "pricing_model");
    assert_eq!(pricing_model.r#type, "TEXT");
    assert_eq!(pricing_model.notnull, 0);

    // 历史行保留，request_model 填 ''（未知）
    let (rm, count, input, cost): (String, i64, i64, String) = conn
        .query_row(
            "SELECT request_model, request_count, input_tokens, total_cost_usd
             FROM usage_daily_rollups WHERE model = 'kimi-k2'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("migrated row");
    assert_eq!(rm, "");
    assert_eq!(count, 7);
    assert_eq!(input, 1000);
    assert_eq!(cost, "0.07");

    // 主键包含 request_model：同 model 不同别名可共存
    conn.execute(
        "INSERT INTO usage_daily_rollups
            (date, app_type, provider_id, model, request_model, request_count)
         VALUES ('2026-05-01', 'claude', 'p1', 'kimi-k2', 'claude-sonnet-4-6', 1)",
        [],
    )
    .expect("insert row with same model but different request_model");

    assert_eq!(
        Database::get_user_version(&conn).expect("version after migration"),
        SCHEMA_VERSION
    );
}

#[test]
fn migration_v11_to_v12_adds_provider_models_table() {
    let conn = Connection::open_in_memory().expect("open memory db");

    // 模拟 v11 库：建 providers 表（供复合外键引用）+ 设 user_version=11，
    // 但尚无 provider_models 表。
    conn.execute_batch(
        r#"
        CREATE TABLE providers (
            id TEXT NOT NULL,
            app_type TEXT NOT NULL,
            name TEXT NOT NULL,
            settings_config TEXT NOT NULL,
            meta TEXT NOT NULL DEFAULT '{}',
            is_current BOOLEAN NOT NULL DEFAULT 0,
            PRIMARY KEY (id, app_type)
        );
        INSERT INTO providers (id, app_type, name, settings_config)
            VALUES ('p1', 'claude', 'P1', '{}');
        "#,
    )
    .expect("seed v11 providers");

    Database::set_user_version(&conn, 11).expect("set user_version=11");
    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    assert_eq!(
        Database::get_user_version(&conn).expect("version after migration"),
        SCHEMA_VERSION
    );
    assert!(
        Database::table_exists(&conn, "provider_models").expect("check table"),
        "provider_models table should exist after v11 -> v12"
    );

    // 列结构：source/fetched_at NOT NULL，owned_by 可空。
    let source = get_column_info(&conn, "provider_models", "source");
    assert_eq!(source.r#type, "TEXT");
    assert_eq!(source.notnull, 1);
    let fetched_at = get_column_info(&conn, "provider_models", "fetched_at");
    assert_eq!(fetched_at.r#type, "INTEGER");
    assert_eq!(fetched_at.notnull, 1);
    let owned_by = get_column_info(&conn, "provider_models", "owned_by");
    assert_eq!(owned_by.notnull, 0);

    // 复合外键 ON DELETE CASCADE 生效：删 provider → 缓存清空。
    conn.execute("PRAGMA foreign_keys = ON;", [])
        .expect("enable fk");
    conn.execute(
        "INSERT INTO provider_models
            (provider_id, app_type, model_id, source, owned_by, fetched_at)
         VALUES ('p1', 'claude', 'm1', 'fetched', NULL, 100)",
        [],
    )
    .expect("insert cached model");
    conn.execute(
        "DELETE FROM providers WHERE id = 'p1' AND app_type = 'claude'",
        [],
    )
    .expect("delete provider");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM provider_models", [], |r| r.get(0))
        .expect("count cached models");
    assert_eq!(remaining, 0, "CASCADE should clear cached models");
}

/// M2：干净 ags v12 库（已含 provider_models + custom_aggregates）升级到 v14 后，
/// profiles 表与两处 input_token_semantics 列就位，且 ags 聚合表完好无损。
#[test]
fn migration_v12_to_v14_adds_profiles_and_input_token_semantics() {
    let conn = Connection::open_in_memory().expect("open memory db");

    // 模拟 ags v12 库形状：provider_models + custom_aggregates 已建，
    // proxy_request_logs / usage_daily_rollups 为 v12 形状（无语义列），
    // user_version=12，但尚无 profiles 表。
    conn.execute_batch(
        r#"
        CREATE TABLE provider_models (
            provider_id TEXT NOT NULL,
            app_type    TEXT NOT NULL,
            model_id    TEXT NOT NULL,
            source      TEXT NOT NULL,
            owned_by    TEXT,
            fetched_at  INTEGER NOT NULL,
            PRIMARY KEY (provider_id, app_type, model_id)
        );
        CREATE TABLE custom_aggregates (
            id              TEXT PRIMARY KEY,
            app_type        TEXT NOT NULL,
            name            TEXT NOT NULL,
            ordered_members TEXT NOT NULL DEFAULT '[]',
            sort_index      INTEGER,
            created_at      INTEGER,
            updated_at      INTEGER
        );
        INSERT INTO custom_aggregates (id, app_type, name, ordered_members)
            VALUES ('agg1', 'claude', 'My Aggregate', '["glm-4.6","gpt-5"]');
        CREATE TABLE proxy_request_logs (
            request_id TEXT PRIMARY KEY,
            app_type TEXT NOT NULL,
            model TEXT NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            status_code INTEGER NOT NULL,
            created_at INTEGER NOT NULL
        );
        INSERT INTO proxy_request_logs
            (request_id, app_type, model, input_tokens, output_tokens,
             cache_read_tokens, cache_creation_tokens, status_code, created_at)
            VALUES ('hist-1', 'codex', 'gpt-5.5', 1000, 200, 300, 0, 200, 1000);
        CREATE TABLE usage_daily_rollups (
            date TEXT NOT NULL,
            app_type TEXT NOT NULL,
            provider_id TEXT NOT NULL,
            model TEXT NOT NULL,
            request_model TEXT NOT NULL DEFAULT '',
            pricing_model TEXT NOT NULL DEFAULT '',
            request_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cache_read_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            total_cost_usd TEXT NOT NULL DEFAULT '0',
            avg_latency_ms INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (date, app_type, provider_id, model, request_model, pricing_model)
        );
        "#,
    )
    .expect("seed ags v12 tables");

    Database::set_user_version(&conn, 12).expect("set user_version=12");
    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    // 最终版本达到 SCHEMA_VERSION（=15）。
    assert_eq!(
        Database::get_user_version(&conn).expect("version after migration"),
        SCHEMA_VERSION
    );

    // v13：profiles 表就位，列结构正确。
    assert!(
        Database::table_exists(&conn, "profiles").expect("check profiles"),
        "profiles table should exist after v12 -> v13"
    );
    let payload = get_column_info(&conn, "profiles", "payload");
    assert_eq!(payload.r#type, "TEXT");
    assert_eq!(payload.notnull, 1);
    let sort_order = get_column_info(&conn, "profiles", "sort_order");
    assert_eq!(sort_order.notnull, 0);

    // v14：两处 input_token_semantics 列就位，NOT NULL DEFAULT 0。
    for table in ["proxy_request_logs", "usage_daily_rollups"] {
        let col = get_column_info(&conn, table, "input_token_semantics");
        assert_eq!(col.r#type, "INTEGER", "{table}.input_token_semantics type");
        assert_eq!(col.notnull, 1, "{table}.input_token_semantics NOT NULL");
    }

    // 历史行的语义列回填为默认 0（LEGACY）。
    let hist_semantics: i64 = conn
        .query_row(
            "SELECT input_token_semantics FROM proxy_request_logs WHERE request_id = 'hist-1'",
            [],
            |r| r.get(0),
        )
        .expect("historical semantics");
    assert_eq!(hist_semantics, 0, "历史行默认 LEGACY(0)");

    // ags 聚合表完好：provider_models 仍在，custom_aggregates 数据未丢。
    assert!(
        Database::table_exists(&conn, "provider_models").expect("check provider_models"),
        "provider_models must survive v12 -> v14"
    );
    let agg_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM custom_aggregates", [], |r| r.get(0))
        .expect("count custom_aggregates");
    assert_eq!(agg_count, 1, "custom_aggregates 数据必须完好");
}

/// M2：全新库（create_tables 直建）也拥有 profiles 表与两处语义列，
/// 证明 create path 与 upgrade path 同构。
#[test]
fn fresh_create_tables_include_profiles_and_input_token_semantics() {
    let db = Database::memory().expect("memory db");
    db.apply_schema_migrations().expect("apply migrations");
    let conn = db.conn.lock().expect("lock");

    assert_eq!(
        Database::get_user_version(&conn).expect("version"),
        SCHEMA_VERSION
    );
    assert!(
        Database::table_exists(&conn, "profiles").expect("check profiles"),
        "fresh create must include profiles table"
    );
    for table in ["proxy_request_logs", "usage_daily_rollups"] {
        let col = get_column_info(&conn, table, "input_token_semantics");
        assert_eq!(col.r#type, "INTEGER", "{table}.input_token_semantics type");
        assert_eq!(col.notnull, 1, "{table}.input_token_semantics NOT NULL");
    }

    let proxy_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM proxy_config", [], |r| r.get(0))
        .expect("count proxy_config");
    assert_eq!(
        proxy_rows, 7,
        "fresh create path must seed seven proxy modules"
    );
    assert!(
        Database::has_column(&conn, "proxy_config", "route_mode").expect("route_mode"),
        "fresh create must include route_mode"
    );
    for table in [
        "upstreams",
        "upstream_credentials",
        "upstream_models",
        "gateway_models",
        "model_aliases",
        "route_targets",
        "route_target_health",
        "gateway_migration_report",
    ] {
        assert!(
            Database::table_exists(&conn, table).unwrap_or(false),
            "fresh create must include {table}"
        );
    }
    assert!(
        Database::has_column(&conn, "gateway_config", "listen_address")
            .expect("gateway_config.listen_address"),
        "fresh create must include full gateway config"
    );
    let log_columns = [
        "ingress_protocol",
        "gateway_model_id",
        "route_target_id",
        "upstream_id",
        "target_model",
    ];
    for column in log_columns {
        assert!(
            Database::has_column(&conn, "proxy_request_logs", column).expect("log dimension"),
            "fresh create must include proxy_request_logs.{column}"
        );
    }
}

fn seed_gateway_domain_dao_fixture(db: &Database) {
    let conn = db.conn.lock().expect("lock gateway fixture");
    conn.execute_batch(
        r#"
        INSERT INTO upstreams
            (id, name, enabled, base_url, protocol, adapter_type, created_at, updated_at)
        VALUES
            ('upstream-a', 'Upstream A', 1, 'https://a.invalid', 'anthropic', 'claude', 1, 1),
            ('upstream-b', 'Upstream B', 1, 'https://b.invalid', 'openai_chat', 'module_openai', 1, 1),
            ('upstream-c', 'Upstream C', 1, 'https://c.invalid', 'openai_responses', 'codex', 1, 1);

        INSERT INTO gateway_models
            (id, model_id, display_name, enabled, source, migration_status, created_at, updated_at)
        VALUES
            ('model-active', 'active-model', 'Active Model', 1, 'manual', 'active', 1, 1),
            ('model-draft', 'draft-model', 'Draft Model', 0, 'manual', 'draft', 1, 1),
            ('model-conflict', 'conflict-model', 'Conflict Model', 0, 'manual', 'conflict', 1, 1),
            ('model-other', 'other-model', 'Other Model', 1, 'manual', 'active', 1, 1),
            ('model-empty', 'empty-model', 'Empty Model', 1, 'manual', 'active', 1, 1);

        INSERT INTO route_targets
            (id, gateway_model_id, upstream_id, target_model, position, enabled, created_at, updated_at)
        VALUES
            ('route-a', 'model-active', 'upstream-a', 'vendor-a', 1000000, 0, 1, 1),
            ('route-b', 'model-active', 'upstream-b', 'vendor-b', 1000001, 1, 1, 1),
            ('route-c', 'model-active', 'upstream-c', 'vendor-c', 1000002, 1, 1, 1),
            ('route-other', 'model-other', 'upstream-a', 'vendor-other', 0, 1, 1, 1),
            ('route-draft', 'model-draft', 'upstream-a', 'vendor-draft', 0, 1, 1, 1),
            ('route-conflict', 'model-conflict', 'upstream-b', 'vendor-conflict', 0, 1, 1, 1);
        "#,
    )
    .expect("seed gateway domain DAO fixture");
}

fn gateway_model_state(db: &Database, id: &str) -> (bool, String) {
    let conn = db.conn.lock().expect("lock gateway model state");
    conn.query_row(
        "SELECT enabled, migration_status FROM gateway_models WHERE id = ?1",
        [id],
        |row| Ok((row.get::<_, i64>(0)? != 0, row.get(1)?)),
    )
    .expect("read gateway model state")
}

fn gateway_route_positions(db: &Database, gateway_model_id: &str) -> Vec<(String, i64)> {
    let conn = db.conn.lock().expect("lock gateway route positions");
    let mut stmt = conn
        .prepare(
            "SELECT id, position FROM route_targets
             WHERE gateway_model_id = ?1 ORDER BY position ASC, id ASC",
        )
        .expect("prepare gateway route positions");
    stmt.query_map([gateway_model_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("query gateway route positions")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect gateway route positions")
}

#[test]
fn gateway_domain_config_update_validates_all_invariants_without_partial_write() {
    let db = Database::memory().expect("memory db");
    let mut valid = db
        .get_gateway_config_record()
        .expect("default gateway config");
    valid.listen_port = 43123;
    valid.enable_logging = false;
    valid.max_retries = 0;
    valid.streaming_first_byte_timeout = 1;
    valid.streaming_idle_timeout = 2;
    valid.non_streaming_timeout = 3;
    valid.circuit_failure_threshold = 1;
    valid.circuit_success_threshold = 1;
    valid.circuit_timeout_seconds = 4;
    valid.circuit_error_rate_threshold = 0.25;
    valid.circuit_min_requests = 1;
    db.update_gateway_config_record(&valid)
        .expect("update valid gateway config");
    assert_eq!(db.get_gateway_config_record().unwrap(), valid);

    let mut invalid_cases = Vec::new();
    let mut invalid = valid.clone();
    invalid.auth_required = false;
    invalid_cases.push(invalid);
    let mut invalid = valid.clone();
    invalid.listen_address = "0.0.0.0".to_string();
    invalid_cases.push(invalid);
    let mut invalid = valid.clone();
    invalid.listen_port = 0;
    invalid_cases.push(invalid);
    for field in [
        "streaming_first_byte_timeout",
        "streaming_idle_timeout",
        "non_streaming_timeout",
        "circuit_timeout_seconds",
    ] {
        let mut invalid = valid.clone();
        match field {
            "streaming_first_byte_timeout" => invalid.streaming_first_byte_timeout = 0,
            "streaming_idle_timeout" => invalid.streaming_idle_timeout = 0,
            "non_streaming_timeout" => invalid.non_streaming_timeout = 0,
            "circuit_timeout_seconds" => invalid.circuit_timeout_seconds = 0,
            _ => unreachable!(),
        }
        invalid_cases.push(invalid);
    }
    for field in [
        "circuit_failure_threshold",
        "circuit_success_threshold",
        "circuit_min_requests",
    ] {
        let mut invalid = valid.clone();
        match field {
            "circuit_failure_threshold" => invalid.circuit_failure_threshold = 0,
            "circuit_success_threshold" => invalid.circuit_success_threshold = 0,
            "circuit_min_requests" => invalid.circuit_min_requests = 0,
            _ => unreachable!(),
        }
        invalid_cases.push(invalid);
    }
    for invalid_rate in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
        let mut invalid = valid.clone();
        invalid.circuit_error_rate_threshold = invalid_rate;
        invalid_cases.push(invalid);
    }

    for invalid in invalid_cases {
        assert!(matches!(
            db.update_gateway_config_record(&invalid),
            Err(crate::error::AppError::InvalidInput(_))
        ));
        assert_eq!(
            db.get_gateway_config_record().unwrap(),
            valid,
            "校验失败不得部分更新配置"
        );
    }
}

#[test]
fn gateway_domain_draft_and_conflict_require_explicit_activation() {
    let db = Database::memory().expect("memory db");
    seed_gateway_domain_dao_fixture(&db);

    for model_id in ["model-draft", "model-conflict"] {
        assert!(matches!(
            db.set_gateway_model_enabled(model_id, true),
            Err(crate::error::AppError::InvalidInput(_))
        ));
        assert!(matches!(
            db.set_gateway_model_state(model_id, true, "draft"),
            Err(crate::error::AppError::InvalidInput(_))
        ));
        assert_ne!(gateway_model_state(&db, model_id), (true, "active".into()));

        assert!(db
            .set_gateway_model_state(model_id, true, "active")
            .expect("explicitly activate gateway model"));
        assert_eq!(gateway_model_state(&db, model_id), (true, "active".into()));

        assert!(db
            .set_gateway_model_enabled(model_id, false)
            .expect("disable active gateway model"));
        assert_eq!(gateway_model_state(&db, model_id), (false, "active".into()));
        assert!(db
            .set_gateway_model_enabled(model_id, true)
            .expect("re-enable confirmed gateway model"));
    }

    assert!(matches!(
        db.set_gateway_model_state("model-active", true, "unknown"),
        Err(crate::error::AppError::InvalidInput(_))
    ));
    assert_eq!(
        gateway_model_state(&db, "model-active"),
        (true, "active".into())
    );
    assert!(!db
        .set_gateway_model_enabled("missing-model", true)
        .expect("missing model is reported as false"));
}

#[test]
fn gateway_domain_route_enable_disable_round_trips_and_reports_missing() {
    let db = Database::memory().expect("memory db");
    seed_gateway_domain_dao_fixture(&db);

    assert!(db
        .set_route_target_enabled("route-a", true)
        .expect("enable route"));
    assert!(
        db.list_route_targets()
            .unwrap()
            .into_iter()
            .find(|route| route.id == "route-a")
            .expect("route-a")
            .enabled
    );
    assert!(db
        .set_route_target_enabled("route-a", false)
        .expect("disable route"));
    assert!(
        !db.list_route_targets()
            .unwrap()
            .into_iter()
            .find(|route| route.id == "route-a")
            .expect("route-a")
            .enabled
    );
    assert!(!db
        .set_route_target_enabled("missing-route", true)
        .expect("missing route is reported as false"));
}

#[test]
fn gateway_domain_reorder_requires_exact_route_set_and_failures_are_atomic() {
    let db = Database::memory().expect("memory db");
    seed_gateway_domain_dao_fixture(&db);

    let reordered = vec!["route-c".into(), "route-a".into(), "route-b".into()];
    db.reorder_route_targets("model-active", &reordered)
        .expect("reorder complete route set");
    assert_eq!(
        gateway_route_positions(&db, "model-active"),
        vec![
            ("route-c".into(), 0),
            ("route-a".into(), 1),
            ("route-b".into(), 2),
        ]
    );

    let stable = gateway_route_positions(&db, "model-active");
    for invalid in [
        vec!["route-a".into(), "route-b".into()],
        vec!["route-a".into(), "route-a".into(), "route-b".into()],
        vec!["route-a".into(), "route-b".into(), "route-other".into()],
    ] {
        assert!(matches!(
            db.reorder_route_targets("model-active", &invalid),
            Err(crate::error::AppError::InvalidInput(_))
        ));
        assert_eq!(gateway_route_positions(&db, "model-active"), stable);
    }
    assert!(matches!(
        db.reorder_route_targets("missing-model", &[]),
        Err(crate::error::AppError::InvalidInput(_))
    ));
    db.reorder_route_targets("model-empty", &[])
        .expect("existing model with no routes accepts empty order");

    {
        let conn = db.conn.lock().expect("lock forced reorder failure");
        conn.execute_batch(
            r#"
            CREATE TRIGGER fail_route_b_reorder
            BEFORE UPDATE OF position ON route_targets
            WHEN OLD.id = 'route-b'
            BEGIN
                SELECT RAISE(ABORT, 'forced reorder failure');
            END;
            "#,
        )
        .expect("install reorder failure trigger");
    }
    let before_forced_failure = gateway_route_positions(&db, "model-active");
    let forced = vec!["route-c".into(), "route-b".into(), "route-a".into()];
    assert!(matches!(
        db.reorder_route_targets("model-active", &forced),
        Err(crate::error::AppError::Database(_))
    ));
    assert_eq!(
        gateway_route_positions(&db, "model-active"),
        before_forced_failure,
        "事务中途失败必须回滚此前位置更新"
    );
}

#[test]
fn migration_v16_to_v17_builds_idempotent_shadow_domain_without_changing_legacy_rows() {
    let db = Database::memory().expect("memory db");
    let conn = db.conn.lock().expect("lock");
    conn.execute_batch(
        r#"
        INSERT INTO providers
            (id, app_type, name, settings_config, created_at, sort_index, meta,
             is_current, in_failover_queue)
        VALUES
            ('p-a', 'claude', 'Claude A',
             '{"env":{"ANTHROPIC_BASE_URL":"https://a.invalid","ANTHROPIC_AUTH_TOKEN":"secret-a"}}',
             100, 0, '{}', 1, 1),
            ('p-b', 'claude', 'Claude B',
             '{"env":{"ANTHROPIC_BASE_URL":"https://b.invalid","ANTHROPIC_API_KEY":"secret-b"}}',
             200, 1, '{}', 0, 1),
            ('p-c', 'codex', 'Codex C',
             '{"auth":{"OPENAI_API_KEY":"secret-c"},"config":"model_provider = \"c\"\n[model_providers.c]\nbase_url = \"https://c.invalid\""}',
             300, 0, '{}', 1, 1);

        INSERT INTO provider_models
            (provider_id, app_type, model_id, source, owned_by, fetched_at)
        VALUES
            ('p-a', 'claude', 'shared-model', 'manual', NULL, 1000),
            ('p-b', 'claude', 'shared-model', 'fetched', 'vendor-b', 1001),
            ('p-c', 'codex', 'shared-model', 'manual', NULL, 1002),
            ('p-b', 'claude', 'other-model', 'manual', NULL, 1003);

        INSERT INTO custom_aggregates
            (id, app_type, name, ordered_members, sort_index, created_at, updated_at)
        VALUES
            ('agg-1', 'claude', 'Legacy Aggregate',
             '["shared-model","other-model"]', 0, 400, 500);

        INSERT INTO settings (key, value) VALUES
            ('cc_aggregate_config:claude',
             '{"enabled":true,"tierSelection":{"sonnet":{"type":"custom","value":"agg-1"}}}');

        UPDATE proxy_config
        SET listen_port = 43123, max_retries = 7
        WHERE app_type = 'claude';
        "#,
    )
    .expect("seed v16 data");
    Database::set_user_version(&conn, 16).expect("set v16");

    Database::apply_schema_migrations_on_conn(&conn).expect("migrate v16 to v17");
    assert_eq!(
        Database::get_user_version(&conn).expect("version"),
        SCHEMA_VERSION
    );

    let legacy_provider_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))
        .expect("legacy providers");
    assert_eq!(legacy_provider_count, 3, "旧 Provider 行必须原样保留");

    let upstream_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM upstreams", [], |row| row.get(0))
        .expect("upstreams");
    let upstream_model_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM upstream_models", [], |row| row.get(0))
        .expect("upstream models");
    assert_eq!(upstream_count, 3);
    assert_eq!(upstream_model_count, 4);

    let config: (i64, i64) = conn
        .query_row(
            "SELECT listen_port, max_retries FROM gateway_config WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("gateway config");
    assert_eq!(config, (43123, 7));

    let status: (String, i64) = conn
        .query_row(
            "SELECT migration_status, enabled FROM gateway_models
             WHERE model_id = 'shared-model' AND source = 'legacy_model'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("shared model status");
    assert_eq!(status, ("conflict".to_string(), 0));

    let active_status: (String, i64) = conn
        .query_row(
            "SELECT migration_status, enabled FROM gateway_models
             WHERE model_id = 'other-model' AND source = 'legacy_model'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("unambiguous model status");
    assert_eq!(active_status, ("active".to_string(), 1));

    let exact_route_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM route_targets rt
             JOIN gateway_models gm ON gm.id = rt.gateway_model_id
             WHERE gm.model_id = 'shared-model'",
            [],
            |row| row.get(0),
        )
        .expect("exact routes");
    assert_eq!(exact_route_count, 3);

    let aggregate_positions: Vec<i64> = {
        let mut stmt = conn
            .prepare(
                "SELECT rt.position FROM route_targets rt
                 JOIN gateway_models gm ON gm.id = rt.gateway_model_id
                 WHERE gm.legacy_source_id = 'agg-1'
                 ORDER BY rt.position",
            )
            .expect("prepare aggregate positions");
        stmt.query_map([], |row| row.get(0))
            .expect("query positions")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect positions")
    };
    assert_eq!(aggregate_positions, vec![0, 1, 2]);

    let conflicts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM gateway_migration_report
             WHERE code = 'cross_namespace_model_conflict'",
            [],
            |row| row.get(0),
        )
        .expect("conflict report");
    assert_eq!(conflicts, 1);

    let credential_plaintext_hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM upstream_credentials
             WHERE CAST(encrypted_payload AS TEXT) LIKE '%secret-%'",
            [],
            |row| row.get(0),
        )
        .expect("credential plaintext search");
    assert_eq!(credential_plaintext_hits, 0);

    let config_secret_hits: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM upstreams
             WHERE config_json LIKE '%secret-a%'
                OR config_json LIKE '%secret-b%'
                OR config_json LIKE '%secret-c%'",
            [],
            |row| row.get(0),
        )
        .expect("config plaintext search");
    assert_eq!(config_secret_hits, 0);

    let counts_before: (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM upstreams),
                (SELECT COUNT(*) FROM upstream_models),
                (SELECT COUNT(*) FROM gateway_models),
                (SELECT COUNT(*) FROM route_targets)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("counts before rerun");
    super::gateway_migration::migrate(&conn).expect("repeat data migration");
    let counts_after: (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM upstreams),
                (SELECT COUNT(*) FROM upstream_models),
                (SELECT COUNT(*) FROM gateway_models),
                (SELECT COUNT(*) FROM route_targets)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("counts after rerun");
    assert_eq!(counts_after, counts_before, "重复迁移不得新增重复行");
}

#[test]
fn gateway_shadow_domain_foreign_keys_reject_orphans_and_cascade() {
    let db = Database::memory().expect("memory db");
    let conn = db.conn.lock().expect("lock");
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO upstreams
            (id, name, enabled, protocol, adapter_type, created_at, updated_at)
         VALUES ('upstream-1', 'One', 1, 'anthropic', 'claude', ?1, ?1)",
        [now],
    )
    .expect("insert upstream");
    conn.execute(
        "INSERT INTO gateway_models
            (id, model_id, display_name, enabled, source, migration_status, created_at, updated_at)
         VALUES ('gateway-model-1', 'model-1', 'Model 1', 0, 'manual', 'draft', ?1, ?1)",
        [now],
    )
    .expect("insert gateway model");
    conn.execute(
        "INSERT INTO route_targets
            (id, gateway_model_id, upstream_id, target_model, position, enabled, created_at, updated_at)
         VALUES ('target-1', 'gateway-model-1', 'upstream-1', 'model-1', 0, 0, ?1, ?1)",
        [now],
    )
    .expect("insert route target");
    conn.execute(
        "INSERT INTO route_target_health
            (route_target_id, state, updated_at) VALUES ('target-1', 'closed', ?1)",
        [now],
    )
    .expect("insert target health");

    let orphan = conn.execute(
        "INSERT INTO route_targets
            (id, gateway_model_id, upstream_id, target_model, position, enabled, created_at, updated_at)
         VALUES ('orphan', 'gateway-model-1', 'missing', 'model-2', 1, 0, ?1, ?1)",
        [now],
    );
    assert!(orphan.is_err(), "外键必须拒绝孤儿 route target");

    conn.execute("DELETE FROM upstreams WHERE id = 'upstream-1'", [])
        .expect("delete upstream");
    let remaining: (i64, i64) = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM route_targets WHERE id = 'target-1'),
                (SELECT COUNT(*) FROM route_target_health WHERE route_target_id = 'target-1')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("cascade counts");
    assert_eq!(remaining, (0, 0));
}

/// C1：v14 三行旧表迁移到 v15 后得到七行，且历史 enabled=1 -> route_mode=proxy。
#[test]
fn migration_v14_to_v15_expands_proxy_config_and_preserves_proxy_route() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute_batch(
        r#"
        CREATE TABLE proxy_config (
            app_type TEXT PRIMARY KEY CHECK (app_type IN ('claude','codex','gemini')),
            proxy_enabled INTEGER NOT NULL DEFAULT 0,
            listen_address TEXT NOT NULL DEFAULT '127.0.0.1',
            listen_port INTEGER NOT NULL DEFAULT 42567,
            enable_logging INTEGER NOT NULL DEFAULT 1,
            enabled INTEGER NOT NULL DEFAULT 0,
            auto_failover_enabled INTEGER NOT NULL DEFAULT 0,
            max_retries INTEGER NOT NULL DEFAULT 3,
            streaming_first_byte_timeout INTEGER NOT NULL DEFAULT 60,
            streaming_idle_timeout INTEGER NOT NULL DEFAULT 120,
            non_streaming_timeout INTEGER NOT NULL DEFAULT 600,
            circuit_failure_threshold INTEGER NOT NULL DEFAULT 4,
            circuit_success_threshold INTEGER NOT NULL DEFAULT 2,
            circuit_timeout_seconds INTEGER NOT NULL DEFAULT 60,
            circuit_error_rate_threshold REAL NOT NULL DEFAULT 0.6,
            circuit_min_requests INTEGER NOT NULL DEFAULT 10,
            default_cost_multiplier TEXT NOT NULL DEFAULT '1',
            pricing_model_source TEXT NOT NULL DEFAULT 'response',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        INSERT INTO proxy_config (app_type, enabled, max_retries) VALUES
            ('claude', 1, 6),
            ('codex', 0, 3),
            ('gemini', 1, 5);
        "#,
    )
    .expect("seed v14 proxy_config");
    Database::set_user_version(&conn, 14).expect("set user_version=14");
    Database::apply_schema_migrations_on_conn_to_version(&conn, 15)
        .expect("apply v14->v15 migration");

    assert_eq!(Database::get_user_version(&conn).expect("version"), 15);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM proxy_config", [], |r| r.get(0))
        .expect("count");
    assert_eq!(count, 7);

    let claude_mode: String = conn
        .query_row(
            "SELECT route_mode FROM proxy_config WHERE app_type = 'claude'",
            [],
            |r| r.get(0),
        )
        .expect("claude mode");
    let codex_mode: String = conn
        .query_row(
            "SELECT route_mode FROM proxy_config WHERE app_type = 'codex'",
            [],
            |r| r.get(0),
        )
        .expect("codex mode");
    assert_eq!(claude_mode, "proxy");
    assert_eq!(codex_mode, "direct");

    // 幂等：重复跑到 v15 不破坏已有 route_mode。
    Database::apply_schema_migrations_on_conn_to_version(&conn, 15)
        .expect("re-apply v15 migrations");
    let claude_mode_again: String = conn
        .query_row(
            "SELECT route_mode FROM proxy_config WHERE app_type = 'claude'",
            [],
            |r| r.get(0),
        )
        .expect("claude mode again");
    assert_eq!(claude_mode_again, "proxy");
}

#[test]
fn schema_create_tables_repairs_legacy_proxy_config_singleton_to_per_app() {
    let conn = Connection::open_in_memory().expect("open memory db");

    // 模拟测试版 v2：user_version=2，但 proxy_config 仍是单例结构（无 app_type）
    Database::set_user_version(&conn, 2).expect("set user_version");
    conn.execute_batch(
        r#"
        CREATE TABLE proxy_config (
            id INTEGER PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 0,
            listen_address TEXT NOT NULL DEFAULT '127.0.0.1',
            listen_port INTEGER NOT NULL DEFAULT 5000,
            max_retries INTEGER NOT NULL DEFAULT 3,
            request_timeout INTEGER NOT NULL DEFAULT 300,
            enable_logging INTEGER NOT NULL DEFAULT 1,
            target_app TEXT NOT NULL DEFAULT 'claude',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        INSERT INTO proxy_config (id, enabled) VALUES (1, 1);
        "#,
    )
    .expect("seed legacy proxy_config");

    Database::create_tables_on_conn(&conn).expect("create tables should repair proxy_config");

    assert!(
        Database::has_column(&conn, "proxy_config", "app_type").expect("check app_type"),
        "proxy_config should be migrated to per-app structure"
    );

    let count: i32 = conn
        .query_row("SELECT COUNT(*) FROM proxy_config", [], |r| r.get(0))
        .expect("count rows");
    assert_eq!(count, 3, "per-app proxy_config should have 3 rows");

    // 新结构下应能按 app_type 查询
    let _: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM proxy_config WHERE app_type = 'claude'",
            [],
            |r| r.get(0),
        )
        .expect("query by app_type");
}

#[test]
fn migration_from_v3_8_schema_v1_to_current_schema_v3() {
    let conn = Connection::open_in_memory().expect("open memory db");
    conn.execute("PRAGMA foreign_keys = ON;", [])
        .expect("enable foreign keys");

    // 模拟 v3.8.* 用户的数据库（schema v1）
    conn.execute_batch(V3_8_SCHEMA_V1_SQL)
        .expect("seed v3.8 schema v1");
    Database::set_user_version(&conn, 1).expect("set user_version=1");

    // 插入一条旧版 Provider + Skill（用于验证迁移不会破坏既有数据）
    conn.execute(
        "INSERT INTO providers (
            id, app_type, name, settings_config, website_url, category,
            created_at, sort_index, notes, icon, icon_color, meta, is_current
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            "p1",
            "claude",
            "Test Provider",
            serde_json::to_string(&json!({ "anthropicApiKey": "sk-test" })).unwrap(),
            Option::<String>::None,
            Option::<String>::None,
            Option::<i64>::None,
            Option::<usize>::None,
            Option::<String>::None,
            Option::<String>::None,
            Option::<String>::None,
            "{}",
            1,
        ],
    )
    .expect("seed provider");

    conn.execute(
        "INSERT INTO skills (key, installed, installed_at) VALUES (?1, ?2, ?3)",
        params!["claude:demo-skill", 1, 1700000000i64],
    )
    .expect("seed legacy skill");

    // 按应用启动流程：先 create_tables（补齐新增表），再 apply_schema_migrations（按 user_version 迁移）
    Database::create_tables_on_conn(&conn).expect("create tables");
    Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");

    assert_eq!(
        Database::get_user_version(&conn).expect("user_version after migration"),
        SCHEMA_VERSION
    );

    // v1 -> v2：providers 新增字段必须补齐
    for column in [
        "cost_multiplier",
        "limit_daily_usd",
        "limit_monthly_usd",
        "provider_type",
        "in_failover_queue",
    ] {
        assert!(
            Database::has_column(&conn, "providers", column).expect("check column"),
            "providers.{column} should exist after migration"
        );
    }

    // 旧 provider 不应丢失，且新增字段应有默认值
    let provider_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM providers WHERE id = 'p1' AND app_type = 'claude'",
            [],
            |r| r.get(0),
        )
        .expect("count providers");
    assert_eq!(provider_count, 1);

    let cost_multiplier: String = conn
        .query_row(
            "SELECT cost_multiplier FROM providers WHERE id = 'p1' AND app_type = 'claude'",
            [],
            |r| r.get(0),
        )
        .expect("read cost_multiplier");
    assert_eq!(cost_multiplier, "1.0");

    // v2 -> v3：skills 表重建为统一结构，并设置 pending 标记（后续由启动时扫描文件系统重建数据）
    assert!(
        Database::has_column(&conn, "skills", "enabled_claude").expect("check skills v3 column"),
        "skills table should be migrated to v3 structure"
    );
    let skills_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
        .expect("count skills");
    assert_eq!(skills_count, 0, "skills table should be rebuilt empty");

    let pending: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'skills_ssot_migration_pending'",
            [],
            |r| r.get(0),
        )
        .ok();
    assert!(
        matches!(pending.as_deref(), Some("true") | Some("1")),
        "skills_ssot_migration_pending should be set after v2->v3 migration"
    );
    let snapshot: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'skills_ssot_migration_snapshot'",
            [],
            |r| r.get(0),
        )
        .ok();
    let snapshot = snapshot.expect("skills migration snapshot should be recorded");
    let snapshot_rows: serde_json::Value =
        serde_json::from_str(&snapshot).expect("parse skills migration snapshot");
    assert!(
        snapshot_rows
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| {
                row.get("directory").and_then(|v| v.as_str()) == Some("demo-skill")
                    && row.get("app_type").and_then(|v| v.as_str()) == Some("claude")
            })),
        "skills migration snapshot should preserve legacy app mapping"
    );

    // v15：proxy_config 七模块 seed 必须存在（否则 UI 会查不到默认值）
    let proxy_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM proxy_config", [], |r| r.get(0))
        .expect("count proxy_config rows");
    assert_eq!(proxy_rows, 7);
    assert!(
        Database::has_column(&conn, "proxy_config", "route_mode").expect("route_mode"),
        "proxy_config.route_mode must exist after full migration"
    );

    // model_pricing 应具备默认数据（迁移时会 seed）
    let pricing_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM model_pricing", [], |r| r.get(0))
        .expect("count model_pricing rows");
    assert!(pricing_rows > 0, "model_pricing should be seeded");
}

#[test]
fn schema_model_pricing_is_seeded_on_init() {
    let db = Database::memory().expect("create memory db");

    let conn = db.conn.lock().expect("lock conn");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM model_pricing", [], |row| row.get(0))
        .expect("count pricing");

    assert!(
        count > 0,
        "模型定价数据应该在初始化时自动填充，实际数量: {}",
        count
    );

    // 验证包含 Claude 模型
    let claude_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_pricing WHERE model_id LIKE 'claude-%'",
            [],
            |row| row.get(0),
        )
        .expect("check claude");
    assert!(
        claude_count > 0,
        "应该包含 Claude 模型定价，实际数量: {}",
        claude_count
    );

    // 验证包含 GPT 模型
    let gpt_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_pricing WHERE model_id LIKE 'gpt-%'",
            [],
            |row| row.get(0),
        )
        .expect("check gpt");
    assert!(
        gpt_count > 0,
        "应该包含 GPT 模型定价，实际数量: {}",
        gpt_count
    );

    // 验证包含 Gemini 模型
    let gemini_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM model_pricing WHERE model_id LIKE 'gemini-%'",
            [],
            |row| row.get(0),
        )
        .expect("check gemini");
    assert!(
        gemini_count > 0,
        "应该包含 Gemini 模型定价，实际数量: {}",
        gemini_count
    );
}

#[test]
fn model_pricing_seed_repairs_known_outdated_builtin_prices() {
    let db = Database::memory().expect("create memory db");

    {
        let conn = db.conn.lock().expect("lock conn");
        conn.execute(
            "UPDATE model_pricing
             SET input_cost_per_million = '1.68',
                 output_cost_per_million = '3.36',
                 cache_read_cost_per_million = '0.14',
                 cache_creation_cost_per_million = '0'
             WHERE model_id = 'deepseek-v4-pro'",
            [],
        )
        .expect("restore old DeepSeek price");
        conn.execute(
            "UPDATE model_pricing
             SET input_cost_per_million = '9',
                 output_cost_per_million = '9',
                 cache_read_cost_per_million = '9',
                 cache_creation_cost_per_million = '0'
             WHERE model_id = 'glm-5.1'",
            [],
        )
        .expect("set custom GLM price");
    }

    db.ensure_model_pricing_seeded()
        .expect("ensure pricing seeded");

    let conn = db.conn.lock().expect("lock conn");
    let deepseek: (String, String, String) = conn
        .query_row(
            "SELECT input_cost_per_million, output_cost_per_million, cache_read_cost_per_million
             FROM model_pricing WHERE model_id = 'deepseek-v4-pro'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("query DeepSeek price");
    assert_eq!(
        deepseek,
        (
            "0.435".to_string(),
            "0.87".to_string(),
            "0.003625".to_string()
        )
    );

    let glm: (String, String, String) = conn
        .query_row(
            "SELECT input_cost_per_million, output_cost_per_million, cache_read_cost_per_million
             FROM model_pricing WHERE model_id = 'glm-5.1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("query GLM price");
    assert_eq!(glm, ("9".to_string(), "9".to_string(), "9".to_string()));
}

#[test]
fn ensure_incremental_auto_vacuum_rebuilds_existing_file_db() {
    let temp = NamedTempFile::new().expect("create temp db file");
    let path = temp.path().to_path_buf();

    let conn = Connection::open(&path).expect("open temp db");
    conn.execute("PRAGMA auto_vacuum = NONE;", [])
        .expect("set none auto_vacuum");
    Database::create_tables_on_conn(&conn).expect("create tables");

    assert_eq!(
        Database::get_auto_vacuum_mode(&conn).expect("auto_vacuum before rebuild"),
        0,
        "existing file db should start with NONE auto_vacuum"
    );

    let rebuilt =
        Database::ensure_incremental_auto_vacuum_on_conn(&conn).expect("enable incremental mode");
    assert!(rebuilt, "existing db should require rebuild via VACUUM");
    drop(conn);

    let reopened = Connection::open(&path).expect("reopen temp db");
    assert_eq!(
        Database::get_auto_vacuum_mode(&reopened).expect("auto_vacuum after rebuild"),
        2,
        "file db should persist INCREMENTAL auto_vacuum after VACUUM rebuild"
    );
}

#[test]
fn v18_credential_readiness_is_fail_closed_for_enabled_routes() {
    use crate::services::credential_protector::CredentialProtector;

    struct TestProtector;
    impl CredentialProtector for TestProtector {
        fn scheme(&self) -> &'static str {
            "test-readiness-v1"
        }
        fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
            Ok(plaintext.iter().map(|byte| byte ^ 0xa5).collect())
        }
        fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
            self.protect(ciphertext)
        }
    }

    let db = Database::memory().expect("memory db");
    {
        let conn = db.conn.lock().expect("lock");
        conn.execute_batch(
            "INSERT INTO upstreams
                (id,name,enabled,base_url,protocol,adapter_type,created_at,updated_at)
             VALUES ('ready-up','Ready',1,'https://ready.invalid','openai_responses','codex',1,1);
             INSERT INTO gateway_models
                (id,model_id,display_name,enabled,source,migration_status,created_at,updated_at)
             VALUES ('ready-gm','ready-model','Ready',1,'manual','active',1,1);
             INSERT INTO route_targets
                (id,gateway_model_id,upstream_id,target_model,position,enabled,created_at,updated_at)
             VALUES ('ready-route','ready-gm','ready-up','vendor-model',0,1,1,1);",
        )
        .expect("seed active route");
    }
    assert!(db
        .ensure_v18_credential_readiness_for_test(&TestProtector)
        .is_err());

    let encrypted = TestProtector.protect(b"sk-ready").expect("protect");
    {
        let conn = db.conn.lock().expect("lock");
        conn.execute(
            "INSERT INTO upstream_credentials
                (id,upstream_id,credential_kind,encrypted_payload,encryption_scheme,created_at,updated_at)
             VALUES ('ready-cred','ready-up','api_key',?1,?2,1,1)",
            rusqlite::params![encrypted, TestProtector.scheme()],
        )
        .expect("seed legacy kind");
    }
    assert!(db
        .ensure_v18_credential_readiness_for_test(&TestProtector)
        .is_err());

    let encrypted = TestProtector.protect(b"sk-ready").expect("protect");
    {
        let conn = db.conn.lock().expect("lock");
        conn.execute(
            "UPDATE upstream_credentials
             SET credential_kind='bearer_token', encrypted_payload=?1, encryption_scheme=?2
             WHERE id='ready-cred'",
            rusqlite::params![encrypted, TestProtector.scheme()],
        )
        .expect("make credential ready");
    }
    db.ensure_v18_credential_readiness_for_test(&TestProtector)
        .expect("typed decryptable adapter-ready credential must pass");

    {
        let conn = db.conn.lock().expect("lock");
        conn.execute(
            "INSERT INTO upstream_credentials
                (id,upstream_id,credential_kind,encrypted_payload,encryption_scheme,created_at,updated_at)
             VALUES ('second-cred','ready-up','x_api_key',X'01',?1,1,1)",
            [TestProtector.scheme()],
        )
        .expect("seed ambiguity");
    }
    assert!(db
        .ensure_v18_credential_readiness_for_test(&TestProtector)
        .is_err());
}

/// v18 真实启动编排必须 fail-closed：先将纯网关回滚导出落盘，成功后才 purge。
/// 同时不得再生成包含客户端快照/旧 providers 明文 secret 的整库 `.db` 预迁移备份。
#[test]
#[serial_test::serial]
fn v18_purge_runs_only_after_readiness_and_verified_local_rollback() {
    struct EnvGuard(Option<std::ffi::OsString>);
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("AGENT_SWITCH_TEST_HOME", value),
                None => std::env::remove_var("AGENT_SWITCH_TEST_HOME"),
            }
        }
    }

    let temp = tempfile::TempDir::new().expect("create isolated test home");
    let _env_guard = EnvGuard(std::env::var_os("AGENT_SWITCH_TEST_HOME"));
    std::env::set_var("AGENT_SWITCH_TEST_HOME", temp.path());
    let config_dir = temp.path().join(".agent-switch");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    let db_path = config_dir.join("agent-switch.db");

    {
        let conn = Connection::open(&db_path).expect("open v17 db");
        Database::create_tables_on_conn(&conn).expect("create schema");
        Database::set_user_version(&conn, 16).expect("mark as v16 before gateway migration");
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, meta)
             VALUES ('legacy-p', 'codex', 'Legacy', ?1, '{}')",
            [r#"{"config":"base_url = \"https://codex.invalid\"\nexperimental_bearer_token = \"must-migrate-before-purge\""}"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proxy_live_backup (app_type, original_config, backed_up_at)
             VALUES ('codex', 'CLIENT_SNAPSHOT_MUST_SURVIVE', '2026-01-01')",
            [],
        )
        .unwrap();
        Database::apply_schema_migrations_on_conn_to_version(&conn, 17)
            .expect("migrate fixture to v17 gateway domain");
        let upstream_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM upstreams", [], |row| row.get(0))
            .expect("count migrated upstreams");
        let credential_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM upstream_credentials", [], |row| {
                row.get(0)
            })
            .expect("count migrated credentials");
        assert_eq!(
            upstream_count, 1,
            "v17 fixture must include migrated upstream"
        );
        assert_eq!(
            credential_count, 1,
            "v17 fixture must include migrated credential"
        );
    }

    let db = Database::init().expect("ready v17 database should purge to v18");
    let conn = db.conn.lock().expect("lock upgraded db");
    assert_eq!(Database::get_user_version(&conn).unwrap(), 18);
    let live_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM proxy_live_backup", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(live_count, 0, "readiness 通过后 v18 应 purge live backup");
    let settings: String = conn
        .query_row(
            "SELECT settings_config FROM providers WHERE id = 'legacy-p'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !settings.contains("must-migrate-before-purge"),
        "v18 应在精确凭据与回滚包验证完成后清除旧 credential source"
    );
    let credential_kind: String = conn
        .query_row(
            "SELECT credential_kind FROM upstream_credentials LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("precise credential must survive purge");
    assert_eq!(credential_kind, "bearer_token");
    drop(conn);

    let rollback_created = config_dir.join("backups").exists()
        && std::fs::read_dir(config_dir.join("backups"))
            .expect("list backups")
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("gateway_rollback_v17_before_v18_"))
            });
    assert!(
        rollback_created,
        "v18 purge 前必须生成并验证 local-gateway-rollback-v1 文件"
    );
}
