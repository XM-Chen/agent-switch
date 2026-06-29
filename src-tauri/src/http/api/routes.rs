/// 路由设置管理 API。
///
/// GET  /api/routes          → 列出所有路由设置 + 各路由候选端点
/// PUT  /api/routes/{id}     → 更新路由设置（部分字段）
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::endpoints::{self, EndpointRow};
use crate::db::dao::route_settings;

/// 路由候选端点（脱敏展示，不含凭据）。
#[derive(Serialize)]
pub struct RouteCandidate {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub protocol_type: String,
    pub priority: i64,
    pub enabled: bool,
    pub cooldown_until: Option<String>,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_error_kind: Option<String>,
}

impl From<EndpointRow> for RouteCandidate {
    fn from(r: EndpointRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            base_url: r.base_url,
            protocol_type: r.protocol_type,
            priority: r.priority,
            enabled: r.enabled,
            cooldown_until: r.cooldown_until,
            last_success_at: r.last_success_at,
            last_failure_at: r.last_failure_at,
            last_error_kind: r.last_error_kind,
        }
    }
}

/// 路由详情响应。
#[derive(Serialize)]
pub struct RouteDetailResponse {
    pub id: String,
    pub label: String,
    pub strategy: String,
    pub protocol_type: String,
    pub failover_enabled: bool,
    pub max_switches: i64,
    pub same_account_retries: i64,
    pub cooldown_multiplier: f64,
    pub updated_at: String,
    pub candidates: Vec<RouteCandidate>,
}

/// 路由更新请求（全字段可选）。
#[derive(Deserialize)]
pub struct UpdateRouteRequest {
    pub strategy: Option<String>,
    pub failover_enabled: Option<bool>,
    pub max_switches: Option<i64>,
    pub same_account_retries: Option<i64>,
    pub cooldown_multiplier: Option<f64>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(list))
        .route("/{id}", get(get_one).put(update))
}

/// GET /api/routes — 列出所有路由设置 + 候选端点。
async fn list(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<RouteDetailResponse>>, (StatusCode, String)> {
    let settings_list =
        route_settings::list_all(&state.db).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let mut out = Vec::new();
    for s in settings_list {
        let candidates = load_candidates(&state, &s.protocol_type)?;
        out.push(RouteDetailResponse {
            id: s.id,
            label: s.label,
            strategy: s.strategy,
            protocol_type: s.protocol_type,
            failover_enabled: s.failover_enabled,
            max_switches: s.max_switches,
            same_account_retries: s.same_account_retries,
            cooldown_multiplier: s.cooldown_multiplier,
            updated_at: s.updated_at,
            candidates,
        });
    }
    Ok(Json(out))
}

/// GET /api/routes/{id} — 单条路由详情。
async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<RouteDetailResponse>, (StatusCode, String)> {
    let s = route_settings::get(&state.db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("路由 '{}' 不存在", id)))?;

    let candidates = load_candidates(&state, &s.protocol_type)?;
    Ok(Json(RouteDetailResponse {
        id: s.id,
        label: s.label,
        strategy: s.strategy,
        protocol_type: s.protocol_type,
        failover_enabled: s.failover_enabled,
        max_switches: s.max_switches,
        same_account_retries: s.same_account_retries,
        cooldown_multiplier: s.cooldown_multiplier,
        updated_at: s.updated_at,
        candidates,
    }))
}

/// PUT /api/routes/{id} — 部分字段更新路由设置。
async fn update(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRouteRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    // 确认路由存在
    let _existing = route_settings::get(&state.db, &id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, format!("路由 '{}' 不存在", id)))?;

    // 验证 strategy 取值
    if let Some(ref strategy) = req.strategy {
        if strategy != "fill-first" && strategy != "round-robin" {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "无效的策略类型: {}，允许的值: fill-first, round-robin",
                    strategy
                ),
            ));
        }
    }

    // 验证 max_switches / retries 不为负数
    if let Some(v) = req.max_switches {
        if v < 1 {
            return Err((
                StatusCode::BAD_REQUEST,
                "max_switches 必须 >= 1".to_string(),
            ));
        }
    }
    if let Some(v) = req.same_account_retries {
        if v < 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                "same_account_retries 必须 >= 0".to_string(),
            ));
        }
    }
    if let Some(v) = req.cooldown_multiplier {
        if v <= 0.0 {
            return Err((
                StatusCode::BAD_REQUEST,
                "cooldown_multiplier 必须 > 0".to_string(),
            ));
        }
    }

    // 部分字段 upsert
    route_settings::upsert_partial(
        &state.db,
        &id,
        None, // label — 不更新
        req.strategy.as_deref(),
        None, // protocol_type — 不更新
        req.failover_enabled,
        req.max_switches,
        req.same_account_retries,
        req.cooldown_multiplier,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(StatusCode::NO_CONTENT)
}

/// 加载指定协议类型的候选端点。
fn load_candidates(
    state: &AppState,
    protocol_type: &str,
) -> Result<Vec<RouteCandidate>, (StatusCode, String)> {
    let rows = endpoints::list_by_protocol(&state.db, protocol_type)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(rows.into_iter().map(RouteCandidate::from).collect())
}
