/// 请求日志查询 API。
///
/// GET  /api/logs         → 分页过滤查询请求日志
/// GET  /api/logs/{id}    → 单条日志详情
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::request_logs::{self, LogFilter, LogTypeFilter};

/// 日志列表条目（缩短字段，不含 body hash 等明细）。
#[derive(Serialize)]
pub struct LogListItem {
    pub id: String,
    pub request_id: String,
    pub tool: Option<String>,
    pub inbound_endpoint: String,
    pub requested_model: Option<String>,
    pub resolved_alias: Option<String>,
    pub resolved_scope: Option<String>,
    pub target_endpoint_id: Option<String>,
    pub upstream_model: Option<String>,
    pub status: Option<i64>,
    pub error_kind: Option<String>,
    pub fallback_chain: Option<String>,
    pub stream: bool,
    pub duration_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub created_at: String,
}

/// 日志单条详情（全字段）。
#[derive(Serialize)]
pub struct LogDetailResponse {
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

/// 日志筛选查询参数。
#[derive(Deserialize, Default)]
pub struct LogQuery {
    pub tool: Option<String>,
    pub log_type: Option<String>,
    pub status: Option<i64>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// 日志分页响应格式。
#[derive(Serialize)]
pub struct LogListResponse {
    pub items: Vec<LogListItem>,
    pub total: i64,
}

impl From<request_logs::RequestLogRow> for LogListItem {
    fn from(r: request_logs::RequestLogRow) -> Self {
        Self {
            id: r.id,
            request_id: r.request_id,
            tool: r.tool,
            inbound_endpoint: r.inbound_endpoint,
            requested_model: r.requested_model,
            resolved_alias: r.resolved_alias,
            resolved_scope: r.resolved_scope,
            target_endpoint_id: r.target_endpoint_id,
            upstream_model: r.upstream_model,
            status: r.status,
            error_kind: r.error_kind,
            fallback_chain: r.fallback_chain,
            stream: r.stream,
            duration_ms: r.duration_ms,
            first_token_ms: r.first_token_ms,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            created_at: r.created_at,
        }
    }
}

impl From<request_logs::RequestLogRow> for LogDetailResponse {
    fn from(r: request_logs::RequestLogRow) -> Self {
        Self {
            id: r.id,
            request_id: r.request_id,
            tool: r.tool,
            inbound_endpoint: r.inbound_endpoint,
            requested_model: r.requested_model,
            resolved_alias: r.resolved_alias,
            resolved_scope: r.resolved_scope,
            target_endpoint_id: r.target_endpoint_id,
            upstream_model: r.upstream_model,
            upstream_endpoint: r.upstream_endpoint,
            protocol_from: r.protocol_from,
            protocol_to: r.protocol_to,
            status: r.status,
            error_kind: r.error_kind,
            fallback_chain: r.fallback_chain,
            stream: r.stream,
            duration_ms: r.duration_ms,
            first_token_ms: r.first_token_ms,
            input_tokens: r.input_tokens,
            output_tokens: r.output_tokens,
            cache_creation_tokens: r.cache_creation_tokens,
            cache_read_tokens: r.cache_read_tokens,
            request_body_hash: r.request_body_hash,
            created_at: r.created_at,
        }
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list))
        .route("/{id}", get(get_one))
}

/// GET /api/logs — 分页过滤查询日志。
async fn list(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LogQuery>,
) -> Result<Json<LogListResponse>, (StatusCode, String)> {
    let log_type = match query.log_type.as_deref() {
        Some("production") => Some(LogTypeFilter::Production),
        Some("test") => Some(LogTypeFilter::Test),
        Some(other) => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("未知日志类型 '{}', 仅支持 production/test", other),
            ));
        }
        None => None,
    };

    let filter = LogFilter {
        tool: query.tool,
        log_type,
        status: query.status,
        from: query.from,
        to: query.to,
        limit: query.limit.unwrap_or(50),
        offset: query.offset.unwrap_or(0),
    };

    let (rows, total) = request_logs::list(&state.db, filter)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let items: Vec<LogListItem> = rows.into_iter().map(LogListItem::from).collect();
    Ok(Json(LogListResponse { items, total }))
}

/// GET /api/logs/{id} — 单条日志详情。
async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<LogDetailResponse>, (StatusCode, String)> {
    let row = request_logs::get(&state.db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("日志 '{}' 不存在", id)))?;

    Ok(Json(LogDetailResponse::from(row)))
}
