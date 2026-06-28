use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::routing::{any, get};
use axum::{body::Body, response::IntoResponse, Router};
use std::sync::Arc;
use tower_http::services::{ServeDir, ServeFile};

use super::api;
use super::health;
use super::placeholders;
use crate::app_state::AppState;

/// Build the main Axum router with all path-isolated scopes.
pub fn build(state: Arc<AppState>) -> Router {
    let web_dist_dir = state.web_dist_dir.clone();
    let index_file = web_dist_dir.join("index.html");

    Router::new()
        .route("/health", get(health::health_check))
        // 管理 API
        .nest("/api/accounts", api::accounts::routes())
        .nest("/api/endpoints", api::endpoints::routes())
        .nest("/api/auth", api::auth::routes())
        .nest("/api/models", api::models::routes())
        .nest("/api/models/aliases", api::aliases::routes())
        .nest("/api/settings", api::settings::routes())
        .nest("/api/tools", api::tools::routes())
        .route("/api/{*path}", any(placeholders::not_implemented))
        // Claude Code 代理路由
        .route("/claude-code/{*path}", any(claude_code_proxy))
        // Codex 代理路由
        .route("/codex/{*path}", any(codex_proxy))
        // /v1 仍为占位（由 openai-compatible-v1-endpoints 子任务实现）
        .route("/v1/{*path}", any(placeholders::not_implemented))
        .fallback_service(ServeDir::new(web_dist_dir).fallback(ServeFile::new(index_file)))
        .with_state(state)
}

/// Claude Code 代理 handler。
async fn claude_code_proxy(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let route_proxy = state.route_proxy.read().await;
    match route_proxy.as_ref() {
        Some(proxy) => proxy.proxy_request("claude-code", req).await,
        None => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "代理服务未初始化".to_string(),
        )),
    }
}

/// Codex 代理 handler。
async fn codex_proxy(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let route_proxy = state.route_proxy.read().await;
    match route_proxy.as_ref() {
        Some(proxy) => proxy.proxy_request("codex", req).await,
        None => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "代理服务未初始化".to_string(),
        )),
    }
}
