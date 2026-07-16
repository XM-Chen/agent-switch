//! SQL fragment helpers shared across usage aggregation queries.
//!
//! Anthropic reports `input_tokens` as fresh (cache reads counted
//! separately); OpenAI Responses API and Google Gemini's
//! `promptTokenCount` both include the cached portion. Any aggregation
//! summing `input_tokens` across providers must route through
//! [`fresh_input_sql`] to recover a consistent semantics.
//!
//! ## 三态语义列（v14 `input_token_semantics`）
//!
//! v14 起每行带一个显式的 `input_token_semantics` 列，取代过去纯靠
//! `app_type` 推断的两态逻辑：
//!
//! | 值 | 常量 | 含义 |
//! |---:|---|---|
//! | 0 | [`INPUT_TOKEN_SEMANTICS_LEGACY`] | 旧行/未知：沿用旧的按 `app_type` 推断（Codex/Gemini 扣 cache_read） |
//! | 1 | [`INPUT_TOKEN_SEMANTICS_TOTAL`] | `input_tokens` 含 cache_read + cache_creation |
//! | 2 | [`INPUT_TOKEN_SEMANTICS_FRESH`] | `input_tokens` 已是 fresh input，不再扣缓存 |
//!
//! LEGACY(0) 分支刻意复刻 v14 之前的两态行为，保证历史行（默认 0）迁移后
//! 归一化结果完全不变。

/// 输入 token 语义：旧行/未知。按 `app_type` 推断（与 v14 之前逻辑一致）。
#[allow(dead_code)]
pub const INPUT_TOKEN_SEMANTICS_LEGACY: i64 = 0;
/// 输入 token 语义：`input_tokens` 含 cache_read + cache_creation。
pub const INPUT_TOKEN_SEMANTICS_TOTAL: i64 = 1;
/// 输入 token 语义：`input_tokens` 已是 fresh input，不再扣缓存。
pub const INPUT_TOKEN_SEMANTICS_FRESH: i64 = 2;

/// Set of `app_type` values whose stored `input_tokens` already includes
/// `cache_read_tokens`. Aggregations subtract cache reads from these rows
/// to recover the fresh-input semantics used by Claude.
///
/// Why list providers explicitly: new providers default to the
/// Claude-style "input excludes cache" semantics, which is safer if the
/// caller forgets to update this list. The wrong direction (a new OpenAI-
/// style provider not added here) shows up loudly as a too-low cache hit
/// rate, which is easier to catch than the silent over-deduction that
/// would happen with the opposite default.
const CACHE_INCLUSIVE_APP_TYPES: &[&str] = &["codex", "gemini"];

/// Build an SQL expression that returns the cache-normalized `input_tokens`
/// for a single row in `proxy_request_logs` or `usage_daily_rollups`.
///
/// 三态列驱动，优先看 `input_token_semantics` 列，仅在 LEGACY(0) 回退到旧的
/// 按 `app_type` 推断：
///
/// - **FRESH(2)**：直接返回 `input_tokens`（已是 fresh，不扣）。
/// - **TOTAL(1)**：`input_tokens` 含全部缓存，扣 cache_read + cache_creation
///   （数值合法时；防负数则原样返回）。
/// - **LEGACY(0) / 其它**：沿用旧两态——`app_type` ∈ [`CACHE_INCLUSIVE_APP_TYPES`]
///   且 `input_tokens >= cache_read_tokens` 时扣 cache_read，否则原样返回。
///
/// Pass an empty string to reference the columns directly (no alias),
/// or a table alias such as `"l"` to emit `l.input_tokens` style references.
pub fn fresh_input_sql(alias: &str) -> String {
    let prefix = if alias.is_empty() {
        String::new()
    } else {
        format!("{alias}.")
    };
    let app_type_list = CACHE_INCLUSIVE_APP_TYPES
        .iter()
        .map(|t| format!("'{t}'"))
        .collect::<Vec<_>>()
        .join(", ");
    // LEGACY 分支：复刻 v14 之前的两态归一化，保证历史行（默认 0）行为不变。
    let legacy_expr = format!(
        "CASE WHEN {prefix}app_type IN ({app_type_list}) AND {prefix}input_tokens >= {prefix}cache_read_tokens \
              THEN ({prefix}input_tokens - {prefix}cache_read_tokens) \
              ELSE {prefix}input_tokens END"
    );
    format!(
        "CASE \
            WHEN {prefix}input_token_semantics = {fresh} THEN {prefix}input_tokens \
            WHEN {prefix}input_token_semantics = {total} \
                 AND {prefix}input_tokens >= ({prefix}cache_read_tokens + {prefix}cache_creation_tokens) \
                THEN ({prefix}input_tokens - {prefix}cache_read_tokens - {prefix}cache_creation_tokens) \
            WHEN {prefix}input_token_semantics = {total} THEN {prefix}input_tokens \
            ELSE {legacy} END",
        fresh = INPUT_TOKEN_SEMANTICS_FRESH,
        total = INPUT_TOKEN_SEMANTICS_TOTAL,
        legacy = legacy_expr,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE proxy_request_logs (
                request_id TEXT PRIMARY KEY,
                app_type TEXT NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                input_token_semantics INTEGER NOT NULL DEFAULT 0
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn fresh_input_with_alias_emits_prefixed_columns() {
        let sql = fresh_input_sql("l");
        assert!(sql.contains("l.app_type"));
        assert!(sql.contains("l.input_tokens"));
        assert!(sql.contains("l.cache_read_tokens"));
    }

    #[test]
    fn fresh_input_without_alias_uses_bare_columns() {
        let sql = fresh_input_sql("");
        assert!(!sql.contains("."));
        assert!(sql.contains("'codex'"));
        assert!(sql.contains("'gemini'"));
    }

    #[test]
    fn fresh_input_subtracts_cache_for_cache_inclusive_providers() {
        let conn = setup_conn();
        // Codex row: OpenAI semantics — input_tokens includes the 600 cached.
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, app_type, input_tokens, cache_read_tokens)
             VALUES ('codex-1', 'codex', 1000, 600)",
            [],
        )
        .unwrap();
        // Gemini row: Google semantics — promptTokenCount includes cachedContentTokenCount.
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, app_type, input_tokens, cache_read_tokens)
             VALUES ('gemini-1', 'gemini', 800, 300)",
            [],
        )
        .unwrap();
        // Claude row: Anthropic semantics — input_tokens already excludes cache.
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, app_type, input_tokens, cache_read_tokens)
             VALUES ('claude-1', 'claude', 200, 5000)",
            [],
        )
        .unwrap();

        let expr = fresh_input_sql("l");
        let sql = format!("SELECT COALESCE(SUM({expr}), 0) FROM proxy_request_logs l");
        let total: i64 = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
        // Codex: 1000-600=400; Gemini: 800-300=500; Claude: 200 unchanged.
        assert_eq!(total, 400 + 500 + 200);
    }

    #[test]
    fn fresh_input_handles_codex_with_cache_exceeding_input() {
        // Defensive: if a malformed Codex row somehow has cache > input,
        // we keep the original value rather than producing a negative number.
        let conn = setup_conn();
        conn.execute(
            "INSERT INTO proxy_request_logs (request_id, app_type, input_tokens, cache_read_tokens)
             VALUES ('codex-broken', 'codex', 100, 999)",
            [],
        )
        .unwrap();
        let expr = fresh_input_sql("l");
        let sql = format!("SELECT {expr} FROM proxy_request_logs l");
        let value: i64 = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
        assert_eq!(value, 100);
    }

    #[test]
    fn fresh_semantics_returns_input_unchanged() {
        // FRESH(2)：input 已是 fresh，即使是 codex 也不扣缓存。
        let conn = setup_conn();
        conn.execute(
            "INSERT INTO proxy_request_logs
             (request_id, app_type, input_tokens, cache_read_tokens, cache_creation_tokens, input_token_semantics)
             VALUES ('fresh-1', 'codex', 1000, 600, 200, 2)",
            [],
        )
        .unwrap();
        let expr = fresh_input_sql("l");
        let sql = format!("SELECT {expr} FROM proxy_request_logs l");
        let value: i64 = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
        // FRESH 分支优先于 app_type 推断：1000 原样返回，不扣 600/200。
        assert_eq!(value, 1000);
    }

    #[test]
    fn total_semantics_subtracts_cache_read_and_creation() {
        // TOTAL(1)：input 含 cache_read + cache_creation，两者都扣。
        let conn = setup_conn();
        conn.execute(
            "INSERT INTO proxy_request_logs
             (request_id, app_type, input_tokens, cache_read_tokens, cache_creation_tokens, input_token_semantics)
             VALUES ('total-1', 'codex', 1000, 600, 200, 1)",
            [],
        )
        .unwrap();
        let expr = fresh_input_sql("l");
        let sql = format!("SELECT {expr} FROM proxy_request_logs l");
        let value: i64 = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
        // TOTAL：1000 - 600 - 200 = 200。
        assert_eq!(value, 200);
    }

    #[test]
    fn total_semantics_guards_against_negative() {
        // TOTAL 但 cache 之和超过 input：防负数，原样返回。
        let conn = setup_conn();
        conn.execute(
            "INSERT INTO proxy_request_logs
             (request_id, app_type, input_tokens, cache_read_tokens, cache_creation_tokens, input_token_semantics)
             VALUES ('total-broken', 'codex', 100, 600, 200, 1)",
            [],
        )
        .unwrap();
        let expr = fresh_input_sql("l");
        let sql = format!("SELECT {expr} FROM proxy_request_logs l");
        let value: i64 = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
        assert_eq!(value, 100);
    }

    #[test]
    fn legacy_semantics_matches_pre_v14_two_state_logic() {
        // LEGACY(0)：显式声明也走旧两态——claude 不扣、codex 扣 cache_read（不扣 creation）。
        let conn = setup_conn();
        conn.execute(
            "INSERT INTO proxy_request_logs
             (request_id, app_type, input_tokens, cache_read_tokens, cache_creation_tokens, input_token_semantics)
             VALUES ('legacy-codex', 'codex', 1000, 600, 200, 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO proxy_request_logs
             (request_id, app_type, input_tokens, cache_read_tokens, cache_creation_tokens, input_token_semantics)
             VALUES ('legacy-claude', 'claude', 200, 5000, 0, 0)",
            [],
        )
        .unwrap();
        let expr = fresh_input_sql("l");
        let sql = format!("SELECT COALESCE(SUM({expr}), 0) FROM proxy_request_logs l");
        let total: i64 = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
        // LEGACY codex: 1000-600=400（只扣 cache_read）；claude: 200 不变。
        assert_eq!(total, 400 + 200);
    }
}
