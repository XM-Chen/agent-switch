#![allow(non_snake_case)]

use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::app_config::AppType;
use crate::codex_config;
use crate::config::{self, get_claude_settings_path, ConfigStatus};
use crate::settings;
use crate::store::AppState;

#[tauri::command]
pub async fn get_claude_config_status() -> Result<ConfigStatus, String> {
    Ok(config::get_claude_config_status())
}

use std::str::FromStr;

fn invalid_json_format_error(error: serde_json::Error) -> String {
    let lang = settings::get_settings()
        .language
        .unwrap_or_else(|| "zh".to_string());

    match lang.as_str() {
        "en" => format!("Invalid JSON format: {error}"),
        "ja" => format!("JSON形式が無効です: {error}"),
        _ => format!("无效的 JSON 格式: {error}"),
    }
}

fn invalid_toml_format_error(error: toml_edit::TomlError) -> String {
    let lang = settings::get_settings()
        .language
        .unwrap_or_else(|| "zh".to_string());

    match lang.as_str() {
        "en" => format!("Invalid TOML format: {error}"),
        "ja" => format!("TOML形式が無効です: {error}"),
        _ => format!("无效的 TOML 格式: {error}"),
    }
}

fn validate_common_config_snippet(app_type: &str, snippet: &str) -> Result<(), String> {
    if snippet.trim().is_empty() {
        return Ok(());
    }

    match app_type {
        "claude" | "gemini" | "omo" | "omo-slim" => {
            serde_json::from_str::<serde_json::Value>(snippet)
                .map_err(invalid_json_format_error)?;
        }
        "codex" => {
            snippet
                .parse::<toml_edit::DocumentMut>()
                .map_err(invalid_toml_format_error)?;
        }
        _ => {}
    }

    Ok(())
}

#[tauri::command]
pub async fn get_config_status(
    state: State<'_, AppState>,
    app: String,
) -> Result<ConfigStatus, String> {
    match AppType::from_str(&app).map_err(|e| e.to_string())? {
        AppType::Claude => Ok(config::get_claude_config_status()),
        AppType::ClaudeDesktop => {
            let status = crate::claude_desktop_config::get_status(
                state.db.as_ref(),
                state.proxy_service.is_running().await,
            )
            .map_err(|e| e.to_string())?;
            Ok(ConfigStatus {
                exists: status.configured,
                path: status.config_library_path.unwrap_or_default(),
            })
        }
        AppType::Codex => {
            let auth_path = codex_config::get_codex_auth_path();
            let config_text = codex_config::read_codex_config_text().unwrap_or_default();
            let exists = auth_path.exists() || !config_text.trim().is_empty();
            let path = codex_config::get_codex_config_dir()
                .to_string_lossy()
                .to_string();

            Ok(ConfigStatus { exists, path })
        }
        AppType::Gemini => {
            let env_path = crate::gemini_config::get_gemini_env_path();
            let exists = env_path.exists();
            let path = crate::gemini_config::get_gemini_dir()
                .to_string_lossy()
                .to_string();

            Ok(ConfigStatus { exists, path })
        }
        AppType::OpenCode => {
            let config_path = crate::opencode_config::get_opencode_config_path();
            let exists = config_path.exists();
            let path = crate::opencode_config::get_opencode_dir()
                .to_string_lossy()
                .to_string();

            Ok(ConfigStatus { exists, path })
        }
        AppType::OpenClaw => {
            let config_path = crate::openclaw_config::get_openclaw_config_path();
            let exists = config_path.exists();
            let path = crate::openclaw_config::get_openclaw_dir()
                .to_string_lossy()
                .to_string();

            Ok(ConfigStatus { exists, path })
        }
        AppType::Hermes => {
            let config_path = crate::hermes_config::get_hermes_config_path();
            let exists = config_path.exists();
            let path = crate::hermes_config::get_hermes_dir()
                .to_string_lossy()
                .to_string();

            Ok(ConfigStatus { exists, path })
        }
    }
}

#[tauri::command]
pub async fn get_claude_code_config_path() -> Result<String, String> {
    Ok(get_claude_settings_path().to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_config_dir(app: String) -> Result<String, String> {
    let dir = match AppType::from_str(&app).map_err(|e| e.to_string())? {
        AppType::Claude => config::get_claude_config_dir(),
        AppType::ClaudeDesktop => {
            crate::claude_desktop_config::get_config_library_path().map_err(|e| e.to_string())?
        }
        AppType::Codex => codex_config::get_codex_config_dir(),
        AppType::Gemini => crate::gemini_config::get_gemini_dir(),
        AppType::OpenCode => crate::opencode_config::get_opencode_dir(),
        AppType::OpenClaw => crate::openclaw_config::get_openclaw_dir(),
        AppType::Hermes => crate::hermes_config::get_hermes_dir(),
    };

    Ok(dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn open_config_folder(handle: AppHandle, app: String) -> Result<bool, String> {
    let config_dir = match AppType::from_str(&app).map_err(|e| e.to_string())? {
        AppType::Claude => config::get_claude_config_dir(),
        AppType::ClaudeDesktop => {
            crate::claude_desktop_config::get_config_library_path().map_err(|e| e.to_string())?
        }
        AppType::Codex => codex_config::get_codex_config_dir(),
        AppType::Gemini => crate::gemini_config::get_gemini_dir(),
        AppType::OpenCode => crate::opencode_config::get_opencode_dir(),
        AppType::OpenClaw => crate::openclaw_config::get_openclaw_dir(),
        AppType::Hermes => crate::hermes_config::get_hermes_dir(),
    };

    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir).map_err(|e| format!("创建目录失败: {e}"))?;
    }

    handle
        .opener()
        .open_path(config_dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("打开文件夹失败: {e}"))?;

    Ok(true)
}

#[tauri::command]
pub async fn pick_directory(
    app: AppHandle,
    #[allow(non_snake_case)] defaultPath: Option<String>,
) -> Result<Option<String>, String> {
    let initial = defaultPath
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());

    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut builder = app.dialog().file();
        if let Some(path) = initial {
            builder = builder.set_directory(path);
        }
        builder.blocking_pick_folder()
    })
    .await
    .map_err(|e| format!("弹出目录选择器失败: {e}"))?;

    match result {
        Some(file_path) => {
            let resolved = file_path
                .simplified()
                .into_path()
                .map_err(|e| format!("解析选择的目录失败: {e}"))?;
            Ok(Some(resolved.to_string_lossy().to_string()))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn get_app_config_path() -> Result<String, String> {
    let config_path = config::get_app_config_path();
    Ok(config_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn open_app_config_folder(handle: AppHandle) -> Result<bool, String> {
    let config_dir = config::get_app_config_dir();

    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir).map_err(|e| format!("创建目录失败: {e}"))?;
    }

    handle
        .opener()
        .open_path(config_dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("打开文件夹失败: {e}"))?;

    Ok(true)
}

#[tauri::command]
pub async fn get_claude_common_config_snippet(
    state: tauri::State<'_, crate::store::AppState>,
) -> Result<Option<String>, String> {
    state
        .db
        .get_config_snippet("claude")
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_claude_common_config_snippet(
    snippet: String,
    state: tauri::State<'_, crate::store::AppState>,
) -> Result<(), String> {
    let is_cleared = snippet.trim().is_empty();

    if !snippet.trim().is_empty() {
        serde_json::from_str::<serde_json::Value>(&snippet).map_err(invalid_json_format_error)?;
    }

    let value = if is_cleared { None } else { Some(snippet) };

    state
        .db
        .set_config_snippet("claude", value)
        .map_err(|e| e.to_string())?;
    state
        .db
        .set_config_snippet_cleared("claude", is_cleared)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_common_config_snippet(
    app_type: String,
    state: tauri::State<'_, crate::store::AppState>,
) -> Result<Option<String>, String> {
    state
        .db
        .get_config_snippet(&app_type)
        .map_err(|e| e.to_string())
}

/// 对前端编辑器里的 config.toml 文本做通用配置片段的合并/剥离。
/// 放后端是为了走 toml_edit（保注释、保键序）；前端 smol-toml 的
/// 整文档重序列化会破坏用户手写格式。
#[tauri::command]
pub async fn update_toml_common_config_snippet(
    config_toml: String,
    snippet_toml: String,
    enabled: bool,
) -> Result<String, String> {
    crate::services::provider::update_toml_common_config_snippet(
        &config_toml,
        &snippet_toml,
        enabled,
    )
    .map_err(|e| e.to_string())
}

fn set_common_config_snippet_inner(
    app_type: String,
    snippet: String,
    state: &AppState,
) -> Result<(), String> {
    let is_cleared = snippet.trim().is_empty();
    let old_snippet = state
        .db
        .get_config_snippet(&app_type)
        .map_err(|e| e.to_string())?;

    validate_common_config_snippet(&app_type, &snippet)?;

    let value = if is_cleared { None } else { Some(snippet) };

    if matches!(app_type.as_str(), "claude" | "codex" | "gemini") {
        if let Some(legacy_snippet) = old_snippet
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            let app = AppType::from_str(&app_type).map_err(|e| e.to_string())?;
            crate::services::provider::ProviderService::migrate_legacy_common_config_usage(
                state,
                app,
                legacy_snippet,
            )
            .map_err(|e| e.to_string())?;
        }
    }

    state
        .db
        .set_config_snippet(&app_type, value)
        .map_err(|e| e.to_string())?;
    state
        .db
        .set_config_snippet_cleared(&app_type, is_cleared)
        .map_err(|e| e.to_string())?;

    if matches!(app_type.as_str(), "claude" | "codex" | "gemini") {
        let app = AppType::from_str(&app_type).map_err(|e| e.to_string())?;
        // Batch 4A 的 ProviderService 顶层入口已经持有 per-app lock/token；此处
        // 不得再包一层 managed generation。
        crate::services::provider::ProviderService::sync_current_provider_for_app(state, app)
            .map_err(|e| e.to_string())?;
    }

    if app_type == "omo"
        && state
            .db
            .get_current_omo_provider("opencode", "omo")
            .map_err(|e| e.to_string())?
            .is_some()
    {
        crate::services::OmoService::write_config_to_file(state, &crate::services::omo::STANDARD)
            .map_err(|e| e.to_string())?;
    }
    if app_type == "omo-slim"
        && state
            .db
            .get_current_omo_provider("opencode", "omo-slim")
            .map_err(|e| e.to_string())?
            .is_some()
    {
        crate::services::OmoService::write_config_to_file(state, &crate::services::omo::SLIM)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_common_config_snippet(
    app_type: String,
    snippet: String,
    state: tauri::State<'_, crate::store::AppState>,
) -> Result<(), String> {
    set_common_config_snippet_inner(app_type, snippet, state.inner())
}

#[cfg(test)]
mod tests {
    use super::{set_common_config_snippet_inner, validate_common_config_snippet};
    use crate::app_config::AppType;
    use crate::database::Database;
    use crate::provider::{Provider, ProviderMeta};
    use crate::proxy::types::RouteMode;
    use crate::store::AppState;
    use serde_json::json;
    use std::ffi::OsString;
    use std::sync::Arc;

    fn restore_env(name: &str, value: Option<OsString>) {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }

    #[test]
    fn validate_common_config_snippet_accepts_comment_only_codex_snippet() {
        validate_common_config_snippet("codex", "# comment only\n")
            .expect("comment-only codex snippet should be valid");
    }

    #[test]
    fn validate_common_config_snippet_rejects_invalid_codex_snippet() {
        let err = validate_common_config_snippet("codex", "[broken")
            .expect_err("invalid codex snippet should be rejected");
        assert!(
            err.contains("TOML") || err.contains("toml") || err.contains("格式"),
            "expected TOML validation error, got {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn common_snippet_sync_uses_provider_guard_without_false_conflict() {
        let previous_test_home = std::env::var_os("AGENT_SWITCH_TEST_HOME");
        let previous_home = std::env::var_os("HOME");
        #[cfg(windows)]
        let previous_userprofile = std::env::var_os("USERPROFILE");
        let temp = tempfile::TempDir::new().expect("create temp home");
        std::env::set_var("AGENT_SWITCH_TEST_HOME", temp.path());
        std::env::set_var("HOME", temp.path());
        #[cfg(windows)]
        std::env::set_var("USERPROFILE", temp.path());
        crate::settings::reload_settings().expect("reload isolated settings");

        let db = Arc::new(Database::memory().expect("create memory database"));
        let state = AppState::new(db.clone());
        let mut provider = Provider::with_id(
            "current".to_string(),
            "Current".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "token",
                    "ANTHROPIC_BASE_URL": "https://relay.example"
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            common_config_enabled: Some(true),
            ..ProviderMeta::default()
        });
        db.save_provider("claude", &provider)
            .expect("save provider");
        db.set_current_provider("claude", &provider.id)
            .expect("set current provider");
        futures::executor::block_on(async {
            let mut config = db
                .get_proxy_config_for_app("claude")
                .await
                .expect("get proxy config");
            config.takeover_enabled = true;
            config.route_mode = RouteMode::Direct;
            db.update_proxy_config_for_app(config)
                .await
                .expect("enable direct takeover");
        });

        set_common_config_snippet_inner(
            "claude".to_string(),
            json!({ "env": { "COMMON_FLAG": "one" } }).to_string(),
            &state,
        )
        .expect("set first common snippet");
        let first = futures::executor::block_on(state.external_config_monitor.get_status())
            .expect("get first monitor status")
            .into_iter()
            .find(|status| status.app_type == AppType::Claude.as_str())
            .expect("claude status");
        assert!(first.generation > 0);
        assert!(!first.conflict);

        set_common_config_snippet_inner(
            "claude".to_string(),
            json!({ "env": { "COMMON_FLAG": "two" } }).to_string(),
            &state,
        )
        .expect("set second common snippet");
        let second = futures::executor::block_on(state.external_config_monitor.get_status())
            .expect("get second monitor status")
            .into_iter()
            .find(|status| status.app_type == AppType::Claude.as_str())
            .expect("claude status");
        assert!(
            second.generation > first.generation,
            "second call proves no nested/in-flight token leak"
        );
        assert!(!second.conflict);
        let live: serde_json::Value =
            crate::config::read_json_file(&crate::config::get_claude_settings_path())
                .expect("read Claude live");
        assert_eq!(
            live.pointer("/env/COMMON_FLAG")
                .and_then(|value| value.as_str()),
            Some("two")
        );

        restore_env("AGENT_SWITCH_TEST_HOME", previous_test_home);
        restore_env("HOME", previous_home);
        #[cfg(windows)]
        restore_env("USERPROFILE", previous_userprofile);
        crate::settings::reload_settings().expect("restore settings cache");
    }
}

#[tauri::command]
pub async fn extract_common_config_snippet(
    appType: String,
    settingsConfig: Option<String>,
    state: tauri::State<'_, crate::store::AppState>,
) -> Result<String, String> {
    let app = AppType::from_str(&appType).map_err(|e| e.to_string())?;

    if let Some(settings_config) = settingsConfig.filter(|s| !s.trim().is_empty()) {
        let settings: serde_json::Value =
            serde_json::from_str(&settings_config).map_err(invalid_json_format_error)?;

        return crate::services::provider::ProviderService::extract_common_config_snippet_from_settings(
            app,
            &settings,
        )
        .map_err(|e| e.to_string());
    }

    crate::services::provider::ProviderService::extract_common_config_snippet(&state, app)
        .map_err(|e| e.to_string())
}
