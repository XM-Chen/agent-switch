//! 七模块外部配置检测的只读 Tauri 命令。
//!
//! 冲突接受/拒绝命令由 C3 Batch 3 提供；本批次只暴露实时状态查询。

use crate::services::external_config_monitor::ExternalConfigModuleStatus;
use crate::store::AppState;

#[tauri::command]
pub async fn get_external_config_status(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ExternalConfigModuleStatus>, String> {
    state.external_config_monitor.get_status().await
}
