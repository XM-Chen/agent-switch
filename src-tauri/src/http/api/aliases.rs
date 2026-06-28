use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::model_aliases::{self, ModelAliasRow, NewModelAlias};

#[derive(Serialize)]
pub struct AliasResponse {
    pub id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub alias_name: String,
    pub target_endpoint_id: Option<String>,
    pub target_model_name: String,
    pub priority: i64,
    pub enabled: bool,
    pub invalid_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<ModelAliasRow> for AliasResponse {
    fn from(r: ModelAliasRow) -> Self {
        Self {
            id: r.id,
            scope_type: r.scope_type,
            scope_id: r.scope_id,
            alias_name: r.alias_name,
            target_endpoint_id: r.target_endpoint_id,
            target_model_name: r.target_model_name,
            priority: r.priority,
            enabled: r.enabled,
            invalid_reason: r.invalid_reason,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Deserialize)]
pub struct AliasQuery {
    pub scope_type: Option<String>,
    pub scope_id: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateAliasRequest {
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub alias_name: String,
    pub target_endpoint_id: Option<String>,
    pub target_model_name: String,
    pub priority: Option<i64>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", axum::routing::delete(delete_one))
}

async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<AliasQuery>,
) -> Result<Json<Vec<AliasResponse>>, (StatusCode, String)> {
    let rows = model_aliases::list(&state.db, q.scope_type.as_deref(), q.scope_id.as_deref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(rows.into_iter().map(AliasResponse::from).collect()))
}

async fn create(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateAliasRequest>,
) -> Result<(StatusCode, Json<AliasResponse>), (StatusCode, String)> {
    let id = uuid::Uuid::new_v4().to_string();
    let new = NewModelAlias {
        id,
        scope_type: req.scope_type,
        scope_id: req.scope_id,
        alias_name: req.alias_name,
        target_endpoint_id: req.target_endpoint_id,
        target_model_name: req.target_model_name,
        priority: req.priority.unwrap_or(0),
    };
    let row = model_aliases::create(&state.db, new)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok((StatusCode::CREATED, Json(AliasResponse::from(row))))
}

async fn delete_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    model_aliases::delete(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}
