use axum::extract::{Path, Request, State};
use axum::http::StatusCode;
use axum::routing::{any, get, post};
use axum::{body::Body, response::IntoResponse, Router};
use std::collections::HashMap;
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
        .nest("/api/routes", api::routes::routes())
        .nest("/api/logs", api::logs::routes())
        .route("/api/tests", post(api::tests::run_test))
        .route("/api/{*path}", any(placeholders::not_implemented))
        // Claude Code 代理路由
        .route("/claude-code/{*path}", any(claude_code_proxy))
        // Codex 代理路由
        .route("/codex/{*path}", any(codex_proxy))
        // OpenAI-compatible v1 多端点路由
        .route("/v1/{*path}", any(v1_handler))
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
        Some(proxy) => proxy.proxy_request("claude-code", req, false).await,
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
        Some(proxy) => proxy.proxy_request("codex", req, false).await,
        None => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "代理服务未初始化".to_string(),
        )),
    }
}

/// OpenAI-compatible v1 多端点 handler。
///
/// 根据子路径分发：
/// - `/v1/models`（GET）→ 聚合模型列表
/// - 其余 `/v1/*` → RouteProxy::proxy_request("v1", req)
async fn v1_handler(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    req: Request<Body>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let normalized = path.trim_start_matches('/');

    // GET /v1/models → 不走代理管道，直接聚合 DB 返回
    if req.method() == axum::http::Method::GET
        && (normalized == "models" || normalized.starts_with("models?"))
    {
        // 解析 query params
        let uri = req.uri().clone();
        let query_str = uri.query().unwrap_or("");
        let params: HashMap<String, String> = url::form_urlencoded::parse(query_str.as_bytes())
            .into_owned()
            .collect();

        return api::v1_models::get_models(State(state), axum::extract::Query(params))
            .await
            .map(|json| {
                (
                    StatusCode::OK,
                    axum::response::Response::builder()
                        .header("content-type", "application/json")
                        .body(Body::from(serde_json::to_vec(&json.0).unwrap_or_default()))
                        .unwrap(),
                )
                    .into_response()
            })
            .map(axum::response::IntoResponse::into_response);
    }

    // 其他 /v1/* → 代理管道
    let route_proxy = state.route_proxy.read().await;
    match route_proxy.as_ref() {
        Some(proxy) => proxy.proxy_request("v1", req, false).await,
        None => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "代理服务未初始化".to_string(),
        )),
    }
}
