//! MCP 服务器管理 API（cc-mcp，仅 Claude Code）。
//!
//! ```text
//! GET    /api/mcp          → list（含富信息，供 UI）
//! POST   /api/mcp          → create（校验规范）→ 同步 live
//! GET    /api/mcp/{id}     → get
//! PUT    /api/mcp/{id}     → update（含 enabled 切换）→ 同步 live
//! DELETE /api/mcp/{id}     → delete → 同步 live
//! POST   /api/mcp/sync     → 手动全量同步（幂等兜底）
//! POST   /api/mcp/import   → 反向导入 ~/.claude.json → 同步 live
//! GET    /api/mcp/status   → live 配置状态
//! ```
//!
//! 任何改动 DB 的写操作成功后调用 `sync_enabled_to_claude`，保证 DB↔live 一致。
//! 校验失败 → 400；`~/.claude.json` 根非对象 → 400（不破坏用户文件）；IO/写入失败 → 500。
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::mcp_servers::{self, McpServerRow, McpServerUpdate, NewMcpServer};
use crate::services::mcp::claude;
use crate::services::mcp::validation::validate_server_spec;

/// MCP 服务器响应体。`server_config` / `tags` 以 JSON Value 返回。
#[derive(Serialize)]
pub struct McpServerResponse {
    pub id: String,
    pub name: String,
    pub server_config: Value,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub docs: Option<String>,
    pub tags: Value,
    pub enabled_claude: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<McpServerRow> for McpServerResponse {
    fn from(r: McpServerRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            server_config: serde_json::from_str(&r.server_config).unwrap_or(Value::Null),
            description: r.description,
            homepage: r.homepage,
            docs: r.docs,
            tags: serde_json::from_str(&r.tags).unwrap_or(Value::Null),
            enabled_claude: r.enabled_claude,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// 创建 MCP 服务器请求。`server_config` 为裸 MCP 规范（写 live 原样投影）。
#[derive(Deserialize)]
pub struct CreateMcpRequest {
    pub name: String,
    pub server_config: Value,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub docs: Option<String>,
    /// JSON 数组；缺省 `[]`。
    pub tags: Option<Value>,
    /// 缺省启用（新建 MCP 默认想让它生效）。
    pub enabled_claude: Option<bool>,
}

/// 更新 MCP 服务器请求（部分字段）。
///
/// 嵌套 `Option`：外层 `Some` 表示「更新该字段」，内层区分「更新为 NULL」。
#[derive(Deserialize)]
pub struct UpdateMcpRequest {
    pub name: Option<String>,
    pub server_config: Option<Value>,
    pub description: Option<Option<String>>,
    pub homepage: Option<Option<String>>,
    pub docs: Option<Option<String>>,
    pub tags: Option<Value>,
    pub enabled_claude: Option<bool>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list).post(create))
        // 固定段先于 /{id} 注册，避免被参数路由吞掉。
        .route("/sync", post(sync))
        .route("/import", post(import))
        .route("/status", get(status))
        .route("/{id}", get(get_one).put(update).delete(delete))
}

/// 把 service 层 error 字符串映射为 HTTP 状态：根非对象 → 400，其余 → 500。
fn map_sync_error(e: String) -> (StatusCode, String) {
    if e.contains("根不是 JSON 对象") {
        (StatusCode::BAD_REQUEST, e)
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, e)
    }
}

/// 校验 tags 为 JSON 数组并序列化；缺省 `[]`。
fn tags_to_text(tags: Option<Value>) -> Result<String, (StatusCode, String)> {
    match tags {
        None => Ok("[]".to_string()),
        Some(v) if v.is_array() => serde_json::to_string(&v)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
        Some(_) => Err((StatusCode::BAD_REQUEST, "tags 必须是 JSON 数组".to_string())),
    }
}

/// 校验 server_config 为合法 MCP 规范并序列化。
fn spec_to_text(spec: &Value) -> Result<String, (StatusCode, String)> {
    validate_server_spec(spec).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    serde_json::to_string(spec).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<McpServerResponse>>, (StatusCode, String)> {
    let rows = mcp_servers::list(&state.db).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(
        rows.into_iter().map(McpServerResponse::from).collect(),
    ))
}

async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<McpServerResponse>, (StatusCode, String)> {
    let row =
        mcp_servers::get(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    match row {
        Some(r) => Ok(Json(McpServerResponse::from(r))),
        None => Err((StatusCode::NOT_FOUND, "MCP 服务器不存在".to_string())),
    }
}

async fn create(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateMcpRequest>,
) -> Result<(StatusCode, Json<McpServerResponse>), (StatusCode, String)> {
    let server_config = spec_to_text(&req.server_config)?;
    let tags = tags_to_text(req.tags)?;

    let new = NewMcpServer {
        id: uuid::Uuid::new_v4().to_string(),
        name: req.name,
        server_config,
        description: req.description,
        homepage: req.homepage,
        docs: req.docs,
        tags,
        enabled_claude: req.enabled_claude.unwrap_or(true),
    };
    let row =
        mcp_servers::create(&state.db, new).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    claude::sync_enabled_to_claude(&state.db).map_err(map_sync_error)?;
    Ok((StatusCode::CREATED, Json(McpServerResponse::from(row))))
}

async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMcpRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    // 确认存在。
    mcp_servers::get(&state.db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "MCP 服务器不存在".to_string()))?;

    let server_config = match &req.server_config {
        Some(spec) => Some(spec_to_text(spec)?),
        None => None,
    };
    let tags = match req.tags {
        Some(v) => Some(tags_to_text(Some(v))?),
        None => None,
    };

    let upd = McpServerUpdate {
        name: req.name,
        server_config,
        description: req.description,
        homepage: req.homepage,
        docs: req.docs,
        tags,
        enabled_claude: req.enabled_claude,
    };
    mcp_servers::update(&state.db, &id, upd).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    claude::sync_enabled_to_claude(&state.db).map_err(map_sync_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    mcp_servers::delete(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    claude::sync_enabled_to_claude(&state.db).map_err(map_sync_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/mcp/sync — 手动全量同步（幂等兜底）。
async fn sync(State(state): State<Arc<AppState>>) -> Result<StatusCode, (StatusCode, String)> {
    claude::sync_enabled_to_claude(&state.db).map_err(map_sync_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/mcp/import — 反向导入 ~/.claude.json，导入后同步 live。
async fn import(
    State(state): State<Arc<AppState>>,
) -> Result<Json<claude::ImportReport>, (StatusCode, String)> {
    let report = claude::import_from_claude(&state.db).map_err(map_sync_error)?;
    claude::sync_enabled_to_claude(&state.db).map_err(map_sync_error)?;
    Ok(Json(report))
}

/// GET /api/mcp/status — live 配置状态。
async fn status(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<claude::McpStatus>, (StatusCode, String)> {
    let s = claude::get_status().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(s))
}
