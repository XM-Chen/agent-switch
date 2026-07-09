//! Claude Code 会话只读浏览 API。
//!
//! ```text
//! GET /api/sessions          → 分页搜索会话列表
//! GET /api/sessions/messages → 单个 JSONL 消息详情
//! ```

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::services::sessions::claude::{
    self, SessionListResponse, SessionMessagesResponse, SessionQuery,
};

const APP_TYPE: &str = "claude-code";
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;

#[derive(Deserialize)]
pub struct ListQuery {
    pub app_type: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub search: Option<String>,
}

#[derive(Deserialize)]
pub struct MessagesQuery {
    pub app_type: Option<String>,
    pub source_path: String,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list))
        .route("/messages", get(messages))
}

fn ensure_claude_code(app_type: Option<&str>) -> Result<(), (StatusCode, String)> {
    match app_type {
        Some(APP_TYPE) => Ok(()),
        Some(other) => Err((
            StatusCode::BAD_REQUEST,
            format!("暂不支持 app_type '{}', 仅支持 claude-code", other),
        )),
        None => Err((StatusCode::BAD_REQUEST, "缺少 app_type 参数".to_string())),
    }
}

async fn list(
    Query(query): Query<ListQuery>,
) -> Result<Json<SessionListResponse>, (StatusCode, String)> {
    ensure_claude_code(query.app_type.as_deref())?;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let resp = claude::scan_sessions(SessionQuery {
        limit,
        offset: query.offset.unwrap_or(0),
        search: query.search,
    })
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}

async fn messages(
    Query(query): Query<MessagesQuery>,
) -> Result<Json<SessionMessagesResponse>, (StatusCode, String)> {
    ensure_claude_code(query.app_type.as_deref())?;
    let resp = claude::read_session_messages(&query.source_path).map_err(|e| {
        if e.contains("source_path")
            || e.contains(".jsonl")
            || e.contains("子代理")
            || e.contains("会话根目录")
        {
            (StatusCode::BAD_REQUEST, e)
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, e)
        }
    })?;
    Ok(Json(resp))
}
