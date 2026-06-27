use axum::{extract::State, response::Json};
use serde::Serialize;
use std::sync::Arc;

use crate::app_state::AppState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub app: &'static str,
    pub version: &'static str,
    pub address: &'static str,
    pub database: String,
}

/// GET /health — returns the service health status.
pub async fn health_check(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    let db_status = match state.db.lock() {
        Ok(conn) => match conn.execute_batch("SELECT 1;") {
            Ok(_) => "ok".to_string(),
            Err(e) => format!("error: {}", e),
        },
        Err(e) => format!("lock_error: {}", e),
    };

    Json(HealthResponse {
        status: "ok".to_string(),
        app: "agent-switch",
        version: "0.1.0",
        address: "127.0.0.1:42567",
        database: db_status,
    })
}
