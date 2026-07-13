//! 聚合命令（CC 聚合功能 C2）
//!
//! 薄封装：解析参数 → 调 `services/aggregate` 或 `custom_aggregates` DAO，
//! 映射 `AppError` 为 `String`。消费方：C3（路由读展平 / 读配置）、C4（聚合列表页、
//! 自定义聚合 UI）。
//!
//! 参数键统一 `{ appType }`（D11）；返回结构体 `#[serde(rename_all="camelCase")]`。

use crate::database::CcAggregateConfig;
use crate::services::aggregate::{self, AggregateView, CustomAggregateView};
use crate::store::AppState;

/// 返回全部自动聚合视图（含每个聚合的有序上游候选）。供 C4 列表。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_aggregates(
    state: tauri::State<'_, AppState>,
    app_type: String,
) -> Result<Vec<AggregateView>, String> {
    aggregate::build_aggregates(&state.db, &app_type).map_err(|e| e.to_string())
}

/// 返回全部自定义聚合的派生视图（含归零标记 `isEmpty` 与 `missingMembers`）。供 C4 列表。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_custom_aggregates(
    state: tauri::State<'_, AppState>,
    app_type: String,
) -> Result<Vec<CustomAggregateView>, String> {
    aggregate::build_custom_aggregates(&state.db, &app_type).map_err(|e| e.to_string())
}

/// 新建一个自定义聚合，返回新 id。
#[tauri::command(rename_all = "camelCase")]
pub async fn create_custom_aggregate(
    state: tauri::State<'_, AppState>,
    app_type: String,
    name: String,
    members: Vec<String>,
) -> Result<String, String> {
    state
        .db
        .create_custom_aggregate(&app_type, &name, &members)
        .map_err(|e| e.to_string())
}

/// 更新自定义聚合的名称和/或成员（改名/改成员/排序）。`None` 表示不改动该字段。
#[tauri::command(rename_all = "camelCase")]
pub async fn update_custom_aggregate(
    state: tauri::State<'_, AppState>,
    id: String,
    name: Option<String>,
    members: Option<Vec<String>>,
) -> Result<(), String> {
    state
        .db
        .update_custom_aggregate(&id, name.as_deref(), members.as_deref())
        .map_err(|e| e.to_string())
}

/// 删除一个自定义聚合（用户显式删除，非归零自动删除）。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_custom_aggregate(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state
        .db
        .delete_custom_aggregate(&id)
        .map_err(|e| e.to_string())
}

/// 按给定顺序重排自定义聚合（拖拽排序）。
#[tauri::command(rename_all = "camelCase")]
pub async fn reorder_custom_aggregates(
    state: tauri::State<'_, AppState>,
    app_type: String,
    ordered_ids: Vec<String>,
) -> Result<(), String> {
    state
        .db
        .reorder_custom_aggregates(&app_type, &ordered_ids)
        .map_err(|e| e.to_string())
}

/// 读取 CC 聚合模式配置（开关 + tierSelection）。空缺 → 默认（`enabled=false`）。供 C3/C4。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_cc_aggregate_config(
    state: tauri::State<'_, AppState>,
    app_type: String,
) -> Result<CcAggregateConfig, String> {
    state
        .db
        .get_cc_aggregate_config(&app_type)
        .map_err(|e| e.to_string())
}

/// 写入 CC 聚合模式配置（开关 + tierSelection）。供 C4。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_cc_aggregate_config(
    state: tauri::State<'_, AppState>,
    app_type: String,
    config: CcAggregateConfig,
) -> Result<(), String> {
    state
        .db
        .set_cc_aggregate_config(&app_type, &config)
        .map_err(|e| e.to_string())
}
