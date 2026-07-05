use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::endpoints::{self, EndpointRow, EndpointUpdate, NewEndpoint};

use super::encrypt_api_key;

/// 端点脱敏响应（不含凭据）。
#[derive(Serialize)]
pub struct EndpointResponse {
    pub id: String,
    pub account_id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub protocol_type: String,
    pub auth_mode: String,
    pub enabled: bool,
    pub priority: i64,
    pub cooldown_until: Option<String>,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_error_kind: Option<String>,
    pub has_api_key: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<EndpointRow> for EndpointResponse {
    fn from(r: EndpointRow) -> Self {
        Self {
            id: r.id,
            account_id: r.account_id,
            name: r.name,
            base_url: r.base_url,
            protocol_type: r.protocol_type,
            auth_mode: r.auth_mode,
            enabled: r.enabled,
            priority: r.priority,
            cooldown_until: r.cooldown_until,
            last_success_at: r.last_success_at,
            last_failure_at: r.last_failure_at,
            last_error_kind: r.last_error_kind,
            has_api_key: r.api_key_encrypted.is_some(),
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Deserialize)]
pub struct CreateEndpointRequest {
    pub account_id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub protocol_type: String,
    pub auth_mode: String,
    pub api_key: Option<String>,
    pub priority: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdateEndpointRequest {
    pub account_id: Option<Option<String>>,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub protocol_type: Option<String>,
    pub auth_mode: Option<String>,
    pub api_key: Option<Option<String>>,
    pub priority: Option<i64>,
}

#[derive(Deserialize)]
pub struct ToggleRequest {
    pub enabled: bool,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(get_one).put(update).delete(delete))
        .route("/{id}/toggle", post(toggle))
}

async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<EndpointResponse>>, (StatusCode, String)> {
    let rows = endpoints::list(&state.db).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(rows.into_iter().map(EndpointResponse::from).collect()))
}

async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<EndpointResponse>, (StatusCode, String)> {
    let row = endpoints::get(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    match row {
        Some(r) => Ok(Json(EndpointResponse::from(r))),
        None => Err((StatusCode::NOT_FOUND, "端点不存在".to_string())),
    }
}

async fn create(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateEndpointRequest>,
) -> Result<(StatusCode, Json<EndpointResponse>), (StatusCode, String)> {
    let id = uuid::Uuid::new_v4().to_string();
    let api_key_encrypted = encrypt_api_key(&state, &id, req.api_key.as_deref())?;

    let new = NewEndpoint {
        id,
        account_id: req.account_id,
        name: req.name,
        base_url: req.base_url,
        protocol_type: req.protocol_type,
        api_key_encrypted,
        auth_mode: req.auth_mode,
        priority: req.priority.unwrap_or(0),
        extra_json: None,
    };
    let row =
        endpoints::create(&state.db, new).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok((StatusCode::CREATED, Json(EndpointResponse::from(row))))
}

async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateEndpointRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut update = EndpointUpdate {
        account_id: req.account_id,
        name: req.name,
        base_url: req.base_url,
        protocol_type: req.protocol_type,
        auth_mode: req.auth_mode,
        priority: req.priority,
        ..Default::default()
    };

    if let Some(opt) = req.api_key {
        let encrypted: Option<Vec<u8>> = match opt {
            None => None,
            Some(key) => encrypt_api_key(&state, &id, Some(&key))?,
        };
        update.api_key_encrypted = Some(encrypted);
    }

    endpoints::update(&state.db, &id, update)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn toggle(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<ToggleRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let update = EndpointUpdate {
        enabled: Some(req.enabled),
        ..Default::default()
    };
    endpoints::update(&state.db, &id, update)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    endpoints::delete(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}
