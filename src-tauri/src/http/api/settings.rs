use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::app_state::AppState;
use crate::db::dao::app_metadata;
use crate::services::model_sync::ModelSyncService;
use crate::services::portability::{self, ExportResult, ImportResult};

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

/// 配置导出请求。
#[derive(Deserialize)]
pub struct ExportRequest {
    /// "full_backup" | "portable"
    pub mode: String,
    /// 脱敏模式必填；完整备份忽略。
    pub password: Option<String>,
}

/// 配置导入请求。
#[derive(Deserialize)]
pub struct ImportRequest {
    /// 导出包 JSON 文本（前端 FileReader 读取）。
    pub package: String,
    /// 脱敏包必填；完整备份忽略。
    pub password: Option<String>,
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/auto-model-refresh",
            get(get_auto_refresh).put(set_auto_refresh),
        )
        .route("/export", post(export_config))
        .route("/import", post(import_config))
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

/// POST /api/settings/export
///
/// body `{ mode, password? }` → `{ package, warnings? }`。
/// - full_backup 主密钥不可用 → 503。
/// - portable 缺密码 → 400。
/// - 响应不回显任何明文凭据（package 内凭据为加密 BLOB）。
async fn export_config(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExportRequest>,
) -> Result<Json<ExportResult>, (StatusCode, String)> {
    let result = portability::export(&state.db, &req.mode, req.password.as_deref())
        .map_err(|e| map_export_error(&e))?;
    Ok(Json(result))
}

/// POST /api/settings/import
///
/// body `{ package, password? }` → `{ imported, pre_import_backup?, warnings? }`。
/// - 解密失败 / 版本不符 → 400。
/// - 事务失败 → 500。
async fn import_config(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ImportRequest>,
) -> Result<Json<ImportResult>, (StatusCode, String)> {
    let result = portability::import(
        &state.db,
        &state.data_dir,
        &req.package,
        req.password.as_deref(),
        None,
    )
    .map_err(|e| map_import_error(&e))?;
    Ok(Json(result))
}

/// 导出错误映射：主密钥不可用 → 503，其余 → 400/500。
fn map_export_error(e: &str) -> (StatusCode, String) {
    if e.contains("系统凭据管理器") || e.contains("主密钥") {
        (StatusCode::SERVICE_UNAVAILABLE, e.to_string())
    } else if e.contains("脱敏导出") || e.contains("未知") {
        (StatusCode::BAD_REQUEST, e.to_string())
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

/// 导入错误映射：版本/解密/包损坏 → 400，事务/写入失败 → 500。
fn map_import_error(e: &str) -> (StatusCode, String) {
    if e.contains("版本")
        || e.contains("解密")
        || e.contains("解析失败")
        || e.contains("密码")
        || e.contains("KDF")
        || e.contains("不支持")
    {
        (StatusCode::BAD_REQUEST, e.to_string())
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}
