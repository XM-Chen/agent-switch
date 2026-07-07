//! Common Config Snippet API（跨 provider 全局层，A1-hybrid）。
//!
//! GET  /api/common-config/{tool}  → 读取全局 common config（未设置返回默认）
//! PUT  /api/common-config/{tool}  → 写入全局 common config（裸 JSON，须为对象）
//!
//! `tool` 取值 `claude-code`（当前仅 Claude Code 有快照层）。common config 在写 live
//! 时 deep-merge 覆盖在 provider 快照之上；per-provider 的三态开关走 provider update
//! 的 `common_config_enabled` 字段。
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde_json::Value;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::services::tool_takeover::{self, Tool, COMMON_CONFIG_CLAUDE_DEFAULT};

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/{tool}", get(get_common).put(put_common))
}

/// 把路径段解析为支持 common config 的 Tool（当前仅 claude-code）。
fn parse_tool(tool: &str) -> Result<Tool, (StatusCode, String)> {
    match Tool::from_str(tool) {
        Some(t @ Tool::ClaudeCode) => Ok(t),
        _ => Err((
            StatusCode::BAD_REQUEST,
            format!("工具 '{}' 不支持 common config", tool),
        )),
    }
}

/// GET /api/common-config/{tool} — 读取 common config；未设置时返回默认值。
async fn get_common(
    State(state): State<Arc<AppState>>,
    Path(tool): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let t = parse_tool(&tool)?;
    let value = tool_takeover::read_common_config(&state.db, t)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let out = match value {
        Some(v) => v,
        // 未设置 → 返回默认，便于前端直接编辑。
        None => serde_json::from_str(COMMON_CONFIG_CLAUDE_DEFAULT).unwrap_or(Value::Null),
    };
    Ok(Json(out))
}

/// PUT /api/common-config/{tool} — 写入 common config（裸 JSON，须为对象）。
async fn put_common(
    State(state): State<Arc<AppState>>,
    Path(tool): Path<String>,
    Json(body): Json<Value>,
) -> Result<StatusCode, (StatusCode, String)> {
    let t = parse_tool(&tool)?;
    if !body.is_object() {
        return Err((
            StatusCode::BAD_REQUEST,
            "common config 必须是 JSON 对象".to_string(),
        ));
    }
    tool_takeover::write_common_config(&state.db, t, &body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}
