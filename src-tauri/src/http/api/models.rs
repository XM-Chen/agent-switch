use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::endpoint_models::{self, EndpointModelRow, NewEndpointModel};
use crate::services::model_alias;
use crate::services::model_sync::SyncReport;

#[derive(Serialize)]
pub struct ModelResponse {
    pub id: String,
    pub endpoint_id: String,
    pub model_name: String,
    pub display_name: String,
    pub source: String,
    pub capabilities: Vec<String>,
    pub context_window: Option<i64>,
    pub is_available: bool,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<EndpointModelRow> for ModelResponse {
    fn from(r: EndpointModelRow) -> Self {
        let capabilities = r
            .capabilities
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .unwrap_or_default();
        Self {
            id: r.id,
            endpoint_id: r.endpoint_id,
            model_name: r.model_name,
            display_name: r.display_name,
            source: r.source,
            capabilities,
            context_window: r.context_window,
            is_available: r.is_available,
            last_seen_at: r.last_seen_at,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Deserialize)]
pub struct ModelQuery {
    pub endpoint_id: Option<String>,
    pub source: Option<String>,
    pub capability: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateCustomModelRequest {
    pub endpoint_id: String,
    pub model_name: String,
    pub display_name: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub context_window: Option<i64>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list))
        .route("/sync", post(sync_all))
        .route("/custom", post(create_custom))
        .route("/{id}", axum::routing::delete(delete_one))
        .route("/resolve/{alias}", get(resolve_alias))
}

async fn list(
    State(state): State<Arc<AppState>>,
    Query(q): Query<ModelQuery>,
) -> Result<Json<Vec<ModelResponse>>, (StatusCode, String)> {
    let rows = endpoint_models::list(
        &state.db,
        q.endpoint_id.as_deref(),
        q.source.as_deref(),
        q.capability.as_deref(),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(rows.into_iter().map(ModelResponse::from).collect()))
}

async fn sync_all(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SyncReport>, (StatusCode, String)> {
    match state.model_sync.sync_all(state.clone()).await {
        Ok(report) => Ok(Json(report)),
        Err(e) => Err((StatusCode::CONFLICT, e)),
    }
}

async fn create_custom(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCustomModelRequest>,
) -> Result<(StatusCode, Json<ModelResponse>), (StatusCode, String)> {
    let id = uuid::Uuid::new_v4().to_string();
    let capabilities = req
        .capabilities
        .map(|c| serde_json::to_string(&c).unwrap_or_default());
    let new = NewEndpointModel {
        id,
        endpoint_id: req.endpoint_id,
        model_name: req.model_name,
        display_name: req.display_name.unwrap_or_default(),
        source: "custom".to_string(),
        capabilities,
        context_window: req.context_window,
        last_seen_at: None,
    };
    let row = endpoint_models::create(&state.db, new)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok((StatusCode::CREATED, Json(ModelResponse::from(row))))
}

async fn delete_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    // 删除前标记关联别名失效。
    if let Some(model) =
        endpoint_models::get(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        let _ = endpoint_models::mark_alias_invalid_for_model(
            &state.db,
            &model.endpoint_id,
            &model.model_name,
        );
    }
    endpoint_models::delete(&state.db, &id).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct ResolveQuery {
    pub tool: Option<String>,
    pub route_id: Option<String>,
    pub endpoint_id: Option<String>,
}

async fn resolve_alias(
    State(state): State<Arc<AppState>>,
    Path(alias): Path<String>,
    Query(q): Query<ResolveQuery>,
) -> Json<model_alias::ResolvedAlias> {
    let ctx = model_alias::ResolutionContext {
        tool: q.tool,
        route_id: q.route_id,
        endpoint_id: q.endpoint_id,
    };
    Json(model_alias::resolve(&state.db, &alias, &ctx))
}
