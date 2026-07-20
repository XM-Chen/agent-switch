//! 七模块外部配置检测与冲突处理 Tauri 命令。

use crate::services::external_config_monitor::ExternalConfigModuleStatus;
use crate::store::AppState;

#[tauri::command]
pub async fn get_external_config_status(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ExternalConfigModuleStatus>, String> {
    state.external_config_monitor.get_status().await
}

#[tauri::command]
pub async fn accept_external_config_change(
    app_type: String,
    generation: u64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .external_config_monitor
        .accept_external_config_change(&app_type, generation)
        .await
}

#[tauri::command]
pub async fn reject_external_config_change(
    app_type: String,
    generation: u64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    state
        .external_config_monitor
        .reject_external_config_change(&app_type, generation)
        .await
}
