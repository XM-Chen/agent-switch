use std::path::PathBuf;

use tauri::State;

use crate::store::AppState;

/// 导入 `local-gateway-rollback-v1` 本机回滚包。
///
/// 只恢复 Agent Switch 自有纯网关表，不运行旧 Provider live 同步，
/// 也不读取、探测或修改任何客户端配置。
#[tauri::command(rename_all = "camelCase")]
pub async fn import_local_gateway_rollback(
    file_path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let db = state.db.clone();
    tauri::async_runtime::spawn_blocking(move || {
        db.import_local_gateway_rollback_file(&PathBuf::from(file_path))
    })
    .await
    .map_err(|e| format!("导入本机网关回滚失败: {e}"))?
    .map_err(|e| e.to_string())
}
