//! 项目 Profile 快照/应用的端到端集成测试
//!
//! 全链路 apply 会写 live 配置文件——support.rs 已把 HOME 指向临时目录，安全。

use std::fs;

use serde_json::json;

use agent_switch_lib::{
    AppType, CcAggregateConfig, InstalledSkill, McpServer, McpService, ProfilePayload,
    ProfileScope, ProfileService, Prompt, PromptService, Provider, ProviderService, RouteMode,
    SkillApps, SkillService,
};

#[path = "support.rs"]
mod support;
use support::{create_test_state, ensure_test_home, reset_test_fs, test_mutex};

fn claude_provider(id: &str, token: &str) -> Provider {
    Provider::with_id(
        id.to_string(),
        id.to_uppercase(),
        json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": token,
                "ANTHROPIC_BASE_URL": "https://api.test"
            }
        }),
        None,
    )
}

/// Claude Desktop 供应商：无 meta 时默认 Direct 模式，只要求 env 里有 token + base_url
fn desktop_provider(id: &str, token: &str) -> Provider {
    Provider::with_id(
        id.to_string(),
        id.to_uppercase(),
        json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": token,
                "ANTHROPIC_BASE_URL": "https://desktop.test"
            }
        }),
        None,
    )
}

fn mcp_server(id: &str, claude_enabled: bool) -> McpServer {
    serde_json::from_value(json!({
        "id": id,
        "name": id,
        "server": { "command": "echo", "args": [] },
        "apps": { "claude": claude_enabled }
    }))
    .expect("construct mcp server")
}

fn prompt(id: &str, enabled: bool) -> Prompt {
    Prompt {
        id: id.to_string(),
        name: id.to_uppercase(),
        content: format!("# prompt {id}\n"),
        description: None,
        enabled,
        created_at: Some(1_000),
        updated_at: Some(1_000),
    }
}

fn installed_skill(id: &str, directory: &str, claude_enabled: bool) -> InstalledSkill {
    InstalledSkill {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        directory: directory.to_string(),
        repo_owner: None,
        repo_name: None,
        repo_branch: None,
        readme_url: None,
        apps: SkillApps {
            claude: claude_enabled,
            ..Default::default()
        },
        installed_at: 1_000,
        content_hash: None,
        updated_at: 0,
    }
}

fn write_ssot_skill(directory: &str) {
    let dir = SkillService::get_ssot_dir()
        .expect("resolve skills SSOT dir")
        .join(directory);
    fs::create_dir_all(&dir).expect("create skill dir");
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {directory}\ndescription: Test skill\n---\n"),
    )
    .expect("write SKILL.md");
}

#[test]
fn profile_snapshot_apply_roundtrip_restores_configuration() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    // ---- 种子数据：2 个 Claude 供应商（p1 为当前）+ 2 个 MCP + 1 个 Skill + 2 个 Prompt ----
    state
        .db
        .save_provider(AppType::Claude.as_str(), &claude_provider("p1", "key-1"))
        .expect("save provider p1");
    state
        .db
        .save_provider(AppType::Claude.as_str(), &claude_provider("p2", "key-2"))
        .expect("save provider p2");
    state
        .db
        .set_current_provider(AppType::Claude.as_str(), "p1")
        .expect("set current provider p1");

    // Claude Desktop 只有供应商一个活跃维度（MCP/Skills/Prompt 对它不适用）
    state
        .db
        .save_provider(
            AppType::ClaudeDesktop.as_str(),
            &desktop_provider("d1", "dk-1"),
        )
        .expect("save desktop provider d1");
    state
        .db
        .save_provider(
            AppType::ClaudeDesktop.as_str(),
            &desktop_provider("d2", "dk-2"),
        )
        .expect("save desktop provider d2");
    state
        .db
        .set_current_provider(AppType::ClaudeDesktop.as_str(), "d1")
        .expect("set current desktop provider d1");

    // 让 live settings.json 与 p1 一致（switch_normal 回填需要）
    let claude_dir = home.join(".claude");
    fs::create_dir_all(&claude_dir).expect("create .claude dir");
    fs::write(
        claude_dir.join("settings.json"),
        serde_json::to_string_pretty(&claude_provider("p1", "key-1").settings_config)
            .expect("serialize p1 settings"),
    )
    .expect("seed live settings.json");

    state
        .db
        .save_mcp_server(&mcp_server("m1", true))
        .expect("save mcp m1");
    state
        .db
        .save_mcp_server(&mcp_server("m2", false))
        .expect("save mcp m2");

    write_ssot_skill("test-skill");
    state
        .db
        .save_skill(&installed_skill("local:test-skill", "test-skill", true))
        .expect("save skill");

    state
        .db
        .save_prompt(AppType::Claude.as_str(), &prompt("pr1", true))
        .expect("save prompt pr1");
    state
        .db
        .save_prompt(AppType::Claude.as_str(), &prompt("pr2", false))
        .expect("save prompt pr2");
    let aggregate_config = CcAggregateConfig {
        enabled: true,
        ..Default::default()
    };
    state
        .db
        .set_cc_aggregate_config(AppType::Claude.as_str(), &aggregate_config)
        .expect("seed aggregate config");

    // ---- 保存项目 A（在 Claude 页新建：只拍 Claude 当前状态）----
    let profile_a = ProfileService::create(&state, "Project A", ProfileScope::Claude)
        .expect("create profile A");
    let payload: ProfilePayload =
        serde_json::from_str(&profile_a.payload).expect("parse profile A payload");
    assert_eq!(payload.providers.claude.as_deref(), Some("p1"));
    assert_eq!(payload.mcp.claude, Some(vec!["m1".to_string()]));
    assert_eq!(
        payload.skills.claude,
        Some(vec!["local:test-skill".to_string()])
    );
    assert_eq!(payload.prompts.claude.as_deref(), Some("pr1"));
    assert_eq!(
        payload.aggregate_config.claude.as_ref(),
        Some(&aggregate_config),
        "claude aggregate config captured"
    );
    assert_eq!(payload.aggregate_config.claude_desktop, None);
    assert_eq!(payload.aggregate_config.codex, None);
    assert_eq!(
        payload.providers.codex, None,
        "codex side not captured when creating from the claude group"
    );
    assert_eq!(payload.mcp.codex, None, "uncaptured side stays None");
    assert_eq!(
        payload.providers.claude_desktop, None,
        "claude desktop has its own profile scope"
    );

    // ---- 改动全部四类配置（走真实切换路径）----
    ProviderService::switch(&state, AppType::Claude, "p2").expect("switch to p2");
    // Desktop 现在有自己的项目分组；Claude 分组 apply 不应再影响 Desktop
    #[cfg(any(target_os = "macos", windows))]
    ProviderService::switch(&state, AppType::ClaudeDesktop, "d2").expect("switch desktop to d2");
    McpService::toggle_app(&state, "m1", AppType::Claude, false).expect("disable m1");
    McpService::toggle_app(&state, "m2", AppType::Claude, true).expect("enable m2");
    SkillService::toggle_app(&state.db, "local:test-skill", &AppType::Claude, false)
        .expect("disable skill");
    PromptService::enable_prompt(&state, AppType::Claude, "pr2").expect("enable pr2");
    state
        .db
        .set_cc_aggregate_config(AppType::Claude.as_str(), &CcAggregateConfig::default())
        .expect("change aggregate config");

    // ---- 应用项目 A（Claude 组）：只复原 Claude 侧 ----
    let (warnings, _) = ProfileService::apply(&state, &profile_a.id, ProfileScope::Claude)
        .expect("apply profile A");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    let current = state
        .db
        .get_current_provider(AppType::Claude.as_str())
        .expect("get current provider");
    assert_eq!(current.as_deref(), Some("p1"), "provider restored to p1");

    // Claude 分组不再管理 Desktop：apply 后 Desktop 保持切换前的状态不变。
    // macOS/Windows 上上面已切到 d2；Linux（CI）不支持 Desktop 切换、那行被 cfg 门控
    // 编译剔除，Desktop 仍是种子值 d1。两种情况都验证 claude-scope apply 不会动 Desktop。
    let current_desktop = state
        .db
        .get_current_provider(AppType::ClaudeDesktop.as_str())
        .expect("get current desktop provider");
    #[cfg(any(target_os = "macos", windows))]
    let expected_desktop = "d2";
    #[cfg(not(any(target_os = "macos", windows)))]
    let expected_desktop = "d1";
    assert_eq!(
        current_desktop.as_deref(),
        Some(expected_desktop),
        "desktop provider untouched by claude-scope apply"
    );

    let servers = state.db.get_all_mcp_servers().expect("get mcp servers");
    assert!(servers.get("m1").expect("m1").apps.claude, "m1 re-enabled");
    assert!(!servers.get("m2").expect("m2").apps.claude, "m2 disabled");

    let skills = state.db.get_all_installed_skills().expect("get skills");
    assert!(
        skills.get("local:test-skill").expect("skill").apps.claude,
        "skill re-enabled"
    );

    let prompts = state
        .db
        .get_prompts(AppType::Claude.as_str())
        .expect("get prompts");
    assert!(prompts.get("pr1").expect("pr1").enabled, "pr1 re-enabled");
    assert!(!prompts.get("pr2").expect("pr2").enabled, "pr2 disabled");
    assert_eq!(
        state
            .db
            .get_cc_aggregate_config(AppType::Claude.as_str())
            .expect("get restored aggregate config"),
        aggregate_config,
        "aggregate config restored"
    );

    let live_prompt = fs::read_to_string(claude_dir.join("CLAUDE.md")).expect("read CLAUDE.md");
    assert_eq!(
        live_prompt,
        prompt("pr1", true).content,
        "live memory file restored"
    );

    assert_eq!(
        state
            .db
            .get_current_profile_id("claude")
            .expect("get current profile id")
            .as_deref(),
        Some(profile_a.id.as_str()),
        "profile A marked as current for claude scope"
    );
    assert_eq!(
        state
            .db
            .get_current_profile_id("codex")
            .expect("get codex current profile id"),
        None,
        "codex scope marker untouched by claude-group apply"
    );
}

#[test]
fn shared_profile_sides_are_isolated_and_mergeable() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    // 种子：Claude 侧有当前供应商 + 启用的 MCP
    state
        .db
        .save_provider(AppType::Claude.as_str(), &claude_provider("p1", "key-1"))
        .expect("save provider p1");
    state
        .db
        .set_current_provider(AppType::Claude.as_str(), "p1")
        .expect("set current provider p1");
    let claude_dir = home.join(".claude");
    fs::create_dir_all(&claude_dir).expect("create .claude dir");
    fs::write(
        claude_dir.join("settings.json"),
        serde_json::to_string_pretty(&claude_provider("p1", "key-1").settings_config)
            .expect("serialize p1 settings"),
    )
    .expect("seed live settings.json");
    state
        .db
        .save_mcp_server(&mcp_server("m1", true))
        .expect("save mcp m1");

    // 在 Codex 页新建项目：快照不应捕获 Claude 侧的任何状态
    let project = ProfileService::create(&state, "Shared Project", ProfileScope::Codex)
        .expect("create project from codex tab");
    let payload: ProfilePayload =
        serde_json::from_str(&project.payload).expect("parse project payload");
    assert_eq!(
        payload.providers.claude, None,
        "claude slot not captured by codex-side snapshot"
    );
    assert_eq!(payload.mcp.claude, None);
    assert_eq!(payload.providers.claude_desktop, None);
    assert_eq!(payload.mcp.codex, Some(vec![]), "codex side captured");

    // 按 Codex 组应用：只动 codex 组的 current 标记，Claude 侧原样不动
    let (warnings, _) = ProfileService::apply(&state, &project.id, ProfileScope::Codex)
        .expect("apply project on codex side");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    assert_eq!(
        state
            .db
            .get_current_provider(AppType::Claude.as_str())
            .expect("get claude current provider")
            .as_deref(),
        Some("p1"),
        "claude provider untouched"
    );
    let servers = state.db.get_all_mcp_servers().expect("get mcp servers");
    assert!(
        servers.get("m1").expect("m1").apps.claude,
        "claude MCP untouched"
    );
    assert_eq!(
        state
            .db
            .get_current_profile_id("codex")
            .expect("get codex current profile id")
            .as_deref(),
        Some(project.id.as_str())
    );
    assert_eq!(
        state
            .db
            .get_current_profile_id("claude")
            .expect("get claude current profile id"),
        None,
        "claude scope marker untouched by codex-side apply"
    );

    // 同一共享项目在 Claude 页应用：该侧未拍过快照 → 不动配置、标记 current、返回提示
    let (warnings, _) = ProfileService::apply(&state, &project.id, ProfileScope::Claude)
        .expect("apply project on claude side");
    assert_eq!(warnings.len(), 1, "uncaptured side yields one hint");
    assert!(warnings[0].contains("no claude configuration captured"));
    let servers = state.db.get_all_mcp_servers().expect("get mcp servers");
    assert!(
        servers.get("m1").expect("m1").apps.claude,
        "claude MCP still untouched by uncaptured apply"
    );
    assert_eq!(
        state
            .db
            .get_current_profile_id("claude")
            .expect("get claude current profile id")
            .as_deref(),
        Some(project.id.as_str()),
        "claude side now bound to the shared project"
    );

    // 在 Claude 页"以当前状态更新"：补拍 claude 侧，codex 侧快照原样保留
    let updated =
        ProfileService::update(&state, &project.id, None, true, Some(ProfileScope::Claude))
            .expect("resnapshot claude side");
    let payload: ProfilePayload =
        serde_json::from_str(&updated.payload).expect("parse updated payload");
    assert_eq!(payload.providers.claude.as_deref(), Some("p1"));
    assert_eq!(payload.mcp.claude, Some(vec!["m1".to_string()]));
    assert_eq!(
        payload.mcp.codex,
        Some(vec![]),
        "codex side snapshot preserved by claude-side resnapshot"
    );
}

#[test]
fn profile_apply_reports_dangling_references_and_continues() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    state
        .db
        .save_mcp_server(&mcp_server("m1", false))
        .expect("save mcp m1");

    // 手工构造引用了不存在资源的 payload
    let payload = json!({
        "providers": { "claude": "ghost-provider" },
        "mcp": { "claude": ["m1", "ghost-mcp"] },
        "skills": { "claude": ["ghost-skill"] },
        "prompts": { "claude": "ghost-prompt" }
    });
    let profile = agent_switch_lib::Profile {
        id: "dangling-test".to_string(),
        name: "Dangling".to_string(),
        payload: payload.to_string(),
        sort_order: None,
        created_at: Some(1_000),
        updated_at: Some(1_000),
    };
    state.db.save_profile(&profile).expect("save profile");

    let (warnings, _) = ProfileService::apply(&state, "dangling-test", ProfileScope::Claude)
        .expect("apply succeeds");
    assert_eq!(
        warnings.len(),
        4,
        "each dangling reference yields one warning: {warnings:?}"
    );

    // 有效条目照常生效：m1 被启用
    let servers = state.db.get_all_mcp_servers().expect("get mcp servers");
    assert!(
        servers.get("m1").expect("m1").apps.claude,
        "m1 enabled despite warnings"
    );

    // best-effort 完成后仍标记为所属分组的当前项目
    assert_eq!(
        state
            .db
            .get_current_profile_id("claude")
            .expect("get current profile id")
            .as_deref(),
        Some("dangling-test")
    );
}

#[test]
fn clear_current_profile_only_clears_scoped_marker() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    state
        .db
        .set_current_profile_id("claude", Some("claude-profile"))
        .expect("set claude current profile");
    state
        .db
        .set_current_profile_id("codex", Some("codex-profile"))
        .expect("set codex current profile");

    // 清除 claude 组不影响 codex 组
    state
        .db
        .set_current_profile_id("claude", None)
        .expect("clear claude current profile");
    assert_eq!(
        state
            .db
            .get_current_profile_id("claude")
            .expect("get claude current profile id"),
        None
    );
    assert_eq!(
        state
            .db
            .get_current_profile_id("codex")
            .expect("get codex current profile id")
            .as_deref(),
        Some("codex-profile")
    );
}

#[test]
fn switching_profile_autosaves_previous_profile_state() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    // ---- 种子：Claude 侧两套供应商 / MCP / Prompt ----
    state
        .db
        .save_provider(AppType::Claude.as_str(), &claude_provider("p1", "key-1"))
        .expect("save provider p1");
    state
        .db
        .save_provider(AppType::Claude.as_str(), &claude_provider("p2", "key-2"))
        .expect("save provider p2");
    state
        .db
        .set_current_provider(AppType::Claude.as_str(), "p1")
        .expect("set current provider p1");

    let claude_dir = home.join(".claude");
    fs::create_dir_all(&claude_dir).expect("create .claude dir");
    fs::write(
        claude_dir.join("settings.json"),
        serde_json::to_string_pretty(&claude_provider("p1", "key-1").settings_config)
            .expect("serialize p1 settings"),
    )
    .expect("seed live settings.json");

    state
        .db
        .save_mcp_server(&mcp_server("m1", true))
        .expect("save mcp m1");
    state
        .db
        .save_mcp_server(&mcp_server("m2", false))
        .expect("save mcp m2");

    state
        .db
        .save_prompt(AppType::Claude.as_str(), &prompt("pr1", true))
        .expect("save prompt pr1");
    state
        .db
        .save_prompt(AppType::Claude.as_str(), &prompt("pr2", false))
        .expect("save prompt pr2");

    // ---- Project A：状态 X（p1 / m1 / pr1）----
    let project_a = ProfileService::create(&state, "Project A", ProfileScope::Claude)
        .expect("create project A");
    let (warnings, _) = ProfileService::apply(&state, &project_a.id, ProfileScope::Claude)
        .expect("apply project A");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    // ---- 在 A 下改到状态 Y（p2 / m2 / pr2），然后据此创建 Project B ----
    ProviderService::switch(&state, AppType::Claude, "p2").expect("switch to p2");
    McpService::toggle_app(&state, "m1", AppType::Claude, false).expect("disable m1");
    McpService::toggle_app(&state, "m2", AppType::Claude, true).expect("enable m2");
    PromptService::enable_prompt(&state, AppType::Claude, "pr2").expect("enable pr2");

    let project_b = ProfileService::create(&state, "Project B", ProfileScope::Claude)
        .expect("create project B");

    // ---- 从 A 切换到 B：自动把当前状态 Y 保存到 A，再加载 B 的 Y ----
    let (warnings, _) = ProfileService::apply(&state, &project_b.id, ProfileScope::Claude)
        .expect("switch to project B");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    assert_eq!(
        state
            .db
            .get_current_provider(AppType::Claude.as_str())
            .expect("get current provider")
            .as_deref(),
        Some("p2"),
        "provider switched to p2"
    );
    let servers = state.db.get_all_mcp_servers().expect("get mcp servers");
    assert!(!servers.get("m1").expect("m1").apps.claude, "m1 disabled");
    assert!(servers.get("m2").expect("m2").apps.claude, "m2 enabled");
    let prompts = state
        .db
        .get_prompts(AppType::Claude.as_str())
        .expect("get prompts");
    assert!(!prompts.get("pr1").expect("pr1").enabled, "pr1 disabled");
    assert!(prompts.get("pr2").expect("pr2").enabled, "pr2 enabled");

    // Project A 被自动保存为离开时的状态 Y
    let saved_a = state
        .db
        .get_profile(&project_a.id)
        .expect("get project A")
        .expect("project A exists");
    let payload_a: ProfilePayload =
        serde_json::from_str(&saved_a.payload).expect("parse project A payload");
    assert_eq!(payload_a.providers.claude.as_deref(), Some("p2"));
    assert_eq!(payload_a.mcp.claude, Some(vec!["m2".to_string()]));
    assert_eq!(payload_a.prompts.claude.as_deref(), Some("pr2"));

    // ---- 在 B 下改回状态 X，再切换回 A ----
    ProviderService::switch(&state, AppType::Claude, "p1").expect("switch to p1");
    McpService::toggle_app(&state, "m1", AppType::Claude, true).expect("enable m1");
    McpService::toggle_app(&state, "m2", AppType::Claude, false).expect("disable m2");
    PromptService::enable_prompt(&state, AppType::Claude, "pr1").expect("enable pr1");

    let (warnings, _) = ProfileService::apply(&state, &project_a.id, ProfileScope::Claude)
        .expect("switch back to project A");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    // 切回 A 时：先自动保存 B 为状态 X，再加载 A 的上次离开状态 Y
    assert_eq!(
        state
            .db
            .get_current_provider(AppType::Claude.as_str())
            .expect("get current provider")
            .as_deref(),
        Some("p2"),
        "project A restored to the state when we left it (p2)"
    );
    let servers = state.db.get_all_mcp_servers().expect("get mcp servers");
    assert!(
        !servers.get("m1").expect("m1").apps.claude,
        "m1 stays disabled"
    );
    assert!(
        servers.get("m2").expect("m2").apps.claude,
        "m2 stays enabled"
    );
    let prompts = state
        .db
        .get_prompts(AppType::Claude.as_str())
        .expect("get prompts");
    assert!(
        !prompts.get("pr1").expect("pr1").enabled,
        "pr1 stays disabled"
    );
    assert!(
        prompts.get("pr2").expect("pr2").enabled,
        "pr2 stays enabled"
    );

    // Project B 被自动保存为离开时的状态 X
    let saved_b = state
        .db
        .get_profile(&project_b.id)
        .expect("get project B")
        .expect("project B exists");
    let payload_b: ProfilePayload =
        serde_json::from_str(&saved_b.payload).expect("parse project B payload");
    assert_eq!(payload_b.providers.claude.as_deref(), Some("p1"));
    assert_eq!(payload_b.mcp.claude, Some(vec!["m1".to_string()]));
    assert_eq!(payload_b.prompts.claude.as_deref(), Some("pr1"));
}

#[test]
fn profile_switch_auto_disables_takeover_before_apply() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    // 使用临时端口，避免测试机器端口冲突
    futures::executor::block_on(async {
        let mut proxy_config = state.db.get_proxy_config().await.expect("get proxy config");
        proxy_config.listen_port = 0;
        state
            .db
            .update_proxy_config(proxy_config)
            .await
            .expect("set ephemeral proxy port");
    });

    // ---- 两个 Claude 供应商：custom1 与 custom2 ----
    let mut custom1 = claude_provider("custom1", "custom-key-1");
    custom1.category = Some("custom".to_string());
    state
        .db
        .save_provider(AppType::Claude.as_str(), &custom1)
        .expect("save custom1 provider");

    let mut custom2 = claude_provider("custom2", "custom-key-2");
    custom2.category = Some("custom".to_string());
    state
        .db
        .save_provider(AppType::Claude.as_str(), &custom2)
        .expect("save custom2 provider");

    // 初始状态：custom1 + 代理接管
    ProviderService::switch(&state, AppType::Claude, "custom1").expect("switch to custom1");
    let rt = tokio::runtime::Runtime::new().expect("create tokio runtime");
    rt.block_on(
        state
            .proxy_service
            .set_takeover_for_app("claude", true, RouteMode::Proxy),
    )
    .expect("enable claude takeover");

    let (proxy_enabled_before, _) = state.db.get_proxy_flags_sync("claude");
    assert!(
        proxy_enabled_before,
        "takeover should be active before apply"
    );

    // ---- 构造一个目标为 custom2 的项目快照 ----
    let project = ProfileService::create(&state, "Custom2 Project", ProfileScope::Claude)
        .expect("create project");
    let mut project = state
        .db
        .get_profile(&project.id)
        .expect("get project")
        .expect("project exists");
    let mut payload: ProfilePayload =
        serde_json::from_str(&project.payload).expect("parse project payload");
    payload.providers.claude = Some("custom2".to_string());
    project.payload = serde_json::to_string(&payload).expect("serialize payload");
    state
        .db
        .save_profile(&project)
        .expect("save updated project");

    // ---- 应用项目：应无条件自动关闭接管，再切换到 custom2 ----
    let (warnings, _) = ProfileService::apply(&state, &project.id, ProfileScope::Claude)
        .expect("apply custom2 project");
    assert!(
        warnings.is_empty(),
        "switching project should not warn: {warnings:?}"
    );

    // 接管已关闭
    let (proxy_enabled_after, _) = state.db.get_proxy_flags_sync("claude");
    assert!(
        !proxy_enabled_after,
        "proxy takeover should be auto-disabled before applying profile"
    );

    // 当前供应商已切到 custom2
    assert_eq!(
        state
            .db
            .get_current_provider(AppType::Claude.as_str())
            .expect("get current provider")
            .as_deref(),
        Some("custom2"),
        "current provider should be custom2"
    );

    // C2a three-dimensional semantics: the pre-takeover target did not exist, so
    // auto-disable must restore it to "absent" (R10). Profile apply then switches
    // DB current to custom2 while takeover remains OFF; hands-off mode must NOT
    // recreate/write live. The real endpoint remains in DB SSOT until the user
    // explicitly re-enables direct takeover.
    let settings_path = home.join(".claude/settings.json");
    assert!(
        !settings_path.exists(),
        "auto-disable should restore missing pre-takeover live, and hands-off profile switch must not recreate it"
    );
}

#[cfg(any(target_os = "macos", windows))]
#[test]
fn claude_desktop_profile_scope_is_independent() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let _home = ensure_test_home();

    let state = create_test_state().expect("create test state");

    state
        .db
        .save_provider(
            AppType::ClaudeDesktop.as_str(),
            &desktop_provider("d1", "dk-1"),
        )
        .expect("save desktop provider d1");
    state
        .db
        .save_provider(
            AppType::ClaudeDesktop.as_str(),
            &desktop_provider("d2", "dk-2"),
        )
        .expect("save desktop provider d2");
    state
        .db
        .set_current_provider(AppType::ClaudeDesktop.as_str(), "d1")
        .expect("set current desktop provider d1");

    // 在 Desktop 页新建项目：只拍 Desktop 供应商
    let project = ProfileService::create(&state, "Desktop Project", ProfileScope::ClaudeDesktop)
        .expect("create desktop profile");
    let payload: ProfilePayload =
        serde_json::from_str(&project.payload).expect("parse desktop payload");
    assert_eq!(payload.providers.claude_desktop.as_deref(), Some("d1"));
    assert_eq!(payload.providers.claude, None, "claude slot untouched");
    assert_eq!(payload.providers.codex, None, "codex slot untouched");

    // 切到 d2
    ProviderService::switch(&state, AppType::ClaudeDesktop, "d2").expect("switch desktop to d2");

    // 应用 Desktop 项目：恢复 d1
    let (warnings, _) = ProfileService::apply(&state, &project.id, ProfileScope::ClaudeDesktop)
        .expect("apply desktop profile");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");

    assert_eq!(
        state
            .db
            .get_current_provider(AppType::ClaudeDesktop.as_str())
            .expect("get current desktop provider")
            .as_deref(),
        Some("d1"),
        "desktop provider restored by desktop-scope apply"
    );
    assert_eq!(
        state
            .db
            .get_current_profile_id(ProfileScope::ClaudeDesktop.as_str())
            .expect("get desktop current profile id")
            .as_deref(),
        Some(project.id.as_str()),
        "desktop scope marker set"
    );
}

#[test]
fn profile_apply_hands_off_when_takeover_is_off() {
    let _guard = test_mutex().lock().expect("acquire test mutex");
    reset_test_fs();
    let home = ensure_test_home();
    let state = create_test_state().expect("create test state");
    let settings_path = home.join(".claude").join("settings.json");
    fs::create_dir_all(settings_path.parent().unwrap()).expect("create claude dir");
    let original = b"{\n  \"userOwned\": true\n}\n";
    fs::write(&settings_path, original).expect("seed user live");

    for provider in [
        claude_provider("p1", "key-1"),
        claude_provider("p2", "key-2"),
    ] {
        state
            .db
            .save_provider(AppType::Claude.as_str(), &provider)
            .expect("save provider");
    }
    state
        .db
        .set_current_provider(AppType::Claude.as_str(), "p1")
        .expect("set current provider");
    let profile = agent_switch_lib::Profile {
        id: "off-profile".to_string(),
        name: "Off Profile".to_string(),
        payload: json!({ "providers": { "claude": "p2" } }).to_string(),
        sort_order: None,
        created_at: Some(1),
        updated_at: Some(1),
    };
    state.db.save_profile(&profile).expect("save profile");

    let (warnings, _) = ProfileService::apply(&state, &profile.id, ProfileScope::Claude)
        .expect("apply hands-off profile");
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(fs::read(&settings_path).unwrap(), original);
    assert_eq!(
        state
            .db
            .get_current_provider(AppType::Claude.as_str())
            .unwrap()
            .as_deref(),
        Some("p2"),
        "profile still updates DB/current SSOT"
    );
}

#[test]
fn profile_apply_direct_and_proxy_restore_snapshot_then_clear_conflict() {
    let _guard = test_mutex().lock().expect("acquire test mutex");

    for route_mode in [RouteMode::Direct, RouteMode::Proxy] {
        reset_test_fs();
        let home = ensure_test_home();
        let state = create_test_state().expect("create test state");
        let settings_path = home.join(".claude").join("settings.json");
        fs::create_dir_all(settings_path.parent().unwrap()).expect("create claude dir");
        let original = format!("{{\n  \"restoreMode\": \"{}\"\n}}\n", route_mode.as_str());
        fs::write(&settings_path, original.as_bytes()).expect("seed original live");

        let mut p1 = claude_provider("p1", "key-1");
        p1.category = Some("custom".to_string());
        let mut p2 = claude_provider("p2", "key-2");
        p2.category = Some("custom".to_string());
        state
            .db
            .save_provider(AppType::Claude.as_str(), &p1)
            .expect("save p1");
        state
            .db
            .save_provider(AppType::Claude.as_str(), &p2)
            .expect("save p2");
        state
            .db
            .set_current_provider(AppType::Claude.as_str(), "p1")
            .expect("set current p1");

        let rt = tokio::runtime::Runtime::new().expect("create profile monitor runtime");
        rt.block_on(async {
            let mut proxy_config = state.db.get_proxy_config().await.expect("get proxy config");
            proxy_config.listen_port = 0;
            state
                .db
                .update_proxy_config(proxy_config)
                .await
                .expect("set ephemeral port");
            state
                .proxy_service
                .set_takeover_for_app("claude", true, route_mode)
                .await
                .expect("enable takeover");
        });
        let backup_before = futures::executor::block_on(state.db.get_live_backup("claude"))
            .expect("read restore snapshot")
            .expect("restore snapshot exists")
            .original_config;

        let profile = agent_switch_lib::Profile {
            id: format!("profile-{}", route_mode.as_str()),
            name: format!("Profile {}", route_mode.as_str()),
            payload: json!({ "providers": { "claude": "p2" } }).to_string(),
            sort_order: None,
            created_at: Some(1),
            updated_at: Some(1),
        };
        state.db.save_profile(&profile).expect("save profile");

        rt.block_on(async {
            state
                .external_config_monitor
                .start()
                .await
                .expect("start monitor");
            tokio::time::sleep(std::time::Duration::from_millis(650)).await;
            fs::write(&settings_path, b"{\"external\":true}").expect("write external conflict");
            tokio::time::sleep(std::time::Duration::from_millis(1_400)).await;
            let status = state
                .external_config_monitor
                .get_status()
                .await
                .expect("get conflict status")
                .into_iter()
                .find(|status| status.app_type == "claude")
                .expect("claude status");
            assert!(
                status.conflict,
                "profile precondition should have a conflict"
            );
            let backup_during = state
                .db
                .get_live_backup("claude")
                .await
                .expect("read snapshot during conflict")
                .expect("snapshot retained")
                .original_config;
            assert_eq!(
                backup_during, backup_before,
                "conflict must not replace C2 snapshot"
            );
            state
                .external_config_monitor
                .stop()
                .await
                .expect("stop monitor");
        });

        let (warnings, _) = ProfileService::apply(&state, &profile.id, ProfileScope::Claude)
            .expect("apply profile with ownership");
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(
            fs::read(&settings_path).expect("read restored live"),
            original.as_bytes(),
            "C2 disable must byte-exact restore before DB-only profile switch"
        );
        assert!(
            futures::executor::block_on(state.db.get_live_backup("claude"))
                .expect("read released snapshot")
                .is_none(),
            "successful C2 restore releases immutable snapshot"
        );
        let status = futures::executor::block_on(state.external_config_monitor.get_status())
            .expect("get resolved status")
            .into_iter()
            .find(|status| status.app_type == "claude")
            .expect("claude status");
        assert!(!status.takeover_enabled);
        assert!(
            !status.conflict,
            "profile apply must not leave a self-conflict"
        );
        assert_eq!(
            state
                .db
                .get_current_provider(AppType::Claude.as_str())
                .unwrap()
                .as_deref(),
            Some("p2")
        );

        if route_mode == RouteMode::Proxy {
            rt.block_on(state.proxy_service.stop()).expect("stop proxy");
        }
    }
}
