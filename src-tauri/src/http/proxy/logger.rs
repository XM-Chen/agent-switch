/// 请求日志记录器。
///
/// 提供 `RequestLogEntry` 结构体（匹配 `request_logs` 表字段）
/// 和 `write_log` 方法将日志写入 DB。
/// 请求体哈希使用 SHA256 散列。
use std::sync::Mutex;

use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// 请求日志条目，对应 `request_logs` 表。
#[derive(Debug, Clone, Serialize)]
pub struct RequestLogEntry {
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
}

impl RequestLogEntry {
    /// 创建新的日志条目（fields 后续逐步填充）。
    pub fn new(request_id: &str, tool: Option<&str>, inbound_endpoint: &str) -> Self {
        Self {
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
        }
    }

    /// 计算请求体的 SHA256 哈希。
    pub fn hash_body(body: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(body);
        format!("{:x}", hasher.finalize())
    }
}

/// 将日志条目写入 `request_logs` 表。
pub fn write_log(db: &Mutex<Connection>, entry: RequestLogEntry) -> Result<(), String> {
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Iso8601::DEFAULT)
        .map_err(|e| format!("时间格式化失败: {}", e))?;

    let id = uuid::Uuid::new_v4().to_string();

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
        rusqlite::params![
            id,
            entry.request_id,
            entry.tool,
            entry.inbound_endpoint,
            entry.requested_model,
            entry.resolved_alias,
            entry.resolved_scope,
            entry.target_endpoint_id,
            entry.upstream_model,
            entry.upstream_endpoint,
            entry.protocol_from,
            entry.protocol_to,
            entry.status,
            entry.error_kind,
            entry.fallback_chain,
            entry.stream as i64,
            entry.duration_ms,
            entry.first_token_ms,
            entry.input_tokens,
            entry.output_tokens,
            entry.cache_creation_tokens,
            entry.cache_read_tokens,
            entry.request_body_hash,
            now,
        ],
    )
    .map_err(|e| format!("写入请求日志失败: {}", e))?;

    Ok(())
}
