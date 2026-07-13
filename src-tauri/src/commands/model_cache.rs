//! 模型缓存命令（CC 聚合功能地基，C1）
//!
//! 薄封装：解析参数 → 调 `services/model_cache` 或 DAO，映射 `AppError` 为 `String`。
//! 消费方：C2（读缓存派生聚合）、C4（手动加模型 UI / 手动刷新按钮）。

use crate::database::ProviderModel;
use crate::services::model_cache::{self, ModelCacheStatus, RefreshSummary};
use crate::store::AppState;

/// 列出模型缓存。`providerId` 为空时返回该应用全部上游的缓存行。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_provider_models(
    state: tauri::State<'_, AppState>,
    app_type: String,
    provider_id: Option<String>,
) -> Result<Vec<ProviderModel>, String> {
    state
        .db
        .list_provider_models(&app_type, provider_id.as_deref())
        .map_err(|e| e.to_string())
}

/// 为某个具体上游手动补录一条模型 id（`source=manual`）。
#[tauri::command(rename_all = "camelCase")]
pub async fn add_manual_model(
    state: tauri::State<'_, AppState>,
    app_type: String,
    provider_id: String,
    model_id: String,
) -> Result<(), String> {
    let fetched_at = chrono::Utc::now().timestamp_millis();
    state
        .db
        .upsert_manual_model(&app_type, &provider_id, &model_id, fetched_at)
        .map_err(|e| e.to_string())
}

/// 删除一条手动补录的模型（仅删 `source=manual` 的指定行）。
#[tauri::command(rename_all = "camelCase")]
pub async fn remove_manual_model(
    state: tauri::State<'_, AppState>,
    app_type: String,
    provider_id: String,
    model_id: String,
) -> Result<(), String> {
    state
        .db
        .delete_manual_model(&app_type, &provider_id, &model_id)
        .map_err(|e| e.to_string())
}

/// 立即刷新：`providerId` 空 = 全队列，否则只刷新该上游。
#[tauri::command(rename_all = "camelCase")]
pub async fn refresh_provider_models_now(
    state: tauri::State<'_, AppState>,
    app_type: String,
    provider_id: Option<String>,
) -> Result<RefreshSummary, String> {
    match provider_id {
        Some(id) => {
            // 单上游刷新：把结果折算成 summary，保持返回类型统一。
            let outcome = model_cache::refresh_one(&state.db, &app_type, &id)
                .await
                .map_err(|e| e.to_string())?;
            let mut summary = RefreshSummary::default();
            match outcome {
                model_cache::RefreshOutcome::Refreshed { count } => {
                    summary.refreshed = 1;
                    summary.total_models = count;
                }
                model_cache::RefreshOutcome::Skipped { .. } => {
                    summary.skipped = 1;
                }
            }
            Ok(summary)
        }
        None => model_cache::refresh_all_queue_members(&state.db, &app_type)
            .await
            .map_err(|e| e.to_string()),
    }
}

/// 读取模型缓存状态（每日全量 last-run + 各上游最近刷新时间），供 C4 展示。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_model_cache_status(
    state: tauri::State<'_, AppState>,
    app_type: String,
) -> Result<ModelCacheStatus, String> {
    model_cache::get_status(&state.db, &app_type).map_err(|e| e.to_string())
}
