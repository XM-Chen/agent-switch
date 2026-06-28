use axum::routing::any;
use axum::{routing::get, Router};
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
        // 管理 API：accounts / endpoints / auth / models / aliases / settings
        .nest("/api/accounts", api::accounts::routes())
        .nest("/api/endpoints", api::endpoints::routes())
        .nest("/api/auth", api::auth::routes())
        .nest("/api/models", api::models::routes())
        .nest("/api/models/aliases", api::aliases::routes())
        .nest("/api/settings", api::settings::routes())
        .route("/api/{*path}", any(placeholders::not_implemented))
        .route("/claude-code/{*path}", any(placeholders::not_implemented))
        .route("/codex/{*path}", any(placeholders::not_implemented))
        .route("/v1/{*path}", any(placeholders::not_implemented))
        .fallback_service(ServeDir::new(web_dist_dir).fallback(ServeFile::new(index_file)))
        .with_state(state)
}
