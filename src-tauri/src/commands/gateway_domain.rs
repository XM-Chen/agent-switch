//! 独立网关控制面命令。
//!
//! 这些命令只访问 Agent Switch 自有数据库中的 gateway domain 表，严禁触达客户端配置。

use crate::database::{
    CreateGatewayModelInput, CreateGatewayUpstreamInput, CreateRouteTargetInput,
    GatewayConfigRecord, GatewayMigrationIssue, GatewayModelRecord, GatewayUpstreamDto,
    ModelAliasRecord, RouteTargetHealthRecord, RouteTargetRecord, UpdateGatewayModelInput,
    UpdateGatewayUpstreamInput, UpdateRouteTargetInput, UpstreamCredentialHintDto,
    UpstreamModelRecord,
};
use crate::store::AppState;

#[tauri::command(rename_all = "camelCase")]
pub fn get_gateway_domain_config(
    state: tauri::State<'_, AppState>,
) -> Result<GatewayConfigRecord, String> {
    state
        .db
        .get_gateway_config_record()
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_gateway_domain_config(
    state: tauri::State<'_, AppState>,
    config: GatewayConfigRecord,
) -> Result<(), String> {
    state
        .db
        .update_gateway_config_record(&config)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_gateway_upstreams(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GatewayUpstreamDto>, String> {
    state
        .db
        .list_gateway_upstream_dtos()
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_gateway_upstream(
    state: tauri::State<'_, AppState>,
    upstream_id: String,
) -> Result<Option<GatewayUpstreamDto>, String> {
    state
        .db
        .get_gateway_upstream_dto(&upstream_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn create_gateway_upstream(
    state: tauri::State<'_, AppState>,
    input: CreateGatewayUpstreamInput,
) -> Result<GatewayUpstreamDto, String> {
    state
        .db
        .create_gateway_upstream(&input)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_gateway_upstream(
    state: tauri::State<'_, AppState>,
    upstream_id: String,
    input: UpdateGatewayUpstreamInput,
) -> Result<GatewayUpstreamDto, String> {
    state
        .db
        .update_gateway_upstream(&upstream_id, &input)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_gateway_upstream(
    state: tauri::State<'_, AppState>,
    upstream_id: String,
) -> Result<bool, String> {
    state
        .db
        .delete_gateway_upstream(&upstream_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn set_gateway_upstream_enabled(
    state: tauri::State<'_, AppState>,
    upstream_id: String,
    enabled: bool,
) -> Result<GatewayUpstreamDto, String> {
    state
        .db
        .set_gateway_upstream_enabled(&upstream_id, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_gateway_upstream_credential_hints(
    state: tauri::State<'_, AppState>,
    upstream_id: String,
) -> Result<Vec<UpstreamCredentialHintDto>, String> {
    state
        .db
        .list_upstream_credential_hints(&upstream_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn replace_gateway_upstream_credential(
    state: tauri::State<'_, AppState>,
    upstream_id: String,
    credential_kind: String,
    secret: String,
) -> Result<UpstreamCredentialHintDto, String> {
    state
        .db
        .replace_upstream_credential(&upstream_id, &credential_kind, &secret)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_gateway_upstream_credential(
    state: tauri::State<'_, AppState>,
    upstream_id: String,
    credential_kind: String,
) -> Result<bool, String> {
    state
        .db
        .delete_upstream_credential(&upstream_id, &credential_kind)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_gateway_upstream_models(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<UpstreamModelRecord>, String> {
    state.db.list_upstream_models().map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_gateway_models(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GatewayModelRecord>, String> {
    state.db.list_gateway_models().map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn create_gateway_model(
    state: tauri::State<'_, AppState>,
    input: CreateGatewayModelInput,
) -> Result<GatewayModelRecord, String> {
    state
        .db
        .create_gateway_model(&input)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_gateway_model(
    state: tauri::State<'_, AppState>,
    gateway_model_id: String,
    input: UpdateGatewayModelInput,
) -> Result<GatewayModelRecord, String> {
    state
        .db
        .update_gateway_model(&gateway_model_id, &input)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_gateway_model(
    state: tauri::State<'_, AppState>,
    gateway_model_id: String,
) -> Result<bool, String> {
    state
        .db
        .delete_gateway_model(&gateway_model_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn set_gateway_model_enabled(
    state: tauri::State<'_, AppState>,
    model_id: String,
    enabled: bool,
) -> Result<bool, String> {
    state
        .db
        .set_gateway_model_enabled(&model_id, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn set_gateway_model_state(
    state: tauri::State<'_, AppState>,
    model_id: String,
    enabled: bool,
    migration_status: String,
) -> Result<bool, String> {
    state
        .db
        .set_gateway_model_state(&model_id, enabled, &migration_status)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_gateway_model_aliases(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ModelAliasRecord>, String> {
    state.db.list_model_aliases().map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn upsert_gateway_model_alias(
    state: tauri::State<'_, AppState>,
    alias: String,
    gateway_model_id: String,
) -> Result<ModelAliasRecord, String> {
    state
        .db
        .upsert_gateway_model_alias(&alias, &gateway_model_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_gateway_model_alias(
    state: tauri::State<'_, AppState>,
    alias: String,
) -> Result<bool, String> {
    state
        .db
        .delete_gateway_model_alias(&alias)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_gateway_routes(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RouteTargetRecord>, String> {
    state.db.list_route_targets().map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn create_gateway_route_target(
    state: tauri::State<'_, AppState>,
    input: CreateRouteTargetInput,
) -> Result<RouteTargetRecord, String> {
    state
        .db
        .create_route_target(&input)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_gateway_route_target(
    state: tauri::State<'_, AppState>,
    route_target_id: String,
    input: UpdateRouteTargetInput,
) -> Result<RouteTargetRecord, String> {
    state
        .db
        .update_route_target(&route_target_id, &input)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_gateway_route_target(
    state: tauri::State<'_, AppState>,
    route_target_id: String,
) -> Result<bool, String> {
    state
        .db
        .delete_route_target(&route_target_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn set_gateway_route_enabled(
    state: tauri::State<'_, AppState>,
    route_target_id: String,
    enabled: bool,
) -> Result<bool, String> {
    state
        .db
        .set_route_target_enabled(&route_target_id, enabled)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn set_gateway_route_target_enabled(
    state: tauri::State<'_, AppState>,
    route_target_id: String,
    enabled: bool,
) -> Result<bool, String> {
    set_gateway_route_enabled(state, route_target_id, enabled)
}

#[tauri::command(rename_all = "camelCase")]
pub fn reorder_gateway_routes(
    state: tauri::State<'_, AppState>,
    gateway_model_id: String,
    ordered_ids: Vec<String>,
) -> Result<(), String> {
    state
        .db
        .reorder_route_targets(&gateway_model_id, &ordered_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn reorder_gateway_route_targets(
    state: tauri::State<'_, AppState>,
    gateway_model_id: String,
    ordered_ids: Vec<String>,
) -> Result<(), String> {
    reorder_gateway_routes(state, gateway_model_id, ordered_ids)
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_gateway_route_health(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RouteTargetHealthRecord>, String> {
    state
        .db
        .list_route_target_health()
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub fn list_gateway_migration_issues(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<GatewayMigrationIssue>, String> {
    state
        .db
        .list_gateway_migration_issues()
        .map_err(|e| e.to_string())
}
