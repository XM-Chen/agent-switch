//! 请求日志 DAO。
//!
//! 对应设计文档 §2 数据模型 `request_logs` 表。
//! 每次转发写一条记录；支持按 tool/status/时间范围/limit/offset 筛选查询。
//!
//! 写入侧 `insert`/`new_log`/`now_iso` 为主循环日志接线预留。
#![allow(dead_code)]
use rusqlite::{params, Connection};
use std::sync::Mutex;
use time::OffsetDateTime;

/// 请求日志行。
#[derive(Debug, Clone)]
pub struct RequestLogRow {
    pub id: String,
    pub request_id: String,
    pub tool: Option<String>,
    pub inbound_endpoint: String,
    pub requested_model: Option<String>,
    pub resolved_alias: Option<String>,
    pub resolved_scope: Option<String>,
    pub target_endpoint_id: Option<String>,
    pub upstream_model: Option<String>,
    pub upstream_endpoint: Option<String>,
    pub protocol_from: Option<String>,
    pub protocol_to: Option<String>,
    pub status: Option<i64>,
    pub error_kind: Option<String>,
    pub fallback_chain: Option<String>,
    pub stream: bool,
    pub duration_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_creation_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub request_body_hash: Option<String>,
    pub created_at: String,
}

/// 日志类型筛选条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogTypeFilter {
    Production,
    Test,
}

/// 日志列表筛选条件。
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    pub tool: Option<String>,
    pub log_type: Option<LogTypeFilter>,
    pub status: Option<i64>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

fn now_iso() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))
}

fn row_to_log(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestLogRow> {
    Ok(RequestLogRow {
        id: row.get("id")?,
        request_id: row.get("request_id")?,
        tool: row.get("tool")?,
        inbound_endpoint: row.get("inbound_endpoint")?,
        requested_model: row.get("requested_model")?,
        resolved_alias: row.get("resolved_alias")?,
        resolved_scope: row.get("resolved_scope")?,
        target_endpoint_id: row.get("target_endpoint_id")?,
        upstream_model: row.get("upstream_model")?,
        upstream_endpoint: row.get("upstream_endpoint")?,
        protocol_from: row.get("protocol_from")?,
        protocol_to: row.get("protocol_to")?,
        status: row.get("status")?,
        error_kind: row.get("error_kind")?,
        fallback_chain: row.get("fallback_chain")?,
        stream: row.get::<_, i64>("stream")? != 0,
        duration_ms: row.get("duration_ms")?,
        first_token_ms: row.get("first_token_ms")?,
        input_tokens: row.get("input_tokens")?,
        output_tokens: row.get("output_tokens")?,
        cache_creation_tokens: row.get("cache_creation_tokens")?,
        cache_read_tokens: row.get("cache_read_tokens")?,
        request_body_hash: row.get("request_body_hash")?,
        created_at: row.get("created_at")?,
    })
}

/// 获取单条日志。
pub fn get(db: &Mutex<Connection>, id: &str) -> Result<Option<RequestLogRow>, String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let mut stmt = db
        .prepare("SELECT * FROM request_logs WHERE id = ?1")
        .map_err(|e| format!("查询日志失败: {}", e))?;
    let mut rows = stmt
        .query_map(params![id], row_to_log)
        .map_err(|e| format!("读取日志失败: {}", e))?;
    rows.next()
        .transpose()
        .map_err(|e| format!("日志行解析失败: {}", e))
}

/// 分页查询请求日志，返回（行列表, 总条数）。
pub fn list(
    db: &Mutex<Connection>,
    filter: LogFilter,
) -> Result<(Vec<RequestLogRow>, i64), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;

    // 构建 WHERE 子句
    let mut conditions: Vec<String> = Vec::new();
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    // log_type 过滤：production 排除 tool='test'，test 等价 tool='test'
    match filter.log_type {
        Some(LogTypeFilter::Test) => {
            // 测试日志类型优先：等价 tool='test'，忽略额外 tool 参数
            params_vec.push(Box::new("test".to_string()));
            conditions.push(format!("tool = ?{}", params_vec.len()));
        }
        Some(LogTypeFilter::Production) => {
            // 生产日志：tool IS NULL OR tool != 'test'；如额外指定 tool，则再收窄到该工具
            params_vec.push(Box::new("test".to_string()));
            conditions.push(format!("(tool IS NULL OR tool != ?{})", params_vec.len()));
            if let Some(ref tool) = filter.tool {
                params_vec.push(Box::new(tool.clone()));
                conditions.push(format!("tool = ?{}", params_vec.len()));
            }
        }
        None => {
            // tool 参数单独存在时保持原语义
            if let Some(ref tool) = filter.tool {
                params_vec.push(Box::new(tool.clone()));
                conditions.push(format!("tool = ?{}", params_vec.len()));
            }
        }
    }

    if let Some(status) = filter.status {
        params_vec.push(Box::new(status));
        conditions.push(format!("status = ?{}", params_vec.len()));
    }
    if let Some(ref from) = filter.from {
        params_vec.push(Box::new(from.clone()));
        conditions.push(format!("created_at >= ?{}", params_vec.len()));
    }
    if let Some(ref to) = filter.to {
        params_vec.push(Box::new(to.clone()));
        conditions.push(format!("created_at <= ?{}", params_vec.len()));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    // 查询总数
    let count_sql = format!("SELECT COUNT(*) FROM request_logs {}", where_clause);
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let total: i64 = db
        .query_row(&count_sql, param_refs.as_slice(), |row| row.get(0))
        .map_err(|e| format!("查询日志总数失败: {}", e))?;

    // 查询分页数据
    let limit = if filter.limit <= 0 {
        50
    } else if filter.limit > 1000 {
        1000
    } else {
        filter.limit
    };
    let offset = if filter.offset < 0 { 0 } else { filter.offset };

    let data_sql = format!(
        "SELECT * FROM request_logs {} ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}",
        where_clause,
        params_vec.len() + 1,
        params_vec.len() + 2,
    );
    params_vec.push(Box::new(limit));
    params_vec.push(Box::new(offset));

    let param_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = db
        .prepare(&data_sql)
        .map_err(|e| format!("查询日志列表失败: {}", e))?;
    let rows = stmt
        .query_map(param_refs.as_slice(), row_to_log)
        .map_err(|e| format!("读取日志列表失败: {}", e))?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("日志行解析失败: {}", e))?);
    }

    Ok((out, total))
}

/// 插入一条请求日志。
pub fn insert(db: &Mutex<Connection>, log: &RequestLogRow) -> Result<(), String> {
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    db.execute(
        "INSERT INTO request_logs (
            id, request_id, tool, inbound_endpoint, requested_model,
            resolved_alias, resolved_scope, target_endpoint_id, upstream_model,
            upstream_endpoint, protocol_from, protocol_to, status, error_kind,
            fallback_chain, stream, duration_ms, first_token_ms,
            input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens,
            request_body_hash, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
        params![
            log.id,
            log.request_id,
            log.tool,
            log.inbound_endpoint,
            log.requested_model,
            log.resolved_alias,
            log.resolved_scope,
            log.target_endpoint_id,
            log.upstream_model,
            log.upstream_endpoint,
            log.protocol_from,
            log.protocol_to,
            log.status,
            log.error_kind,
            log.fallback_chain,
            log.stream as i64,
            log.duration_ms,
            log.first_token_ms,
            log.input_tokens,
            log.output_tokens,
            log.cache_creation_tokens,
            log.cache_read_tokens,
            log.request_body_hash,
            log.created_at,
        ],
    )
    .map_err(|e| format!("插入日志失败: {}", e))?;
    Ok(())
}

/// 删除超过保留条数的旧日志（按 created_at ASC / id ASC 删除最旧的）。
///
/// 先计算超额行数，再只删除最旧的 `overflow` 条 rowid，避免 `id NOT IN
/// (SELECT ... LIMIT max_rows)` 对同表做大范围反连接。
pub fn prune_old(db: &Mutex<Connection>, max_rows: i64) -> Result<usize, String> {
    if max_rows <= 0 {
        return Ok(0);
    }
    let db = db.lock().map_err(|e| format!("无法锁定数据库: {}", e))?;
    let total: i64 = db
        .query_row("SELECT COUNT(*) FROM request_logs", [], |row| row.get(0))
        .map_err(|e| format!("查询日志总数失败: {}", e))?;
    let overflow = total - max_rows;
    if overflow <= 0 {
        return Ok(0);
    }

    let count = db
        .execute(
            "DELETE FROM request_logs
             WHERE rowid IN (
                 SELECT rowid FROM request_logs
                 ORDER BY created_at ASC, id ASC
                 LIMIT ?1
             )",
            params![overflow],
        )
        .map_err(|e| format!("清理旧日志失败: {}", e))?;
    Ok(count)
}

/// 创建一个新的 RequestLogRow（生成 id 和 created_at）。
pub fn new_log(
    request_id: &str,
    tool: Option<&str>,
    inbound_endpoint: &str,
) -> Result<RequestLogRow, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let created = now_iso()?;
    Ok(RequestLogRow {
        id,
        request_id: request_id.to_string(),
        tool: tool.map(|s| s.to_string()),
        inbound_endpoint: inbound_endpoint.to_string(),
        requested_model: None,
        resolved_alias: None,
        resolved_scope: None,
        target_endpoint_id: None,
        upstream_model: None,
        upstream_endpoint: None,
        protocol_from: None,
        protocol_to: None,
        status: None,
        error_kind: None,
        fallback_chain: None,
        stream: false,
        duration_ms: None,
        first_token_ms: None,
        input_tokens: None,
        output_tokens: None,
        cache_creation_tokens: None,
        cache_read_tokens: None,
        request_body_hash: None,
        created_at: created,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations::run_migrations;

    fn fresh_db() -> Mutex<Connection> {
        let db = Mutex::new(Connection::open_in_memory().expect("内存数据库应可创建"));
        run_migrations(&db).expect("迁移应成功");
        db
    }

    #[test]
    fn prune_old_deletes_only_overflow_rows() {
        let db = fresh_db();
        {
            let conn = db.lock().expect("锁数据库");
            for i in 0..5 {
                conn.execute(
                    "INSERT INTO request_logs (id, request_id, inbound_endpoint, stream, created_at)
                     VALUES (?1, ?2, '/claude-code', 0, ?3)",
                    params![
                        format!("log-{}", i),
                        format!("req-{}", i),
                        format!("2024-01-0{}T00:00:00Z", i + 1),
                    ],
                )
                .expect("插入日志");
            }
        }

        let deleted = prune_old(&db, 3).expect("清理日志应成功");
        assert_eq!(deleted, 2);

        let conn = db.lock().expect("锁数据库");
        let mut stmt = conn
            .prepare("SELECT id FROM request_logs ORDER BY created_at ASC, id ASC")
            .expect("prepare");
        let kept: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("collect");
        assert_eq!(kept, vec!["log-2", "log-3", "log-4"]);
    }
}
