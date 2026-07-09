//! Deep Link API：解析/预览与用户确认后的导入。

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::post;
use axum::Router;
use serde::Deserialize;
use std::sync::Arc;

use crate::app_state::AppState;
use crate::services::deeplink;

#[derive(Deserialize)]
pub struct DeepLinkUrlRequest {
    pub url: String,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/preview", post(preview))
        .route("/import", post(import))
}

async fn preview(
    Json(req): Json<DeepLinkUrlRequest>,
) -> Result<Json<deeplink::DeepLinkPreview>, (StatusCode, String)> {
    let preview = deeplink::preview(&req.url).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(preview))
}

async fn import(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DeepLinkUrlRequest>,
) -> Result<Json<deeplink::DeepLinkImportResult>, (StatusCode, String)> {
    let result = deeplink::import(
        &state.db,
        state.crypto.as_deref(),
        &state.data_dir,
        &req.url,
    )
    .await
    .map_err(map_import_error)?;
    Ok(Json(result))
}

fn map_import_error(e: String) -> (StatusCode, String) {
    if e.contains("不支持")
        || e.contains("缺少")
        || e.contains("无效")
        || e.contains("Base64")
        || e.contains("UTF-8")
        || e.contains("JSON")
        || e.contains("必须")
    {
        (StatusCode::BAD_REQUEST, e)
    } else if e.contains("凭据管理器不可用") || e.contains("加密服务不可用") {
        (StatusCode::SERVICE_UNAVAILABLE, e)
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, e)
    }
}
