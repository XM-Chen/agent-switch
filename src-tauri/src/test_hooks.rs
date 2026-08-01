use crate::app_config::AppType;
use crate::provider::Provider;
use crate::store::AppState;

fn validate_db_only_provider(app_type: &AppType, provider: &Provider) -> Result<(), String> {
    if provider.id.trim().is_empty() {
        return Err("上游 ID 不能为空".to_string());
    }
    if provider.name.trim().is_empty() {
        return Err("上游名称不能为空".to_string());
    }

    match app_type {
        AppType::Codex if provider.settings_config.get("config").is_none() => {
            Err("Codex 上游缺少 config 配置".to_string())
        }
        _ if !provider.settings_config.is_object() => Err("上游配置必须是 JSON 对象".to_string()),
        _ => Ok(()),
    }
}

#[doc(hidden)]
pub fn add_provider_test_hook(
    state: &AppState,
    app_type: AppType,
    provider: Provider,
) -> Result<bool, String> {
    validate_db_only_provider(&app_type, &provider)?;
    state
        .db
        .save_provider(app_type.as_str(), &provider)
        .map(|_| true)
        .map_err(|e| e.to_string())
}

#[doc(hidden)]
pub fn update_provider_test_hook(
    state: &AppState,
    app_type: AppType,
    provider: Provider,
    original_id: Option<&str>,
) -> Result<bool, String> {
    validate_db_only_provider(&app_type, &provider)?;
    if let Some(original_id) = original_id.filter(|id| *id != provider.id.as_str()) {
        state
            .db
            .delete_provider(app_type.as_str(), original_id)
            .map_err(|e| e.to_string())?;
    }
    state
        .db
        .save_provider(app_type.as_str(), &provider)
        .map(|_| true)
        .map_err(|e| e.to_string())
}

#[doc(hidden)]
pub fn delete_provider_test_hook(
    state: &AppState,
    app_type: AppType,
    id: &str,
) -> Result<bool, String> {
    state
        .db
        .delete_provider(app_type.as_str(), id)
        .map(|_| true)
        .map_err(|e| e.to_string())
}
