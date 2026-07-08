//! Prompts 管理 API（cc-prompts，仅 Claude Code）。
//!
//! ```text
//! GET    /api/prompts              → list
//! POST   /api/prompts              → create
//! GET    /api/prompts/{id}         → get
//! PUT    /api/prompts/{id}         → update（不导航启用态）
//! DELETE /api/prompts/{id}         → delete（启用项拒删 → 400）
//! POST   /api/prompts/{id}/enable  → 单激活 + 回填 + 投影
//! POST   /api/prompts/{id}/disable → 禁用（唯一激活项则清空 live）
//! POST   /api/prompts/import       → 反向导入 ~/.claude/CLAUDE.md
//! GET    /api/prompts/status       → live 配置状态
//! ```
//!
//! 任何改动 DB 的写操作成功后按语义投影 live。校验失败 → 400；删除启用项 → 400；
//! IO/写入失败 → 500。
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::prompts::{self, NewPrompt, PromptRow, PromptUpdate};
use crate::services::prompts::claude;

/// Prompt 响应体。
#[derive(Serialize)]
pub struct PromptResponse {
    pub id: String,
    pub name: String,
    pub content: String,
    pub description: Option<String>,
    pub enabled_claude: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<PromptRow> for PromptResponse {
    fn from(r: PromptRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            content: r.content,
            description: r.description,
            enabled_claude: r.enabled_claude,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// 创建 Prompt 请求。缺省不启用（启用走 enable 端点）。
#[derive(Deserialize)]
pub struct CreatePromptRequest {
    pub name: String,
    pub content: String,
    pub description: Option<String>,
}

/// 更新 Prompt 请求（部分字段）。
///
/// 嵌套 `Option`：外层 `Some` 表示「更新该字段」，内层区分「更新为 NULL」。
#[derive(Deserialize)]
pub struct UpdatePromptRequest {
    pub name: Option<String>,
    pub content: Option<String>,
    pub description: Option<Option<String>>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list).post(create))
        // 固定段先于 /{id} 注册，避免被参数路由吞掉。
        .route("/import", post(import))
        .route("/status", get(status))
        .route("/{id}", get(get_one).put(update).delete(delete))
        .route("/{id}/enable", post(enable))
        .route("/{id}/disable", post(disable))
}

fn map_write_error(e: String) -> (StatusCode, String) {
    // 删除启用项等业务校验失败 → 400；其余 IO/写入失败 → 500。
    if e.contains("已启用") || e.contains("不存在") || e.contains("不能") {
        (StatusCode::BAD_REQUEST, e)
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, e)
    }
}

async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PromptResponse>>, (StatusCode, String)> {
    let rows = prompts::list(&state.db).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(rows.into_iter().map(PromptResponse::from).collect()))
}

async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<PromptResponse>, (StatusCode, String)> {
    match prompts::get(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))? {
        Some(r) => Ok(Json(PromptResponse::from(r))),
        None => Err((StatusCode::NOT_FOUND, "prompt 不存在".to_string())),
    }
}

async fn create(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreatePromptRequest>,
) -> Result<(StatusCode, Json<PromptResponse>), (StatusCode, String)> {
    if req.name.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "名称不能为空".to_string()));
    }
    let row = prompts::create(
        &state.db,
        NewPrompt {
            id: uuid::Uuid::new_v4().to_string(),
            name: req.name,
            content: req.content,
            description: req.description,
            enabled_claude: false, // 创建不自动启用
        },
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok((StatusCode::CREATED, Json(PromptResponse::from(row))))
}

async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdatePromptRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    prompts::get(&state.db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "prompt 不存在".to_string()))?;
    if let Some(ref name) = req.name {
        if name.trim().is_empty() {
            return Err((StatusCode::BAD_REQUEST, "名称不能为空".to_string()));
        }
    }
    prompts::update(
        &state.db,
        &id,
        PromptUpdate {
            name: req.name,
            content: req.content,
            description: req.description,
            ..Default::default()
        },
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    claude::delete_prompt(&state.db, &id).map_err(map_write_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/prompts/{id}/enable — 单激活 + 回填保护 + 投影 live。
async fn enable(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    claude::enable_prompt(&state.db, &id).map_err(map_write_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/prompts/{id}/disable — 禁用目标；若唯一激活项则清空 live。
async fn disable(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    claude::disable_prompt(&state.db, &id).map_err(map_write_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/prompts/import — 反向导入 ~/.claude/CLAUDE.md。
async fn import(
    State(state): State<Arc<AppState>>,
) -> Result<Json<claude::ImportReport>, (StatusCode, String)> {
    let report = claude::import_from_claude(&state.db).map_err(map_write_error)?;
    Ok(Json(report))
}

/// GET /api/prompts/status — live 配置状态。
async fn status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<claude::PromptStatus>, (StatusCode, String)> {
    let s = claude::get_status(&state.db).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(s))
}
