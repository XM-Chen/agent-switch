use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::app_metadata;
use crate::services::model_sync::ModelSyncService;

const SETTING_LAST_SYNC_AT: &str = "last_model_sync_at";
const SETTING_LAST_SYNC_ERROR: &str = "last_model_sync_error";

#[derive(Serialize)]
pub struct AutoRefreshResponse {
    pub enabled: bool,
    pub last_sync_at: Option<String>,
    pub last_sync_error: Option<String>,
}

#[derive(Deserialize)]
pub struct SetAutoRefreshRequest {
    pub enabled: bool,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route(
        "/auto-model-refresh",
        get(get_auto_refresh).put(set_auto_refresh),
    )
}

async fn get_auto_refresh(State(state): State<Arc<AppState>>) -> Json<AutoRefreshResponse> {
    let enabled = ModelSyncService::is_auto_refresh_enabled(&state.db);
    let last_sync_at = app_metadata::get(&state.db, SETTING_LAST_SYNC_AT)
        .ok()
        .flatten();
    let last_sync_error = app_metadata::get(&state.db, SETTING_LAST_SYNC_ERROR)
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
    Json(AutoRefreshResponse {
        enabled,
        last_sync_at,
        last_sync_error,
    })
}

async fn set_auto_refresh(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SetAutoRefreshRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    ModelSyncService::set_auto_refresh(&state.db, req.enabled)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}
