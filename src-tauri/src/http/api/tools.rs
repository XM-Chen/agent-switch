use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::services::tool_takeover;

/// 工具状态响应(含实时检测结果)。
#[derive(Serialize)]
pub struct ToolStatusResponse {
    pub tool: String,
    pub supports_takeover: bool,
    pub enabled: bool,
    pub live_category: String,
    pub last_applied_at: Option<String>,
    pub last_target: Option<String>,
    pub last_error: Option<String>,
}

impl From<tool_takeover::ToolStatus> for ToolStatusResponse {
    fn from(s: tool_takeover::ToolStatus) -> Self {
        Self {
            tool: s.tool,
            supports_takeover: s.supports_takeover,
            enabled: s.enabled,
            live_category: serde_json::to_value(&s.live_category)
                .and_then(serde_json::from_value)
                .unwrap_or_else(|_| "unrecognized".to_string()),
            last_applied_at: s.last_applied_at,
            last_target: s.last_target,
            last_error: s.last_error,
        }
    }
}

/// 备份记录响应。
#[derive(Serialize)]
pub struct BackupResponse {
    pub id: String,
    pub original_path: String,
    pub backup_path: String,
    pub original_existed: bool,
    pub takeover_target: Option<String>,
    pub created_at: String,
}

/// 接管启停请求。
#[derive(Deserialize)]
pub struct TakeoverRequest {
    pub enabled: bool,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list_all))
        .route("/{tool}", get(get_one))
        .route("/{tool}/takeover", post(set_takeover))
        .route("/{tool}/reapply", post(reapply_takeover))
        .route("/{tool}/backups", get(list_tool_backups))
}

/// GET /api/tools — 列出所有工具状态。
async fn list_all(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ToolStatusResponse>>, (StatusCode, String)> {
    let statuses = tool_takeover::list_statuses(&state.db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(statuses.into_iter().map(Into::into).collect()))
}

/// GET /api/tools/{tool} — 获取单个工具完整状态。
async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(tool): Path<String>,
) -> Result<Json<ToolStatusResponse>, (StatusCode, String)> {
    let t = tool_takeover::Tool::from_str(&tool)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("未知工具: {}", tool)))?;
    let s =
        tool_takeover::status(&state.db, t).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(s.into()))
}

/// POST /api/tools/{tool}/takeover  body {enabled:bool}
/// - enabled=true  → 备份+写入+置 enabled=1
/// - enabled=false → 仅置 enabled=0，不改写工具文件
/// - opencode → 400
async fn set_takeover(
    State(state): State<Arc<AppState>>,
    Path(tool): Path<String>,
    Json(req): Json<TakeoverRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let t = tool_takeover::Tool::from_str(&tool)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("未知工具: {}", tool)))?;

    if !t.supports_takeover() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("工具 '{}' 不支持自动接管", tool),
        ));
    }

    if req.enabled {
        tool_takeover::enable(&state.db, t, &state.data_dir)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    } else {
        tool_takeover::disable(&state.db, t, &state.data_dir)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/tools/{tool}/reapply — 幂等重新应用接管。
/// 要求该工具已开启接管，否则 409。
async fn reapply_takeover(
    State(state): State<Arc<AppState>>,
    Path(tool): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let t = tool_takeover::Tool::from_str(&tool)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("未知工具: {}", tool)))?;

    if !t.supports_takeover() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("工具 '{}' 不支持自动接管", tool),
        ));
    }

    tool_takeover::reapply(&state.db, t, &state.data_dir, state.crypto.as_deref()).map_err(
        |e| {
            if e.contains("未开启接管") {
                (StatusCode::CONFLICT, e)
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, e)
            }
        },
    )?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/tools/{tool}/backups — 列出备份记录。
async fn list_tool_backups(
    State(state): State<Arc<AppState>>,
    Path(tool): Path<String>,
) -> Result<Json<Vec<BackupResponse>>, (StatusCode, String)> {
    let t = tool_takeover::Tool::from_str(&tool)
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("未知工具: {}", tool)))?;

    let backups = tool_takeover::list_backups(&state.db, t)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(
        backups
            .into_iter()
            .map(|b| BackupResponse {
                id: b.id,
                original_path: b.original_path,
                backup_path: b.backup_path,
                original_existed: b.original_existed,
                takeover_target: b.takeover_target,
                created_at: b.created_at,
            })
            .collect(),
    ))
}
