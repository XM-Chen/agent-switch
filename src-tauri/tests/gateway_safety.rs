use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_switch_lib::{
    add_provider_test_hook, delete_provider_test_hook, update_provider_test_hook, AppState,
    AppType, Database, Provider,
};
use serde_json::json;

const ENV_KEYS: &[&str] = &[
    "AGENT_SWITCH_TEST_HOME",
    "HOME",
    "USERPROFILE",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "HERMES_HOME",
    "OPENCODE_DB",
];

struct TestEnvironment {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl TestEnvironment {
    fn isolated(root: &Path) -> Self {
        let previous = ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        let local_app_data = root.join("local-app-data");
        let opencode_db = root.join(".local/share/opencode/opencode.db");

        for key in ["AGENT_SWITCH_TEST_HOME", "HOME", "USERPROFILE"] {
            std::env::set_var(key, root);
        }
        std::env::set_var("LOCALAPPDATA", &local_app_data);
        std::env::set_var("XDG_CONFIG_HOME", root.join(".config"));
        std::env::set_var("XDG_DATA_HOME", root.join(".local/share"));
        std::env::set_var("HERMES_HOME", root.join(".hermes"));
        std::env::set_var("OPENCODE_DB", opencode_db);

        Self { previous }
    }
}

impl Drop for TestEnvironment {
    fn drop(&mut self) {
        let _ = agent_switch_lib::update_settings(agent_switch_lib::AppSettings::default());
        for (key, value) in self.previous.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        let _ = agent_switch_lib::reload_settings_for_test_hook();
    }
}

fn seed_canaries(root: &Path) -> Vec<PathBuf> {
    let relative_paths = [
        ".claude/settings.json",
        ".claude.json",
        ".codex/config.toml",
        ".codex/auth.json",
        ".gemini/.env",
        ".gemini/settings.json",
        ".config/opencode/opencode.json",
        ".local/share/opencode/opencode.db",
        ".openclaw/openclaw.json",
        ".hermes/config.yaml",
        "local-app-data/Claude/claude_desktop_config.json",
    ];

    relative_paths
        .iter()
        .enumerate()
        .map(|(index, relative)| {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().expect("canary parent"))
                .expect("create canary parent");
            fs::write(&path, format!("client-canary-{index}-do-not-touch"))
                .expect("write client canary");
            path
        })
        .collect()
}

fn snapshot_files(paths: &[PathBuf]) -> BTreeMap<PathBuf, Vec<u8>> {
    paths
        .iter()
        .map(|path| {
            (
                path.clone(),
                fs::read(path).unwrap_or_else(|error| {
                    panic!("failed to read canary {}: {error}", path.display())
                }),
            )
        })
        .collect()
}

fn assert_snapshot_unchanged(before: &BTreeMap<PathBuf, Vec<u8>>) {
    for (path, expected) in before {
        assert_eq!(
            fs::read(path).unwrap_or_else(|error| {
                panic!("client canary disappeared {}: {error}", path.display())
            }),
            *expected,
            "client canary changed: {}",
            path.display()
        );
    }
}

#[test]
fn gateway_production_wiring_has_no_client_config_entry_points() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("src");
    let cases: &[(&str, &[&str])] = &[
        (
            "src/lib.rs",
            &[
                "MultiAppConfig::load",
                "migrate_skills_to_ssot(",
                "session_usage::sync_",
                "start_external_config_monitor",
                "recover_from_crash(",
                "stop_with_restore(",
                "commands::get_config_status",
                "commands::get_claude_code_config_path",
                "commands::get_config_dir",
                "commands::open_config_folder",
                "commands::get_external_config_status",
                "commands::set_proxy_takeover_for_app",
                "commands::switch_proxy_provider",
                "commands::sync_session_usage",
                "Deep link URL (raw)",
                "\"url\": url_str",
                "RunEvent::Opened with URL: {url_str}",
            ],
        ),
        (
            "src/store.rs",
            &["ExternalConfigMonitor", "external_config_monitor"],
        ),
        (
            "src/tray.rs",
            &[
                "get_claude_config_dir",
                "get_codex_config_path",
                "get_gemini_env_path",
                "get_opencode_config_path",
                "get_openclaw_config_path",
                "get_hermes_config_path",
                "recover_from_crash",
                "stop_with_restore",
                "takeover",
            ],
        ),
        (
            "src/proxy/server.rs",
            &[
                "std::fs",
                "read_codex_config_text",
                "get_codex_model_catalog_path",
                "ExternalConfigMonitor",
                "snapshot_adapter",
            ],
        ),
        (
            "src/proxy/handlers.rs",
            &[
                "std::fs",
                "read_codex_config_text",
                "get_codex_model_catalog_path",
                "get_claude_settings_path",
                "get_gemini_env_path",
                "get_opencode_config_path",
                "get_openclaw_config_path",
                "get_hermes_config_path",
            ],
        ),
        (
            "src/commands/settings.rs",
            &["maybe_migrate_codex_official_history_to_unified_bucket"],
        ),
        (
            "src/commands/provider.rs",
            &["subscription::get_subscription_quota"],
        ),
        (
            "src/commands/sync_support.rs",
            &["ProviderService::sync_current_to_live"],
        ),
    ];

    for (relative, forbidden) in cases {
        let path = manifest.join(relative);
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for symbol in *forbidden {
            assert!(
                !content.contains(symbol),
                "{relative} references forbidden client-config capability `{symbol}`"
            );
        }
    }

    let lib = fs::read_to_string(manifest.join("src/lib.rs")).expect("read src/lib.rs");
    let (_, invoke_tail) = lib
        .split_once(".invoke_handler(tauri::generate_handler![")
        .expect("find production invoke_handler");
    let (invoke_block, _) = invoke_tail
        .split_once("]);")
        .expect("find end of production invoke_handler");
    let registered: Vec<&str> = invoke_block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .filter_map(|line| line.strip_suffix(','))
        .collect();
    let actual: BTreeSet<&str> = registered.iter().copied().collect();
    let expected = BTreeSet::from([
        "commands::open_external",
        "commands::get_init_error",
        "commands::get_settings",
        "commands::save_settings",
        "commands::install_update_and_restart",
        "commands::check_app_update_available",
        "update_tray_menu",
        "commands::set_window_theme",
        "commands::start_proxy_server",
        "commands::stop_proxy_server",
        "commands::get_proxy_status",
        "commands::get_gateway_auth_status",
        "commands::create_gateway_api_key",
        "commands::revoke_gateway_api_key",
        "commands::set_gateway_auth_required",
        "commands::get_gateway_domain_config",
        "commands::update_gateway_domain_config",
        "commands::list_gateway_upstreams",
        "commands::get_gateway_upstream",
        "commands::create_gateway_upstream",
        "commands::update_gateway_upstream",
        "commands::delete_gateway_upstream",
        "commands::set_gateway_upstream_enabled",
        "commands::list_gateway_upstream_credential_hints",
        "commands::replace_gateway_upstream_credential",
        "commands::delete_gateway_upstream_credential",
        "commands::list_gateway_upstream_models",
        "commands::list_gateway_models",
        "commands::create_gateway_model",
        "commands::update_gateway_model",
        "commands::delete_gateway_model",
        "commands::set_gateway_model_enabled",
        "commands::set_gateway_model_state",
        "commands::list_gateway_model_aliases",
        "commands::upsert_gateway_model_alias",
        "commands::delete_gateway_model_alias",
        "commands::list_gateway_routes",
        "commands::create_gateway_route_target",
        "commands::update_gateway_route_target",
        "commands::delete_gateway_route_target",
        "commands::set_gateway_route_enabled",
        "commands::set_gateway_route_target_enabled",
        "commands::reorder_gateway_routes",
        "commands::reorder_gateway_route_targets",
        "commands::list_gateway_route_health",
        "commands::list_gateway_migration_issues",
        "commands::import_local_gateway_rollback",
    ]);
    assert_eq!(
        registered.len(),
        actual.len(),
        "production invoke_handler contains duplicate command registrations"
    );
    assert_eq!(
        actual, expected,
        "production invoke_handler must expose only GatewayShell, gateway auth/domain, update/settings, and tray commands"
    );

    let commands =
        fs::read_to_string(source.join("commands/mod.rs")).expect("read commands/mod.rs");
    for retired_module in [
        "mod config;",
        "mod external_config;",
        "mod env;",
        "mod mcp;",
        "mod prompt;",
        "mod session_manager;",
        "mod subscription;",
        "mod workspace;",
    ] {
        assert!(
            !commands.contains(retired_module),
            "production command graph still declares `{retired_module}`"
        );
    }
}

#[test]
#[serial_test::serial]
fn gateway_lifecycle_and_db_crud_leave_client_canaries_unchanged() {
    let temp = tempfile::TempDir::new().expect("create isolated home");
    let _environment = TestEnvironment::isolated(temp.path());
    agent_switch_lib::reload_settings_for_test_hook().expect("load isolated settings");
    let canaries = seed_canaries(temp.path());
    let before = snapshot_files(&canaries);

    let db = Arc::new(Database::memory().expect("create gateway database"));
    let state = AppState::new(db.clone());
    let provider = Provider::with_id(
        "canary-upstream".to_string(),
        "Canary Upstream".to_string(),
        json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://upstream.invalid",
                "ANTHROPIC_AUTH_TOKEN": "database-only-secret"
            }
        }),
        None,
    );

    add_provider_test_hook(&state, AppType::Claude, provider.clone()).expect("add DB upstream");
    let mut renamed = provider;
    renamed.name = "Renamed Upstream".to_string();
    update_provider_test_hook(&state, AppType::Claude, renamed, None).expect("update DB upstream");
    delete_provider_test_hook(&state, AppType::Claude, "canary-upstream")
        .expect("delete DB upstream");

    let runtime = tokio::runtime::Runtime::new().expect("create gateway runtime");
    runtime.block_on(async {
        let mut config = state
            .proxy_service
            .get_config()
            .await
            .expect("get gateway config");
        config.listen_address = "127.0.0.1".to_string();
        config.listen_port = 0;
        state
            .proxy_service
            .update_config(&config)
            .await
            .expect("set ephemeral gateway port");

        let info = state.proxy_service.start().await.expect("start gateway");
        let response = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{}/health", info.port))
            .send()
            .await
            .expect("request gateway health");
        assert!(response.status().is_success());
        assert_eq!(
            response
                .json::<serde_json::Value>()
                .await
                .expect("health JSON"),
            json!({"status": "healthy"})
        );

        state.proxy_service.stop().await.expect("stop gateway");
        db.reset_proxy_runtime_mirror()
            .await
            .expect("reset runtime mirror");
    });

    assert_snapshot_unchanged(&before);
    assert!(
        db.get_all_providers(AppType::Claude.as_str())
            .expect("list DB upstreams")
            .is_empty(),
        "Provider CRUD must remain DB-only"
    );
}

#[test]
fn environment_key_list_has_no_duplicates() {
    let mut sorted = ENV_KEYS.to_vec();
    sorted.sort_unstable_by(|left, right| OsStr::new(left).cmp(OsStr::new(right)));
    sorted.dedup();
    assert_eq!(sorted.len(), ENV_KEYS.len());
}
