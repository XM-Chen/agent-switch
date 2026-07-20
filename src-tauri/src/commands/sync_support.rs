use serde_json::{json, Value};

use crate::error::AppError;
use crate::services::provider::ProviderService;
use crate::settings;
use crate::store::AppState;

/// DB 导入/云下载后的统一投影。必须接收 Tauri 正在管理的真实 AppState（或其
/// shared-handle clone），绝不能仅凭同一个 DB 构造隔离的 ProxyService/monitor/locks。
pub(crate) fn run_post_import_sync(app_state: &AppState) -> Result<(), AppError> {
    ProviderService::sync_current_to_live(app_state)?;
    settings::reload_settings()?;
    Ok(())
}

fn post_sync_warning<E: std::fmt::Display>(err: E) -> String {
    AppError::localized(
        "sync.post_operation_sync_failed",
        format!("后置同步状态失败: {err}"),
        format!("Post-operation synchronization failed: {err}"),
    )
    .to_string()
}

pub(crate) fn post_sync_warning_from_result(
    result: Result<Result<(), AppError>, String>,
) -> Option<String> {
    match result {
        Ok(Ok(())) => None,
        Ok(Err(err)) => Some(post_sync_warning(err)),
        Err(err) => Some(post_sync_warning(err)),
    }
}

pub(crate) fn attach_warning(mut value: Value, warning: Option<String>) -> Value {
    if let Some(message) = warning {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("warning".to_string(), Value::String(message));
        }
    }
    value
}

pub(crate) fn success_payload_with_warning(backup_id: String, warning: Option<String>) -> Value {
    attach_warning(
        json!({
            "success": true,
            "message": "SQL imported successfully",
            "backupId": backup_id
        }),
        warning,
    )
}

#[cfg(test)]
mod tests {
    use super::{attach_warning, post_sync_warning_from_result, run_post_import_sync};
    use crate::app_config::{McpApps, McpServer};
    use crate::database::Database;
    use crate::provider::Provider;
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
    fn post_sync_warning_from_result_returns_none_on_success() {
        let warning = post_sync_warning_from_result(Ok(Ok(())));
        assert!(warning.is_none());
    }

    #[test]
    fn post_sync_warning_from_result_returns_some_on_sync_error() {
        let warning =
            post_sync_warning_from_result(Ok(Err(crate::error::AppError::Config("boom".into()))));
        assert!(warning.is_some());
    }

    #[tokio::test]
    async fn post_sync_warning_from_result_returns_some_on_join_error() {
        let handle = tokio::spawn(async move {
            panic!("forced join error");
        });
        let join_err = handle.await.expect_err("task should panic");
        let warning = post_sync_warning_from_result(Err(join_err.to_string()));
        assert!(warning.is_some());
    }

    #[test]
    fn attach_warning_adds_warning_without_dropping_existing_fields() {
        let payload = json!({ "status": "downloaded" });
        let updated = attach_warning(payload, Some("post sync warning".to_string()));
        assert_eq!(
            updated.get("status").and_then(|v| v.as_str()),
            Some("downloaded")
        );
        assert_eq!(
            updated.get("warning").and_then(|v| v.as_str()),
            Some("post sync warning")
        );
    }

    #[test]
    #[serial_test::serial]
    fn post_import_sync_reuses_real_monitor_and_honors_off_direct_proxy() {
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
        let shared_state = state.clone();
        assert!(Arc::ptr_eq(
            &state.external_config_monitor,
            &shared_state.external_config_monitor
        ));

        let provider = Provider::with_id(
            "codex-current".to_string(),
            "Codex Current".to_string(),
            json!({
                "auth": { "OPENAI_API_KEY": "real-token" },
                "config": "model_provider = \"real\"\n[model_providers.real]\nbase_url = \"https://relay.example/v1\"\n"
            }),
            None,
        );
        db.save_provider("codex", &provider).expect("save provider");
        db.set_current_provider("codex", &provider.id)
            .expect("set current provider");
        db.save_mcp_server(&McpServer {
            id: "post-sync-mcp".to_string(),
            name: "Post Sync MCP".to_string(),
            server: json!({ "type": "stdio", "command": "echo" }),
            apps: McpApps {
                codex: true,
                ..McpApps::default()
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        })
        .expect("save MCP");

        let config_path = crate::codex_config::get_codex_config_path();
        std::fs::create_dir_all(config_path.parent().unwrap()).expect("create codex dir");
        let user_owned = b"user_owned = true\n";
        std::fs::write(&config_path, user_owned).expect("seed hands-off live");
        run_post_import_sync(&shared_state).expect("hands-off post sync");
        assert_eq!(std::fs::read(&config_path).unwrap(), user_owned);
        let off_status = futures::executor::block_on(state.external_config_monitor.get_status())
            .expect("get off status")
            .into_iter()
            .find(|status| status.app_type == "codex")
            .expect("codex status");
        assert_eq!(off_status.generation, 0);

        futures::executor::block_on(async {
            let mut config = db
                .get_proxy_config_for_app("codex")
                .await
                .expect("get direct config");
            config.takeover_enabled = true;
            config.route_mode = RouteMode::Direct;
            db.update_proxy_config_for_app(config)
                .await
                .expect("set direct mode");
        });
        run_post_import_sync(&shared_state).expect("direct post sync");
        let direct_live = std::fs::read_to_string(&config_path).expect("read direct live");
        assert!(direct_live.contains("https://relay.example/v1"));
        assert!(direct_live.contains("post-sync-mcp"));
        let direct_status = futures::executor::block_on(state.external_config_monitor.get_status())
            .expect("get direct status")
            .into_iter()
            .find(|status| status.app_type == "codex")
            .expect("codex status");
        assert!(direct_status.generation > 0);
        assert!(!direct_status.conflict);

        let proxy_live = r#"model_provider = "ags_proxy"
[model_providers.ags_proxy]
base_url = "http://127.0.0.1:42567/v1"
experimental_bearer_token = "gateway-token"
"#;
        std::fs::write(&config_path, proxy_live).expect("seed proxy live");
        futures::executor::block_on(async {
            let mut config = db
                .get_proxy_config_for_app("codex")
                .await
                .expect("get proxy config");
            config.route_mode = RouteMode::Proxy;
            db.update_proxy_config_for_app(config)
                .await
                .expect("set proxy mode");
        });
        run_post_import_sync(&shared_state).expect("proxy post sync");
        let merged = std::fs::read_to_string(&config_path).expect("read proxy live");
        assert!(merged.contains("http://127.0.0.1:42567/v1"));
        assert!(merged.contains("gateway-token"));
        assert!(merged.contains("post-sync-mcp"));
        let proxy_status = futures::executor::block_on(state.external_config_monitor.get_status())
            .expect("get proxy status")
            .into_iter()
            .find(|status| status.app_type == "codex")
            .expect("codex status");
        assert!(proxy_status.generation > direct_status.generation);
        assert!(!proxy_status.conflict);

        restore_env("AGENT_SWITCH_TEST_HOME", previous_test_home);
        restore_env("HOME", previous_home);
        #[cfg(windows)]
        restore_env("USERPROFILE", previous_userprofile);
        crate::settings::reload_settings().expect("restore settings cache");
    }
}
