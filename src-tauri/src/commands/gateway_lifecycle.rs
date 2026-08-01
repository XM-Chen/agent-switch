use tauri::State;

use crate::proxy::types::{ProxyConfig, ProxyServerInfo, ProxyStatus, ProxyStopError};
use crate::store::AppState;

/// 启动独立本地网关；不接管任何客户端配置。
#[tauri::command]
pub async fn start_proxy_server(state: State<'_, AppState>) -> Result<ProxyServerInfo, String> {
    state.proxy_service.start().await
}

/// 停止本进程内的网关；不恢复或探测客户端配置。
#[tauri::command]
pub async fn stop_proxy_server(state: State<'_, AppState>) -> Result<(), ProxyStopError> {
    state
        .proxy_service
        .stop()
        .await
        .map_err(ProxyStopError::stop_failed)
}

#[tauri::command]
pub async fn get_proxy_status(state: State<'_, AppState>) -> Result<ProxyStatus, String> {
    state.proxy_service.get_status().await
}

#[tauri::command]
pub async fn is_proxy_running(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.proxy_service.is_running().await)
}

#[tauri::command]
pub async fn get_proxy_config(state: State<'_, AppState>) -> Result<ProxyConfig, String> {
    state.proxy_service.get_config().await
}

#[tauri::command]
pub async fn update_proxy_config(
    state: State<'_, AppState>,
    config: ProxyConfig,
) -> Result<(), String> {
    state.proxy_service.update_config(&config).await
}
