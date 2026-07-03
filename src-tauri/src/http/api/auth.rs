use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::Serialize;
use std::sync::Arc;

use crate::app_state::AppState;

#[derive(Serialize)]
pub struct LoginResponse {
    pub auth_url: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct StatusResponse {
    pub login_in_progress: bool,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/codex/login", post(start_codex_login))
        .route("/codex/status", get(codex_status))
}

async fn start_codex_login(
    State(state): State<Arc<AppState>>,
) -> Result<(StatusCode, Json<LoginResponse>), (StatusCode, String)> {
    match state.codex_oauth.start_login(state.clone()).await {
        Ok(auth_url) => Ok((
            StatusCode::OK,
            Json(LoginResponse {
                auth_url,
                message: "请在浏览器中完成 OpenAI 登录授权".to_string(),
            }),
        )),
        Err(e) => {
            if e.contains("已有 Codex OAuth 登录进行中") {
                Err((StatusCode::CONFLICT, e))
            } else {
                Err((StatusCode::INTERNAL_SERVER_ERROR, e))
            }
        }
    }
}

async fn codex_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let in_progress = state.codex_oauth.status().await;
    Json(StatusResponse {
        login_in_progress: in_progress,
    })
}
