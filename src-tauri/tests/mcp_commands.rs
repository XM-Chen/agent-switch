use std::collections::HashMap;
use std::fs;

use serde_json::json;

use agent_switch_lib::{
    get_claude_mcp_path, get_claude_mcp_status, get_claude_settings_path,
    import_default_config_test_hook, read_claude_mcp_config, update_settings, AppError,
    AppSettings, AppType, McpApps, McpServer, McpService, MultiAppConfig, RouteMode,
};

#[path = "support.rs"]
mod support;
use support::{
    create_test_state, create_test_state_with_config, enable_direct_takeover, ensure_test_home,
    reset_test_fs, test_mutex,
};

fn set_takeover_mode(state: &agent_switch_lib::AppState, app: AppType, route_mode: RouteMode) {
    futures::executor::block_on(async {
        let mut config = state
            .db
            .get_proxy_config_for_app(app.as_str())
            .await
            .expect("get proxy config");
        config.takeover_enabled = true;
        config.route_mode = route_mode;
        state
            .db
            .update_proxy_config_for_app(config)
            .await
            .expect("set takeover mode");
    });
}

fn embedded_mcp_server(command: &str) -> McpServer {
    McpServer {
        id: "managed-mcp".to_string(),
        name: "Managed MCP".to_string(),
        server: json!({
            "type": "stdio",
            "command": command,
            "args": ["managed"]
        }),
        apps: McpApps {
            claude: false,
            codex: true,
            gemini: false,
            opencode: true,
            hermes: true,
        },
        description: None,
        homepage: None,
        docs: None,
        tags: Vec::new(),
    }
}

fn seed_embedded_mcp_targets(
    home: &std::path::Path,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let codex_path = home.join(".codex").join("config.toml");
    let opencode_path = home.join(".config").join("opencode").join("opencode.json");
    let hermes_path = agent_switch_lib::hermes_config::get_hermes_config_path();
    for path in [&codex_path, &opencode_path, &hermes_path] {
        fs::create_dir_all(path.parent().expect("target parent")).expect("create target dir");
    }
    fs::write(
        &codex_path,
        r#"model_provider = "ags_proxy"

[model_providers.ags_proxy]
base_url = "http://127.0.0.1:42567/v1"
experimental_bearer_token = "gateway-token"
"#,
    )
    .expect("seed codex target");
    fs::write(
        &opencode_path,
        serde_json::to_vec_pretty(&json!({
            "$schema": "https://opencode.ai/config.json",
            "provider": {
                "ags-proxy": {
                    "npm": "@ai-sdk/openai-compatible",
                    "options": {
                        "baseURL": "http://127.0.0.1:42567/opencode/v1",
                        "apiKey": "gateway-token"
                    }
                }
            }
        }))
        .expect("serialize opencode target"),
    )
    .expect("seed opencode target");
    fs::write(
        &hermes_path,
        "model: ags-proxy\ncustom_providers:\n  ags-proxy:\n    base_url: http://127.0.0.1:42567/hermes/v1\n    api_key: gateway-token\n",
    )
    .expect("seed hermes target");
    (codex_path, opencode_path, hermes_path)
}

#[test]
fn import_default_config_claude_persists_provider() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let settings_path = get_claude_settings_path();
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).expect("create claude settings dir");
    }
    let settings = json!({
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "test-key",
            "ANTHROPIC_BASE_URL": "https://api.test"
        }
    });
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings).expect("serialize settings"),
    )
    .expect("seed claude settings.json");

    let mut config = MultiAppConfig::default();
    config.ensure_app(&AppType::Claude);
    let state = create_test_state_with_config(&config).expect("create test state");

    import_default_config_test_hook(&state, AppType::Claude)
        .expect("import default config succeeds");

    // 验证内存状态
    let providers = state
        .db
        .get_all_providers(AppType::Claude.as_str())
        .expect("get all providers");
    let current_id = state
        .db
        .get_current_provider(AppType::Claude.as_str())
        .expect("get current provider");
    assert_eq!(current_id.as_deref(), Some("default"));
    let default_provider = providers.get("default").expect("default provider");
    assert_eq!(
        default_provider.settings_config, settings,
        "default provider should capture live settings"
    );

    // 验证数据已持久化到数据库（v3.7.0+ 使用 SQLite 而非 config.json）
    let db_path = home.join(".agent-switch").join("agent-switch.db");
    assert!(
        db_path.exists(),
        "importing default config should persist to agent-switch.db"
    );
}

#[test]
fn import_default_config_without_live_file_returns_error() {
    use support::create_test_state;

    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    let err = import_default_config_test_hook(&state, AppType::Claude)
        .expect_err("missing live file should error");
    match err {
        AppError::Localized { zh, .. } => assert!(
            zh.contains("Claude Code 配置文件不存在"),
            "unexpected error message: {zh}"
        ),
        AppError::Message(msg) => assert!(
            msg.contains("Claude Code 配置文件不存在"),
            "unexpected error message: {msg}"
        ),
        other => panic!("unexpected error variant: {other:?}"),
    }

    // 使用数据库架构，不再检查 config.json
    // 失败的导入不应该向数据库写入任何供应商
    let providers = state
        .db
        .get_all_providers(AppType::Claude.as_str())
        .expect("get all providers");
    assert!(
        providers.is_empty(),
        "failed import should not create any providers in database"
    );
}

#[test]
fn import_mcp_from_claude_creates_config_and_enables_servers() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let mcp_path = get_claude_mcp_path();
    let claude_json = json!({
        "mcpServers": {
            "echo": {
                "type": "stdio",
                "command": "echo"
            }
        }
    });
    fs::write(
        &mcp_path,
        serde_json::to_string_pretty(&claude_json).expect("serialize claude mcp"),
    )
    .expect("seed ~/.claude.json");

    let config = MultiAppConfig::default();
    let state = create_test_state_with_config(&config).expect("create test state");

    let changed = McpService::import_from_claude(&state).expect("import mcp from claude succeeds");
    assert!(
        changed > 0,
        "import should report inserted or normalized entries"
    );

    let servers = state.db.get_all_mcp_servers().expect("get all mcp servers");
    let entry = servers
        .get("echo")
        .expect("server imported into unified structure");
    assert!(
        entry.apps.claude,
        "imported server should have Claude app enabled"
    );

    // 验证数据已持久化到数据库
    let db_path = home.join(".agent-switch").join("agent-switch.db");
    assert!(
        db_path.exists(),
        "state.save should persist to agent-switch.db when changes detected"
    );
}

#[test]
fn import_mcp_from_codex_does_not_rewrite_codex_config() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let codex_dir = home.join(".codex");
    fs::create_dir_all(&codex_dir).expect("create codex dir");
    let config_path = codex_dir.join("config.toml");
    let original = r#"# keep user formatting intact
model = "gpt-5"

[mcp.servers.legacy]
type = "stdio"
command = "echo"

[mcp_servers.echo]
type = "stdio"
command = "echo"
"#;
    fs::write(&config_path, original).expect("seed codex config");

    let state = create_test_state().expect("create test state");
    let changed = McpService::import_from_codex(&state).expect("import from codex");
    assert!(changed > 0, "should import servers from Codex config");

    let after = fs::read_to_string(&config_path).expect("read codex config");
    assert_eq!(
        after, original,
        "importing from Codex should not rewrite ~/.codex/config.toml"
    );
}

#[test]
fn import_mcp_from_claude_does_not_sync_existing_codex_enabled_server() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let codex_dir = home.join(".codex");
    fs::create_dir_all(&codex_dir).expect("create codex dir");
    let codex_config_path = codex_dir.join("config.toml");
    let codex_original = r#"[mcp.servers.keep_me]
type = "stdio"
command = "echo"
"#;
    fs::write(&codex_config_path, codex_original).expect("seed codex config");

    let claude_json = json!({
        "mcpServers": {
            "shared": {
                "type": "stdio",
                "command": "echo"
            }
        }
    });
    fs::write(
        get_claude_mcp_path(),
        serde_json::to_string_pretty(&claude_json).expect("serialize claude mcp"),
    )
    .expect("seed claude mcp");

    let state = create_test_state().expect("create test state");
    state
        .db
        .save_mcp_server(&McpServer {
            id: "shared".to_string(),
            name: "shared".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: false,
                codex: true,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        })
        .expect("seed existing mcp server");

    let changed = McpService::import_from_claude(&state).expect("import from claude");
    assert_eq!(changed, 0, "existing server should not count as new");

    let after = fs::read_to_string(&codex_config_path).expect("read codex config");
    assert_eq!(
        after, codex_original,
        "importing from Claude should not sync an existing Codex-enabled server"
    );

    let servers = state.db.get_all_mcp_servers().expect("get all mcp servers");
    let shared = servers.get("shared").expect("shared server exists");
    assert!(
        shared.apps.claude,
        "import should enable Claude in database"
    );
    assert!(shared.apps.codex, "existing Codex flag should be preserved");
}

#[test]
fn import_mcp_from_claude_invalid_json_preserves_state() {
    use support::create_test_state;

    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let mcp_path = get_claude_mcp_path();
    fs::write(&mcp_path, "{\"mcpServers\":") // 不完整 JSON
        .expect("seed invalid ~/.claude.json");

    let state = create_test_state().expect("create test state");

    let err =
        McpService::import_from_claude(&state).expect_err("invalid json should bubble up error");
    match err {
        AppError::McpValidation(msg) => assert!(
            msg.contains("解析 ~/.claude.json 失败"),
            "unexpected error message: {msg}"
        ),
        other => panic!("unexpected error variant: {other:?}"),
    }

    // 使用数据库架构，检查 MCP 服务器未被写入
    let servers = state.db.get_all_mcp_servers().expect("get all mcp servers");
    assert!(
        servers.is_empty(),
        "failed import should not persist any MCP servers to database"
    );
}

/// "从应用导入"是 best-effort：单个应用的坏配置文件不阻断其余应用的
/// 导入，但失败必须聚合上报——历史实现逐应用 `unwrap_or(0)` 吞错，
/// 坏 config.toml 只会表现为"导入成功 0 个"，用户无从得知出了什么问题。
#[test]
fn import_from_all_apps_reports_broken_app_but_imports_the_rest() {
    use support::create_test_state;

    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // 好的 ~/.claude.json：应正常导入
    let claude_json = json!({
        "mcpServers": {
            "alpha": { "type": "stdio", "command": "echo" }
        }
    });
    fs::write(
        get_claude_mcp_path(),
        serde_json::to_string_pretty(&claude_json).expect("serialize claude mcp"),
    )
    .expect("seed ~/.claude.json");

    // 坏的 ~/.codex/config.toml：解析必然失败
    let codex_dir = home.join(".codex");
    fs::create_dir_all(&codex_dir).expect("create codex dir");
    fs::write(codex_dir.join("config.toml"), "not = = valid toml")
        .expect("seed broken codex config");

    let state = create_test_state().expect("create test state");

    let err = McpService::import_from_all_apps(&state)
        .expect_err("broken codex config must surface, not be swallowed as zero imports");
    let message = err.to_string();
    assert!(
        message.contains("codex"),
        "aggregated error should name the failing app, got: {message}"
    );

    // Codex 的失败不阻断 Claude：alpha 应已入库并启用 Claude
    let servers = state.db.get_all_mcp_servers().expect("get all mcp servers");
    let entry = servers
        .get("alpha")
        .expect("claude server imported despite codex failure");
    assert!(
        entry.apps.claude,
        "imported server should have Claude app enabled"
    );
}

#[test]
fn set_mcp_enabled_for_codex_writes_live_config() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // 创建 Codex 配置目录和文件
    let codex_dir = home.join(".codex");
    fs::create_dir_all(&codex_dir).expect("create codex dir");
    fs::write(
        codex_dir.join("auth.json"),
        r#"{"OPENAI_API_KEY":"test-key"}"#,
    )
    .expect("create auth.json");
    fs::write(codex_dir.join("config.toml"), "").expect("create empty config.toml");

    let mut config = MultiAppConfig::default();
    config.ensure_app(&AppType::Codex);

    // v3.7.0: 使用统一结构
    config.mcp.servers = Some(HashMap::new());
    config.mcp.servers.as_mut().unwrap().insert(
        "codex-server".into(),
        McpServer {
            id: "codex-server".to_string(),
            name: "Codex Server".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: false,
                codex: false, // 初始未启用
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    );

    let state = create_test_state_with_config(&config).expect("create test state");
    enable_direct_takeover(&state, AppType::Codex);

    // v3.7.0: 使用 toggle_app 替代 set_enabled
    McpService::toggle_app(&state, "codex-server", AppType::Codex, true)
        .expect("toggle_app should succeed");

    let servers = state.db.get_all_mcp_servers().expect("get all mcp servers");
    let entry = servers.get("codex-server").expect("codex server exists");
    assert!(
        entry.apps.codex,
        "server should have Codex app enabled after toggle"
    );

    let toml_path = agent_switch_lib::get_codex_config_path();
    assert!(
        toml_path.exists(),
        "enabling server should trigger sync to ~/.codex/config.toml"
    );
    let toml_text = fs::read_to_string(&toml_path).expect("read codex config");
    assert!(
        toml_text.contains("codex-server"),
        "codex config should include the enabled server definition"
    );
}

#[test]
fn enabling_codex_mcp_skips_when_codex_dir_missing() {
    use support::create_test_state;

    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // 确认 Codex 配置目录不存在（模拟“未安装/未运行过 Codex CLI”）
    assert!(
        !home.join(".codex").exists(),
        "~/.codex should not exist in fresh test environment"
    );

    let state = create_test_state().expect("create test state");
    enable_direct_takeover(&state, AppType::Codex);

    // 先插入一个未启用 Codex 的 MCP 服务器（避免 upsert 触发同步）
    McpService::upsert_server(
        &state,
        McpServer {
            id: "codex-server".to_string(),
            name: "Codex Server".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: false,
                codex: false,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("insert server without syncing");

    // 启用 Codex：目录缺失时应跳过写入（不创建 ~/.codex/config.toml）
    McpService::toggle_app(&state, "codex-server", AppType::Codex, true)
        .expect("toggle codex should succeed even when ~/.codex is missing");

    assert!(
        !home.join(".codex").exists(),
        "~/.codex should still not exist after skipped sync"
    );
}

#[test]
fn upsert_mcp_server_disabling_app_removes_from_claude_live_config() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // 模拟 Claude 已安装/已初始化：存在 ~/.claude 目录
    fs::create_dir_all(home.join(".claude")).expect("create ~/.claude dir");

    // 先创建一个启用 Claude 的 MCP 服务器
    let state = support::create_test_state().expect("create test state");
    enable_direct_takeover(&state, AppType::Claude);
    McpService::upsert_server(
        &state,
        McpServer {
            id: "echo".to_string(),
            name: "echo".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: true,
                codex: false,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("upsert should sync to Claude live config");

    // 确认已写入 ~/.claude.json
    let mcp_path = get_claude_mcp_path();
    let text = fs::read_to_string(&mcp_path).expect("read ~/.claude.json");
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse ~/.claude.json");
    assert!(
        v.pointer("/mcpServers/echo").is_some(),
        "echo should exist in Claude live config after enabling"
    );

    // 再次 upsert：取消勾选 Claude（apps.claude=false），应从 Claude live 配置中移除
    McpService::upsert_server(
        &state,
        McpServer {
            id: "echo".to_string(),
            name: "echo".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: false,
                codex: false,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("upsert disabling app should remove from Claude live config");

    let text = fs::read_to_string(&mcp_path).expect("read ~/.claude.json after disable");
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse ~/.claude.json");
    assert!(
        v.pointer("/mcpServers/echo").is_none(),
        "echo should be removed from Claude live config after disabling"
    );
}

#[test]
fn import_mcp_from_multiple_apps_merges_enabled_flags() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // 1) Claude: ~/.claude.json
    let mcp_path = get_claude_mcp_path();
    let claude_json = json!({
        "mcpServers": {
            "shared": {
                "type": "stdio",
                "command": "echo"
            }
        }
    });
    fs::write(
        &mcp_path,
        serde_json::to_string_pretty(&claude_json).expect("serialize claude mcp"),
    )
    .expect("seed ~/.claude.json");

    // 2) Codex: ~/.codex/config.toml
    let codex_dir = home.join(".codex");
    fs::create_dir_all(&codex_dir).expect("create codex dir");
    fs::write(
        codex_dir.join("config.toml"),
        r#"[mcp_servers.shared]
type = "stdio"
command = "echo"
"#,
    )
    .expect("seed ~/.codex/config.toml");

    let state = support::create_test_state().expect("create test state");

    McpService::import_from_claude(&state).expect("import from claude");
    McpService::import_from_codex(&state).expect("import from codex");

    let servers = state.db.get_all_mcp_servers().expect("get all mcp servers");
    let entry = servers.get("shared").expect("shared server exists");
    assert!(entry.apps.claude, "shared should enable Claude");
    assert!(entry.apps.codex, "shared should enable Codex");
}

#[test]
fn import_mcp_from_gemini_sse_url_only_is_valid() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // Gemini MCP 位于 ~/.gemini/settings.json
    let gemini_dir = home.join(".gemini");
    fs::create_dir_all(&gemini_dir).expect("create gemini dir");
    let settings_path = gemini_dir.join("settings.json");

    // Gemini SSE：只包含 url（Gemini 不使用 type 字段）
    let gemini_settings = json!({
        "mcpServers": {
            "sse-server": {
                "url": "https://example.com/sse"
            }
        }
    });
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&gemini_settings).expect("serialize gemini settings"),
    )
    .expect("seed ~/.gemini/settings.json");

    let state = support::create_test_state().expect("create test state");
    let changed = McpService::import_from_gemini(&state).expect("import from gemini");
    assert!(changed > 0, "should import at least 1 server");

    let servers = state.db.get_all_mcp_servers().expect("get all mcp servers");
    let entry = servers.get("sse-server").expect("sse-server exists");
    assert!(entry.apps.gemini, "imported server should enable Gemini");
    assert_eq!(
        entry.server.get("type").and_then(|v| v.as_str()),
        Some("sse"),
        "Gemini url-only server should be normalized to type=sse in unified structure"
    );
}

#[test]
fn enabling_gemini_mcp_skips_when_gemini_dir_missing() {
    use support::create_test_state;

    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // 确认 Gemini 配置目录不存在（模拟“未安装/未运行过 Gemini CLI”）
    assert!(
        !home.join(".gemini").exists(),
        "~/.gemini should not exist in fresh test environment"
    );

    let state = create_test_state().expect("create test state");
    enable_direct_takeover(&state, AppType::Gemini);

    // 先插入一个未启用 Gemini 的 MCP 服务器（避免 upsert 触发同步）
    McpService::upsert_server(
        &state,
        McpServer {
            id: "gemini-server".to_string(),
            name: "Gemini Server".to_string(),
            server: json!({
                "type": "sse",
                "url": "https://example.com/sse"
            }),
            apps: McpApps {
                claude: false,
                codex: false,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("insert server without syncing");

    // 启用 Gemini：目录缺失时应跳过写入（不创建 ~/.gemini/settings.json）
    McpService::toggle_app(&state, "gemini-server", AppType::Gemini, true)
        .expect("toggle gemini should succeed even when ~/.gemini is missing");

    assert!(
        !home.join(".gemini").exists(),
        "~/.gemini should still not exist after skipped sync"
    );
}

#[test]
fn enabling_claude_mcp_skips_when_claude_config_absent() {
    use support::create_test_state;

    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    // 确认 Claude 相关目录/文件都不存在（模拟“未安装/未运行过 Claude”）
    assert!(
        !home.join(".claude").exists(),
        "~/.claude should not exist in fresh test environment"
    );
    assert!(
        !home.join(".claude.json").exists(),
        "~/.claude.json should not exist in fresh test environment"
    );

    let state = create_test_state().expect("create test state");
    enable_direct_takeover(&state, AppType::Claude);

    // 先插入一个未启用 Claude 的 MCP 服务器（避免 upsert 触发同步）
    McpService::upsert_server(
        &state,
        McpServer {
            id: "claude-server".to_string(),
            name: "Claude Server".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: false,
                codex: false,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("insert server without syncing");

    // 启用 Claude：配置缺失时应跳过写入（不创建 ~/.claude.json）
    McpService::toggle_app(&state, "claude-server", AppType::Claude, true)
        .expect("toggle claude should succeed even when ~/.claude is missing");

    assert!(
        !home.join(".claude.json").exists(),
        "~/.claude.json should still not exist after skipped sync"
    );
}

#[test]
fn explicit_default_claude_dir_keeps_default_split_mcp_path() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let claude_dir = home.join(".claude");
    fs::create_dir_all(&claude_dir).expect("create explicit default claude dir");

    update_settings(AppSettings {
        claude_config_dir: Some(claude_dir.to_string_lossy().to_string()),
        ..AppSettings::default()
    })
    .expect("set explicit default claude config dir");

    assert_eq!(
        get_claude_mcp_path(),
        home.join(".claude.json"),
        "explicit default Claude dir should keep Claude Code's split MCP path"
    );

    let state = create_test_state().expect("create test state");
    enable_direct_takeover(&state, AppType::Claude);
    McpService::upsert_server(
        &state,
        McpServer {
            id: "claude-default".to_string(),
            name: "Claude Default".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: true,
                codex: false,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("sync default Claude MCP");

    assert!(
        home.join(".claude.json").exists(),
        "default split MCP file should be written at home/.claude.json"
    );
    assert!(
        !claude_dir.join(".claude.json").exists(),
        "explicit default dir should not use nested .claude/.claude.json"
    );
}

#[test]
fn custom_claude_dir_writes_mcp_inside_config_dir() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let custom_dir = home.join("profiles").join(".claude");
    fs::create_dir_all(&custom_dir).expect("create custom claude dir");

    update_settings(AppSettings {
        claude_config_dir: Some(custom_dir.to_string_lossy().to_string()),
        ..AppSettings::default()
    })
    .expect("set custom claude config dir");

    let expected_mcp_path = custom_dir.join(".claude.json");
    assert_eq!(
        get_claude_mcp_path(),
        expected_mcp_path,
        "custom Claude dir should keep MCP state inside the config dir"
    );

    let state = create_test_state().expect("create test state");
    enable_direct_takeover(&state, AppType::Claude);
    McpService::upsert_server(
        &state,
        McpServer {
            id: "claude-custom".to_string(),
            name: "Claude Custom".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: true,
                codex: false,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("sync custom Claude MCP");

    assert!(
        expected_mcp_path.exists(),
        "custom Claude MCP file should be written inside custom dir"
    );
    assert!(
        !home.join("profiles").join(".claude.json").exists(),
        "custom Claude dir should not write sibling .claude.json"
    );
}

#[test]
fn custom_claude_dir_sync_does_not_copy_default_profile() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let home_mcp_path = home.join(".claude.json");
    let default_profile = json!({
        "hasCompletedOnboarding": true,
        "projects": {
            "/home-project": {
                "hasTrustDialogAccepted": true
            }
        },
        "mcpServers": {
            "home-only": {
                "type": "stdio",
                "command": "home-command"
            }
        },
        "profileSentinel": "home-profile"
    });
    let default_profile_text =
        serde_json::to_string_pretty(&default_profile).expect("serialize default profile");
    fs::write(&home_mcp_path, &default_profile_text).expect("seed default Claude profile");

    let custom_dir = home.join("profiles").join("work").join(".claude");
    fs::create_dir_all(&custom_dir).expect("create custom claude dir");
    update_settings(AppSettings {
        claude_config_dir: Some(custom_dir.to_string_lossy().to_string()),
        ..AppSettings::default()
    })
    .expect("set custom claude config dir");

    let expected_mcp_path = custom_dir.join(".claude.json");
    assert_eq!(
        get_claude_mcp_path(),
        expected_mcp_path,
        "custom Claude dir should use nested .claude.json"
    );
    assert!(
        !expected_mcp_path.exists(),
        "custom profile should start without a live MCP file"
    );

    let state = create_test_state().expect("create test state");
    enable_direct_takeover(&state, AppType::Claude);
    McpService::upsert_server(
        &state,
        McpServer {
            id: "custom-only".to_string(),
            name: "Custom Only".to_string(),
            server: json!({
                "type": "stdio",
                "command": "custom-command"
            }),
            apps: McpApps {
                claude: true,
                codex: false,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        },
    )
    .expect("sync custom Claude MCP");

    let text = fs::read_to_string(&expected_mcp_path).expect("read custom Claude MCP");
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse custom Claude MCP");
    let servers = value
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .expect("custom profile should contain mcpServers");
    assert!(
        servers.contains_key("custom-only"),
        "custom profile should contain DB-managed Claude server"
    );
    assert!(
        !servers.contains_key("home-only"),
        "custom profile should not inherit default profile MCP servers"
    );
    assert!(
        value.get("hasCompletedOnboarding").is_none(),
        "custom profile should not inherit onboarding state"
    );
    assert!(
        value.get("projects").is_none(),
        "custom profile should not inherit project trust state"
    );
    assert!(
        value.get("profileSentinel").is_none(),
        "custom profile should not inherit unrelated default profile fields"
    );
    assert_eq!(
        fs::read_to_string(&home_mcp_path).expect("reread default Claude profile"),
        default_profile_text,
        "default Claude profile should remain unchanged"
    );
}

#[test]
fn custom_claude_dir_read_only_mcp_queries_do_not_create_profile() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let home_mcp_path = home.join(".claude.json");
    fs::write(
        &home_mcp_path,
        serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "home-only": {
                    "type": "stdio",
                    "command": "home-command"
                }
            },
            "profileSentinel": "home-profile"
        }))
        .expect("serialize default profile"),
    )
    .expect("seed default Claude profile");

    let custom_dir = home.join("profiles").join("work").join(".claude");
    fs::create_dir_all(&custom_dir).expect("create custom claude dir");
    update_settings(AppSettings {
        claude_config_dir: Some(custom_dir.to_string_lossy().to_string()),
        ..AppSettings::default()
    })
    .expect("set custom claude config dir");

    let expected_mcp_path = custom_dir.join(".claude.json");
    assert!(
        !expected_mcp_path.exists(),
        "custom profile should start without a live MCP file"
    );

    let status =
        futures::executor::block_on(get_claude_mcp_status()).expect("get Claude MCP status");
    assert_eq!(
        status.user_config_path,
        expected_mcp_path.to_string_lossy(),
        "status should report the custom profile MCP path"
    );
    assert!(
        !status.user_config_exists,
        "status should report missing custom profile MCP file"
    );
    let text =
        futures::executor::block_on(read_claude_mcp_config()).expect("read Claude MCP config");
    assert_eq!(text, None, "missing custom profile should read as None");
    assert!(
        !expected_mcp_path.exists(),
        "read-only MCP queries should not copy or create the custom profile"
    );
}

#[test]
fn sync_all_enabled_removes_known_disabled_but_preserves_unknown_live_entries() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let mcp_path = get_claude_mcp_path();
    fs::write(
        &mcp_path,
        serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "managed-disabled": {
                    "type": "stdio",
                    "command": "echo"
                },
                "external-only": {
                    "type": "stdio",
                    "command": "external"
                }
            }
        }))
        .expect("serialize claude mcp"),
    )
    .expect("seed claude mcp");

    let state = create_test_state().expect("create test state");
    enable_direct_takeover(&state, AppType::Claude);

    state
        .db
        .save_mcp_server(&McpServer {
            id: "managed-disabled".to_string(),
            name: "Managed Disabled".to_string(),
            server: json!({
                "type": "stdio",
                "command": "echo"
            }),
            apps: McpApps {
                claude: false,
                codex: false,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        })
        .expect("save disabled server");
    state
        .db
        .save_mcp_server(&McpServer {
            id: "managed-enabled".to_string(),
            name: "Managed Enabled".to_string(),
            server: json!({
                "type": "stdio",
                "command": "managed"
            }),
            apps: McpApps {
                claude: true,
                codex: false,
                gemini: false,
                opencode: false,
                hermes: false,
            },
            description: None,
            homepage: None,
            docs: None,
            tags: Vec::new(),
        })
        .expect("save enabled server");

    McpService::sync_all_enabled(&state).expect("reconcile mcp");

    let text = fs::read_to_string(&mcp_path).expect("read claude mcp");
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse claude mcp");
    let servers = value
        .get("mcpServers")
        .and_then(|entry| entry.as_object())
        .expect("mcpServers object");

    assert!(
        !servers.contains_key("managed-disabled"),
        "DB-known disabled server should be removed from live config"
    );
    assert!(
        servers.contains_key("managed-enabled"),
        "DB-known enabled server should be present in live config"
    );
    assert!(
        servers.contains_key("external-only"),
        "live entries unknown to DB should be preserved"
    );
}

#[test]
fn mcp_off_toggle_upsert_delete_are_live_hands_off() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let (codex_path, opencode_path, hermes_path) = seed_embedded_mcp_targets(home);
    let before = [
        fs::read(&codex_path).expect("read codex baseline"),
        fs::read(&opencode_path).expect("read opencode baseline"),
        fs::read(&hermes_path).expect("read hermes baseline"),
    ];
    let state = create_test_state().expect("create test state");

    let mut server = embedded_mcp_server("echo");
    server.apps = McpApps::default();
    state
        .db
        .save_mcp_server(&server)
        .expect("seed disabled MCP");
    for app in [AppType::Codex, AppType::OpenCode, AppType::Hermes] {
        McpService::toggle_app(&state, &server.id, app, true).expect("toggle while hands-off");
    }
    assert_eq!(fs::read(&codex_path).unwrap(), before[0]);
    assert_eq!(fs::read(&opencode_path).unwrap(), before[1]);
    assert_eq!(fs::read(&hermes_path).unwrap(), before[2]);

    McpService::upsert_server(&state, embedded_mcp_server("printf"))
        .expect("upsert while hands-off");
    McpService::delete_server(&state, "managed-mcp").expect("delete while hands-off");
    assert_eq!(fs::read(&codex_path).unwrap(), before[0]);
    assert_eq!(fs::read(&opencode_path).unwrap(), before[1]);
    assert_eq!(fs::read(&hermes_path).unwrap(), before[2]);

    let statuses = futures::executor::block_on(state.external_config_monitor.get_status())
        .expect("get monitor status");
    for app in ["codex", "opencode", "hermes"] {
        let status = statuses
            .iter()
            .find(|status| status.app_type == app)
            .expect("embedded app status");
        assert_eq!(status.generation, 0, "hands-off must not begin {app} token");
        assert!(!status.conflict);
    }
}

#[test]
fn mcp_direct_updates_expected_for_embedded_targets_and_preserves_snapshot() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let (codex_path, opencode_path, hermes_path) = seed_embedded_mcp_targets(home);
    let state = create_test_state().expect("create test state");
    for app in [AppType::Codex, AppType::OpenCode, AppType::Hermes] {
        set_takeover_mode(&state, app.clone(), RouteMode::Direct);
        futures::executor::block_on(
            state
                .db
                .save_live_backup(app.as_str(), "immutable-snapshot"),
        )
        .expect("save immutable marker");
    }

    McpService::upsert_server(&state, embedded_mcp_server("echo")).expect("direct upsert MCP");
    assert!(fs::read_to_string(&codex_path)
        .unwrap()
        .contains("managed-mcp"));
    assert!(fs::read_to_string(&opencode_path)
        .unwrap()
        .contains("managed-mcp"));
    assert!(fs::read_to_string(&hermes_path)
        .unwrap()
        .contains("managed-mcp"));

    let first_statuses = futures::executor::block_on(state.external_config_monitor.get_status())
        .expect("get direct statuses");
    for app in ["codex", "opencode", "hermes"] {
        let status = first_statuses
            .iter()
            .find(|status| status.app_type == app)
            .expect("embedded app status");
        assert!(status.generation > 0, "{app} expected should be committed");
        assert!(!status.conflict);
    }

    McpService::toggle_app(&state, "managed-mcp", AppType::Codex, false)
        .expect("direct toggle removes Codex MCP");
    assert!(!fs::read_to_string(&codex_path)
        .unwrap()
        .contains("managed-mcp"));

    let mut updated = embedded_mcp_server("printf");
    updated.apps.codex = false;
    McpService::upsert_server(&state, updated).expect("direct update MCP");
    assert!(fs::read_to_string(&opencode_path)
        .unwrap()
        .contains("printf"));
    assert!(fs::read_to_string(&hermes_path).unwrap().contains("printf"));
    McpService::delete_server(&state, "managed-mcp").expect("direct delete MCP");
    assert!(!fs::read_to_string(&opencode_path)
        .unwrap()
        .contains("managed-mcp"));
    assert!(!fs::read_to_string(&hermes_path)
        .unwrap()
        .contains("managed-mcp"));

    for app in [AppType::Codex, AppType::OpenCode, AppType::Hermes] {
        let backup = futures::executor::block_on(state.db.get_live_backup(app.as_str()))
            .expect("read immutable marker")
            .expect("marker exists");
        assert_eq!(backup.original_config, "immutable-snapshot");
    }
}

#[test]
fn mcp_proxy_merge_preserves_each_gateway_namespace_and_token() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let (codex_path, opencode_path, hermes_path) = seed_embedded_mcp_targets(home);
    let state = create_test_state().expect("create test state");
    for app in [AppType::Codex, AppType::OpenCode, AppType::Hermes] {
        set_takeover_mode(&state, app, RouteMode::Proxy);
    }

    McpService::upsert_server(&state, embedded_mcp_server("echo")).expect("proxy merge MCP");

    let codex = fs::read_to_string(&codex_path).expect("read codex proxy target");
    assert!(codex.contains("http://127.0.0.1:42567/v1"));
    assert!(codex.contains("gateway-token"));
    assert!(codex.contains("managed-mcp"));

    let opencode: serde_json::Value =
        serde_json::from_slice(&fs::read(&opencode_path).unwrap()).expect("parse opencode target");
    assert_eq!(
        opencode
            .pointer("/provider/ags-proxy/options/baseURL")
            .and_then(|value| value.as_str()),
        Some("http://127.0.0.1:42567/opencode/v1")
    );
    assert_eq!(
        opencode
            .pointer("/provider/ags-proxy/options/apiKey")
            .and_then(|value| value.as_str()),
        Some("gateway-token")
    );
    assert!(opencode.pointer("/mcp/managed-mcp").is_some());

    let hermes = fs::read_to_string(&hermes_path).expect("read hermes proxy target");
    assert!(hermes.contains("http://127.0.0.1:42567/hermes/v1"));
    assert!(hermes.contains("gateway-token"));
    assert!(hermes.contains("managed-mcp"));

    let statuses = futures::executor::block_on(state.external_config_monitor.get_status())
        .expect("get proxy statuses");
    for app in ["codex", "opencode", "hermes"] {
        let status = statuses
            .iter()
            .find(|status| status.app_type == app)
            .expect("embedded app status");
        assert!(status.generation > 0);
        assert!(!status.conflict);
        assert_eq!(status.route_mode, RouteMode::Proxy);
    }
}

#[test]
fn mcp_writer_failure_aborts_generation_and_preserves_existing_conflict() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let (codex_path, _, _) = seed_embedded_mcp_targets(home);
    let state = create_test_state().expect("create test state");
    set_takeover_mode(&state, AppType::Codex, RouteMode::Direct);
    let mut server = embedded_mcp_server("echo");
    server.apps.opencode = false;
    server.apps.hermes = false;
    McpService::upsert_server(&state, server.clone()).expect("initialize Codex expected");

    let rt = tokio::runtime::Runtime::new().expect("create monitor runtime");
    rt.block_on(async {
        state
            .external_config_monitor
            .start()
            .await
            .expect("start monitor");
        tokio::time::sleep(std::time::Duration::from_millis(650)).await;
        fs::write(&codex_path, "model = [\n").expect("write external invalid TOML");
        tokio::time::sleep(std::time::Duration::from_millis(1_400)).await;
        let status = state
            .external_config_monitor
            .get_status()
            .await
            .expect("get conflict status")
            .into_iter()
            .find(|status| status.app_type == "codex")
            .expect("codex status");
        assert!(
            status.conflict,
            "external invalid TOML should create conflict"
        );

        server.server["command"] = json!("printf");
        let error = McpService::upsert_server(&state, server.clone())
            .expect_err("invalid TOML must fail MCP writer");
        assert!(error.to_string().contains("config.toml"));
        let failed_status = state
            .external_config_monitor
            .get_status()
            .await
            .expect("get failed status")
            .into_iter()
            .find(|status| status.app_type == "codex")
            .expect("codex status");
        assert!(
            failed_status.conflict,
            "failed self-write must not silently clear conflict"
        );
        state
            .external_config_monitor
            .stop()
            .await
            .expect("stop monitor");
    });

    fs::write(&codex_path, "model = \"recovered\"\n").expect("repair config TOML");
    McpService::sync_enabled_for_app(&state, &AppType::Codex)
        .expect("next managed write proves no in-flight leak");
    let status = futures::executor::block_on(state.external_config_monitor.get_status())
        .expect("get resolved status")
        .into_iter()
        .find(|status| status.app_type == "codex")
        .expect("codex status");
    assert!(
        !status.conflict,
        "successful explicit write becomes new expected"
    );
}
