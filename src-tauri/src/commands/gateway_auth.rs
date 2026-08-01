use serde::Serialize;
use tauri::State;

use crate::services::gateway_auth::{
    CreatedGatewayApiKey, GatewayApiKeySummary, GatewayAuthService,
};
use crate::store::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayAuthStatus {
    pub auth_required: bool,
    pub keys: Vec<GatewayApiKeySummary>,
}

#[tauri::command]
pub fn get_gateway_auth_status(state: State<'_, AppState>) -> Result<GatewayAuthStatus, String> {
    let config = state
        .db
        .get_gateway_auth_config()
        .map_err(|e| e.to_string())?;
    let keys = GatewayAuthService::list_keys(&state.db).map_err(|e| e.to_string())?;
    Ok(GatewayAuthStatus {
        auth_required: config.auth_required,
        keys,
    })
}

#[tauri::command]
pub fn create_gateway_api_key(
    state: State<'_, AppState>,
    name: String,
) -> Result<CreatedGatewayApiKey, String> {
    GatewayAuthService::create_key(&state.db, &name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn revoke_gateway_api_key(state: State<'_, AppState>, key_id: String) -> Result<bool, String> {
    GatewayAuthService::revoke_key(&state.db, &key_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_gateway_auth_required(
    _state: State<'_, AppState>,
    required: bool,
) -> Result<(), String> {
    if !required {
        return Err("独立网关必须启用 Bearer token 鉴权".to_string());
    }
    Ok(())
}
