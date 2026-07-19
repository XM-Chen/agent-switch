//! 代理服务业务逻辑层
//!
//! 提供代理服务器的启动、停止和配置管理

use crate::app_config::AppType;
use crate::config::{get_claude_settings_path, read_json_file, write_json_file};
use crate::database::Database;
use crate::provider::Provider;
use crate::proxy::server::ProxyServer;
use crate::proxy::snapshot::{
    abandon_snapshot_ownership, capture_snapshot_once, restore_snapshot_and_release,
    CaptureSnapshotOutcome, SnapshotManifest, SnapshotModuleAdapter,
};
use crate::proxy::snapshot_adapters::snapshot_adapter_for as c2b_snapshot_adapter_for;
use crate::proxy::switch_lock::SwitchLockManager;
use crate::proxy::types::*;
use crate::services::provider::{
    build_effective_settings_with_common_config, write_live_with_common_config,
};
use crate::services::proxy_snapshot_adapters::snapshot_adapter_for_app as c2a_snapshot_adapter_for;
use serde_json::{json, Map, Value};
use std::str::FromStr;
use std::sync::Arc;
use tauri::Emitter;
use tokio::sync::RwLock;

/// 用于接管 Live 配置时的占位符（避免客户端提示缺少 key，同时不泄露真实 Token）
const PROXY_TOKEN_PLACEHOLDER: &str = "PROXY_MANAGED";

/// 代理接管模式下需要从 Claude Live 配置中移除的"模型覆盖"字段。
///
/// 原因：接管模式下 `*_MODEL` 必须由 CC Switch 写成稳定的 Claude 角色别名，
/// 再由本地代理映射到当前供应商真实模型；`*_MODEL_NAME` 也需要同步接管，
/// 否则 Claude Code 模型菜单会残留上一个供应商的显示名称。
const CLAUDE_MODEL_OVERRIDE_ENV_KEYS: [&str; 12] = [
    "ANTHROPIC_MODEL",
    "ANTHROPIC_REASONING_MODEL", // legacy: 已废弃，但旧配置可能残留
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
    "ANTHROPIC_SMALL_FAST_MODEL", // Legacy key (已废弃)：历史版本使用该字段区分 small/fast 模型
    "CLAUDE_CODE_SUBAGENT_MODEL",
];

const CLAUDE_TAKEOVER_HAIKU_MODEL: &str = "claude-haiku-4-5";
const CLAUDE_TAKEOVER_SONNET_MODEL: &str = "claude-sonnet-4-6";
const CLAUDE_TAKEOVER_OPUS_MODEL: &str = "claude-opus-4-8";
const CLAUDE_TAKEOVER_FABLE_MODEL: &str = "claude-fable-5";
// 写给 Claude Code 时沿用文档示例的大写形式；解析侧大小写不敏感。
const CLAUDE_ONE_M_MARKER_FOR_CLIENT: &str = "[1M]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeTakeoverAuthPolicy {
    PreserveExistingOrAuthToken,
    ManagedAccount { keep_auth_token: bool },
}

#[derive(Clone)]
pub struct ProxyService {
    db: Arc<Database>,
    server: Arc<RwLock<Option<ProxyServer>>>,
    /// AppHandle，用于传递给 ProxyServer 以支持故障转移时的 UI 更新
    app_handle: Arc<RwLock<Option<tauri::AppHandle>>>,
    switch_locks: SwitchLockManager,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HotSwitchOutcome {
    pub logical_target_changed: bool,
}

/// 基于 `(takeover_enabled, route_mode)` 的三态 Live 写入判定。
///
/// 真相来源是 C1 的 `proxy_config`，取代旧的"有备份/占位符即视为接管"信号。
/// provider 的保存/切换/同步路径据此决定是否写 Live 及写向何处。
/// C2a 落地公共骨架，C2b rebase 后复用同一入口扩四模块。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveWriteDecision {
    /// 接管关闭：Agent-Switch 不得写该模块 Live 配置，只更新 DB SSOT。
    Skip,
    /// 接管开启 + direct：写真实上游配置（复用 provider live 写入），不依赖网关。
    DirectUpstream,
    /// 接管开启 + proxy：维护网关接管态（更新受管 live，不破坏占位符）。
    ProxyManaged,
}

impl LiveWriteDecision {
    pub fn from_config(config: &AppProxyConfig) -> Self {
        if !config.takeover_enabled {
            Self::Skip
        } else if config.route_mode == RouteMode::Proxy {
            Self::ProxyManaged
        } else {
            Self::DirectUpstream
        }
    }
}

impl ProxyService {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            db,
            server: Arc::new(RwLock::new(None)),
            app_handle: Arc::new(RwLock::new(None)),
            switch_locks: SwitchLockManager::new(),
        }
    }

    #[cfg(test)]
    fn apply_claude_takeover_fields(config: &mut Value, proxy_url: &str) {
        Self::apply_claude_takeover_fields_with_policy(
            config,
            proxy_url,
            ClaudeTakeoverAuthPolicy::PreserveExistingOrAuthToken,
        );
    }

    fn apply_claude_takeover_fields_for_provider(
        config: &mut Value,
        proxy_url: &str,
        provider: &Provider,
    ) {
        let auth_policy = if provider.uses_managed_account_auth() {
            // Codex 系（含仅凭 base_url 识别、无 provider_type meta 的）必须保留
            // ANTHROPIC_AUTH_TOKEN 占位符：Claude Code 缺该键会弹登录提示（#3784）。
            // Copilot 维持仅 API_KEY 占位，避免与 /login 管理的 key 冲突（#1049）。
            ClaudeTakeoverAuthPolicy::ManagedAccount {
                keep_auth_token: !provider.is_github_copilot(),
            }
        } else {
            ClaudeTakeoverAuthPolicy::PreserveExistingOrAuthToken
        };
        // Copilot/Codex 接管时 live config 可能还是旧供应商；显示模型必须跟随目标 provider。
        let takeover_model_fields = if provider.uses_managed_account_auth() {
            Self::build_claude_takeover_model_fields(&provider.settings_config)
        } else {
            Self::build_claude_takeover_model_fields(config)
        };

        Self::apply_claude_takeover_fields_with_policy_and_models(
            config,
            proxy_url,
            auth_policy,
            takeover_model_fields,
        );
    }

    #[cfg(test)]
    fn apply_claude_takeover_fields_with_policy(
        config: &mut Value,
        proxy_url: &str,
        auth_policy: ClaudeTakeoverAuthPolicy,
    ) {
        // 必须在 remove/insert 前 snapshot：避免读到自己刚写入的接管别名。
        let takeover_model_fields = Self::build_claude_takeover_model_fields(config);

        Self::apply_claude_takeover_fields_with_policy_and_models(
            config,
            proxy_url,
            auth_policy,
            takeover_model_fields,
        );
    }

    fn apply_claude_takeover_fields_with_policy_and_models(
        config: &mut Value,
        proxy_url: &str,
        auth_policy: ClaudeTakeoverAuthPolicy,
        takeover_model_fields: Vec<(&'static str, String)>,
    ) {
        if !config.is_object() {
            *config = json!({});
        }

        let root = config
            .as_object_mut()
            .expect("Claude config should be normalized to an object");
        let env = root.entry("env".to_string()).or_insert_with(|| json!({}));
        if !env.is_object() {
            *env = json!({});
        }

        let env = env
            .as_object_mut()
            .expect("Claude env should be normalized to an object");
        env.insert("ANTHROPIC_BASE_URL".to_string(), json!(proxy_url));

        for key in CLAUDE_MODEL_OVERRIDE_ENV_KEYS {
            env.remove(key);
        }

        for (key, value) in takeover_model_fields {
            env.insert(key.to_string(), Value::String(value));
        }

        let token_keys = [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "OPENROUTER_API_KEY",
            "OPENAI_API_KEY",
        ];

        match auth_policy {
            ClaudeTakeoverAuthPolicy::PreserveExistingOrAuthToken => {
                let mut replaced_any = false;
                for key in token_keys {
                    if env.contains_key(key) {
                        env.insert(key.to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
                        replaced_any = true;
                    }
                }

                if !replaced_any {
                    env.insert(
                        "ANTHROPIC_AUTH_TOKEN".to_string(),
                        json!(PROXY_TOKEN_PLACEHOLDER),
                    );
                }
            }
            ClaudeTakeoverAuthPolicy::ManagedAccount { keep_auth_token } => {
                for key in token_keys {
                    env.remove(key);
                }
                // 只注入一个认证键：两者同时存在会触发 Claude Code 的
                // "Both ANTHROPIC_AUTH_TOKEN and ANTHROPIC_API_KEY set" 警告（#4919）。
                // - Codex 系保留 AUTH_TOKEN：缺该键 Claude Code 会弹登录提示（#3784）。
                //   无条件注入而非"已存在才保留"：热切换路径传入的是 provider
                //   settings（预设不含该键），且旧版接管已把存量用户 live 中的键删光。
                // - Copilot 仅 API_KEY：避免与 /login 管理的 key 冲突（#1049）。
                if keep_auth_token {
                    env.insert(
                        "ANTHROPIC_AUTH_TOKEN".to_string(),
                        json!(PROXY_TOKEN_PLACEHOLDER),
                    );
                } else {
                    env.insert(
                        "ANTHROPIC_API_KEY".to_string(),
                        json!(PROXY_TOKEN_PLACEHOLDER),
                    );
                }
            }
        }
    }

    fn build_claude_takeover_model_fields(config: &Value) -> Vec<(&'static str, String)> {
        let Some(env) = config.get("env").and_then(Value::as_object) else {
            return Vec::new();
        };

        let default_model = Self::claude_env_string(env, "ANTHROPIC_MODEL");
        let small_fast_model = Self::claude_env_string(env, "ANTHROPIC_SMALL_FAST_MODEL");
        let haiku_model = Self::claude_env_string(env, "ANTHROPIC_DEFAULT_HAIKU_MODEL")
            .or(small_fast_model)
            .or(default_model);
        let sonnet_model = Self::claude_env_string(env, "ANTHROPIC_DEFAULT_SONNET_MODEL")
            .or(default_model)
            .or(small_fast_model);
        let opus_model = Self::claude_env_string(env, "ANTHROPIC_DEFAULT_OPUS_MODEL")
            .or(default_model)
            .or(small_fast_model);
        // Fable 未配置时不写稳定别名；映射侧会 fable→opus 降级（与官方一致）。
        let fable_model = Self::claude_env_string(env, "ANTHROPIC_DEFAULT_FABLE_MODEL");

        let subagent_model = Self::claude_env_string(env, "CLAUDE_CODE_SUBAGENT_MODEL");

        let mut fields = Vec::with_capacity(9);
        Self::push_claude_takeover_role_fields(
            &mut fields,
            env,
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            CLAUDE_TAKEOVER_HAIKU_MODEL,
            false,
            haiku_model,
        );
        Self::push_claude_takeover_role_fields(
            &mut fields,
            env,
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            CLAUDE_TAKEOVER_SONNET_MODEL,
            true,
            sonnet_model,
        );
        Self::push_claude_takeover_role_fields(
            &mut fields,
            env,
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
            CLAUDE_TAKEOVER_OPUS_MODEL,
            true,
            opus_model,
        );
        Self::push_claude_takeover_role_fields(
            &mut fields,
            env,
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
            CLAUDE_TAKEOVER_FABLE_MODEL,
            true,
            fable_model,
        );
        if let Some(subagent_model) = subagent_model {
            fields.push(("CLAUDE_CODE_SUBAGENT_MODEL", subagent_model.to_string()));
        }
        fields
    }

    fn push_claude_takeover_role_fields(
        fields: &mut Vec<(&'static str, String)>,
        env: &Map<String, Value>,
        model_key: &'static str,
        name_key: &'static str,
        takeover_model: &'static str,
        supports_one_m: bool,
        upstream_model: Option<&str>,
    ) {
        let Some(upstream_model) = upstream_model else {
            return;
        };

        let mut client_model = takeover_model.to_string();
        if supports_one_m && Self::has_claude_one_m_marker(upstream_model) {
            client_model.push_str(CLAUDE_ONE_M_MARKER_FOR_CLIENT);
        }
        fields.push((model_key, client_model));

        let display_name = Self::claude_env_string(env, name_key)
            .map(str::to_string)
            .unwrap_or_else(|| Self::strip_claude_one_m_marker(upstream_model));
        if !display_name.is_empty() {
            fields.push((name_key, display_name));
        }
    }

    fn claude_env_string<'a>(env: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
        env.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    fn has_claude_one_m_marker(model: &str) -> bool {
        model
            .trim_end()
            .to_ascii_lowercase()
            .ends_with(crate::claude_desktop_config::ONE_M_CONTEXT_MARKER)
    }

    fn strip_claude_one_m_marker(model: &str) -> String {
        crate::proxy::model_mapper::strip_one_m_suffix_for_upstream(model)
            .trim()
            .to_string()
    }

    fn claude_provider_with_effective_settings(
        &self,
        provider: &Provider,
    ) -> Result<Provider, String> {
        let mut effective_provider = provider.clone();
        effective_provider.settings_config = build_effective_settings_with_common_config(
            self.db.as_ref(),
            &AppType::Claude,
            provider,
        )
        .map_err(|e| format!("构建 claude 有效配置失败: {e}"))?;
        Ok(effective_provider)
    }

    pub async fn sync_claude_live_from_provider_while_proxy_active(
        &self,
        provider: &Provider,
    ) -> Result<(), String> {
        let effective_provider = self.claude_provider_with_effective_settings(provider)?;
        let mut effective_settings = effective_provider.settings_config.clone();
        let (proxy_url, _) = self.build_proxy_urls().await?;

        Self::apply_claude_takeover_fields_for_provider(
            &mut effective_settings,
            &proxy_url,
            &effective_provider,
        );
        self.write_claude_live(&effective_settings)?;
        Ok(())
    }

    pub async fn sync_codex_live_from_provider_while_proxy_active(
        &self,
        provider: &Provider,
    ) -> Result<(), String> {
        let existing_live = self.read_codex_live().ok();
        let mut effective_settings = build_effective_settings_with_common_config(
            self.db.as_ref(),
            &AppType::Codex,
            provider,
        )
        .map_err(|e| format!("构建 codex 有效配置失败: {e}"))?;
        if let Some(existing_live) = existing_live.as_ref() {
            Self::preserve_codex_mcp_servers_from_existing_config(
                &mut effective_settings,
                existing_live,
            )?;
        }
        let (_, proxy_codex_base_url) = self.build_proxy_urls().await?;

        if let Some(auth) = effective_settings
            .get_mut("auth")
            .and_then(|v| v.as_object_mut())
        {
            auth.insert("OPENAI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
        } else if let Some(root) = effective_settings.as_object_mut() {
            root.insert(
                "auth".to_string(),
                json!({ "OPENAI_API_KEY": PROXY_TOKEN_PLACEHOLDER }),
            );
        }

        let config_str = effective_settings
            .get("config")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let updated_config = Self::apply_codex_proxy_toml_config_for_provider(
            config_str,
            &proxy_codex_base_url,
            Some(provider),
        );
        effective_settings["config"] = json!(updated_config);
        Self::attach_codex_model_catalog_from_provider(&mut effective_settings, Some(provider));

        self.write_codex_takeover_live_for_provider(&effective_settings, Some(provider))?;
        Ok(())
    }

    fn get_current_provider_for_app(&self, app_type: &AppType) -> Result<Option<Provider>, String> {
        let Some(current_id) = crate::settings::get_effective_current_provider(&self.db, app_type)
            .map_err(|e| format!("获取 {app_type:?} 当前供应商失败: {e}"))?
        else {
            return Ok(None);
        };

        self.db
            .get_provider_by_id(&current_id, app_type.as_str())
            .map_err(|e| format!("读取 {app_type:?} 当前供应商失败: {e}"))
    }

    fn require_current_provider_for_app(&self, app_type: &AppType) -> Result<Provider, String> {
        self.get_current_provider_for_app(app_type)?
            .ok_or_else(|| format!("{app_type:?} 当前供应商不存在，无法接管 Live 配置"))
    }

    /// 复用 C2a 三模块 registry，并把 C2b 四模块 adapter 接到同一 C1 snapshot 契约。
    ///
    /// 两侧 adapter 仍各自拥有模块目标实现；这里只做唯一 dispatcher 的组合，不复制
    /// manifest/capture/restore 语义。
    pub(crate) fn snapshot_adapter_for_app(
        app_type: &AppType,
    ) -> Result<Option<Box<dyn SnapshotModuleAdapter>>, String> {
        if let Some(adapter) = c2a_snapshot_adapter_for(app_type) {
            return Ok(Some(adapter));
        }
        c2b_snapshot_adapter_for(app_type)
    }

    fn is_c2b_takeover_app(app_type: &AppType) -> bool {
        matches!(
            app_type,
            AppType::ClaudeDesktop | AppType::OpenCode | AppType::OpenClaw | AppType::Hermes
        )
    }

    /// C2b provider 热切换失败时恢复 DB 与本机 settings 中原先的 current 指针。
    ///
    /// `Database::set_current_provider` 会先清空本模块的 `is_current`；传入空字符串时
    /// 第二条 UPDATE 不命中任何合法 provider，等价于精确恢复“原先无 DB current”。
    fn restore_c2b_current_provider_pointers(
        &self,
        app_type: &AppType,
        previous_db_current: Option<&str>,
        previous_settings_current: Option<&str>,
    ) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Err(error) = self
            .db
            .set_current_provider(app_type.as_str(), previous_db_current.unwrap_or_default())
        {
            errors.push(format!("恢复 DB current 失败: {error}"));
        }
        if let Err(error) =
            crate::settings::set_current_provider(app_type, previous_settings_current)
        {
            errors.push(format!("恢复本机 current 失败: {error}"));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }

    fn rollback_c2b_current_provider_pointers(
        &self,
        app_type: &AppType,
        previous_db_current: Option<&str>,
        previous_settings_current: Option<&str>,
        operation_error: &str,
    ) -> String {
        match self.restore_c2b_current_provider_pointers(
            app_type,
            previous_db_current,
            previous_settings_current,
        ) {
            Ok(()) => operation_error.to_string(),
            Err(rollback_error) => {
                format!("{operation_error}；current 指针回滚失败: {rollback_error}")
            }
        }
    }

    /// C2b proxy 热切换在 current 指针提交后才可按新 provider 写 namespaced live。
    /// 任一后续步骤失败时，必须同时恢复切换前的完整 live 字节和两处 current 指针，
    /// 否则调用方会收到失败，但磁盘或 provider 选择已部分生效。
    fn rollback_c2b_hot_switch(
        &self,
        app_type: &AppType,
        previous_db_current: Option<&str>,
        previous_settings_current: Option<&str>,
        adapter: &dyn SnapshotModuleAdapter,
        live_before: &SnapshotManifest,
        operation_error: &str,
    ) -> String {
        let mut rollback_errors = Vec::new();
        if let Err(error) = adapter.restore_manifest_transactional(live_before) {
            rollback_errors.push(format!("Live 回滚失败: {error}"));
        }
        if let Err(error) = self.restore_c2b_current_provider_pointers(
            app_type,
            previous_db_current,
            previous_settings_current,
        ) {
            rollback_errors.push(format!("current 指针回滚失败: {error}"));
        }
        if rollback_errors.is_empty() {
            operation_error.to_string()
        } else {
            format!("{operation_error}；{}", rollback_errors.join("；"))
        }
    }

    /// C2b 模块的 proxy 能力门。调用点必须位于任何 snapshot/live/route DB 副作用之前。
    fn validate_proxy_capability_for_app(&self, app_type: &AppType) -> Result<(), String> {
        if !Self::is_c2b_takeover_app(app_type) {
            return Ok(());
        }
        let provider = self.require_current_provider_for_app(app_type)?;
        crate::proxy::providers::validate_module_proxy_capability(app_type, &provider).map(|_| ())
    }

    fn provider_with_claude_desktop_mode(
        provider: &Provider,
        mode: crate::provider::ClaudeDesktopMode,
    ) -> Provider {
        let mut provider = provider.clone();
        let mut meta = provider.meta.clone().unwrap_or_default();
        meta.claude_desktop_mode = Some(mode);
        provider.meta = Some(meta);
        provider
    }

    fn effective_provider_for_app(&self, app_type: &AppType) -> Result<Provider, String> {
        let mut provider = self.require_current_provider_for_app(app_type)?;
        provider.settings_config =
            build_effective_settings_with_common_config(self.db.as_ref(), app_type, &provider)
                .map_err(|error| format!("构建 {} 有效配置失败: {error}", app_type.as_str()))?;
        Ok(provider)
    }

    /// 设置 AppHandle（在应用初始化时调用）
    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        futures::executor::block_on(async {
            *self.app_handle.write().await = Some(handle);
        });
    }

    pub(crate) async fn lock_switch_for_app(
        &self,
        app_type: &str,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        self.switch_locks.lock_for_app(app_type).await
    }

    /// 启动代理服务器
    pub async fn start(&self) -> Result<ProxyServerInfo, String> {
        // 1. 启动时自动设置 proxy_enabled = true
        let mut global_config = self
            .db
            .get_global_proxy_config()
            .await
            .map_err(|e| format!("获取全局代理配置失败: {e}"))?;

        if !global_config.proxy_enabled {
            global_config.proxy_enabled = true;
            self.db
                .update_global_proxy_config(global_config.clone())
                .await
                .map_err(|e| format!("更新代理总开关失败: {e}"))?;
        }

        // 2. 获取配置
        let config = self
            .db
            .get_proxy_config()
            .await
            .map_err(|e| format!("获取代理配置失败: {e}"))?;

        // 3. 若已在运行：确保持久化状态（如需要）并返回当前信息
        if let Some(server) = self.server.read().await.as_ref() {
            let status = server.get_status().await;
            return Ok(ProxyServerInfo {
                address: status.address,
                port: status.port,
                // 无法精确取回首次启动时间，返回当前时间用于 UI 展示即可
                started_at: chrono::Utc::now().to_rfc3339(),
            });
        }

        // 4. 创建并启动服务器
        let app_handle = self.app_handle.read().await.clone();
        let server = ProxyServer::new(config.clone(), self.db.clone(), app_handle);
        let info = server
            .start()
            .await
            .map_err(|e| format!("启动代理服务器失败: {e}"))?;
        if let Err(e) = self
            .persist_ephemeral_listen_port_if_needed(&config, info.port)
            .await
        {
            let _ = server.stop().await;
            return Err(e);
        }

        // 5. 保存服务器实例
        *self.server.write().await = Some(server);

        log::info!("代理服务器已启动: {}:{}", info.address, info.port);
        Ok(info)
    }

    async fn persist_ephemeral_listen_port_if_needed(
        &self,
        config: &ProxyConfig,
        actual_port: u16,
    ) -> Result<(), String> {
        if config.listen_port != 0 {
            return Ok(());
        }

        let mut resolved_config = config.clone();
        resolved_config.listen_port = actual_port;
        self.db
            .update_proxy_config(resolved_config)
            .await
            .map_err(|e| format!("保存动态代理端口失败: {e}"))
    }

    /// 旧版“启动并批量接管三模块”兼容入口。
    ///
    /// C1/C2a 起网关运行态与模块接管正交：此入口只启动网关，绝不读取、备份或写入
    /// 任一模块 Live。调用方必须通过 `set_takeover_for_app` 显式选择模块与 route_mode。
    #[deprecated(note = "请使用 start + set_takeover_for_app 显式控制模块接管")]
    pub async fn start_with_takeover(&self) -> Result<ProxyServerInfo, String> {
        self.start().await
    }

    /// 获取七模块接管状态。
    pub async fn get_takeover_status(&self) -> Result<ProxyTakeoverStatus, String> {
        let mut status = ProxyTakeoverStatus::default();
        for app in AppType::all() {
            let config = self
                .db
                .get_proxy_config_for_app(app.as_str())
                .await
                .map_err(|e| format!("获取 {} 接管状态失败: {e}", app.as_str()))?;
            status.set_for_app(
                &app,
                ProxyModuleTakeoverStatus {
                    takeover_enabled: config.takeover_enabled,
                    route_mode: config.route_mode,
                },
            );
        }
        Ok(status)
    }

    /// 查询指定模块当前的 Live 写入判定（真相来源 = `proxy_config`）。
    ///
    /// provider 保存/切换/同步路径用它取代旧的"有备份即接管"信号。C2a 拥有此公共入口，
    /// C2b 直接复用。
    pub async fn live_write_decision_for_app(
        &self,
        app_type: &AppType,
    ) -> Result<LiveWriteDecision, String> {
        let config = self
            .db
            .get_proxy_config_for_app(app_type.as_str())
            .await
            .map_err(|e| format!("获取 {} 接管状态失败: {e}", app_type.as_str()))?;
        Ok(LiveWriteDecision::from_config(&config))
    }

    /// 同步版本（profile apply / provider service 在 tokio runtime 外调用）。
    pub fn live_write_decision_for_app_sync(
        &self,
        app_type: &AppType,
    ) -> Result<LiveWriteDecision, String> {
        futures::executor::block_on(self.live_write_decision_for_app(app_type))
    }

    /// 返回会阻止用户停止网关的 proxy 模块列表。
    pub async fn proxy_route_takeovers(&self) -> Result<Vec<String>, String> {
        self.db
            .list_proxy_route_takeovers()
            .await
            .map_err(|e| format!("查询代理路由接管状态失败: {e}"))
    }

    /// 为指定应用开启/关闭 Live 接管（三维语义）。
    ///
    /// - `enabled=true`：按 `route_mode` 写入。`direct` 写真实上游（不依赖网关，不启动
    ///   网关）；`proxy` 确保网关运行并写本地网关入口。首次开启前用 `capture_snapshot_once`
    ///   捕获原始文件字节（幂等，不覆盖已有快照）。
    /// - `enabled=false`：忽略 `route_mode`，走 `restore_snapshot_and_release` 精确恢复首次
    ///   开启前状态后原子清所有权与快照。关闭或切 direct 都不自动停网关。
    pub async fn set_takeover_for_app(
        &self,
        app_type: &str,
        enabled: bool,
        route_mode: RouteMode,
    ) -> Result<(), String> {
        let app = AppType::from_str(app_type).map_err(|e| format!("无效的应用类型: {e}"))?;
        let app_type_str = app.as_str();
        let _guard = self.switch_locks.lock_for_app(app_type_str).await;

        if enabled {
            self.enable_takeover_locked(&app, route_mode).await
        } else {
            self.disable_takeover_locked(&app).await
        }
    }

    /// 已持有 per-app 切换锁时开启接管。
    async fn enable_takeover_locked(
        &self,
        app: &AppType,
        route_mode: RouteMode,
    ) -> Result<(), String> {
        let app_type_str = app.as_str();
        let current_config = self
            .db
            .get_proxy_config_for_app(app_type_str)
            .await
            .map_err(|e| format!("获取 {app_type_str} 配置失败: {e}"))?;

        // C2b proxy 能力门必须早于 snapshot capture、网关启动、live 写入和状态提交。
        // direct 完全绕过，保持能力矩阵外 provider 的直连可用性。
        if route_mode == RouteMode::Proxy {
            self.validate_proxy_capability_for_app(app)?;
        }

        // 已接管时不重新 capture：同模式只重建受管 live，不同模式走原子 mode switch。
        if current_config.takeover_enabled {
            return if current_config.route_mode == route_mode {
                self.reapply_current_route_locked(app, route_mode).await
            } else {
                self.switch_route_mode_locked(app, current_config, route_mode)
                    .await
            };
        }

        let adapter = Self::snapshot_adapter_for_app(app)?
            .ok_or_else(|| format!("{app_type_str} 尚无精确快照 adapter"))?;

        // 首次快照必须先于任何网关/live 副作用；已有有效快照幂等保留。
        let capture_outcome = capture_snapshot_once(&self.db, adapter.as_ref()).await?;
        let is_fresh = capture_outcome == CaptureSnapshotOutcome::Captured;

        let write_result = async {
            if route_mode == RouteMode::Proxy {
                if is_fresh
                    && Self::primary_live_exists_for_app(app)
                    && matches!(app, AppType::Claude | AppType::Codex | AppType::Gemini)
                {
                    // 三旧模块沿用 C2a 的首次接管前 token 同步；四新模块以各自
                    // provider SSOT 构建受管 fragment，不能把整个 additive live 反灌 DB。
                    self.sync_live_to_provider(app).await?;
                } else if !is_fresh {
                    // 失败残留/旧快照重试：先恢复唯一首次快照，再从干净态重建 proxy live。
                    self.restore_first_open_snapshot(app).await?;
                }
            }
            self.apply_route_mode_locked(app, route_mode).await
        }
        .await;

        if let Err(error) = write_result {
            return Err(self.rollback_failed_enable(app, route_mode, &error).await);
        }

        // Live 写入成功后才提交 takeover_enabled + route_mode。提交失败同样必须回滚，
        // 否则会出现 DB=off 但 live 已由 AGS 改写的 hands-off 违约。
        let mut updated_config = current_config;
        updated_config.takeover_enabled = true;
        updated_config.route_mode = route_mode;
        if let Err(error) = self.db.update_proxy_config_for_app(updated_config).await {
            return Err(self
                .rollback_failed_enable(
                    app,
                    route_mode,
                    &format!("设置 {app_type_str} 接管状态失败: {error}"),
                )
                .await);
        }

        // 兼容旧逻辑：写入 any-of 标志（失败不影响功能）。
        let _ = self.db.set_live_takeover_active(true).await;

        // 只有 proxy 路由访问官方 API 才需要封号风险告警；direct 是正常直连。
        if route_mode == RouteMode::Proxy {
            if let Ok(Some(current_id)) =
                crate::settings::get_effective_current_provider(&self.db, app)
            {
                if let Ok(Some(provider)) = self.db.get_provider_by_id(&current_id, app_type_str) {
                    if provider.category.as_deref() == Some("official") {
                        if let Some(handle) = self.app_handle.read().await.as_ref() {
                            let _ = handle.emit(
                                "proxy-official-warning",
                                serde_json::json!({
                                    "appType": app_type_str,
                                    "providerName": provider.name,
                                }),
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn apply_route_mode_locked(
        &self,
        app: &AppType,
        route_mode: RouteMode,
    ) -> Result<(), String> {
        match route_mode {
            RouteMode::Direct => self.write_direct_upstream_for_app(app).await,
            RouteMode::Proxy => {
                if !self.is_running().await {
                    self.start().await?;
                }
                self.takeover_live_config_strict(app).await
            }
        }
    }

    fn capture_transient_live_snapshot(
        &self,
        app: &AppType,
        adapter: &dyn crate::proxy::snapshot::SnapshotModuleAdapter,
    ) -> Result<SnapshotManifest, String> {
        SnapshotManifest::new(app, adapter.capture_targets()?)
    }

    async fn mark_takeover_recovery_required(&self, app: &AppType) -> Result<(), String> {
        let mut config = self
            .db
            .get_proxy_config_for_app(app.as_str())
            .await
            .map_err(|e| format!("读取恢复标记状态失败: {e}"))?;
        config.takeover_enabled = true;
        // 回滚失败时强制使用 proxy 恢复语义，确保 crash recovery 会恢复 immutable
        // snapshot，而不是按 direct 残留放弃所有权并留下半写 live。
        config.route_mode = RouteMode::Proxy;
        self.db
            .update_proxy_config_for_app(config)
            .await
            .map_err(|e| format!("保留恢复所需接管状态失败: {e}"))
    }

    async fn rollback_failed_enable(
        &self,
        app: &AppType,
        route_mode: RouteMode,
        operation_error: &str,
    ) -> String {
        log::error!(
            "{} {:?} 接管失败，尝试恢复首次快照: {operation_error}",
            app.as_str(),
            route_mode
        );
        match self.restore_first_open_snapshot(app).await {
            Ok(()) => match self.db.release_takeover_ownership(app.as_str()).await {
                Ok(()) => operation_error.to_string(),
                Err(cleanup_error) => format!(
                    "{operation_error}；Live 已恢复，但清理接管所有权/快照失败: {cleanup_error}"
                ),
            },
            Err(restore_error) => {
                let mark_error = self.mark_takeover_recovery_required(app).await.err();
                match mark_error {
                    Some(mark_error) => format!(
                        "{operation_error}；恢复首次快照失败: {restore_error}；{mark_error}"
                    ),
                    None => format!(
                        "{operation_error}；恢复首次快照失败: {restore_error}；已保留接管状态与快照供重试"
                    ),
                }
            }
        }
    }

    async fn rollback_transient_route_change(
        &self,
        app: &AppType,
        adapter: &dyn crate::proxy::snapshot::SnapshotModuleAdapter,
        before: &SnapshotManifest,
        operation_error: &str,
    ) -> String {
        match adapter.restore_manifest_transactional(before) {
            Ok(()) => operation_error.to_string(),
            Err(restore_error) => {
                let mark_error = self.mark_takeover_recovery_required(app).await.err();
                match mark_error {
                    Some(mark_error) => format!(
                        "{operation_error}；回滚模式切换前 Live 失败: {restore_error}；{mark_error}"
                    ),
                    None => format!(
                        "{operation_error}；回滚模式切换前 Live 失败: {restore_error}；已切为 proxy 恢复标记并保留首次快照"
                    ),
                }
            }
        }
    }

    async fn reapply_current_route_locked(
        &self,
        app: &AppType,
        route_mode: RouteMode,
    ) -> Result<(), String> {
        let adapter = Self::snapshot_adapter_for_app(app)
            .map_err(|error| format!("解析 {} 快照 adapter 失败: {error}", app.as_str()))?
            .ok_or_else(|| format!("{} 尚无精确快照 adapter", app.as_str()))?;
        let before = self.capture_transient_live_snapshot(app, adapter.as_ref())?;
        let result = async {
            if route_mode == RouteMode::Proxy {
                self.restore_first_open_snapshot(app).await?;
            }
            self.apply_route_mode_locked(app, route_mode).await
        }
        .await;
        match result {
            Ok(()) => Ok(()),
            Err(error) => Err(self
                .rollback_transient_route_change(app, adapter.as_ref(), &before, &error)
                .await),
        }
    }

    async fn switch_route_mode_locked(
        &self,
        app: &AppType,
        current_config: AppProxyConfig,
        route_mode: RouteMode,
    ) -> Result<(), String> {
        let adapter = Self::snapshot_adapter_for_app(app)
            .map_err(|error| format!("解析 {} 快照 adapter 失败: {error}", app.as_str()))?
            .ok_or_else(|| format!("{} 尚无精确快照 adapter", app.as_str()))?;
        let before = self.capture_transient_live_snapshot(app, adapter.as_ref())?;

        if let Err(error) = self.apply_route_mode_locked(app, route_mode).await {
            return Err(self
                .rollback_transient_route_change(app, adapter.as_ref(), &before, &error)
                .await);
        }

        let mut updated_config = current_config;
        updated_config.route_mode = route_mode;
        if let Err(error) = self.db.update_proxy_config_for_app(updated_config).await {
            let error = format!("设置 {} 路由模式失败: {error}", app.as_str());
            return Err(self
                .rollback_transient_route_change(app, adapter.as_ref(), &before, &error)
                .await);
        }
        Ok(())
    }

    /// 已持有 per-app 切换锁时关闭接管：精确恢复首次开启前状态后原子清所有权。
    async fn disable_takeover_locked(&self, app: &AppType) -> Result<(), String> {
        let app_type_str = app.as_str();

        let current_config = self
            .db
            .get_proxy_config_for_app(app_type_str)
            .await
            .map_err(|e| format!("获取 {app_type_str} 配置失败: {e}"))?;

        if !current_config.takeover_enabled {
            return Ok(()); // 未接管，幂等返回
        }

        // direct 与 proxy 都走精确恢复（R10：回滚到首次开启前）。
        self.restore_snapshot_and_release_for_app(app).await?;

        self.db
            .clear_provider_health_for_app(app_type_str)
            .await
            .map_err(|e| format!("清除 {app_type_str} 健康状态失败: {e}"))?;

        // 关闭接管或切到 direct 不自动停止网关。
        let _ = self.db.set_live_takeover_active(false).await;
        Ok(())
    }

    /// Whether the module has any pre-existing primary live target.
    ///
    /// Used during first proxy enable: if the target was absent (captured as
    /// `existed=false`), there is no live token to sync into DB; takeover writes
    /// from the current provider and creates the target. If it existed, parsing
    /// remains strict so malformed user config is not silently discarded.
    fn primary_live_exists_for_app(app: &AppType) -> bool {
        match app {
            AppType::Claude => get_claude_settings_path().exists(),
            AppType::Codex => {
                crate::codex_config::get_codex_auth_path().exists()
                    || crate::codex_config::get_codex_config_path().exists()
            }
            AppType::Gemini => crate::gemini_config::get_gemini_env_path().exists(),
            AppType::ClaudeDesktop => crate::claude_desktop_config::snapshot_target_paths()
                .map(|paths| paths.into_iter().any(|(_, path)| path.exists()))
                .unwrap_or(false),
            AppType::OpenCode => crate::opencode_config::get_opencode_config_path().exists(),
            AppType::OpenClaw => crate::openclaw_config::get_openclaw_config_path().exists(),
            AppType::Hermes => crate::hermes_config::get_hermes_config_path().exists(),
        }
    }

    /// 写入某模块的真实上游 Live 配置（direct 模式）。复用 provider live 写入逻辑。
    async fn write_direct_upstream_for_app(&self, app: &AppType) -> Result<(), String> {
        let provider = self.require_current_provider_for_app(app)?;
        let provider = if matches!(app, AppType::ClaudeDesktop) {
            Self::provider_with_claude_desktop_mode(
                &provider,
                crate::provider::ClaudeDesktopMode::Direct,
            )
        } else {
            provider
        };
        write_live_with_common_config(self.db.as_ref(), app, &provider)
            .map_err(|e| format!("写入 {} 真实上游配置失败: {e}", app.as_str()))
    }

    /// 从首次快照恢复 Live，但**不**清所有权/删快照（重建、失败回滚用）。
    async fn restore_first_open_snapshot(&self, app: &AppType) -> Result<(), String> {
        let app_type_str = app.as_str();
        let adapter = Self::snapshot_adapter_for_app(app)
            .map_err(|error| format!("解析 {app_type_str} 快照 adapter 失败: {error}"))?
            .ok_or_else(|| format!("{app_type_str} 尚无精确快照 adapter"))?;
        let Some(backup) = self
            .db
            .get_live_backup(app_type_str)
            .await
            .map_err(|e| format!("读取 {app_type_str} 快照失败: {e}"))?
        else {
            return Ok(()); // 无快照可恢复
        };
        match crate::proxy::snapshot::decode_stored_snapshot(app_type_str, &backup.original_config)?
        {
            crate::proxy::snapshot::DecodedSnapshot::Manifest(manifest) => {
                adapter.restore_manifest_transactional(&manifest)
            }
            crate::proxy::snapshot::DecodedSnapshot::Legacy(legacy) => {
                adapter.restore_legacy(&legacy)
            }
        }
    }

    /// 精确恢复首次快照后原子清所有权与快照（关闭接管用）。
    /// 无快照时只清所有权，避免卡在 enabled=true 的不一致态。
    async fn restore_snapshot_and_release_for_app(&self, app: &AppType) -> Result<(), String> {
        let app_type_str = app.as_str();
        let has_backup = self
            .db
            .get_live_backup(app_type_str)
            .await
            .map_err(|e| format!("读取 {app_type_str} 快照失败: {e}"))?
            .is_some();

        if has_backup {
            let adapter = Self::snapshot_adapter_for_app(app)
                .map_err(|error| format!("解析 {app_type_str} 快照 adapter 失败: {error}"))?
                .ok_or_else(|| format!("{app_type_str} 尚无精确快照 adapter"))?;
            restore_snapshot_and_release(&self.db, adapter.as_ref()).await
        } else {
            abandon_snapshot_ownership(&self.db, app)
                .await
                .map_err(|e| format!("清理 {app_type_str} 接管所有权失败: {e}"))
        }
    }

    /// 在 direct↔proxy 间切换某模块的路由模式（仅 `takeover_enabled=true` 时生效）。
    ///
    /// **不重新 capture 快照**（首次快照 immutable，AC4）：只把 Live 从当前模式重写为
    /// 目标模式。切 proxy 前确保网关运行、写 proxy 入口；切 direct 写真实上游。只改当前
    /// 模块（R5）。目标模式与当前一致时幂等返回。
    pub async fn switch_route_mode(
        &self,
        app_type: &str,
        route_mode: RouteMode,
    ) -> Result<(), String> {
        let app = AppType::from_str(app_type).map_err(|e| format!("无效的应用类型: {e}"))?;
        let app_type_str = app.as_str();
        let _guard = self.switch_locks.lock_for_app(app_type_str).await;

        let current_config = self
            .db
            .get_proxy_config_for_app(app_type_str)
            .await
            .map_err(|e| format!("获取 {app_type_str} 配置失败: {e}"))?;

        if !current_config.takeover_enabled {
            return Err(format!("{app_type_str} 未接管，无法切换路由模式"));
        }
        if current_config.route_mode == route_mode {
            return Ok(()); // 模式未变，幂等返回
        }
        if route_mode == RouteMode::Proxy {
            self.validate_proxy_capability_for_app(&app)?;
        }

        self.switch_route_mode_locked(&app, current_config, route_mode)
            .await
    }

    /// Synchronously disable one app's takeover without stopping the proxy process.
    /// Profile apply runs outside a Tokio runtime and needs the live config restored
    /// before it can switch the underlying provider.
    pub fn disable_takeover_for_app_sync(&self, app_type: &AppType) -> Result<(), String> {
        let app_type_str = app_type.as_str();
        let _guard = futures::executor::block_on(self.switch_locks.lock_for_app(app_type_str));

        let config = futures::executor::block_on(self.db.get_proxy_config_for_app(app_type_str))
            .map_err(|e| format!("获取 {app_type_str} 配置失败: {e}"))?;

        if !config.takeover_enabled {
            return Ok(());
        }

        // direct 与 proxy 都走精确恢复（R10）。
        futures::executor::block_on(self.restore_snapshot_and_release_for_app(app_type))?;

        futures::executor::block_on(self.db.clear_provider_health_for_app(app_type_str))
            .map_err(|e| format!("清除 {app_type_str} 健康状态失败: {e}"))?;
        let _ = futures::executor::block_on(self.db.set_live_takeover_active(false));

        Ok(())
    }

    /// 同步 Live 配置中的 Token 到数据库
    ///
    /// 在清空 Live Token 之前调用，确保数据库中的 Provider 配置有最新的 Token。
    /// 这样代理才能从数据库读取到正确的认证信息。
    async fn sync_live_to_provider(&self, app_type: &AppType) -> Result<(), String> {
        let live_config = match app_type {
            AppType::Claude => self.read_claude_live()?,
            AppType::Codex => self.read_codex_live()?,
            AppType::Gemini => self.read_gemini_live()?,
            _ => return Err("该应用不支持代理功能".to_string()),
        };

        self.sync_live_config_to_provider(app_type, &live_config)
            .await
    }

    async fn sync_live_config_to_provider(
        &self,
        app_type: &AppType,
        live_config: &Value,
    ) -> Result<(), String> {
        match app_type {
            AppType::Claude => {
                let provider_id =
                    crate::settings::get_effective_current_provider(&self.db, &AppType::Claude)
                        .map_err(|e| format!("获取 Claude 当前供应商失败: {e}"))?;

                if let Some(provider_id) = provider_id {
                    if let Ok(Some(mut provider)) =
                        self.db.get_provider_by_id(&provider_id, "claude")
                    {
                        if let Some(env) = live_config.get("env").and_then(|v| v.as_object()) {
                            let token_pair = [
                                "ANTHROPIC_AUTH_TOKEN",
                                "ANTHROPIC_API_KEY",
                                "OPENROUTER_API_KEY",
                                "OPENAI_API_KEY",
                            ]
                            .into_iter()
                            .find_map(|key| {
                                env.get(key)
                                    .and_then(|v| v.as_str())
                                    .map(|s| (key, s.trim()))
                            })
                            .filter(|(_, token)| {
                                !token.is_empty() && *token != PROXY_TOKEN_PLACEHOLDER
                            });

                            if let Some((token_key, token)) = token_pair {
                                let env_obj = provider
                                    .settings_config
                                    .get_mut("env")
                                    .and_then(|v| v.as_object_mut());

                                match env_obj {
                                    Some(obj) => {
                                        if token_key == "ANTHROPIC_AUTH_TOKEN"
                                            || token_key == "ANTHROPIC_API_KEY"
                                        {
                                            let mut updated = false;
                                            if obj.contains_key("ANTHROPIC_AUTH_TOKEN") {
                                                obj.insert(
                                                    "ANTHROPIC_AUTH_TOKEN".to_string(),
                                                    json!(token),
                                                );
                                                updated = true;
                                            }
                                            if obj.contains_key("ANTHROPIC_API_KEY") {
                                                obj.insert(
                                                    "ANTHROPIC_API_KEY".to_string(),
                                                    json!(token),
                                                );
                                                updated = true;
                                            }
                                            if !updated {
                                                obj.insert(token_key.to_string(), json!(token));
                                            }
                                        } else {
                                            obj.insert(token_key.to_string(), json!(token));
                                        }
                                    }
                                    None => {
                                        // 至少写入一份可用的 Token
                                        if provider.settings_config.is_null() {
                                            provider.settings_config = json!({});
                                        }

                                        if let Some(root) = provider.settings_config.as_object_mut()
                                        {
                                            root.insert(
                                                "env".to_string(),
                                                json!({ token_key: token }),
                                            );
                                        } else {
                                            log::warn!(
                                                "Claude provider settings_config 格式异常（非对象），跳过写入 Token (provider: {provider_id})"
                                            );
                                        }
                                    }
                                }

                                if let Err(e) = self.db.update_provider_settings_config(
                                    "claude",
                                    &provider_id,
                                    &provider.settings_config,
                                ) {
                                    log::warn!("同步 Claude Token 到数据库失败: {e}");
                                } else {
                                    log::info!(
                                        "已同步 Claude Token 到数据库 (provider: {provider_id})"
                                    );
                                }
                            }
                        }
                    }
                }
            }
            AppType::Codex => {
                let provider_id =
                    crate::settings::get_effective_current_provider(&self.db, &AppType::Codex)
                        .map_err(|e| format!("获取 Codex 当前供应商失败: {e}"))?;

                if let Some(provider_id) = provider_id {
                    if let Ok(Some(mut provider)) =
                        self.db.get_provider_by_id(&provider_id, "codex")
                    {
                        if let Some(token) = live_config
                            .get("auth")
                            .and_then(|v| v.get("OPENAI_API_KEY"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty() && *s != PROXY_TOKEN_PLACEHOLDER)
                        {
                            if let Some(auth_obj) = provider
                                .settings_config
                                .get_mut("auth")
                                .and_then(|v| v.as_object_mut())
                            {
                                auth_obj.insert("OPENAI_API_KEY".to_string(), json!(token));
                            } else {
                                if provider.settings_config.is_null() {
                                    provider.settings_config = json!({});
                                }

                                if let Some(root) = provider.settings_config.as_object_mut() {
                                    root.insert(
                                        "auth".to_string(),
                                        json!({ "OPENAI_API_KEY": token }),
                                    );
                                } else {
                                    log::warn!(
                                        "Codex provider settings_config 格式异常（非对象），跳过写入 Token (provider: {provider_id})"
                                    );
                                }
                            }

                            if let Err(e) = self.db.update_provider_settings_config(
                                "codex",
                                &provider_id,
                                &provider.settings_config,
                            ) {
                                log::warn!("同步 Codex Token 到数据库失败: {e}");
                            } else {
                                log::info!("已同步 Codex Token 到数据库 (provider: {provider_id})");
                            }
                        }
                    }
                }
            }
            AppType::Gemini => {
                let provider_id =
                    crate::settings::get_effective_current_provider(&self.db, &AppType::Gemini)
                        .map_err(|e| format!("获取 Gemini 当前供应商失败: {e}"))?;

                if let Some(provider_id) = provider_id {
                    if let Ok(Some(mut provider)) =
                        self.db.get_provider_by_id(&provider_id, "gemini")
                    {
                        if let Some(token) = live_config
                            .get("env")
                            .and_then(|v| v.get("GEMINI_API_KEY"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty() && *s != PROXY_TOKEN_PLACEHOLDER)
                        {
                            if let Some(env_obj) = provider
                                .settings_config
                                .get_mut("env")
                                .and_then(|v| v.as_object_mut())
                            {
                                env_obj.insert("GEMINI_API_KEY".to_string(), json!(token));
                            } else {
                                if provider.settings_config.is_null() {
                                    provider.settings_config = json!({});
                                }

                                if let Some(root) = provider.settings_config.as_object_mut() {
                                    root.insert(
                                        "env".to_string(),
                                        json!({ "GEMINI_API_KEY": token }),
                                    );
                                } else {
                                    log::warn!(
                                        "Gemini provider settings_config 格式异常（非对象），跳过写入 Token (provider: {provider_id})"
                                    );
                                }
                            }

                            if let Err(e) = self.db.update_provider_settings_config(
                                "gemini",
                                &provider_id,
                                &provider.settings_config,
                            ) {
                                log::warn!("同步 Gemini Token 到数据库失败: {e}");
                            } else {
                                log::info!(
                                    "已同步 Gemini Token 到数据库 (provider: {provider_id})"
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    #[cfg(test)]
    #[allow(dead_code)] // 仅保留旧批量接管回归夹具；生产入口已退役
    async fn sync_live_to_providers(&self) -> Result<(), String> {
        if let Ok(live_config) = self.read_claude_live() {
            self.sync_live_config_to_provider(&AppType::Claude, &live_config)
                .await?;
        }

        if let Ok(live_config) = self.read_codex_live() {
            self.sync_live_config_to_provider(&AppType::Codex, &live_config)
                .await?;
        }

        if let Ok(live_config) = self.read_gemini_live() {
            self.sync_live_config_to_provider(&AppType::Gemini, &live_config)
                .await?;
        }

        log::info!("Live 配置 Token 同步完成");
        Ok(())
    }

    /// 停止代理服务器
    pub async fn stop(&self) -> Result<(), String> {
        if let Some(server) = self.server.write().await.take() {
            server
                .stop()
                .await
                .map_err(|e| format!("停止代理服务器失败: {e}"))?;

            // 停止时设置 proxy_enabled = false
            let mut global_config = self
                .db
                .get_global_proxy_config()
                .await
                .map_err(|e| format!("获取全局代理配置失败: {e}"))?;

            if global_config.proxy_enabled {
                global_config.proxy_enabled = false;
                if let Err(e) = self.db.update_global_proxy_config(global_config).await {
                    log::warn!("更新代理总开关失败: {e}");
                }
            }

            log::info!("代理服务器已停止");
            Ok(())
        } else {
            Err("代理服务器未运行".to_string())
        }
    }

    /// 应用退出/崩溃恢复内部原语：按模块 route_mode 清理所有权后停止网关。
    /// 普通 UI 停止入口不得调用本方法。
    #[allow(dead_code)]
    pub(crate) async fn stop_with_restore(&self) -> Result<(), String> {
        self.cleanup_takeover_and_stop_for_exit().await
    }

    /// 兼容旧调用名；新语义不再保留接管状态或安排下次自动接管。
    pub async fn stop_with_restore_keep_state(&self) -> Result<(), String> {
        self.cleanup_takeover_and_stop_for_exit().await
    }

    async fn cleanup_takeover_and_stop_for_exit(&self) -> Result<(), String> {
        let cleanup_result = self.recover_from_crash().await;
        let stop_result = if self.is_running().await {
            self.stop().await
        } else {
            self.db
                .reset_proxy_runtime_mirror()
                .await
                .map_err(|e| format!("归零代理运行镜像失败: {e}"))
        };
        if let Err(error) = self.db.clear_all_provider_health().await {
            log::warn!("退出时重置健康状态失败: {error}");
        }

        match (cleanup_result, stop_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(cleanup), Ok(())) => Err(cleanup),
            (Ok(()), Err(stop)) => Err(stop),
            (Err(cleanup), Err(stop)) => Err(format!("{cleanup}；{stop}")),
        }
    }

    /// 备份各应用的 Live 配置
    #[cfg(test)]
    async fn backup_live_configs(&self) -> Result<(), String> {
        // Claude
        if let Ok(config) = self.read_claude_live() {
            // 跳过已被代理接管的 Live：避免把代理占位符当作"原始 Live"存进备份槽。
            // 否则下次 start_with_takeover 在异常历史状态下（Live 已是占位符）再次
            // 调用本函数，会用代理配置覆盖一个原本正常的备份；之后 stop 恢复时
            // 即便走到备份路径也会把代理占位符再写回 Live，永久卡在 127.0.0.1:42567。
            if Self::live_has_proxy_placeholder_for_app(&AppType::Claude, &config) {
                log::warn!("claude Live 已被代理接管，不备份（避免把代理配置固化进备份槽）；下次 stop 会从 SSOT 重建 Live");
            } else {
                let json_str = serde_json::to_string(&config)
                    .map_err(|e| format!("序列化 Claude 配置失败: {e}"))?;
                self.db
                    .save_live_backup("claude", &json_str)
                    .await
                    .map_err(|e| format!("备份 Claude 配置失败: {e}"))?;
            }
        }

        // Codex
        if let Ok(config) = self.read_codex_live() {
            if Self::live_has_proxy_placeholder_for_app(&AppType::Codex, &config) {
                log::warn!("codex Live 已被代理接管，不备份（避免把代理配置固化进备份槽）；下次 stop 会从 SSOT 重建 Live");
            } else {
                let json_str = serde_json::to_string(&config)
                    .map_err(|e| format!("序列化 Codex 配置失败: {e}"))?;
                self.db
                    .save_live_backup("codex", &json_str)
                    .await
                    .map_err(|e| format!("备份 Codex 配置失败: {e}"))?;
            }
        }

        // Gemini
        if let Ok(config) = self.read_gemini_live() {
            if Self::live_has_proxy_placeholder_for_app(&AppType::Gemini, &config) {
                log::warn!("gemini Live 已被代理接管，不备份（避免把代理配置固化进备份槽）；下次 stop 会从 SSOT 重建 Live");
            } else {
                let json_str = serde_json::to_string(&config)
                    .map_err(|e| format!("序列化 Gemini 配置失败: {e}"))?;
                self.db
                    .save_live_backup("gemini", &json_str)
                    .await
                    .map_err(|e| format!("备份 Gemini 配置失败: {e}"))?;
            }
        }

        log::info!("已备份所有应用的 Live 配置");
        Ok(())
    }

    /// 旧版严格备份回归夹具。
    ///
    /// 生产 enable 路径只允许 `capture_snapshot_once` + 模块 adapter；此旧 JSON
    /// 备份函数仅编译进测试，防止 C2b 或后续代码误用并覆盖 immutable snapshot。
    #[cfg(test)]
    async fn backup_live_config_strict(&self, app_type: &AppType) -> Result<(), String> {
        if Self::is_c2b_takeover_app(app_type) {
            let adapter = Self::snapshot_adapter_for_app(app_type)?
                .ok_or_else(|| format!("{} 尚无精确快照 adapter", app_type.as_str()))?;
            capture_snapshot_once(&self.db, adapter.as_ref()).await?;
            return Ok(());
        }

        let (app_type_str, config) = match app_type {
            AppType::Claude => ("claude", self.read_claude_live()?),
            AppType::Codex => ("codex", self.read_codex_live()?),
            AppType::Gemini => ("gemini", self.read_gemini_live()?),
            _ => return Err("该应用不支持代理功能".to_string()),
        };

        // 跳过已被代理接管的 Live：避免把代理占位符当作"原始 Live"存进备份槽
        // （见 backup_live_configs 中的注释）。
        if Self::live_has_proxy_placeholder_for_app(app_type, &config) {
            log::warn!(
                "{app_type_str} Live 已被代理接管，不备份（避免把代理配置固化进备份槽）；下次 stop 会从 SSOT 重建 Live"
            );
            return Ok(());
        }

        let json_str = serde_json::to_string(&config)
            .map_err(|e| format!("序列化 {app_type_str} 配置失败: {e}"))?;
        self.db
            .save_live_backup(app_type_str, &json_str)
            .await
            .map_err(|e| format!("备份 {app_type_str} 配置失败: {e}"))?;

        Ok(())
    }

    /// 构造写入 Live 的代理地址（处理 0.0.0.0 / IPv6 等特殊情况）
    async fn build_proxy_urls(&self) -> Result<(String, String), String> {
        let config = self
            .db
            .get_proxy_config()
            .await
            .map_err(|e| format!("获取代理配置失败: {e}"))?;

        // listen_address 可能是 0.0.0.0（用于监听所有网卡），但客户端无法用 0.0.0.0 连接；
        // 因此写回到各应用配置时，优先使用本机回环地址。
        let connect_host = match config.listen_address.as_str() {
            "0.0.0.0" => "127.0.0.1".to_string(),
            "::" => "::1".to_string(),
            _ => config.listen_address.clone(),
        };
        let connect_host_for_url = if connect_host.contains(':') && !connect_host.starts_with('[') {
            format!("[{connect_host}]")
        } else {
            connect_host
        };

        let mut listen_port = config.listen_port;
        if let Some(server) = self.server.read().await.as_ref() {
            let status = server.get_status().await;
            if status.running {
                listen_port = status.port;
            }
        }
        if listen_port == 0 {
            return Err("代理监听端口为 0，但代理服务器尚未运行，无法生成接管地址".to_string());
        }

        let proxy_origin = format!("http://{}:{}", connect_host_for_url, listen_port);
        let proxy_url = proxy_origin.clone();
        let proxy_codex_base_url = format!("{}/v1", proxy_origin.trim_end_matches('/'));

        Ok((proxy_url, proxy_codex_base_url))
    }

    fn namespaced_proxy_base_url(proxy_origin: &str, app_type: &AppType) -> Result<String, String> {
        let suffix = match app_type {
            AppType::ClaudeDesktop => "claude-desktop",
            AppType::OpenCode => "opencode/v1",
            AppType::OpenClaw => "openclaw/v1",
            AppType::Hermes => "hermes/v1",
            _ => return Err(format!("{} 不使用 C2b 独立命名空间", app_type.as_str())),
        };
        Ok(format!("{}/{}", proxy_origin.trim_end_matches('/'), suffix))
    }

    fn gateway_token_for_live(&self) -> Result<String, String> {
        crate::claude_desktop_config::get_or_create_gateway_token(self.db.as_ref())
            .map_err(|error| format!("获取本地网关 token 失败: {error}"))
    }

    fn ensure_json_object<'a>(
        value: &'a mut Value,
        context: &str,
    ) -> Result<&'a mut Map<String, Value>, String> {
        value
            .as_object_mut()
            .ok_or_else(|| format!("{context} 必须是 JSON 对象"))
    }

    fn write_c2b_proxy_live(&self, app_type: &AppType, proxy_origin: &str) -> Result<(), String> {
        let mut provider = self.effective_provider_for_app(app_type)?;
        let proxy_base_url = Self::namespaced_proxy_base_url(proxy_origin, app_type)?;

        match app_type {
            AppType::ClaudeDesktop => {
                provider = Self::provider_with_claude_desktop_mode(
                    &provider,
                    crate::provider::ClaudeDesktopMode::Proxy,
                );
                write_live_with_common_config(self.db.as_ref(), app_type, &provider)
                    .map_err(|error| format!("写入 Claude Desktop proxy profile 失败: {error}"))?;
            }
            AppType::OpenCode => {
                let gateway_token = self.gateway_token_for_live()?;
                let root = Self::ensure_json_object(
                    &mut provider.settings_config,
                    "OpenCode provider settings_config",
                )?;
                let options = root
                    .entry("options".to_string())
                    .or_insert_with(|| Value::Object(Map::new()));
                let options = Self::ensure_json_object(options, "OpenCode provider options")?;
                options.insert("baseURL".to_string(), Value::String(proxy_base_url.clone()));
                options.insert("apiKey".to_string(), Value::String(gateway_token));
                crate::opencode_config::set_provider(
                    &provider.id,
                    provider.settings_config.clone(),
                )
                .map_err(|error| format!("写入 OpenCode proxy 配置失败: {error}"))?;
            }
            AppType::OpenClaw => {
                let gateway_token = self.gateway_token_for_live()?;
                let root = Self::ensure_json_object(
                    &mut provider.settings_config,
                    "OpenClaw provider settings_config",
                )?;
                root.insert("baseUrl".to_string(), Value::String(proxy_base_url.clone()));
                root.insert("apiKey".to_string(), Value::String(gateway_token));
                // `api` 只读不改，客户端继续按 capability gate 已确认的协议调用命名空间。
                crate::openclaw_config::set_provider(
                    &provider.id,
                    provider.settings_config.clone(),
                )
                .map_err(|error| format!("写入 OpenClaw proxy 配置失败: {error}"))?;
            }
            AppType::Hermes => {
                let gateway_token = self.gateway_token_for_live()?;
                let root = Self::ensure_json_object(
                    &mut provider.settings_config,
                    "Hermes provider settings_config",
                )?;
                root.insert(
                    "base_url".to_string(),
                    Value::String(proxy_base_url.clone()),
                );
                root.insert("api_key".to_string(), Value::String(gateway_token));
                // `api_mode` 只读不改，Bedrock/未知值已在 capture 前被原子拒绝。
                crate::hermes_config::set_provider(&provider.id, provider.settings_config.clone())
                    .map_err(|error| format!("写入 Hermes proxy 配置失败: {error}"))?;
            }
            _ => return Err(format!("{} 不是 C2b 接管模块", app_type.as_str())),
        }

        log::info!(
            "{} Live 配置已接管，代理地址: {proxy_base_url}",
            app_type.as_str()
        );
        Ok(())
    }

    /// 接管各应用的 Live 配置（写入代理地址）
    ///
    /// 代理服务器的路由已经根据 API 端点自动区分应用类型：
    /// - `/v1/messages` → Claude
    /// - `/v1/chat/completions`, `/v1/responses` → Codex
    /// - `/v1beta/*` → Gemini
    ///
    /// 因此不需要在 URL 中添加应用前缀。
    #[cfg(test)]
    #[allow(dead_code)] // 旧批量回归夹具的唯一入口；生产代码只允许逐模块接管
    async fn takeover_live_configs(&self) -> Result<(), String> {
        let (proxy_url, proxy_codex_base_url) = self.build_proxy_urls().await?;

        // Claude: 修改 ANTHROPIC_BASE_URL，使用占位符替代真实 Token（代理会注入真实 Token）
        if let Ok(mut live_config) = self.read_claude_live() {
            let claude_provider = self.require_current_provider_for_app(&AppType::Claude)?;
            let claude_provider = self.claude_provider_with_effective_settings(&claude_provider)?;
            Self::apply_claude_takeover_fields_for_provider(
                &mut live_config,
                &proxy_url,
                &claude_provider,
            );
            self.write_claude_live(&live_config)?;
            log::info!("Claude Live 配置已接管，代理地址: {proxy_url}");
        }

        // Codex: 修改 config.toml 的 base_url，auth.json 的 OPENAI_API_KEY（代理会注入真实 Token）
        if let Ok(mut live_config) = self.read_codex_live() {
            // 1. 修改 auth.json 中的 OPENAI_API_KEY（使用占位符）
            if let Some(auth) = live_config.get_mut("auth").and_then(|v| v.as_object_mut()) {
                auth.insert("OPENAI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
            }

            // 2. 修改 config.toml 中的 base_url
            let config_str = live_config
                .get("config")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let codex_provider = self
                .get_current_provider_for_app(&AppType::Codex)
                .ok()
                .flatten();
            let updated_config = Self::apply_codex_proxy_toml_config_for_provider(
                config_str,
                &proxy_codex_base_url,
                codex_provider.as_ref(),
            );
            live_config["config"] = json!(updated_config);
            Self::attach_codex_model_catalog_from_provider(
                &mut live_config,
                codex_provider.as_ref(),
            );

            self.write_codex_takeover_live_for_provider(&live_config, codex_provider.as_ref())?;
            log::info!("Codex Live 配置已接管，代理地址: {proxy_codex_base_url}");
        }

        // Gemini: 修改 GOOGLE_GEMINI_BASE_URL，使用占位符替代真实 Token（代理会注入真实 Token）
        if let Ok(mut live_config) = self.read_gemini_live() {
            if let Some(env) = live_config.get_mut("env").and_then(|v| v.as_object_mut()) {
                env.insert("GOOGLE_GEMINI_BASE_URL".to_string(), json!(&proxy_url));
                // 使用占位符，避免显示缺少 key 的警告
                env.insert("GEMINI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
            } else {
                live_config["env"] = json!({
                    "GOOGLE_GEMINI_BASE_URL": &proxy_url,
                    "GEMINI_API_KEY": PROXY_TOKEN_PLACEHOLDER
                });
            }
            self.write_gemini_live(&live_config)?;
            log::info!("Gemini Live 配置已接管，代理地址: {proxy_url}");
        }

        Ok(())
    }

    /// 接管指定应用的 Live 配置。
    ///
    /// 原目标存在时严格解析现有 live（保留客户端内的非供应商字段）；原目标不存在时
    /// 以当前 provider 的 effective settings 为基线创建受管文件。首次快照已在调用前
    /// 记录 `existed=false`，关闭接管会删除该文件并恢复为不存在（R10）。
    async fn takeover_live_config_strict(&self, app_type: &AppType) -> Result<(), String> {
        if Self::is_c2b_takeover_app(app_type) {
            self.validate_proxy_capability_for_app(app_type)?;
        }
        let (proxy_url, proxy_codex_base_url) = self.build_proxy_urls().await?;

        match app_type {
            AppType::Claude => {
                let claude_provider = self.require_current_provider_for_app(&AppType::Claude)?;
                let claude_provider =
                    self.claude_provider_with_effective_settings(&claude_provider)?;
                let mut live_config = if Self::primary_live_exists_for_app(&AppType::Claude) {
                    self.read_claude_live()?
                } else {
                    claude_provider.settings_config.clone()
                };
                Self::apply_claude_takeover_fields_for_provider(
                    &mut live_config,
                    &proxy_url,
                    &claude_provider,
                );
                self.write_claude_live(&live_config)?;
                log::info!("Claude Live 配置已接管，代理地址: {proxy_url}");
            }
            AppType::Codex => {
                let codex_provider = self.require_current_provider_for_app(&AppType::Codex)?;
                let mut live_config = if Self::primary_live_exists_for_app(&AppType::Codex) {
                    self.read_codex_live()?
                } else {
                    build_effective_settings_with_common_config(
                        self.db.as_ref(),
                        &AppType::Codex,
                        &codex_provider,
                    )
                    .map_err(|e| format!("构建 codex 有效配置失败: {e}"))?
                };

                if let Some(auth) = live_config.get_mut("auth").and_then(|v| v.as_object_mut()) {
                    auth.insert("OPENAI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
                } else if let Some(root) = live_config.as_object_mut() {
                    root.insert(
                        "auth".to_string(),
                        json!({ "OPENAI_API_KEY": PROXY_TOKEN_PLACEHOLDER }),
                    );
                }

                let config_str = live_config
                    .get("config")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let updated_config = Self::apply_codex_proxy_toml_config_for_provider(
                    config_str,
                    &proxy_codex_base_url,
                    Some(&codex_provider),
                );
                live_config["config"] = json!(updated_config);
                Self::attach_codex_model_catalog_from_provider(
                    &mut live_config,
                    Some(&codex_provider),
                );

                self.write_codex_takeover_live_for_provider(&live_config, Some(&codex_provider))?;
                log::info!("Codex Live 配置已接管，代理地址: {proxy_codex_base_url}");
            }
            AppType::Gemini => {
                let gemini_provider = self.require_current_provider_for_app(&AppType::Gemini)?;
                let mut live_config = if Self::primary_live_exists_for_app(&AppType::Gemini) {
                    self.read_gemini_live()?
                } else {
                    build_effective_settings_with_common_config(
                        self.db.as_ref(),
                        &AppType::Gemini,
                        &gemini_provider,
                    )
                    .map_err(|e| format!("构建 gemini 有效配置失败: {e}"))?
                };

                if let Some(env) = live_config.get_mut("env").and_then(|v| v.as_object_mut()) {
                    env.insert("GOOGLE_GEMINI_BASE_URL".to_string(), json!(&proxy_url));
                    env.insert("GEMINI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
                } else {
                    live_config["env"] = json!({
                        "GOOGLE_GEMINI_BASE_URL": &proxy_url,
                        "GEMINI_API_KEY": PROXY_TOKEN_PLACEHOLDER
                    });
                }

                self.write_gemini_live(&live_config)?;
                log::info!("Gemini Live 配置已接管，代理地址: {proxy_url}");
            }
            AppType::ClaudeDesktop | AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => {
                self.write_c2b_proxy_live(app_type, &proxy_url)?;
            }
        }

        Ok(())
    }

    /// 接管指定应用的 Live 配置（尽力而为：配置不存在/读取失败则跳过）
    #[cfg(test)]
    #[allow(dead_code)] // 旧批量接管回归夹具的内部步骤
    async fn takeover_live_config_best_effort(&self, app_type: &AppType) -> Result<(), String> {
        let (proxy_url, proxy_codex_base_url) = self.build_proxy_urls().await?;

        match app_type {
            AppType::Claude => {
                if let Ok(mut live_config) = self.read_claude_live() {
                    let claude_provider = self
                        .get_current_provider_for_app(&AppType::Claude)
                        .ok()
                        .flatten();
                    if let Some(provider) = claude_provider.as_ref() {
                        let provider = self.claude_provider_with_effective_settings(provider)?;
                        Self::apply_claude_takeover_fields_for_provider(
                            &mut live_config,
                            &proxy_url,
                            &provider,
                        );
                    } else {
                        Self::apply_claude_takeover_fields_with_policy(
                            &mut live_config,
                            &proxy_url,
                            ClaudeTakeoverAuthPolicy::PreserveExistingOrAuthToken,
                        );
                    }
                    let _ = self.write_claude_live(&live_config);
                }
            }
            AppType::Codex => {
                if let Ok(mut live_config) = self.read_codex_live() {
                    if let Some(auth) = live_config.get_mut("auth").and_then(|v| v.as_object_mut())
                    {
                        auth.insert("OPENAI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
                    }

                    let config_str = live_config
                        .get("config")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let codex_provider = self
                        .get_current_provider_for_app(&AppType::Codex)
                        .ok()
                        .flatten();
                    let updated_config = Self::apply_codex_proxy_toml_config_for_provider(
                        config_str,
                        &proxy_codex_base_url,
                        codex_provider.as_ref(),
                    );
                    live_config["config"] = json!(updated_config);
                    Self::attach_codex_model_catalog_from_provider(
                        &mut live_config,
                        codex_provider.as_ref(),
                    );

                    let _ = self.write_codex_takeover_live_for_provider(
                        &live_config,
                        codex_provider.as_ref(),
                    );
                }
            }
            AppType::Gemini => {
                if let Ok(mut live_config) = self.read_gemini_live() {
                    if let Some(env) = live_config.get_mut("env").and_then(|v| v.as_object_mut()) {
                        env.insert("GOOGLE_GEMINI_BASE_URL".to_string(), json!(&proxy_url));
                        env.insert("GEMINI_API_KEY".to_string(), json!(PROXY_TOKEN_PLACEHOLDER));
                    } else {
                        live_config["env"] = json!({
                            "GOOGLE_GEMINI_BASE_URL": &proxy_url,
                            "GEMINI_API_KEY": PROXY_TOKEN_PLACEHOLDER
                        });
                    }

                    let _ = self.write_gemini_live(&live_config);
                }
            }
            AppType::ClaudeDesktop | AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => {
                let _ = self.write_c2b_proxy_live(app_type, &proxy_url);
            }
        }

        Ok(())
    }

    /// 恢复原始 Live 配置
    #[cfg(test)]
    #[allow(dead_code)] // 旧批量接管失败回归夹具；生产恢复走逐模块 adapter
    async fn restore_live_configs(&self) -> Result<(), String> {
        let mut errors = Vec::new();

        for app_type in [AppType::Claude, AppType::Codex, AppType::Gemini] {
            if let Err(e) = self
                .restore_live_config_for_app_with_fallback(&app_type)
                .await
            {
                errors.push(e);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }

    async fn restore_live_config_for_app_with_fallback(
        &self,
        app_type: &AppType,
    ) -> Result<(), String> {
        let _guard = self.switch_locks.lock_for_app(app_type.as_str()).await;
        self.restore_live_config_for_app_with_fallback_inner(app_type)
            .await
    }

    async fn restore_live_config_for_app_with_fallback_inner(
        &self,
        app_type: &AppType,
    ) -> Result<(), String> {
        let app_type_str = app_type.as_str();

        // 1) 优先从 Live 备份恢复（这是"原始 Live"的唯一可靠来源）
        let backup = self
            .db
            .get_live_backup(app_type_str)
            .await
            .map_err(|e| format!("获取 {app_type_str} Live 备份失败: {e}"))?;
        if let Some(backup) = backup {
            // C2a：版本化精确快照走 adapter 逐字节恢复（proxy 残留恢复），恢复成功即返回。
            // decode 分流：Manifest → adapter；无版本旧 JSON → legacy 兜底逻辑。
            match crate::proxy::snapshot::decode_stored_snapshot(
                app_type_str,
                &backup.original_config,
            )? {
                crate::proxy::snapshot::DecodedSnapshot::Manifest(manifest) => {
                    let adapter = Self::snapshot_adapter_for_app(app_type)
                        .map_err(|error| {
                            format!("解析 {app_type_str} 快照 adapter 失败: {error}")
                        })?
                        .ok_or_else(|| {
                            format!("{app_type_str} 存在版本化快照，但精确恢复 adapter 尚未接入；已保留快照")
                        })?;
                    adapter.restore_manifest_transactional(&manifest)?;
                    log::info!("{app_type_str} Live 配置已从版本化快照恢复");
                    return Ok(());
                }
                crate::proxy::snapshot::DecodedSnapshot::Legacy(legacy) => {
                    let config: Value = serde_json::from_str(&legacy.original_config)
                        .map_err(|error| format!("解析 {app_type_str} 旧版备份失败: {error}"))?;

                    // 备份若是代理占位符（异常历史：上次 stop 失败导致 Live 留在了代理状态，
                    // 下次接管时又被错误地备份成"原始 Live"），不能直接用 — 否则 stop 后
                    // Live 永远卡在 127.0.0.1:42567。落到下面的 SSOT 兜底重建。
                    if Self::live_has_proxy_placeholder_for_app(app_type, &config) {
                        log::warn!(
                            "{app_type_str} 备份本身已是代理占位符（异常历史状态），跳过备份，改走 SSOT 重建 Live"
                        );
                    } else {
                        self.write_live_config_for_app(app_type, &config)?;
                        log::info!("{app_type_str} Live 配置已从旧版备份恢复");
                        return Ok(());
                    }
                }
            }
        }

        // 2) 兜底：备份缺失，但 Live 仍包含接管占位符（异常退出/历史 bug 场景）
        if !self.detect_takeover_in_live_config_for_app(app_type) {
            return Ok(());
        }

        // 2.1) 优先从 SSOT（当前供应商）重建 Live（比"清理字段"更可用）
        match self.restore_live_from_ssot_for_app(app_type) {
            Ok(true) => {
                log::info!("{app_type_str} Live 配置已从 SSOT 恢复（无备份兜底）");
                return Ok(());
            }
            Ok(false) => {
                log::warn!(
                    "{app_type_str} Live 备份缺失，且无法从 SSOT 恢复，将尝试清理接管占位符"
                );
            }
            Err(e) => {
                log::error!(
                    "{app_type_str} Live 备份缺失，SSOT 恢复失败，将尝试清理接管占位符: {e}"
                );
            }
        }

        // 2.2) 最后兜底：尽力清理占位符与本地代理地址，避免长期卡在代理占位符状态
        self.cleanup_takeover_placeholders_in_live_for_app(app_type)?;
        log::info!("{app_type_str} Live 接管占位符已清理（无备份兜底）");
        Ok(())
    }

    fn write_live_config_for_app(&self, app_type: &AppType, config: &Value) -> Result<(), String> {
        match app_type {
            AppType::Claude => self.write_claude_live(config),
            AppType::Codex => self.write_codex_live(config),
            AppType::Gemini => self.write_gemini_live(config),
            _ => Err("该应用不支持旧版 JSON 备份恢复".to_string()),
        }
    }

    fn read_c2b_live_config_for_app(&self, app_type: &AppType) -> Result<Value, String> {
        match app_type {
            AppType::ClaudeDesktop => {
                let profile_path = crate::claude_desktop_config::snapshot_target_paths()
                    .map_err(|error| format!("解析 Claude Desktop profile 路径失败: {error}"))?
                    .into_iter()
                    .find_map(|(id, path)| (id == "profile").then_some(path))
                    .ok_or_else(|| "Claude Desktop 快照目标缺少 profile".to_string())?;
                if !profile_path.exists() {
                    return Ok(json!({}));
                }
                read_json_file(&profile_path)
                    .map_err(|error| format!("读取 Claude Desktop profile 失败: {error}"))
            }
            AppType::OpenCode => crate::opencode_config::read_opencode_config()
                .map_err(|error| format!("读取 OpenCode 配置失败: {error}")),
            AppType::OpenClaw => crate::openclaw_config::read_openclaw_config()
                .map_err(|error| format!("读取 OpenClaw 配置失败: {error}")),
            AppType::Hermes => {
                let config = crate::hermes_config::read_hermes_config()
                    .map_err(|error| format!("读取 Hermes 配置失败: {error}"))?;
                crate::hermes_config::yaml_to_json(&config)
                    .map_err(|error| format!("转换 Hermes 配置失败: {error}"))
            }
            _ => Err(format!("{} 不是 C2b 接管模块", app_type.as_str())),
        }
    }

    pub fn detect_takeover_in_live_config_for_app(&self, app_type: &AppType) -> bool {
        match app_type {
            AppType::Claude => match self.read_claude_live() {
                Ok(config) => Self::is_claude_live_taken_over(&config),
                Err(_) => false,
            },
            AppType::Codex => match self.read_codex_live() {
                Ok(config) => Self::is_codex_live_taken_over(&config),
                Err(_) => false,
            },
            AppType::Gemini => match self.read_gemini_live() {
                Ok(config) => Self::is_gemini_live_taken_over(&config),
                Err(_) => false,
            },
            AppType::ClaudeDesktop | AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => {
                self.read_c2b_live_config_for_app(app_type)
                    .map(|config| Self::live_has_proxy_placeholder_for_app(app_type, &config))
                    .unwrap_or(false)
            }
        }
    }

    /// 当 Live 备份缺失时，尝试用 SSOT（当前供应商）写回 Live，以解除占位符接管。
    ///
    /// 返回值：
    /// - Ok(true)：已成功写回
    /// - Ok(false)：缺少当前供应商/供应商不存在/供应商本身含占位符，无法写回
    fn restore_live_from_ssot_for_app(&self, app_type: &AppType) -> Result<bool, String> {
        let current_id = crate::settings::get_effective_current_provider(&self.db, app_type)
            .map_err(|e| format!("获取 {app_type:?} 当前供应商失败: {e}"))?;

        let Some(current_id) = current_id else {
            return Ok(false);
        };

        let providers = self
            .db
            .get_all_providers(app_type.as_str())
            .map_err(|e| format!("读取 {app_type:?} 供应商列表失败: {e}"))?;

        let Some(provider) = providers.get(&current_id) else {
            return Ok(false);
        };

        // 供应商配置本身含接管占位符时不可写回（历史异常：接管期间 Live 被
        // 误导入成了供应商）。写回只会把占位符固化进 Live；返回 Ok(false)
        // 让调用方落到"清理占位符"兜底。
        if Self::live_has_proxy_placeholder_for_app(app_type, &provider.settings_config) {
            log::warn!(
                "{app_type:?} 当前供应商配置含代理接管占位符（疑似接管期间被导入的残留），跳过 SSOT 写回，改走占位符清理"
            );
            return Ok(false);
        }

        let provider = if matches!(app_type, AppType::ClaudeDesktop) {
            Self::provider_with_claude_desktop_mode(
                provider,
                crate::provider::ClaudeDesktopMode::Direct,
            )
        } else {
            provider.clone()
        };
        write_live_with_common_config(self.db.as_ref(), app_type, &provider)
            .map_err(|e| format!("写入 {app_type:?} Live 配置失败: {e}"))?;

        Ok(true)
    }

    fn cleanup_takeover_placeholders_in_live_for_app(
        &self,
        app_type: &AppType,
    ) -> Result<(), String> {
        match app_type {
            AppType::Claude => self.cleanup_claude_takeover_placeholders_in_live(),
            AppType::Codex => self.cleanup_codex_takeover_placeholders_in_live(),
            AppType::Gemini => self.cleanup_gemini_takeover_placeholders_in_live(),
            AppType::ClaudeDesktop => self.cleanup_claude_desktop_takeover_in_live(),
            AppType::OpenCode => self.cleanup_opencode_takeover_in_live(),
            AppType::OpenClaw => self.cleanup_openclaw_takeover_in_live(),
            AppType::Hermes => self.cleanup_hermes_takeover_in_live(),
        }
    }

    fn is_local_proxy_url(url: &str) -> bool {
        let url = url.trim();
        if !url.starts_with("http://") {
            return false;
        }
        let rest = &url["http://".len()..];
        rest.starts_with("127.0.0.1")
            || rest.starts_with("localhost")
            || rest.starts_with("0.0.0.0")
            || rest.starts_with("[::1]")
            || rest.starts_with("[::]")
            || rest.starts_with("::1")
            || rest.starts_with("::")
    }

    #[cfg(test)]
    fn proxy_urls_match(actual: &str, expected: &str) -> bool {
        actual.trim().trim_end_matches('/') == expected.trim().trim_end_matches('/')
    }

    #[cfg(test)]
    fn codex_config_has_base_url_matching(
        config_text: &str,
        predicate: impl Fn(&str) -> bool,
    ) -> bool {
        let Ok(doc) = toml::from_str::<toml::Value>(config_text) else {
            return false;
        };

        let active_provider = doc
            .get("model_provider")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|id| !id.is_empty());

        if let Some(provider_id) = active_provider {
            if doc
                .get("model_providers")
                .and_then(|value| value.get(provider_id))
                .and_then(|value| value.get("base_url"))
                .and_then(|value| value.as_str())
                .is_some_and(&predicate)
            {
                return true;
            }
        }

        doc.get("base_url")
            .and_then(|value| value.as_str())
            .is_some_and(predicate)
    }

    #[cfg(test)]
    async fn live_takeover_matches_current_proxy(
        &self,
        app_type: &AppType,
    ) -> Result<bool, String> {
        let (proxy_url, proxy_codex_base_url) = self.build_proxy_urls().await?;

        match app_type {
            AppType::Claude => {
                let config = self.read_claude_live()?;
                let base_url_matches = config
                    .get("env")
                    .and_then(|value| value.get("ANTHROPIC_BASE_URL"))
                    .and_then(|value| value.as_str())
                    .is_some_and(|url| Self::proxy_urls_match(url, &proxy_url));
                Ok(Self::is_claude_live_taken_over(&config) && base_url_matches)
            }
            AppType::Codex => {
                let config = self.read_codex_live()?;
                let base_url_matches = config
                    .get("config")
                    .and_then(|value| value.as_str())
                    .is_some_and(|config_text| {
                        Self::codex_config_has_base_url_matching(config_text, |url| {
                            Self::proxy_urls_match(url, &proxy_codex_base_url)
                        })
                    });
                Ok(Self::codex_live_has_proxy_placeholder(&config) && base_url_matches)
            }
            AppType::Gemini => {
                let config = self.read_gemini_live()?;
                let base_url_matches = config
                    .get("env")
                    .and_then(|value| value.get("GOOGLE_GEMINI_BASE_URL"))
                    .and_then(|value| value.as_str())
                    .is_some_and(|url| Self::proxy_urls_match(url, &proxy_url));
                Ok(Self::is_gemini_live_taken_over(&config) && base_url_matches)
            }
            AppType::ClaudeDesktop | AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => {
                let config = self.read_c2b_live_config_for_app(app_type)?;
                let expected = Self::namespaced_proxy_base_url(&proxy_url, app_type)?;
                Ok(Self::live_has_proxy_placeholder_for_app(app_type, &config)
                    && Self::c2b_proxy_url_matches(&config, app_type, &expected))
            }
        }
    }

    fn cleanup_claude_takeover_placeholders_in_live(&self) -> Result<(), String> {
        let mut config = self.read_claude_live()?;

        let Some(env) = config.get_mut("env").and_then(|v| v.as_object_mut()) else {
            return Ok(());
        };

        for key in [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "OPENROUTER_API_KEY",
            "OPENAI_API_KEY",
        ] {
            if env.get(key).and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER) {
                env.remove(key);
            }
        }

        if env
            .get("ANTHROPIC_BASE_URL")
            .and_then(|v| v.as_str())
            .map(Self::is_local_proxy_url)
            .unwrap_or(false)
        {
            env.remove("ANTHROPIC_BASE_URL");
        }

        self.write_claude_live(&config)?;
        Ok(())
    }

    fn cleanup_codex_takeover_placeholders_in_live(&self) -> Result<(), String> {
        let mut config = self.read_codex_live()?;

        if let Some(auth) = config.get_mut("auth").and_then(|v| v.as_object_mut()) {
            if auth.get("OPENAI_API_KEY").and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER)
            {
                auth.remove("OPENAI_API_KEY");
            }
        }

        if let Some(cfg_str) = config.get("config").and_then(|v| v.as_str()) {
            let updated = Self::remove_local_toml_base_url(cfg_str);
            let updated =
                crate::codex_config::remove_codex_experimental_bearer_token_if(&updated, |token| {
                    token == PROXY_TOKEN_PLACEHOLDER
                })
                .map_err(|e| format!("清理 Codex 接管占位符失败: {e}"))?;
            config["config"] = json!(updated);
        }

        self.write_codex_live(&config)?;
        Ok(())
    }

    /// Remove local proxy base_url from TOML（委托给 codex_config 共享实现）
    fn remove_local_toml_base_url(toml_str: &str) -> String {
        crate::codex_config::remove_codex_toml_base_url_if(toml_str, Self::is_local_proxy_url)
    }

    fn cleanup_gemini_takeover_placeholders_in_live(&self) -> Result<(), String> {
        let mut config = self.read_gemini_live()?;

        let Some(env) = config.get_mut("env").and_then(|v| v.as_object_mut()) else {
            return Ok(());
        };

        if env.get("GEMINI_API_KEY").and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER) {
            env.remove("GEMINI_API_KEY");
        }

        if env
            .get("GOOGLE_GEMINI_BASE_URL")
            .and_then(|v| v.as_str())
            .map(Self::is_local_proxy_url)
            .unwrap_or(false)
        {
            env.remove("GOOGLE_GEMINI_BASE_URL");
        }

        self.write_gemini_live(&config)?;
        Ok(())
    }

    fn cleanup_claude_desktop_takeover_in_live(&self) -> Result<(), String> {
        let official = Provider::with_id(
            crate::database::CLAUDE_DESKTOP_OFFICIAL_PROVIDER_ID.to_string(),
            "Claude Desktop 官方".to_string(),
            json!({}),
            None,
        );
        crate::claude_desktop_config::apply_provider(self.db.as_ref(), &official)
            .map_err(|error| format!("清理 Claude Desktop 网关 profile 失败: {error}"))
    }

    fn cleanup_opencode_takeover_in_live(&self) -> Result<(), String> {
        let mut config = crate::opencode_config::read_opencode_config()
            .map_err(|error| format!("读取 OpenCode 配置失败: {error}"))?;
        let mut changed = false;
        if let Some(providers) = config.get_mut("provider").and_then(Value::as_object_mut) {
            for provider in providers.values_mut() {
                let Some(options) = provider.get_mut("options").and_then(Value::as_object_mut)
                else {
                    continue;
                };
                if Self::c2b_base_url_matches_namespace(options.get("baseURL"), "opencode/v1") {
                    options.remove("baseURL");
                    options.remove("apiKey");
                    changed = true;
                }
            }
        }
        if changed {
            crate::opencode_config::write_opencode_config(&config)
                .map_err(|error| format!("清理 OpenCode 接管配置失败: {error}"))?;
        }
        Ok(())
    }

    fn cleanup_openclaw_takeover_in_live(&self) -> Result<(), String> {
        let providers = crate::openclaw_config::get_providers()
            .map_err(|error| format!("读取 OpenClaw 配置失败: {error}"))?;
        for (id, mut provider) in providers {
            if Self::c2b_base_url_matches_namespace(provider.get("baseUrl"), "openclaw/v1") {
                if let Some(root) = provider.as_object_mut() {
                    root.remove("baseUrl");
                    root.remove("apiKey");
                }
                crate::openclaw_config::set_provider(&id, provider)
                    .map_err(|error| format!("清理 OpenClaw provider '{id}' 失败: {error}"))?;
            }
        }
        Ok(())
    }

    fn cleanup_hermes_takeover_in_live(&self) -> Result<(), String> {
        let providers = crate::hermes_config::get_providers()
            .map_err(|error| format!("读取 Hermes 配置失败: {error}"))?;
        for (id, mut provider) in providers {
            if Self::c2b_base_url_matches_namespace(provider.get("base_url"), "hermes/v1") {
                if let Some(root) = provider.as_object_mut() {
                    root.remove("base_url");
                    root.remove("api_key");
                }
                crate::hermes_config::set_provider(&id, provider)
                    .map_err(|error| format!("清理 Hermes provider '{id}' 失败: {error}"))?;
            }
        }
        Ok(())
    }

    /// 检查是否处于 Live 接管模式
    pub async fn is_takeover_active(&self) -> Result<bool, String> {
        let status = self.get_takeover_status().await?;
        Ok(AppType::all().any(|app| status.for_app(&app).takeover_enabled))
    }

    /// 启动/退出时回收遗留接管所有权，不会重新接管或启动网关。
    pub async fn recover_from_crash(&self) -> Result<(), String> {
        let mut errors = Vec::new();

        for app in AppType::all() {
            let app_type = app.as_str();
            let config = match self.db.get_proxy_config_for_app(app_type).await {
                Ok(config) => config,
                Err(error) => {
                    errors.push(format!("读取 {app_type} 接管状态失败: {error}"));
                    continue;
                }
            };
            let has_backup = match self.db.get_live_backup(app_type).await {
                Ok(backup) => backup.is_some(),
                Err(error) => {
                    errors.push(format!("读取 {app_type} 快照失败: {error}"));
                    continue;
                }
            };
            // direct 只放弃所有权，不恢复首次快照，也不改写当前真实上游。
            if config.takeover_enabled && config.route_mode == RouteMode::Direct {
                if let Err(error) = abandon_snapshot_ownership(&self.db, &app).await {
                    errors.push(format!("放弃 {app_type} direct 所有权失败: {error}"));
                }
                continue;
            }

            let has_proxy_residue = self.detect_takeover_in_live_config_for_app(&app);
            // C2b 的 namespaced URL + gateway token 也可能是用户手工配置的网关入口。
            // 在 takeover_enabled=false 且没有快照时，它不是 AGS 所有的残留；否则启动
            // 恢复会违反“关闭接管不得写 live”，把用户手工配置改回当前 provider。
            // C1 三模块沿用原有 placeholder 兜底，因为其 AGS 占位符与普通配置可区分。
            let recoverable_proxy_residue = has_proxy_residue
                && (!Self::is_c2b_takeover_app(&app) || config.takeover_enabled || has_backup);

            // backup/占位符代表确有 proxy 残留；仅凭历史 enabled 不写 Live。
            if has_backup || recoverable_proxy_residue {
                if let Err(error) = self.restore_live_config_for_app_with_fallback(&app).await {
                    errors.push(format!("恢复 {app_type} proxy 残留失败: {error}"));
                    continue;
                }
            }

            if config.takeover_enabled || has_backup || recoverable_proxy_residue {
                if let Err(error) = self.db.release_takeover_ownership(app_type).await {
                    errors.push(format!("清理 {app_type} 接管所有权失败: {error}"));
                }
            }
        }

        let _ = self.db.set_live_takeover_active(false).await;
        if errors.is_empty() {
            log::info!("遗留接管状态已回收，启动不会自动重新接管");
            Ok(())
        } else {
            Err(errors.join("；"))
        }
    }

    /// 检测 Live 配置是否处于"被接管"的残留状态
    ///
    /// 用于兜底处理：当数据库备份缺失但 Live 文件已经写成代理占位符时，
    /// 启动流程可以据此触发恢复逻辑。
    pub fn detect_takeover_in_live_configs(&self) -> bool {
        AppType::all().any(|app_type| self.detect_takeover_in_live_config_for_app(&app_type))
    }

    fn is_claude_live_taken_over(config: &Value) -> bool {
        let env = match config.get("env").and_then(|v| v.as_object()) {
            Some(env) => env,
            None => return false,
        };

        for key in [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "OPENROUTER_API_KEY",
            "OPENAI_API_KEY",
        ] {
            if env.get(key).and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER) {
                return true;
            }
        }

        false
    }

    fn codex_live_has_proxy_placeholder(config: &Value) -> bool {
        if config
            .get("auth")
            .and_then(|v| v.as_object())
            .and_then(|auth| auth.get("OPENAI_API_KEY"))
            .and_then(|v| v.as_str())
            == Some(PROXY_TOKEN_PLACEHOLDER)
        {
            return true;
        }

        config
            .get("config")
            .and_then(|v| v.as_str())
            .and_then(crate::codex_config::extract_codex_experimental_bearer_token)
            .as_deref()
            == Some(PROXY_TOKEN_PLACEHOLDER)
    }

    fn is_codex_live_taken_over(config: &Value) -> bool {
        Self::codex_live_has_proxy_placeholder(config)
    }

    fn is_gemini_live_taken_over(config: &Value) -> bool {
        let env = match config.get("env").and_then(|v| v.as_object()) {
            Some(env) => env,
            None => return false,
        };
        env.get("GEMINI_API_KEY").and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER)
    }

    fn c2b_base_url_matches_namespace(value: Option<&Value>, namespace: &str) -> bool {
        let Some(url) = value.and_then(Value::as_str).map(str::trim) else {
            return false;
        };
        if !Self::is_local_proxy_url(url) {
            return false;
        }
        let Some((_, path)) = url["http://".len()..].split_once('/') else {
            return false;
        };
        path.trim_matches('/').eq_ignore_ascii_case(namespace)
    }

    fn is_claude_desktop_live_taken_over(config: &Value) -> bool {
        config.get("inferenceProvider").and_then(Value::as_str) == Some("gateway")
            && Self::c2b_base_url_matches_namespace(
                config.get("inferenceGatewayBaseUrl"),
                "claude-desktop",
            )
    }

    fn is_opencode_provider_taken_over(provider: &Value) -> bool {
        let options = provider.get("options");
        Self::c2b_base_url_matches_namespace(
            options.and_then(|value| value.get("baseURL")),
            "opencode/v1",
        ) && options
            .and_then(|value| value.get("apiKey"))
            .and_then(Value::as_str)
            .is_some_and(|key| !key.trim().is_empty())
    }

    fn is_opencode_live_taken_over(config: &Value) -> bool {
        if config.get("options").is_some() {
            return Self::is_opencode_provider_taken_over(config);
        }
        config
            .get("provider")
            .and_then(Value::as_object)
            .is_some_and(|providers| {
                providers
                    .values()
                    .any(Self::is_opencode_provider_taken_over)
            })
    }

    fn is_openclaw_provider_taken_over(provider: &Value) -> bool {
        Self::c2b_base_url_matches_namespace(provider.get("baseUrl"), "openclaw/v1")
            && provider
                .get("apiKey")
                .and_then(Value::as_str)
                .is_some_and(|key| !key.trim().is_empty())
    }

    fn is_openclaw_live_taken_over(config: &Value) -> bool {
        if config.get("baseUrl").is_some() {
            return Self::is_openclaw_provider_taken_over(config);
        }
        config
            .pointer("/models/providers")
            .and_then(Value::as_object)
            .is_some_and(|providers| {
                providers
                    .values()
                    .any(Self::is_openclaw_provider_taken_over)
            })
    }

    fn is_hermes_provider_taken_over(provider: &Value) -> bool {
        Self::c2b_base_url_matches_namespace(provider.get("base_url"), "hermes/v1")
            && provider
                .get("api_key")
                .and_then(Value::as_str)
                .is_some_and(|key| !key.trim().is_empty())
    }

    fn is_hermes_live_taken_over(config: &Value) -> bool {
        if config.get("base_url").is_some() {
            return Self::is_hermes_provider_taken_over(config);
        }
        config
            .get("custom_providers")
            .and_then(Value::as_array)
            .is_some_and(|providers| providers.iter().any(Self::is_hermes_provider_taken_over))
    }

    #[cfg(test)]
    fn c2b_proxy_url_matches(config: &Value, app_type: &AppType, expected: &str) -> bool {
        match app_type {
            AppType::ClaudeDesktop => config
                .get("inferenceGatewayBaseUrl")
                .and_then(Value::as_str)
                .is_some_and(|url| Self::proxy_urls_match(url, expected)),
            AppType::OpenCode => config
                .get("provider")
                .and_then(Value::as_object)
                .is_some_and(|providers| {
                    providers.values().any(|provider| {
                        provider
                            .pointer("/options/baseURL")
                            .and_then(Value::as_str)
                            .is_some_and(|url| Self::proxy_urls_match(url, expected))
                    })
                }),
            AppType::OpenClaw => config
                .pointer("/models/providers")
                .and_then(Value::as_object)
                .is_some_and(|providers| {
                    providers.values().any(|provider| {
                        provider
                            .get("baseUrl")
                            .and_then(Value::as_str)
                            .is_some_and(|url| Self::proxy_urls_match(url, expected))
                    })
                }),
            AppType::Hermes => config
                .get("custom_providers")
                .and_then(Value::as_array)
                .is_some_and(|providers| {
                    providers.iter().any(|provider| {
                        provider
                            .get("base_url")
                            .and_then(Value::as_str)
                            .is_some_and(|url| Self::proxy_urls_match(url, expected))
                    })
                }),
            _ => false,
        }
    }

    /// 判断给定的 Live/备份配置是否已被代理接管（包含 C1 三模块占位符或 C2b
    /// 独立命名空间）。这只是 crash/legacy 兜底信号，绝不替代 proxy_config SSOT。
    ///
    /// 用途：检测"备份里存的其实是代理配置"这种异常历史状态。
    /// 如果发现，备份不可信，备份路径不能写入（否则会把代理配置固化进备份槽），
    /// 恢复路径不能读取（否则会把代理占位符原样写回 Live，永久卡在代理地址）。
    /// 两种情况下都应该走 SSOT 兜底重建 Live。
    fn live_has_proxy_placeholder_for_app(app_type: &AppType, config: &Value) -> bool {
        match app_type {
            AppType::Claude => Self::is_claude_live_taken_over(config),
            AppType::Codex => Self::codex_live_has_proxy_placeholder(config),
            AppType::Gemini => Self::is_gemini_live_taken_over(config),
            AppType::ClaudeDesktop => Self::is_claude_desktop_live_taken_over(config),
            AppType::OpenCode => Self::is_opencode_live_taken_over(config),
            AppType::OpenClaw => Self::is_openclaw_live_taken_over(config),
            AppType::Hermes => Self::is_hermes_live_taken_over(config),
        }
    }

    /// 从供应商配置更新 Live 备份（用于代理模式下的热切换）
    ///
    /// 与 backup_live_configs() 不同，此方法从供应商的 settings_config 生成备份，
    /// 而不是从 Live 文件读取（因为 Live 文件已被代理接管）。
    pub async fn update_live_backup_from_provider(
        &self,
        app_type: &str,
        provider: &Provider,
    ) -> Result<(), String> {
        let _guard = self.switch_locks.lock_for_app(app_type).await;
        self.update_live_backup_from_provider_inner(app_type, provider)
            .await
    }

    /// 仅供已持有 per-app 切换锁的调用方使用。
    async fn update_live_backup_from_provider_inner(
        &self,
        app_type: &str,
        provider: &Provider,
    ) -> Result<(), String> {
        let app_type_enum =
            AppType::from_str(app_type).map_err(|_| format!("未知的应用类型: {app_type}"))?;

        // C2b 起 proxy_live_backup 是 immutable 首次快照；四模块热切换只重写受管
        // live，不得把 provider JSON 覆盖进 manifest 槽。C3 的 managed expected 另管。
        if Self::is_c2b_takeover_app(&app_type_enum) {
            if let Some(backup) = self
                .db
                .get_live_backup(app_type)
                .await
                .map_err(|e| format!("读取 {app_type} 现有快照失败: {e}"))?
            {
                crate::proxy::snapshot::decode_stored_snapshot(app_type, &backup.original_config)?;
            }
            return Ok(());
        }

        let mut effective_settings =
            build_effective_settings_with_common_config(self.db.as_ref(), &app_type_enum, provider)
                .map_err(|e| format!("构建 {app_type} 有效配置失败: {e}"))?;

        if matches!(app_type_enum, AppType::Codex) {
            let existing_backup_value = self
                .db
                .get_live_backup(app_type)
                .await
                .map_err(|e| format!("读取 {app_type} 现有备份失败: {e}"))?
                .map(|backup| {
                    serde_json::from_str::<Value>(&backup.original_config)
                        .map_err(|e| format!("解析 {app_type} 现有备份失败: {e}"))
                })
                .transpose()?;

            if let Some(existing_value) = existing_backup_value.as_ref() {
                Self::preserve_codex_mcp_servers_from_existing_config(
                    &mut effective_settings,
                    existing_value,
                )?;
                Self::preserve_codex_oauth_auth_in_backup(&mut effective_settings, existing_value)?;
            }

            // 统一会话开关：备份是接管释放时恢复 live 的来源，官方配置的
            // 共享 custom 路由注入必须落在备份里，否则恢复后开关失效。
            crate::codex_config::apply_codex_unified_session_bucket_to_settings(
                provider.category.as_deref(),
                &mut effective_settings,
            )
            .map_err(|e| format!("注入统一会话路由失败: {e}"))?;
        }

        let backup_json = match app_type_enum {
            AppType::Claude => serde_json::to_string(&effective_settings)
                .map_err(|e| format!("序列化 Claude 配置失败: {e}"))?,
            AppType::Codex => serde_json::to_string(&effective_settings)
                .map_err(|e| format!("序列化 Codex 配置失败: {e}"))?,
            AppType::Gemini => {
                // Gemini takeover 仅修改 .env；settings.json（含 mcpServers）保持原样。
                let env_backup = if let Some(env) = effective_settings.get("env") {
                    json!({ "env": env })
                } else {
                    json!({ "env": {} })
                };
                serde_json::to_string(&env_backup)
                    .map_err(|e| format!("序列化 Gemini 配置失败: {e}"))?
            }
            _ => return Err(format!("未知的应用类型: {app_type}")),
        };

        self.db
            .save_live_backup(app_type, &backup_json)
            .await
            .map_err(|e| format!("更新 {app_type} 备份失败: {e}"))?;

        log::info!("已更新 {app_type} Live 备份（热切换）");
        Ok(())
    }

    /// ProviderService 的同步 CRUD/switch 路径使用的 C2b proxy-safe 热切换。
    ///
    /// 不把整个 async hot-switch 包进 `futures::executor::block_on`：Claude Desktop
    /// writer 内部会同步读取 DB，外层 LocalPool 会形成嵌套 executor。这里把必要的
    /// async 读取拆开完成，再在同步栈写模块专属 proxy fragment；immutable snapshot
    /// 从始至终不参与。
    pub(crate) fn hot_switch_c2b_provider_sync(
        &self,
        app_type: &AppType,
        provider_id: &str,
    ) -> Result<HotSwitchOutcome, String> {
        if !Self::is_c2b_takeover_app(app_type) {
            return Err(format!("{} 不是 C2b 接管模块", app_type.as_str()));
        }

        let provider = self
            .db
            .get_provider_by_id(provider_id, app_type.as_str())
            .map_err(|e| format!("读取供应商失败: {e}"))?
            .ok_or_else(|| format!("供应商不存在: {provider_id}"))?;
        if provider.category.as_deref() == Some("official") {
            return Err(
                "代理接管模式下不能切换到官方供应商 (Cannot switch to official provider during proxy takeover)"
                    .to_string(),
            );
        }

        let takeover =
            futures::executor::block_on(self.db.get_proxy_config_for_app(app_type.as_str()))
                .map_err(|e| format!("读取 {} 接管状态失败: {e}", app_type.as_str()))?;
        let proxy_managed = takeover.takeover_enabled && takeover.route_mode == RouteMode::Proxy;
        if proxy_managed {
            crate::proxy::providers::validate_module_proxy_capability(app_type, &provider)?;
        }

        // Resolve every fallible prerequisite and capture the complete pre-switch Live before
        // changing either current pointer. This makes URL/config failures side-effect free and
        // gives writer failures a byte-exact rollback source independent of the first-open snapshot.
        let proxy_write_context = if proxy_managed {
            let adapter = Self::snapshot_adapter_for_app(app_type)?
                .ok_or_else(|| format!("{} 尚无精确快照 adapter", app_type.as_str()))?;
            let live_before = self.capture_transient_live_snapshot(app_type, adapter.as_ref())?;
            let (proxy_origin, _) = futures::executor::block_on(self.build_proxy_urls())?;
            Some((adapter, live_before, proxy_origin))
        } else {
            None
        };

        let logical_target_changed =
            crate::settings::get_effective_current_provider(&self.db, app_type)
                .map_err(|e| format!("读取当前供应商失败: {e}"))?
                .as_deref()
                != Some(provider_id);
        let previous_db_current = self
            .db
            .get_current_provider(app_type.as_str())
            .map_err(|e| format!("读取原 DB current 失败: {e}"))?;
        let previous_settings_current = crate::settings::get_current_provider(app_type);

        self.db
            .set_current_provider(app_type.as_str(), provider_id)
            .map_err(|e| format!("更新当前供应商失败: {e}"))?;
        if let Err(error) = crate::settings::set_current_provider(app_type, Some(provider_id)) {
            return Err(self.rollback_c2b_current_provider_pointers(
                app_type,
                previous_db_current.as_deref(),
                previous_settings_current.as_deref(),
                &format!("更新本地当前供应商失败: {error}"),
            ));
        }

        if let Some((adapter, live_before, proxy_origin)) = proxy_write_context.as_ref() {
            // `write_c2b_proxy_live` is deliberately outside the block_on above so
            // Claude Desktop may perform its existing synchronous DB reads safely.
            if let Err(error) = self.write_c2b_proxy_live(app_type, proxy_origin) {
                return Err(self.rollback_c2b_hot_switch(
                    app_type,
                    previous_db_current.as_deref(),
                    previous_settings_current.as_deref(),
                    adapter.as_ref(),
                    live_before,
                    &format!("写入代理受管 Live 配置失败: {error}"),
                ));
            }
        }

        futures::executor::block_on(async {
            if let Some(server) = self.server.read().await.as_ref() {
                server
                    .set_active_target(app_type.as_str(), &provider.id, &provider.name)
                    .await;
            }
        });

        Ok(HotSwitchOutcome {
            logical_target_changed,
        })
    }

    pub async fn hot_switch_provider(
        &self,
        app_type: &str,
        provider_id: &str,
    ) -> Result<HotSwitchOutcome, String> {
        let _guard = self.switch_locks.lock_for_app(app_type).await;
        self.hot_switch_provider_inner(app_type, provider_id).await
    }

    pub(crate) async fn hot_switch_provider_inner(
        &self,
        app_type: &str,
        provider_id: &str,
    ) -> Result<HotSwitchOutcome, String> {
        let app_type_enum =
            AppType::from_str(app_type).map_err(|_| format!("无效的应用类型: {app_type}"))?;
        let provider = self
            .db
            .get_provider_by_id(provider_id, app_type)
            .map_err(|e| format!("读取供应商失败: {e}"))?
            .ok_or_else(|| format!("供应商不存在: {provider_id}"))?;

        // Defense-in-depth: block official providers during proxy takeover
        if provider.category.as_deref() == Some("official") {
            return Err(
                "代理接管模式下不能切换到官方供应商 (Cannot switch to official provider during proxy takeover)"
                    .to_string(),
            );
        }

        let c2b_proxy_managed = if Self::is_c2b_takeover_app(&app_type_enum) {
            let takeover = self
                .db
                .get_proxy_config_for_app(app_type_enum.as_str())
                .await
                .map_err(|e| format!("读取 {} 接管状态失败: {e}", app_type_enum.as_str()))?;
            if takeover.takeover_enabled && takeover.route_mode == RouteMode::Proxy {
                crate::proxy::providers::validate_module_proxy_capability(
                    &app_type_enum,
                    &provider,
                )?;
            }
            takeover.takeover_enabled && takeover.route_mode == RouteMode::Proxy
        } else {
            false
        };

        let c2b_proxy_context = if c2b_proxy_managed {
            let adapter = Self::snapshot_adapter_for_app(&app_type_enum)?
                .ok_or_else(|| format!("{} 尚无精确快照 adapter", app_type_enum.as_str()))?;
            let live_before =
                self.capture_transient_live_snapshot(&app_type_enum, adapter.as_ref())?;
            // Resolve URL/config prerequisites before current-pointer side effects.
            let (proxy_origin, _) = self.build_proxy_urls().await?;
            Some((adapter, live_before, proxy_origin))
        } else {
            None
        };

        let logical_target_changed =
            crate::settings::get_effective_current_provider(&self.db, &app_type_enum)
                .map_err(|e| format!("读取当前供应商失败: {e}"))?
                .as_deref()
                != Some(provider_id);
        let c2b_previous_pointers = if Self::is_c2b_takeover_app(&app_type_enum) {
            Some((
                self.db
                    .get_current_provider(app_type_enum.as_str())
                    .map_err(|e| format!("读取原 DB current 失败: {e}"))?,
                crate::settings::get_current_provider(&app_type_enum),
            ))
        } else {
            None
        };

        // Option A (C2a): the restore snapshot in proxy_live_backup is the IMMUTABLE
        // first-open capture. Hot-switch under ProxyManaged only refreshes the
        // proxy-safe live display (Claude/Codex labels follow the selected provider
        // while endpoints stay local). It never rewrites the snapshot slot — the
        // "managed expected" baseline is C3's in-memory concern.
        let live_taken_over = self.detect_takeover_in_live_config_for_app(&app_type_enum);

        self.db
            .set_current_provider(app_type_enum.as_str(), provider_id)
            .map_err(|e| format!("更新当前供应商失败: {e}"))?;
        if let Err(error) = crate::settings::set_current_provider(&app_type_enum, Some(provider_id))
        {
            if let Some((previous_db_current, previous_settings_current)) =
                c2b_previous_pointers.as_ref()
            {
                return Err(self.rollback_c2b_current_provider_pointers(
                    &app_type_enum,
                    previous_db_current.as_deref(),
                    previous_settings_current.as_deref(),
                    &format!("更新本地当前供应商失败: {error}"),
                ));
            }
            return Err(format!("更新本地当前供应商失败: {error}"));
        }

        if matches!(app_type_enum, AppType::Claude) {
            self.sync_claude_live_from_provider_while_proxy_active(&provider)
                .await?;
        } else if live_taken_over && matches!(app_type_enum, AppType::Codex) {
            // Even when the gateway is stopped, if live still carries the proxy
            // placeholder we refresh the Codex-visible provider label so the
            // client menu tracks the selected provider.
            self.sync_codex_live_from_provider_while_proxy_active(&provider)
                .await?;
        } else if let Some((adapter, live_before, proxy_origin)) = c2b_proxy_context.as_ref() {
            // 四模块的 ProxyManaged 写入由 proxy_config SSOT 决定，不能依赖 live
            // 占位符检测：外部格式漂移也不得退回真实上游 writer。只重写当前模块
            // 的命名空间 fragment，immutable first-open snapshot 保持不变。
            if let Err(error) = self.write_c2b_proxy_live(&app_type_enum, proxy_origin) {
                if let Some((previous_db_current, previous_settings_current)) =
                    c2b_previous_pointers.as_ref()
                {
                    return Err(self.rollback_c2b_hot_switch(
                        &app_type_enum,
                        previous_db_current.as_deref(),
                        previous_settings_current.as_deref(),
                        adapter.as_ref(),
                        live_before,
                        &format!("刷新代理受管 Live 配置失败: {error}"),
                    ));
                }
                return Err(format!("刷新代理受管 Live 配置失败: {error}"));
            }
        }

        if let Some(server) = self.server.read().await.as_ref() {
            server
                .set_active_target(app_type_enum.as_str(), &provider.id, &provider.name)
                .await;
        }

        Ok(HotSwitchOutcome {
            logical_target_changed,
        })
    }

    #[cfg(test)]
    async fn lock_switch_for_test(&self, app_type: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.switch_locks.lock_for_app(app_type).await
    }

    fn preserve_codex_mcp_servers_from_existing_config(
        target_settings: &mut Value,
        existing_config: &Value,
    ) -> Result<(), String> {
        let target_obj = target_settings
            .as_object_mut()
            .ok_or_else(|| "Codex 备份必须是 JSON 对象".to_string())?;

        let target_config = target_obj
            .get("config")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut target_doc = if target_config.trim().is_empty() {
            toml_edit::DocumentMut::new()
        } else {
            target_config
                .parse::<toml_edit::DocumentMut>()
                .map_err(|e| format!("解析新的 Codex config.toml 失败: {e}"))?
        };

        let existing_config = existing_config
            .get("config")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if existing_config.trim().is_empty() {
            target_obj.insert("config".to_string(), json!(target_doc.to_string()));
            return Ok(());
        }

        let existing_doc = existing_config
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| format!("解析现有 Codex 备份失败: {e}"))?;

        if let Some(existing_mcp_servers) = existing_doc.get("mcp_servers") {
            match target_doc.get_mut("mcp_servers") {
                Some(target_mcp_servers) => {
                    if let (Some(target_table), Some(existing_table)) = (
                        target_mcp_servers.as_table_like_mut(),
                        existing_mcp_servers.as_table_like(),
                    ) {
                        for (server_id, server_item) in existing_table.iter() {
                            if target_table.get(server_id).is_none() {
                                target_table.insert(server_id, server_item.clone());
                            }
                        }
                    } else {
                        log::warn!(
                            "Codex config contains a non-table mcp_servers section; skipping MCP merge"
                        );
                    }
                }
                None => {
                    target_doc["mcp_servers"] = existing_mcp_servers.clone();
                }
            }
        }

        target_obj.insert("config".to_string(), json!(target_doc.to_string()));
        Ok(())
    }

    fn preserve_codex_oauth_auth_in_backup(
        target_settings: &mut Value,
        existing_backup: &Value,
    ) -> Result<(), String> {
        if !crate::settings::preserve_codex_official_auth_on_switch() {
            return Ok(());
        }

        let Some(existing_auth) = existing_backup
            .get("auth")
            .filter(|auth| crate::codex_config::codex_auth_has_oauth_login_material(auth))
            .cloned()
        else {
            return Ok(());
        };

        let Some(target_obj) = target_settings.as_object_mut() else {
            return Ok(());
        };

        let provider_auth = target_obj.get("auth").cloned().unwrap_or_else(|| json!({}));
        if let Some(config_text) = target_obj.get("config").and_then(|value| value.as_str()) {
            let live_config = crate::codex_config::prepare_codex_provider_live_config(
                &provider_auth,
                config_text,
            )
            .map_err(|e| format!("更新 Codex 备份配置失败: {e}"))?;
            target_obj.insert("config".to_string(), json!(live_config));
        }
        target_obj.insert("auth".to_string(), existing_auth);

        Ok(())
    }

    /// 代理模式下切换供应商（热切换，并按需刷新代理安全的 Live 显示字段）
    pub async fn switch_proxy_target(
        &self,
        app_type: &str,
        provider_id: &str,
    ) -> Result<(), String> {
        let outcome = self.hot_switch_provider(app_type, provider_id).await?;

        if outcome.logical_target_changed {
            log::info!("代理模式：已切换 {app_type} 的目标供应商为 {provider_id}");
        } else {
            log::debug!("代理模式：{app_type} 已对齐到目标供应商 {provider_id}");
        }
        Ok(())
    }

    // ==================== Live 配置读写辅助方法 ====================

    /// 更新 TOML 字符串中的 base_url（委托给 codex_config 共享实现）
    fn update_toml_base_url(toml_str: &str, new_url: &str) -> String {
        crate::codex_config::update_codex_toml_field(toml_str, "base_url", new_url)
            .unwrap_or_else(|_| toml_str.to_string())
    }

    /// 接管 Codex 时，本地客户端必须继续以 Responses wire API 访问代理。
    /// 真实上游是否走 Chat Completions 由 provider 配置决定，并在代理内部转换。
    fn apply_codex_proxy_toml_config_for_provider(
        toml_str: &str,
        proxy_url: &str,
        provider: Option<&Provider>,
    ) -> String {
        let updated = Self::update_toml_base_url(toml_str, proxy_url);
        let mut updated =
            crate::codex_config::update_codex_toml_field(&updated, "wire_api", "responses")
                .unwrap_or(updated);

        if let Some(upstream_model) =
            provider.and_then(crate::proxy::providers::codex_provider_upstream_model)
        {
            updated =
                crate::codex_config::update_codex_toml_field(&updated, "model", &upstream_model)
                    .unwrap_or(updated);
        }

        updated
    }

    fn attach_codex_model_catalog_from_provider(
        live_config: &mut Value,
        provider: Option<&Provider>,
    ) {
        let Some(provider) = provider else {
            return;
        };

        let model_catalog = provider
            .settings_config
            .get("modelCatalog")
            .cloned()
            .unwrap_or_else(|| json!({ "models": [] }));

        if let Some(root) = live_config.as_object_mut() {
            root.insert("modelCatalog".to_string(), model_catalog);
        }
    }

    fn read_claude_live(&self) -> Result<Value, String> {
        let path = get_claude_settings_path();
        if !path.exists() {
            return Err("Claude 配置文件不存在".to_string());
        }

        let mut value: Value =
            read_json_file(&path).map_err(|e| format!("读取 Claude 配置失败: {e}"))?;

        if value.is_null() {
            value = json!({});
        }

        if !value.is_object() {
            let kind = match &value {
                Value::Null => "null",
                Value::Bool(_) => "boolean",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            return Err(format!(
                "Claude 配置文件格式错误：根节点必须是 JSON 对象（当前为 {kind}），路径: {}",
                path.display()
            ));
        }

        Ok(value)
    }

    fn write_claude_live(&self, config: &Value) -> Result<(), String> {
        let path = get_claude_settings_path();
        let settings = crate::services::provider::sanitize_claude_settings_for_live(config);
        write_json_file(&path, &settings).map_err(|e| format!("写入 Claude 配置失败: {e}"))
    }

    fn read_codex_live(&self) -> Result<Value, String> {
        crate::codex_config::read_codex_live_settings()
            .map_err(|e| format!("读取 Codex Live 配置失败: {e}"))
    }

    fn write_codex_live(&self, config: &Value) -> Result<(), String> {
        self.write_codex_live_verbatim(config)
    }

    fn write_codex_live_for_provider(
        &self,
        config: &Value,
        provider: Option<&Provider>,
    ) -> Result<(), String> {
        let Some(provider) = provider else {
            if crate::settings::preserve_codex_official_auth_on_switch() {
                if let (Some(auth), Some(config_str)) = (
                    config.get("auth"),
                    config.get("config").and_then(|v| v.as_str()),
                ) {
                    if auth.get("OPENAI_API_KEY").and_then(|v| v.as_str())
                        == Some(PROXY_TOKEN_PLACEHOLDER)
                    {
                        let live_config = crate::codex_config::prepare_codex_provider_live_config(
                            auth, config_str,
                        )
                        .map_err(|e| format!("写入 Codex 配置失败: {e}"))?;
                        crate::codex_config::write_codex_live_config_atomic(Some(&live_config))
                            .map_err(|e| format!("写入 Codex 配置失败: {e}"))?;
                        return Ok(());
                    }
                }
            }

            return self.write_codex_live_verbatim(config);
        };

        let auth = config
            .get("auth")
            .ok_or_else(|| "Codex 配置缺少 auth 字段".to_string())?;
        let config_str = config.get("config").and_then(|v| v.as_str());
        let profile = crate::proxy::providers::resolve_codex_catalog_tool_profile(provider);

        crate::codex_config::write_codex_provider_live_with_catalog(
            config,
            provider.category.as_deref(),
            auth,
            config_str,
            profile,
        )
        .map_err(|e| format!("写入 Codex 配置失败: {e}"))
    }

    fn codex_auth_has_proxy_placeholder(auth: &Value) -> bool {
        auth.get("OPENAI_API_KEY").and_then(|v| v.as_str()) == Some(PROXY_TOKEN_PLACEHOLDER)
    }

    fn write_codex_takeover_live_for_provider(
        &self,
        config: &Value,
        provider: Option<&Provider>,
    ) -> Result<(), String> {
        if crate::settings::preserve_codex_official_auth_on_switch() {
            if let Some(auth) = config
                .get("auth")
                .filter(|auth| Self::codex_auth_has_proxy_placeholder(auth))
            {
                let config_str = config.get("config").and_then(|v| v.as_str()).unwrap_or("");
                let profile = provider
                    .map(crate::proxy::providers::resolve_codex_catalog_tool_profile)
                    .unwrap_or(crate::codex_config::CodexCatalogToolProfile::ProxyChat);
                let prepared_config =
                    crate::codex_config::prepare_codex_live_config_text_with_optional_catalog(
                        config, config_str, profile,
                    )
                    .map_err(|e| format!("写入 Codex 配置失败: {e}"))?;
                let live_config =
                    crate::codex_config::prepare_codex_provider_live_config(auth, &prepared_config)
                        .map_err(|e| format!("写入 Codex 配置失败: {e}"))?;
                crate::codex_config::write_codex_live_config_atomic(Some(&live_config))
                    .map_err(|e| format!("写入 Codex 配置失败: {e}"))?;
                return Ok(());
            }
        }

        self.write_codex_live_for_provider(config, provider)
    }

    fn write_codex_live_verbatim(&self, config: &Value) -> Result<(), String> {
        crate::codex_config::write_codex_live_verbatim(config)
            .map_err(|e| format!("写入 Codex 配置失败: {e}"))
    }

    fn read_gemini_live(&self) -> Result<Value, String> {
        use crate::gemini_config::{env_to_json, get_gemini_env_path, read_gemini_env};

        let env_path = get_gemini_env_path();
        if !env_path.exists() {
            return Err("Gemini .env 文件不存在".to_string());
        }

        let env_map = read_gemini_env().map_err(|e| format!("读取 Gemini env 失败: {e}"))?;
        Ok(env_to_json(&env_map))
    }

    fn write_gemini_live(&self, config: &Value) -> Result<(), String> {
        use crate::gemini_config::{json_to_env, write_gemini_env_atomic};

        let env_map = json_to_env(config).map_err(|e| format!("转换 Gemini 配置失败: {e}"))?;
        write_gemini_env_atomic(&env_map).map_err(|e| format!("写入 Gemini env 失败: {e}"))?;
        Ok(())
    }

    // ==================== 原有方法 ====================

    /// 获取服务器状态
    pub async fn get_status(&self) -> Result<ProxyStatus, String> {
        if let Some(server) = self.server.read().await.as_ref() {
            Ok(server.get_status().await)
        } else {
            // 服务器未运行时返回默认状态
            Ok(ProxyStatus {
                running: false,
                ..Default::default()
            })
        }
    }

    /// 获取代理配置
    pub async fn get_config(&self) -> Result<ProxyConfig, String> {
        self.db
            .get_proxy_config()
            .await
            .map_err(|e| format!("获取代理配置失败: {e}"))
    }

    /// 更新代理配置
    pub async fn update_config(&self, config: &ProxyConfig) -> Result<(), String> {
        // 记录旧配置用于判定是否需要重启
        let previous = self
            .db
            .get_proxy_config()
            .await
            .map_err(|e| format!("获取代理配置失败: {e}"))?;

        // 保存到数据库（保持 live_takeover_active 状态不变）
        let mut new_config = config.clone();
        new_config.live_takeover_active = previous.live_takeover_active;

        self.db
            .update_proxy_config(new_config.clone())
            .await
            .map_err(|e| format!("保存代理配置失败: {e}"))?;

        // 检查服务器当前状态
        let mut server_guard = self.server.write().await;
        if server_guard.is_none() {
            return Ok(());
        }

        // 判断是否需要重启（地址或端口变更）
        let require_restart = new_config.listen_address != previous.listen_address
            || new_config.listen_port != previous.listen_port;

        if require_restart {
            if let Some(server) = server_guard.take() {
                server
                    .stop()
                    .await
                    .map_err(|e| format!("重启前停止代理服务器失败: {e}"))?;
            }

            let app_handle = self.app_handle.read().await.clone();
            let new_server = ProxyServer::new(new_config.clone(), self.db.clone(), app_handle);
            let info = new_server
                .start()
                .await
                .map_err(|e| format!("重启代理服务器失败: {e}"))?;
            if let Err(e) = self
                .persist_ephemeral_listen_port_if_needed(&new_config, info.port)
                .await
            {
                let _ = new_server.stop().await;
                return Err(e);
            }

            *server_guard = Some(new_server);
            log::info!("代理配置已更新，服务器已自动重启应用最新配置");

            // 网关地址变更后，只重写 SSOT 明确为 takeover+proxy 的三模块；逐模块加锁并
            // 使用严格 writer，避免 best-effort 静默留下旧端口或与 provider 写入竞态。
            drop(server_guard);
            let mut updated_any = false;
            for app in AppType::all() {
                let _guard = self.switch_locks.lock_for_app(app.as_str()).await;
                let config = self
                    .db
                    .get_proxy_config_for_app(app.as_str())
                    .await
                    .map_err(|e| format!("读取 {} 接管状态失败: {e}", app.as_str()))?;
                if config.takeover_enabled && config.route_mode == RouteMode::Proxy {
                    self.takeover_live_config_strict(&app).await?;
                    updated_any = true;
                }
            }
            if updated_any {
                log::info!("已同步更新 Live 配置中的代理地址");
            }

            return Ok(());
        } else if let Some(server) = server_guard.as_ref() {
            server.apply_runtime_config(&new_config).await;
            log::info!("代理配置已实时应用，无需重启代理服务器");
        }

        Ok(())
    }

    /// 检查服务器是否正在运行
    pub async fn is_running(&self) -> bool {
        self.server.read().await.is_some()
    }

    /// 热更新熔断器配置
    ///
    /// 如果代理服务器正在运行，将新配置应用到所有已创建的熔断器实例
    pub async fn update_circuit_breaker_configs(
        &self,
        config: crate::proxy::CircuitBreakerConfig,
    ) -> Result<(), String> {
        if let Some(server) = self.server.read().await.as_ref() {
            server.update_circuit_breaker_configs(config).await;
            log::info!("已热更新运行中的熔断器配置");
        } else {
            log::debug!("代理服务器未运行，熔断器配置将在下次启动时生效");
        }
        Ok(())
    }

    /// 热更新指定应用的熔断器配置
    pub async fn update_circuit_breaker_config_for_app(
        &self,
        app_type: &str,
        config: crate::proxy::CircuitBreakerConfig,
    ) -> Result<(), String> {
        if let Some(server) = self.server.read().await.as_ref() {
            server
                .update_circuit_breaker_config_for_app(app_type, config)
                .await;
            log::info!("已热更新 {app_type} 运行中的熔断器配置");
        } else {
            log::debug!("{app_type} 熔断器配置将在下次代理启动时生效");
        }
        Ok(())
    }

    /// 重置指定 Provider 的熔断器
    ///
    /// 如果代理服务器正在运行，立即重置内存中的熔断器状态
    pub async fn reset_provider_circuit_breaker(
        &self,
        provider_id: &str,
        app_type: &str,
    ) -> Result<(), String> {
        if let Some(server) = self.server.read().await.as_ref() {
            server
                .reset_provider_circuit_breaker(provider_id, app_type)
                .await;
            log::info!("已重置 Provider {provider_id} (app: {app_type}) 的熔断器");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ProviderMeta;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct TempHome {
        #[allow(dead_code)]
        dir: TempDir,
        original_home: Option<String>,
        original_userprofile: Option<String>,
        original_test_home: Option<String>,
        original_localappdata: Option<String>,
        original_hermes_home: Option<String>,
        original_opencode_db: Option<String>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp home");
            let original_home = env::var("HOME").ok();
            let original_userprofile = env::var("USERPROFILE").ok();
            let original_test_home = env::var("AGENT_SWITCH_TEST_HOME").ok();
            let original_localappdata = env::var("LOCALAPPDATA").ok();
            let original_hermes_home = env::var("HERMES_HOME").ok();
            let original_opencode_db = env::var("OPENCODE_DB").ok();

            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());
            env::set_var("AGENT_SWITCH_TEST_HOME", dir.path());
            env::set_var("LOCALAPPDATA", dir.path().join("AppData").join("Local"));
            env::set_var("HERMES_HOME", dir.path().join("hermes"));
            env::set_var("OPENCODE_DB", dir.path().join("opencode.db"));

            Self {
                dir,
                original_home,
                original_userprofile,
                original_test_home,
                original_localappdata,
                original_hermes_home,
                original_opencode_db,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }

            match &self.original_userprofile {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }

            match &self.original_test_home {
                Some(value) => env::set_var("AGENT_SWITCH_TEST_HOME", value),
                None => env::remove_var("AGENT_SWITCH_TEST_HOME"),
            }
            match &self.original_localappdata {
                Some(value) => env::set_var("LOCALAPPDATA", value),
                None => env::remove_var("LOCALAPPDATA"),
            }
            match &self.original_hermes_home {
                Some(value) => env::set_var("HERMES_HOME", value),
                None => env::remove_var("HERMES_HOME"),
            }
            match &self.original_opencode_db {
                Some(value) => env::set_var("OPENCODE_DB", value),
                None => env::remove_var("OPENCODE_DB"),
            }
        }
    }

    fn assert_env_str(env: &Map<String, Value>, key: &str, expected: Option<&str>) {
        assert_eq!(env.get(key).and_then(|value| value.as_str()), expected);
    }

    async fn use_ephemeral_proxy_port(db: &Arc<Database>) {
        let mut proxy_config = db.get_proxy_config().await.expect("get test proxy config");
        proxy_config.listen_port = 0;
        db.update_proxy_config(proxy_config)
            .await
            .expect("set test proxy config to an ephemeral port");
    }

    async fn running_codex_base_url(service: &ProxyService) -> String {
        let status = service.get_status().await.expect("get proxy status");
        format!("http://127.0.0.1:{}/v1", status.port)
    }

    fn seed_codex_model_template() {
        let codex_dir = crate::codex_config::get_codex_config_dir();
        std::fs::create_dir_all(&codex_dir).expect("create codex dir");
        std::fs::write(
            codex_dir.join("models_cache.json"),
            serde_json::to_string(&serde_json::json!({
                "models": [{
                    "slug": "gpt-5.5",
                    "display_name": "GPT-5.5",
                    "model_messages": { "instructions_template": "t" },
                    "additional_speed_tiers": [],
                    "context_window": 128000
                }]
            }))
            .expect("serialize models_cache"),
        )
        .expect("write models_cache.json");
    }

    fn c2b_test_provider(app_type: &AppType, supported_proxy_protocol: bool) -> Provider {
        let id = "c2b-provider";
        let settings = match app_type {
            AppType::ClaudeDesktop => json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://upstream.example/v1",
                    "ANTHROPIC_AUTH_TOKEN": "upstream-key"
                }
            }),
            AppType::OpenCode => json!({
                "npm": if supported_proxy_protocol { "@ai-sdk/openai-compatible" } else { "@vendor/unknown" },
                "options": {
                    "baseURL": "https://upstream.example/v1",
                    "apiKey": "upstream-key"
                },
                "models": { "test-model": { "name": "Test" } }
            }),
            AppType::OpenClaw => json!({
                "baseUrl": "https://upstream.example/v1",
                "apiKey": "upstream-key",
                "api": if supported_proxy_protocol { "openai-completions" } else { "unknown-api" },
                "models": [{ "id": "test-model", "name": "Test" }]
            }),
            AppType::Hermes => json!({
                "base_url": "https://upstream.example/v1",
                "api_key": "upstream-key",
                "api_mode": if supported_proxy_protocol { "chat_completions" } else { "bedrock_converse" },
                "models": [{ "id": "test-model", "name": "Test" }]
            }),
            _ => panic!("not a C2b test app: {app_type:?}"),
        };
        let mut provider = Provider::with_id(
            id.to_string(),
            format!("{} Test", app_type.as_str()),
            settings,
            None,
        );
        if matches!(app_type, AppType::ClaudeDesktop) {
            provider.meta = Some(ProviderMeta {
                claude_desktop_mode: Some(crate::provider::ClaudeDesktopMode::Direct),
                api_format: Some(
                    if supported_proxy_protocol {
                        "anthropic"
                    } else {
                        "bedrock_converse"
                    }
                    .to_string(),
                ),
                claude_desktop_model_routes: std::collections::HashMap::from([(
                    "claude-sonnet-4-6".to_string(),
                    crate::provider::ClaudeDesktopModelRoute {
                        model: "claude-sonnet-4-6".to_string(),
                        label_override: Some("Claude Sonnet".to_string()),
                        supports_1m: Some(false),
                    },
                )]),
                ..Default::default()
            });
        }
        provider
    }

    fn seed_c2b_current_provider(db: &Database, app_type: &AppType, provider: &Provider) {
        db.save_provider(app_type.as_str(), provider)
            .expect("save C2b provider");
        db.set_current_provider(app_type.as_str(), &provider.id)
            .expect("set DB current C2b provider");
        crate::settings::set_current_provider(app_type, Some(&provider.id))
            .expect("set settings current C2b provider");
    }

    fn c2b_live_target_paths(app_type: &AppType) -> Vec<std::path::PathBuf> {
        match app_type {
            AppType::ClaudeDesktop => crate::claude_desktop_config::snapshot_target_paths()
                .expect("resolve Claude Desktop test paths")
                .into_iter()
                .map(|(_, path)| path)
                .collect(),
            AppType::OpenCode => vec![crate::opencode_config::get_opencode_config_path()],
            AppType::OpenClaw => vec![crate::openclaw_config::get_openclaw_config_path()],
            AppType::Hermes => vec![crate::hermes_config::get_hermes_config_path()],
            _ => panic!("not a C2b test app: {app_type:?}"),
        }
    }

    fn seed_c2b_original_live(
        app_type: &AppType,
        provider: &Provider,
    ) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        let targets = c2b_live_target_paths(app_type);
        let contents = match app_type {
            AppType::ClaudeDesktop => vec![
                br#"{
  "deploymentMode": "1p",
  "custom": "normal"
}
"#
                .to_vec(),
                br#"{
  "deploymentMode": "1p",
  "custom": "threep"
}
"#
                .to_vec(),
                br#"{
  "userProfile": true
}
"#
                .to_vec(),
                br#"{
  "entries": [{"id": "user", "name": "User"}],
  "appliedId": "user"
}
"#
                .to_vec(),
            ],
            AppType::OpenCode => vec![format!(
                "{{\n  // preserve this exact OpenCode comment\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"provider\": {{\n    \"{}\": {}\n  }},\n  \"plugin\": [\"user-plugin\"]\n}}\n",
                provider.id, provider.settings_config
            )
            .into_bytes()],
            AppType::OpenClaw => vec![format!(
                "{{\n  // preserve this exact OpenClaw comment\n  models: {{\n    mode: 'merge',\n    providers: {{\n      '{}': {}\n    }}\n  }},\n  custom: true\n}}\n",
                provider.id, provider.settings_config
            )
            .into_bytes()],
            AppType::Hermes => vec![format!(
                "# preserve this exact Hermes comment\ncustom_providers:\n  - name: {}\n    base_url: https://upstream.example/v1\n    api_key: upstream-key\n    api_mode: {}\n    model: test-model\n    models:\n      test-model:\n        context_length: 1000\ncustom_section:\n  keep: true\n",
                provider.id,
                provider
                    .settings_config
                    .get("api_mode")
                    .and_then(Value::as_str)
                    .unwrap_or("chat_completions")
            )
            .into_bytes()],
            _ => unreachable!(),
        };
        assert_eq!(targets.len(), contents.len());
        targets
            .into_iter()
            .zip(contents)
            .map(|(path, bytes)| {
                std::fs::create_dir_all(path.parent().expect("live target parent"))
                    .expect("create live target parent");
                std::fs::write(&path, &bytes).expect("seed C2b live target");
                (path, bytes)
            })
            .collect()
    }

    fn read_c2b_live_fragment(app_type: &AppType, provider_id: &str) -> Value {
        match app_type {
            AppType::ClaudeDesktop => {
                let profile = crate::claude_desktop_config::snapshot_target_paths()
                    .expect("resolve Claude Desktop paths")
                    .into_iter()
                    .find_map(|(id, path)| (id == "profile").then_some(path))
                    .expect("Claude Desktop profile target");
                read_json_file(&profile).expect("read Claude Desktop profile")
            }
            AppType::OpenCode => crate::opencode_config::get_providers()
                .expect("read OpenCode providers")
                .remove(provider_id)
                .expect("OpenCode live provider"),
            AppType::OpenClaw => crate::openclaw_config::get_provider(provider_id)
                .expect("read OpenClaw provider")
                .expect("OpenClaw live provider"),
            AppType::Hermes => crate::hermes_config::get_provider(provider_id)
                .expect("read Hermes provider")
                .expect("Hermes live provider"),
            _ => unreachable!(),
        }
    }

    fn c2b_live_endpoint_key_protocol(
        app_type: &AppType,
        provider_id: &str,
    ) -> (String, String, Option<String>) {
        let fragment = read_c2b_live_fragment(app_type, provider_id);
        let (base_url, api_key, protocol) = match app_type {
            AppType::ClaudeDesktop => (
                fragment.get("inferenceGatewayBaseUrl"),
                fragment.get("inferenceGatewayApiKey"),
                None,
            ),
            AppType::OpenCode => (
                fragment.pointer("/options/baseURL"),
                fragment.pointer("/options/apiKey"),
                fragment.get("npm"),
            ),
            AppType::OpenClaw => (
                fragment.get("baseUrl"),
                fragment.get("apiKey"),
                fragment.get("api"),
            ),
            AppType::Hermes => (
                fragment.get("base_url"),
                fragment.get("api_key"),
                fragment.get("api_mode"),
            ),
            _ => unreachable!(),
        };
        (
            base_url
                .and_then(Value::as_str)
                .expect("C2b live base URL")
                .to_string(),
            api_key
                .and_then(Value::as_str)
                .expect("C2b live API key")
                .to_string(),
            protocol.and_then(Value::as_str).map(str::to_string),
        )
    }

    fn expected_c2b_proxy_base_url(app_type: &AppType, port: u16) -> String {
        let suffix = match app_type {
            AppType::ClaudeDesktop => "claude-desktop",
            AppType::OpenCode => "opencode/v1",
            AppType::OpenClaw => "openclaw/v1",
            AppType::Hermes => "hermes/v1",
            _ => unreachable!(),
        };
        format!("http://127.0.0.1:{port}/{suffix}")
    }

    async fn assert_c2b_route_lifecycle(app_type: AppType) {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        use_ephemeral_proxy_port(&db).await;
        let service = ProxyService::new(db.clone());
        let provider = c2b_test_provider(&app_type, true);
        seed_c2b_current_provider(&db, &app_type, &provider);
        let originals = seed_c2b_original_live(&app_type, &provider);

        let opencode_db_before = if matches!(app_type, AppType::OpenCode) {
            let path = crate::opencode_config::get_opencode_db_path();
            std::fs::create_dir_all(path.parent().expect("OpenCode DB parent"))
                .expect("create OpenCode DB parent");
            std::fs::write(&path, b"sqlite-session-usage-must-stay-untouched")
                .expect("seed OpenCode DB");
            Some((
                path.clone(),
                std::fs::read(&path).expect("read OpenCode DB"),
                std::fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .expect("OpenCode DB mtime"),
            ))
        } else {
            None
        };

        service
            .set_takeover_for_app(app_type.as_str(), true, RouteMode::Direct)
            .await
            .expect("enable C2b direct takeover");
        assert!(!service.is_running().await, "direct must not start gateway");
        let (direct_url, direct_key, direct_protocol) =
            c2b_live_endpoint_key_protocol(&app_type, &provider.id);
        assert_eq!(direct_url, "https://upstream.example/v1");
        assert_eq!(direct_key, "upstream-key");

        let immutable_snapshot = db
            .get_live_backup(app_type.as_str())
            .await
            .expect("read C2b snapshot")
            .expect("C2b snapshot exists")
            .original_config;

        service
            .switch_route_mode(app_type.as_str(), RouteMode::Proxy)
            .await
            .expect("switch C2b module to proxy");
        let status = service.get_status().await.expect("proxy status");
        assert!(status.running);
        let (proxy_url, proxy_key, proxy_protocol) =
            c2b_live_endpoint_key_protocol(&app_type, &provider.id);
        assert_eq!(
            proxy_url,
            expected_c2b_proxy_base_url(&app_type, status.port)
        );
        assert_eq!(
            proxy_key,
            db.get_setting("claude_desktop_gateway_token")
                .expect("read gateway token")
                .expect("gateway token exists")
        );
        assert_eq!(
            proxy_protocol, direct_protocol,
            "protocol field must be preserved"
        );
        assert!(service
            .live_takeover_matches_current_proxy(&app_type)
            .await
            .expect("check C2b proxy live"));
        assert_eq!(
            db.get_live_backup(app_type.as_str())
                .await
                .expect("read immutable C2b snapshot")
                .expect("snapshot still exists")
                .original_config,
            immutable_snapshot,
            "direct/proxy switch must not overwrite first-open snapshot"
        );

        service
            .set_takeover_for_app(app_type.as_str(), false, RouteMode::Proxy)
            .await
            .expect("disable C2b takeover");
        for (path, original) in originals {
            assert_eq!(
                std::fs::read(&path).expect("read restored C2b target"),
                original,
                "{} target must restore byte-for-byte: {}",
                app_type.as_str(),
                path.display()
            );
        }
        assert!(
            !db.get_proxy_config_for_app(app_type.as_str())
                .await
                .expect("read released state")
                .takeover_enabled
        );
        assert!(db
            .get_live_backup(app_type.as_str())
            .await
            .expect("read released snapshot")
            .is_none());

        if let Some((path, bytes, modified)) = opencode_db_before {
            assert_eq!(
                std::fs::read(&path).expect("read untouched OpenCode DB"),
                bytes
            );
            assert_eq!(
                std::fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .expect("OpenCode DB mtime after takeover"),
                modified,
                "OpenCode takeover must not touch opencode.db"
            );
        }

        service.stop().await.expect("stop independent gateway");
    }

    async fn assert_c2b_missing_target_deleted(app_type: AppType) {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        use_ephemeral_proxy_port(&db).await;
        let service = ProxyService::new(db.clone());
        let provider = c2b_test_provider(&app_type, true);
        seed_c2b_current_provider(&db, &app_type, &provider);
        let targets = c2b_live_target_paths(&app_type);
        assert!(targets.iter().all(|path| !path.exists()));

        service
            .set_takeover_for_app(app_type.as_str(), true, RouteMode::Direct)
            .await
            .expect("enable missing-target direct takeover");
        assert!(
            targets.iter().all(|path| path.exists()),
            "direct takeover should create every managed target for {}",
            app_type.as_str()
        );
        service
            .switch_route_mode(app_type.as_str(), RouteMode::Proxy)
            .await
            .expect("switch missing-target module to proxy");
        service
            .set_takeover_for_app(app_type.as_str(), false, RouteMode::Proxy)
            .await
            .expect("disable missing-target takeover");

        assert!(
            targets.iter().all(|path| !path.exists()),
            "existed=false targets must be deleted for {}",
            app_type.as_str()
        );
        assert!(db
            .get_live_backup(app_type.as_str())
            .await
            .expect("read missing-target snapshot")
            .is_none());
        service.stop().await.expect("stop independent gateway");
    }

    async fn assert_c2b_unsupported_proxy_is_atomic(app_type: AppType) {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        use_ephemeral_proxy_port(&db).await;
        let service = ProxyService::new(db.clone());
        let provider = c2b_test_provider(&app_type, false);
        seed_c2b_current_provider(&db, &app_type, &provider);
        let originals = seed_c2b_original_live(&app_type, &provider);

        let error = service
            .set_takeover_for_app(app_type.as_str(), true, RouteMode::Proxy)
            .await
            .expect_err("unsupported proxy protocol must be rejected");
        assert!(
            error.contains("不支持") || error.contains("能力矩阵"),
            "unexpected capability error for {}: {error}",
            app_type.as_str()
        );
        assert!(
            !service.is_running().await,
            "gate must run before gateway start"
        );
        let config = db
            .get_proxy_config_for_app(app_type.as_str())
            .await
            .expect("read rejected takeover state");
        assert!(!config.takeover_enabled);
        assert_eq!(config.route_mode, RouteMode::Direct);
        assert!(db
            .get_live_backup(app_type.as_str())
            .await
            .expect("read rejected snapshot")
            .is_none());
        for (path, original) in &originals {
            assert_eq!(
                std::fs::read(path).expect("read unchanged live after rejection"),
                *original,
                "capability rejection must not mutate {}",
                path.display()
            );
        }

        service
            .set_takeover_for_app(app_type.as_str(), true, RouteMode::Direct)
            .await
            .expect("same unsupported provider must remain usable in direct mode");
        let direct = db
            .get_proxy_config_for_app(app_type.as_str())
            .await
            .expect("read direct state");
        assert!(direct.takeover_enabled);
        assert_eq!(direct.route_mode, RouteMode::Direct);
        service
            .set_takeover_for_app(app_type.as_str(), false, RouteMode::Direct)
            .await
            .expect("disable direct after capability test");
        for (path, original) in originals {
            assert_eq!(
                std::fs::read(path).expect("read exact restore after direct"),
                original
            );
        }
    }

    #[tokio::test]
    #[serial]
    async fn c2b_missing_targets_are_deleted_for_all_four_modules() {
        for app_type in [
            AppType::ClaudeDesktop,
            AppType::OpenCode,
            AppType::OpenClaw,
            AppType::Hermes,
        ] {
            assert_c2b_missing_target_deleted(app_type).await;
        }
    }

    #[tokio::test]
    #[serial]
    async fn c2b_unsupported_proxy_protocols_are_rejected_before_mutation() {
        for app_type in [AppType::OpenCode, AppType::OpenClaw, AppType::Hermes] {
            assert_c2b_unsupported_proxy_is_atomic(app_type).await;
        }
    }

    #[tokio::test]
    #[serial]
    async fn c2b_module_takeover_isolated_from_other_three_live_targets() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        use_ephemeral_proxy_port(&db).await;
        let service = ProxyService::new(db.clone());
        let mut originals = Vec::new();
        for app_type in [
            AppType::ClaudeDesktop,
            AppType::OpenCode,
            AppType::OpenClaw,
            AppType::Hermes,
        ] {
            let provider = c2b_test_provider(&app_type, true);
            seed_c2b_current_provider(&db, &app_type, &provider);
            originals.push((
                app_type.clone(),
                seed_c2b_original_live(&app_type, &provider),
            ));
        }

        service
            .set_takeover_for_app("opencode", true, RouteMode::Proxy)
            .await
            .expect("enable isolated OpenCode proxy takeover");

        for (app_type, targets) in &originals {
            if matches!(app_type, AppType::OpenCode) {
                continue;
            }
            for (path, original) in targets {
                assert_eq!(
                    std::fs::read(path).expect("read isolated target"),
                    *original,
                    "OpenCode takeover must not mutate {} target {}",
                    app_type.as_str(),
                    path.display()
                );
            }
        }
        let status = service
            .get_takeover_status()
            .await
            .expect("takeover status");
        for app_type in AppType::all() {
            assert_eq!(
                status.for_app(&app_type).takeover_enabled,
                matches!(app_type, AppType::OpenCode),
                "only OpenCode should be taken over"
            );
        }

        service
            .set_takeover_for_app("opencode", false, RouteMode::Proxy)
            .await
            .expect("disable isolated OpenCode takeover");
        let opencode_originals = originals
            .iter()
            .find(|(app_type, _)| matches!(app_type, AppType::OpenCode))
            .expect("OpenCode originals");
        for (path, original) in &opencode_originals.1 {
            assert_eq!(
                std::fs::read(path).expect("read restored OpenCode"),
                *original
            );
        }
        service.stop().await.expect("stop independent gateway");
    }

    #[tokio::test]
    #[serial]
    async fn c2b_unowned_manual_namespaces_survive_crash_recovery() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init manual namespace DB"));
        let service = ProxyService::new(db.clone());

        let mut baselines = Vec::new();
        for app_type in [
            AppType::ClaudeDesktop,
            AppType::OpenCode,
            AppType::OpenClaw,
            AppType::Hermes,
        ] {
            let provider = c2b_test_provider(&app_type, true);
            seed_c2b_current_provider(&db, &app_type, &provider);
            let targets = c2b_live_target_paths(&app_type);
            let manual_url = expected_c2b_proxy_base_url(&app_type, 42567);
            let bytes = match app_type {
                AppType::ClaudeDesktop => vec![
                    br#"{"manual":"normal"}"#.to_vec(),
                    br#"{"manual":"threep"}"#.to_vec(),
                    format!(
                        "{{\"inferenceProvider\":\"gateway\",\"inferenceGatewayBaseUrl\":\"{manual_url}\",\"inferenceGatewayApiKey\":\"user-token\"}}"
                    )
                    .into_bytes(),
                    br#"{"manual":"meta"}"#.to_vec(),
                ],
                AppType::OpenCode => vec![format!(
                    "{{\"provider\":{{\"{}\":{{\"npm\":\"@ai-sdk/openai-compatible\",\"options\":{{\"baseURL\":\"{manual_url}\",\"apiKey\":\"user-token\"}}}}}}}}",
                    provider.id
                )
                .into_bytes()],
                AppType::OpenClaw => vec![format!(
                    "{{models:{{providers:{{'{}':{{baseUrl:'{manual_url}',apiKey:'user-token',api:'openai-completions'}}}}}}}}",
                    provider.id
                )
                .into_bytes()],
                AppType::Hermes => vec![format!(
                    "custom_providers:\n  - name: {}\n    base_url: {}\n    api_key: user-token\n    api_mode: chat_completions\n",
                    provider.id, manual_url
                )
                .into_bytes()],
                _ => unreachable!(),
            };
            assert_eq!(targets.len(), bytes.len());
            let target_bytes = targets
                .into_iter()
                .zip(bytes)
                .map(|(path, bytes)| {
                    std::fs::create_dir_all(path.parent().expect("manual target parent"))
                        .expect("create manual target parent");
                    std::fs::write(&path, &bytes).expect("seed manual namespace target");
                    (path, bytes)
                })
                .collect::<Vec<_>>();
            assert!(service.detect_takeover_in_live_config_for_app(&app_type));
            assert!(
                !db.get_proxy_config_for_app(app_type.as_str())
                    .await
                    .expect("read unowned state")
                    .takeover_enabled
            );
            assert!(db
                .get_live_backup(app_type.as_str())
                .await
                .expect("read absent snapshot")
                .is_none());
            baselines.push((app_type, target_bytes));
        }

        service
            .recover_from_crash()
            .await
            .expect("manual namespaces must not be treated as AGS residue");

        for (app_type, targets) in baselines {
            for (path, bytes) in targets {
                assert_eq!(
                    std::fs::read(&path).expect("read preserved manual namespace"),
                    bytes,
                    "crash recovery rewrote unowned {} namespace target {}",
                    app_type.as_str(),
                    path.display()
                );
            }
            assert!(
                !db.get_proxy_config_for_app(app_type.as_str())
                    .await
                    .expect("read still-unowned state")
                    .takeover_enabled
            );
            assert!(db
                .get_live_backup(app_type.as_str())
                .await
                .expect("read still-absent snapshot")
                .is_none());
        }
    }

    #[tokio::test]
    #[serial]
    async fn c2b_crash_recovery_restores_all_four_without_retakeover() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        use_ephemeral_proxy_port(&db).await;
        let service = ProxyService::new(db.clone());
        let mut originals = Vec::new();
        for app_type in [
            AppType::ClaudeDesktop,
            AppType::OpenCode,
            AppType::OpenClaw,
            AppType::Hermes,
        ] {
            let provider = c2b_test_provider(&app_type, true);
            seed_c2b_current_provider(&db, &app_type, &provider);
            originals.push((
                app_type.clone(),
                seed_c2b_original_live(&app_type, &provider),
            ));
            service
                .set_takeover_for_app(app_type.as_str(), true, RouteMode::Proxy)
                .await
                .expect("enable C2b proxy before crash recovery");
        }

        service
            .recover_from_crash()
            .await
            .expect("recover all C2b proxy residue");

        for (app_type, targets) in originals {
            for (path, original) in targets {
                assert_eq!(
                    std::fs::read(&path).expect("read crash-restored target"),
                    original,
                    "crash recovery must exactly restore {} target {}",
                    app_type.as_str(),
                    path.display()
                );
            }
            let config = db
                .get_proxy_config_for_app(app_type.as_str())
                .await
                .expect("read crash-released state");
            assert!(!config.takeover_enabled);
            assert!(db
                .get_live_backup(app_type.as_str())
                .await
                .expect("read crash-released snapshot")
                .is_none());
        }
        service.stop().await.expect("stop independent gateway");
    }

    #[tokio::test]
    #[serial]
    async fn c2b_claude_desktop_malformed_manifest_preflight_keeps_live_and_ownership() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());
        let targets = crate::claude_desktop_config::snapshot_target_paths()
            .expect("resolve Claude Desktop paths");
        for (_, path) in &targets {
            std::fs::create_dir_all(path.parent().expect("target parent"))
                .expect("create target parent");
            std::fs::write(path, br#"{"managed":true}"#).expect("seed managed target");
        }

        let malformed = SnapshotManifest::new(
            &AppType::ClaudeDesktop,
            vec![
                crate::proxy::snapshot::SnapshotTarget::file_bytes(
                    "normal_config",
                    Some(b"original-normal"),
                ),
                crate::proxy::snapshot::SnapshotTarget::file_bytes(
                    "threep_config",
                    Some(b"original-threep"),
                ),
                crate::proxy::snapshot::SnapshotTarget::file_bytes("profile", None),
                // 故意缺少 meta；若 adapter 不预检，前三个目标可能先被部分恢复。
            ],
        )
        .expect("build structurally valid but module-incomplete manifest")
        .encode()
        .expect("encode malformed manifest");
        db.save_live_backup("claude-desktop", &malformed)
            .await
            .expect("save malformed manifest");
        let mut state = db
            .get_proxy_config_for_app("claude-desktop")
            .await
            .expect("read takeover state");
        state.takeover_enabled = true;
        state.route_mode = RouteMode::Proxy;
        db.update_proxy_config_for_app(state)
            .await
            .expect("mark takeover active");

        assert!(service
            .set_takeover_for_app("claude-desktop", false, RouteMode::Proxy)
            .await
            .is_err());
        for (_, path) in &targets {
            assert_eq!(
                std::fs::read(path).expect("read untouched managed target"),
                br#"{"managed":true}"#
            );
        }
        assert!(
            db.get_proxy_config_for_app("claude-desktop")
                .await
                .expect("read retained state")
                .takeover_enabled
        );
        assert!(db
            .get_live_backup("claude-desktop")
            .await
            .expect("read retained malformed snapshot")
            .is_some());
    }

    #[tokio::test]
    #[serial]
    async fn c2b_claude_desktop_direct_proxy_and_exact_restore() {
        assert_c2b_route_lifecycle(AppType::ClaudeDesktop).await;
    }

    #[tokio::test]
    #[serial]
    async fn c2b_opencode_direct_proxy_exact_restore_and_db_untouched() {
        assert_c2b_route_lifecycle(AppType::OpenCode).await;
    }

    #[tokio::test]
    #[serial]
    async fn c2b_openclaw_direct_proxy_and_exact_restore() {
        assert_c2b_route_lifecycle(AppType::OpenClaw).await;
    }

    #[tokio::test]
    #[serial]
    async fn c2b_hermes_direct_proxy_and_exact_restore() {
        assert_c2b_route_lifecycle(AppType::Hermes).await;
    }

    #[test]
    fn managed_account_claude_takeover_uses_api_key_placeholder() {
        let mut provider = Provider::with_id(
            "copilot".to_string(),
            "GitHub Copilot".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.githubcopilot.com",
                    "ANTHROPIC_MODEL": "claude-haiku-4.5"
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            provider_type: Some("github_copilot".to_string()),
            ..Default::default()
        });

        let mut live_config = provider.settings_config.clone();
        ProxyService::apply_claude_takeover_fields_for_provider(
            &mut live_config,
            "http://127.0.0.1:42567",
            &provider,
        );

        let env = live_config
            .get("env")
            .and_then(|value| value.as_object())
            .expect("env should exist");
        assert_eq!(
            env.get("ANTHROPIC_API_KEY")
                .and_then(|value| value.as_str()),
            Some(PROXY_TOKEN_PLACEHOLDER)
        );
        assert!(
            env.get("ANTHROPIC_AUTH_TOKEN").is_none(),
            "managed OAuth providers should avoid Claude Auth Token login semantics"
        );
    }

    #[test]
    fn managed_account_claude_takeover_sources_copilot_models_from_provider() {
        let mut provider = Provider::with_id(
            "copilot".to_string(),
            "GitHub Copilot".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.githubcopilot.com",
                    "ANTHROPIC_MODEL": "claude-sonnet-4.6",
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "claude-haiku-4.5",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL": "claude-sonnet-4.6",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL": "claude-sonnet-4.6",
                    "CLAUDE_CODE_SUBAGENT_MODEL": "claude-sonnet-4.6[1M]"
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            provider_type: Some("github_copilot".to_string()),
            ..Default::default()
        });

        let mut live_config = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://stale.example.com",
                "ANTHROPIC_API_KEY": "stale-key",
                "ANTHROPIC_MODEL": "stale-model",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "stale-haiku",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME": "Stale Haiku",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "stale-sonnet",
                "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "Stale Sonnet",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "stale-opus",
                "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "Stale Opus",
                "CLAUDE_CODE_SUBAGENT_MODEL": "stale-subagent"
            }
        });
        ProxyService::apply_claude_takeover_fields_for_provider(
            &mut live_config,
            "http://127.0.0.1:42567",
            &provider,
        );

        let env = live_config
            .get("env")
            .and_then(|value| value.as_object())
            .expect("env should exist");
        assert_env_str(env, "ANTHROPIC_MODEL", None);
        assert_env_str(
            env,
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            Some("claude-haiku-4-5"),
        );
        assert_env_str(
            env,
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            Some("claude-haiku-4.5"),
        );
        assert_env_str(
            env,
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            Some("claude-sonnet-4-6"),
        );
        assert_env_str(
            env,
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            Some("claude-sonnet-4.6"),
        );
        assert_env_str(env, "ANTHROPIC_DEFAULT_OPUS_MODEL", Some("claude-opus-4-8"));
        assert_env_str(
            env,
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
            Some("claude-sonnet-4.6"),
        );
        assert_env_str(
            env,
            "CLAUDE_CODE_SUBAGENT_MODEL",
            Some("claude-sonnet-4.6[1M]"),
        );
        assert_env_str(env, "ANTHROPIC_API_KEY", Some(PROXY_TOKEN_PLACEHOLDER));
        assert_env_str(env, "ANTHROPIC_AUTH_TOKEN", None);
    }

    #[test]
    fn managed_account_claude_takeover_removes_stale_subagent_model_when_provider_omits_it() {
        let mut provider = Provider::with_id(
            "codex".to_string(),
            "Codex".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://chatgpt.com/backend-api/codex",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL": "provider-sonnet"
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            provider_type: Some("codex_oauth".to_string()),
            ..Default::default()
        });

        let mut live_config = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://stale.example.com",
                "ANTHROPIC_API_KEY": "stale-key",
                "CLAUDE_CODE_SUBAGENT_MODEL": "stale-subagent"
            }
        });
        ProxyService::apply_claude_takeover_fields_for_provider(
            &mut live_config,
            "http://127.0.0.1:42567",
            &provider,
        );

        let env = live_config
            .get("env")
            .and_then(|value| value.as_object())
            .expect("env should exist");
        assert_env_str(env, "CLAUDE_CODE_SUBAGENT_MODEL", None);
    }

    #[test]
    fn managed_account_claude_takeover_sources_codex_models_from_provider() {
        let mut provider = Provider::with_id(
            "codex".to_string(),
            "Codex".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://chatgpt.com/backend-api/codex",
                    "ANTHROPIC_MODEL": "gpt-5.4",
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "gpt-5.4-mini",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL": "gpt-5.4",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL": "gpt-5.4"
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            provider_type: Some("codex_oauth".to_string()),
            ..Default::default()
        });

        let mut live_config = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://stale.example.com",
                "ANTHROPIC_AUTH_TOKEN": "stale-token",
                "ANTHROPIC_MODEL": "stale-model",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "stale-haiku",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME": "Stale Haiku",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "stale-sonnet",
                "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "Stale Sonnet",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "stale-opus",
                "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "Stale Opus"
            }
        });
        ProxyService::apply_claude_takeover_fields_for_provider(
            &mut live_config,
            "http://127.0.0.1:42567",
            &provider,
        );

        let env = live_config
            .get("env")
            .and_then(|value| value.as_object())
            .expect("env should exist");
        assert_env_str(env, "ANTHROPIC_MODEL", None);
        assert_env_str(
            env,
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            Some("claude-haiku-4-5"),
        );
        assert_env_str(
            env,
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            Some("gpt-5.4-mini"),
        );
        assert_env_str(
            env,
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            Some("claude-sonnet-4-6"),
        );
        assert_env_str(env, "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME", Some("gpt-5.4"));
        assert_env_str(env, "ANTHROPIC_DEFAULT_OPUS_MODEL", Some("claude-opus-4-8"));
        assert_env_str(env, "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME", Some("gpt-5.4"));
        // Codex 系只保留 AUTH_TOKEN；双键会触发 Claude Code 告警（#4919）
        assert_env_str(env, "ANTHROPIC_API_KEY", None);
        assert_env_str(env, "ANTHROPIC_AUTH_TOKEN", Some(PROXY_TOKEN_PLACEHOLDER));
    }

    #[test]
    fn managed_account_claude_takeover_codex_injects_auth_token_without_preexisting_key() {
        let mut provider = Provider::with_id(
            "codex".to_string(),
            "Codex".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://chatgpt.com/backend-api/codex"
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            provider_type: Some("codex_oauth".to_string()),
            ..Default::default()
        });

        // 全新安装/热切换形态：传入的 env 没有任何 token 键。
        let mut live_config = provider.settings_config.clone();
        ProxyService::apply_claude_takeover_fields_for_provider(
            &mut live_config,
            "http://127.0.0.1:42567",
            &provider,
        );

        let env = live_config
            .get("env")
            .and_then(|value| value.as_object())
            .expect("env should exist");
        assert_env_str(env, "ANTHROPIC_API_KEY", None);
        assert_env_str(env, "ANTHROPIC_AUTH_TOKEN", Some(PROXY_TOKEN_PLACEHOLDER));
    }

    #[test]
    fn managed_account_claude_takeover_codex_by_base_url_keeps_auth_token() {
        // 无 provider_type meta、仅凭 base_url 识别为受管 codex 的供应商，
        // 也必须保留 AUTH_TOKEN 占位符（与策略选择共用同一判定族）。
        let provider = Provider::with_id(
            "codex-url-only".to_string(),
            "Codex (URL only)".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://chatgpt.com/backend-api/codex"
                }
            }),
            None,
        );
        assert!(provider.uses_managed_account_auth());
        assert!(!provider.is_codex_oauth());

        let mut live_config = provider.settings_config.clone();
        ProxyService::apply_claude_takeover_fields_for_provider(
            &mut live_config,
            "http://127.0.0.1:42567",
            &provider,
        );

        let env = live_config
            .get("env")
            .and_then(|value| value.as_object())
            .expect("env should exist");
        assert_env_str(env, "ANTHROPIC_API_KEY", None);
        assert_env_str(env, "ANTHROPIC_AUTH_TOKEN", Some(PROXY_TOKEN_PLACEHOLDER));
    }

    // #4919 复现场景：从第三方 Claude 供应商（live 已有 AUTH_TOKEN）切换到
    // Codex 受管供应商时，只应保留 AUTH_TOKEN 占位符，不得同时写入 API_KEY。
    #[test]
    fn managed_account_claude_takeover_codex_from_third_party_keeps_single_auth_key() {
        let mut provider = Provider::with_id(
            "codex".to_string(),
            "Codex".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://chatgpt.com/backend-api/codex"
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            provider_type: Some("codex_oauth".to_string()),
            ..Default::default()
        });

        let mut live_config = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                "ANTHROPIC_AUTH_TOKEN": "sk-third-party"
            }
        });
        ProxyService::apply_claude_takeover_fields_for_provider(
            &mut live_config,
            "http://127.0.0.1:42567",
            &provider,
        );

        let env = live_config
            .get("env")
            .and_then(|value| value.as_object())
            .expect("env should exist");
        assert_env_str(env, "ANTHROPIC_AUTH_TOKEN", Some(PROXY_TOKEN_PLACEHOLDER));
        assert_env_str(env, "ANTHROPIC_API_KEY", None);
    }

    #[test]
    fn managed_account_claude_takeover_copilot_removes_stale_auth_token() {
        let mut provider = Provider::with_id(
            "copilot".to_string(),
            "GitHub Copilot".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.githubcopilot.com"
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            provider_type: Some("github_copilot".to_string()),
            ..Default::default()
        });

        let mut live_config = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://stale.example.com",
                "ANTHROPIC_AUTH_TOKEN": "stale-token"
            }
        });
        ProxyService::apply_claude_takeover_fields_for_provider(
            &mut live_config,
            "http://127.0.0.1:42567",
            &provider,
        );

        let env = live_config
            .get("env")
            .and_then(|value| value.as_object())
            .expect("env should exist");
        assert_env_str(env, "ANTHROPIC_API_KEY", Some(PROXY_TOKEN_PLACEHOLDER));
        assert_env_str(env, "ANTHROPIC_AUTH_TOKEN", None);
    }

    #[test]
    fn normal_claude_takeover_without_token_keeps_auth_token_fallback() {
        let mut live_config = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.example.com",
                "ANTHROPIC_MODEL": "claude-haiku-4.5"
            }
        });

        ProxyService::apply_claude_takeover_fields(&mut live_config, "http://127.0.0.1:42567");

        assert_eq!(
            live_config
                .get("env")
                .and_then(|env| env.get("ANTHROPIC_AUTH_TOKEN"))
                .and_then(|value| value.as_str()),
            Some(PROXY_TOKEN_PLACEHOLDER)
        );
        assert!(
            live_config
                .get("env")
                .and_then(|env| env.get("ANTHROPIC_API_KEY"))
                .is_none(),
            "non-managed providers should retain the legacy fallback behavior"
        );
    }

    #[tokio::test]
    #[serial]
    async fn start_with_takeover_ephemeral_port_writes_actual_live_url() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        use_ephemeral_proxy_port(&db).await;
        let service = ProxyService::new(db.clone());

        let provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "provider-key",
                    "ANTHROPIC_BASE_URL": "https://api.anthropic.com"
                }
            }),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set db current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("p1"))
            .expect("set local current provider");
        service
            .write_claude_live(&json!({
                "env": {
                    "ANTHROPIC_API_KEY": "live-key",
                    "ANTHROPIC_BASE_URL": "https://api.anthropic.com"
                }
            }))
            .expect("seed claude live config");

        let info = service.start().await.expect("start proxy gateway");
        service
            .set_takeover_for_app("claude", true, RouteMode::Proxy)
            .await
            .expect("enable Claude proxy takeover explicitly");
        assert_ne!(info.port, 0, "OS should assign a concrete port");

        let stored_config = db.get_proxy_config().await.expect("read proxy config");
        assert_eq!(
            stored_config.listen_port, info.port,
            "resolved dynamic port should be persisted for DB-only proxy URL paths"
        );

        let live = service.read_claude_live().expect("read taken-over live");
        let base_url = live
            .get("env")
            .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
            .and_then(|value| value.as_str())
            .expect("taken-over base url");
        assert_eq!(base_url, format!("http://127.0.0.1:{}", info.port));
        assert!(
            !base_url.contains(":0"),
            "takeover must never write an unresolved :0 port"
        );

        service
            .stop_with_restore()
            .await
            .expect("stop proxy and restore live config");
    }

    #[tokio::test]
    #[serial]
    async fn c2a_route_modes_keep_gateway_and_first_open_snapshot_independent() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        use_ephemeral_proxy_port(&db).await;
        let service = ProxyService::new(db.clone());
        let provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "provider-key",
                    "ANTHROPIC_BASE_URL": "https://upstream.example"
                }
            }),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set db current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("p1"))
            .expect("set local current provider");

        let settings_path = get_claude_settings_path();
        std::fs::create_dir_all(settings_path.parent().expect("settings parent"))
            .expect("create settings directory");
        let original =
            b"{\n  \"env\": {\"ANTHROPIC_API_KEY\": \"original\"},\n  \"custom\": 1\n}\n";
        std::fs::write(&settings_path, original).expect("seed original settings");

        service
            .set_takeover_for_app("claude", true, RouteMode::Direct)
            .await
            .expect("enable direct takeover");
        assert!(
            !service.is_running().await,
            "direct takeover must not start gateway"
        );
        let snapshot = db
            .get_live_backup("claude")
            .await
            .expect("read snapshot")
            .expect("snapshot exists")
            .original_config;

        service
            .switch_route_mode("claude", RouteMode::Proxy)
            .await
            .expect("switch to proxy");
        assert!(service.is_running().await, "proxy route must start gateway");
        assert_eq!(
            db.get_live_backup("claude")
                .await
                .expect("read snapshot")
                .expect("snapshot exists")
                .original_config,
            snapshot,
            "route switch must not recapture or overwrite first-open snapshot"
        );

        service
            .switch_route_mode("claude", RouteMode::Direct)
            .await
            .expect("switch back to direct");
        assert!(
            service.is_running().await,
            "switching direct must not stop an independently running gateway"
        );
        let direct_live: Value = read_json_file(&settings_path).expect("read direct live");
        assert_eq!(
            direct_live["env"]["ANTHROPIC_BASE_URL"],
            "https://upstream.example"
        );

        service
            .set_takeover_for_app("claude", false, RouteMode::Direct)
            .await
            .expect("disable takeover");
        assert_eq!(
            std::fs::read(&settings_path).expect("read restored settings"),
            original
        );
        assert!(
            !db.get_proxy_config_for_app("claude")
                .await
                .expect("read takeover state")
                .takeover_enabled
        );
        assert!(
            db.get_live_backup("claude")
                .await
                .expect("read snapshot")
                .is_none(),
            "successful exact restore must release snapshot"
        );
        service.stop().await.expect("stop independent gateway");
    }

    #[test]
    #[serial]
    fn codex_custom_provider_live_write_preserves_oauth_auth_json() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        crate::settings::update_settings(crate::settings::AppSettings {
            preserve_codex_official_auth_on_switch: true,
            ..Default::default()
        })
        .expect("enable Codex official auth preservation");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db);
        let oauth_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "oauth-id",
                "access_token": "oauth-access"
            }
        });
        crate::codex_config::write_codex_live_atomic(
            &oauth_auth,
            Some(
                r#"model_provider = "openai"
model = "gpt-5-codex"
"#,
            ),
        )
        .expect("seed live OAuth auth");

        let mut provider = Provider::with_id(
            "rightcode".to_string(),
            "RightCode".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "rightcode-key"
                },
                "config": r#"model_provider = "rightcode"
model = "gpt-5-codex"

[model_providers.rightcode]
name = "RightCode"
base_url = "https://rightcode.example/v1"
wire_api = "responses"
"#
            }),
            None,
        );
        provider.category = Some("custom".to_string());
        let takeover_settings = json!({
            "auth": {
                "OPENAI_API_KEY": PROXY_TOKEN_PLACEHOLDER
            },
            "config": r#"model_provider = "rightcode"
model = "gpt-5-codex"

[model_providers.rightcode]
name = "RightCode"
base_url = "http://127.0.0.1:42567/v1"
wire_api = "responses"
"#
        });

        service
            .write_codex_live_for_provider(&takeover_settings, Some(&provider))
            .expect("write provider-driven Codex live config");

        let live_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read live auth");
        assert_eq!(
            live_auth, oauth_auth,
            "third-party Codex proxy writes must not overwrite ChatGPT OAuth login state"
        );

        let live_config = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read live config");
        assert!(
            live_config.contains("experimental_bearer_token"),
            "proxy placeholder should move into config.toml instead of auth.json"
        );
        assert!(
            live_config.contains(PROXY_TOKEN_PLACEHOLDER),
            "live config should carry the proxy placeholder token"
        );

        crate::settings::update_settings(crate::settings::AppSettings::default())
            .expect("reset settings");
    }

    #[tokio::test]
    #[serial]
    async fn codex_takeover_preserves_oauth_auth_json_when_preserve_enabled() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        crate::settings::update_settings(crate::settings::AppSettings {
            preserve_codex_official_auth_on_switch: true,
            ..Default::default()
        })
        .expect("enable Codex official auth preservation");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());
        let oauth_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "oauth-id",
                "access_token": "oauth-access"
            }
        });
        let deepseek_live_config = r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
experimental_bearer_token = "deepseek-key"
"#;
        crate::codex_config::write_codex_live_atomic(&oauth_auth, Some(deepseek_live_config))
            .expect("seed live OAuth auth with DeepSeek config");

        let mut provider = Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "deepseek-key"
                },
                "config": r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#
            }),
            None,
        );
        provider.category = Some("cn_official".to_string());
        db.save_provider("codex", &provider)
            .expect("save DeepSeek provider");
        db.set_current_provider("codex", "deepseek")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("deepseek"))
            .expect("set local current provider");

        service
            .takeover_live_config_strict(&AppType::Codex)
            .await
            .expect("take over Codex live config");

        let live_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read live auth");
        assert_eq!(
            live_auth, oauth_auth,
            "Codex takeover should not overwrite ChatGPT OAuth auth when preservation is enabled"
        );

        let live_config = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read live config");
        assert!(
            live_config.contains(PROXY_TOKEN_PLACEHOLDER),
            "takeover placeholder should move into config.toml"
        );
        assert!(
            service.detect_takeover_in_live_config_for_app(&AppType::Codex),
            "Codex takeover detection should recognize config.toml placeholders"
        );

        crate::settings::update_settings(crate::settings::AppSettings::default())
            .expect("reset settings");
    }

    #[tokio::test]
    #[serial]
    async fn codex_takeover_preserves_oauth_auth_json_even_when_provider_category_is_official() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        crate::settings::update_settings(crate::settings::AppSettings {
            preserve_codex_official_auth_on_switch: true,
            ..Default::default()
        })
        .expect("enable Codex official auth preservation");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());
        let oauth_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "oauth-id",
                "access_token": "oauth-access"
            }
        });
        let deepseek_live_config = r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
experimental_bearer_token = "deepseek-key"
"#;
        crate::codex_config::write_codex_live_atomic(&oauth_auth, Some(deepseek_live_config))
            .expect("seed live OAuth auth with DeepSeek config");

        let mut provider = Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "deepseek-key"
                },
                "config": r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#
            }),
            None,
        );
        provider.category = Some("official".to_string());
        db.save_provider("codex", &provider)
            .expect("save misclassified DeepSeek provider");
        db.set_current_provider("codex", "deepseek")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("deepseek"))
            .expect("set local current provider");

        service
            .takeover_live_config_strict(&AppType::Codex)
            .await
            .expect("take over Codex live config");

        let live_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read live auth");
        assert_eq!(
            live_auth, oauth_auth,
            "Codex takeover must not rewrite auth.json when preservation is enabled, even if provider category is stale or misclassified"
        );

        let live_config = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read live config");
        assert!(
            live_config.contains(PROXY_TOKEN_PLACEHOLDER),
            "takeover placeholder should move into config.toml"
        );

        crate::settings::update_settings(crate::settings::AppSettings::default())
            .expect("reset settings");
    }

    #[tokio::test]
    #[serial]
    async fn codex_set_takeover_for_app_preserves_oauth_auth_json_when_preserve_enabled() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        crate::settings::update_settings(crate::settings::AppSettings {
            preserve_codex_official_auth_on_switch: true,
            ..Default::default()
        })
        .expect("enable Codex official auth preservation");

        let db = Arc::new(Database::memory().expect("init db"));
        use_ephemeral_proxy_port(&db).await;
        let service = ProxyService::new(db.clone());
        let oauth_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "oauth-id",
                "access_token": "oauth-access"
            }
        });
        let deepseek_live_config = r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
experimental_bearer_token = "deepseek-key"
"#;
        crate::codex_config::write_codex_live_atomic(&oauth_auth, Some(deepseek_live_config))
            .expect("seed live OAuth auth with DeepSeek config");

        let mut provider = Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "deepseek-key"
                },
                "config": r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#
            }),
            None,
        );
        provider.category = Some("official".to_string());
        db.save_provider("codex", &provider)
            .expect("save misclassified DeepSeek provider");
        db.set_current_provider("codex", "deepseek")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("deepseek"))
            .expect("set local current provider");

        service
            .set_takeover_for_app("codex", true, RouteMode::Proxy)
            .await
            .expect("enable Codex takeover");

        let live_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read live auth");
        assert_eq!(
            live_auth, oauth_auth,
            "the public takeover command path must not rewrite auth.json when preservation is enabled"
        );

        service
            .set_takeover_for_app("codex", false, RouteMode::Proxy)
            .await
            .expect("disable Codex takeover");
        crate::settings::update_settings(crate::settings::AppSettings::default())
            .expect("reset settings");
    }

    #[tokio::test]
    #[serial]
    async fn codex_sync_current_to_live_during_takeover_preserves_oauth_auth_json() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        crate::settings::update_settings(crate::settings::AppSettings {
            preserve_codex_official_auth_on_switch: true,
            ..Default::default()
        })
        .expect("enable Codex official auth preservation");

        let db = Arc::new(Database::memory().expect("init db"));
        use_ephemeral_proxy_port(&db).await;
        let state = crate::store::AppState::new(db.clone());
        let oauth_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "oauth-id",
                "access_token": "oauth-access"
            }
        });
        let deepseek_live_config = r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
experimental_bearer_token = "deepseek-key"
"#;
        crate::codex_config::write_codex_live_atomic(&oauth_auth, Some(deepseek_live_config))
            .expect("seed live OAuth auth with DeepSeek config");

        let mut provider = Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "deepseek-key"
                },
                "config": r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#
            }),
            None,
        );
        provider.category = Some("official".to_string());
        db.save_provider("codex", &provider)
            .expect("save misclassified DeepSeek provider");
        db.set_current_provider("codex", "deepseek")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("deepseek"))
            .expect("set local current provider");

        state
            .proxy_service
            .set_takeover_for_app("codex", true, RouteMode::Proxy)
            .await
            .expect("enable Codex takeover");

        crate::services::provider::ProviderService::sync_current_to_live(&state)
            .expect("sync current providers while Codex is taken over");

        let live_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read live auth");
        assert_eq!(
            live_auth, oauth_auth,
            "post-change provider sync must not rewrite Codex auth.json during takeover"
        );

        // C2a Option A: the restore backup is now the IMMUTABLE first-open
        // versioned snapshot manifest (byte-exact original files), no longer a
        // provider-derived `{auth, config}` JSON. Decode the manifest and assert
        // its `auth` target round-trips to the original OAuth material, and its
        // `config` target carries the original DeepSeek key.
        let backup = db
            .get_live_backup("codex")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let manifest = crate::proxy::snapshot::SnapshotManifest::decode(&backup.original_config)
            .expect("backup must be a versioned snapshot manifest");
        let auth_bytes = manifest
            .targets
            .iter()
            .find(|t| t.id() == "auth")
            .expect("manifest has auth target")
            .file_payload()
            .expect("decode auth payload")
            .expect("auth existed at first open");
        let backup_auth: Value =
            serde_json::from_slice(&auth_bytes).expect("parse captured auth.json bytes");
        assert_eq!(
            backup_auth, oauth_auth,
            "immutable first-open snapshot should capture the original official OAuth auth verbatim"
        );
        let config_bytes = manifest
            .targets
            .iter()
            .find(|t| t.id() == "config")
            .expect("manifest has config target")
            .file_payload()
            .expect("decode config payload")
            .expect("config existed at first open");
        let backup_config = String::from_utf8(config_bytes).expect("config.toml is utf-8");
        assert!(
            backup_config.contains("deepseek-key"),
            "first-open config.toml snapshot should carry the original provider token"
        );

        state
            .proxy_service
            .set_takeover_for_app("codex", false, RouteMode::Proxy)
            .await
            .expect("disable Codex takeover");
        let restored_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read restored auth");
        assert_eq!(
            restored_auth, oauth_auth,
            "turning takeover off should restore the preserved official OAuth auth"
        );

        crate::settings::update_settings(crate::settings::AppSettings::default())
            .expect("reset settings");
    }

    #[tokio::test]
    #[serial]
    async fn codex_sync_current_to_live_during_takeover_activation_keeps_proxy_live_config() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        crate::settings::update_settings(crate::settings::AppSettings {
            preserve_codex_official_auth_on_switch: true,
            ..Default::default()
        })
        .expect("enable Codex official auth preservation");

        let db = Arc::new(Database::memory().expect("init db"));
        let state = crate::store::AppState::new(db.clone());
        let oauth_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "oauth-id",
                "access_token": "oauth-access"
            }
        });
        let deepseek_live_config = r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
experimental_bearer_token = "deepseek-key"
"#;
        crate::codex_config::write_codex_live_atomic(&oauth_auth, Some(deepseek_live_config))
            .expect("seed live OAuth auth with DeepSeek config");

        let mut provider = Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "deepseek-key"
                },
                "config": r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#
            }),
            None,
        );
        provider.category = Some("official".to_string());
        db.save_provider("codex", &provider)
            .expect("save misclassified DeepSeek provider");
        db.set_current_provider("codex", "deepseek")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("deepseek"))
            .expect("set local current provider");

        state
            .proxy_service
            .backup_live_config_strict(&AppType::Codex)
            .await
            .expect("backup Codex live config");
        state
            .proxy_service
            .takeover_live_config_strict(&AppType::Codex)
            .await
            .expect("take over Codex live config");
        assert!(
            !db.get_proxy_config_for_app("codex")
                .await
                .expect("get Codex proxy config")
                .takeover_enabled,
            "this reproduces the activation window before set_takeover_for_app marks takeover_enabled=true"
        );

        crate::services::provider::ProviderService::sync_current_to_live(&state)
            .expect("sync current providers during takeover activation");

        let live_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read live auth");
        assert_eq!(
            live_auth, oauth_auth,
            "activation-time provider sync must not rewrite Codex OAuth auth.json"
        );

        let live_config = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read live config");
        assert!(
            live_config.contains(PROXY_TOKEN_PLACEHOLDER),
            "activation-time provider sync must keep the proxy bearer placeholder"
        );
        assert!(
            live_config.contains("http://127.0.0.1"),
            "activation-time provider sync must keep the local proxy base_url"
        );
        assert!(
            state
                .proxy_service
                .detect_takeover_in_live_config_for_app(&AppType::Codex),
            "Codex live config should still be detected as taken over"
        );

        crate::settings::update_settings(crate::settings::AppSettings::default())
            .expect("reset settings");
    }

    #[tokio::test]
    #[serial]
    async fn codex_set_takeover_rebuilds_stale_enabled_state_without_overwriting_backup() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        crate::settings::update_settings(crate::settings::AppSettings {
            preserve_codex_official_auth_on_switch: true,
            ..Default::default()
        })
        .expect("enable Codex official auth preservation");

        let db = Arc::new(Database::memory().expect("init db"));
        use_ephemeral_proxy_port(&db).await;
        let service = ProxyService::new(db.clone());
        let oauth_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "oauth-id",
                "access_token": "oauth-access"
            }
        });
        let original_deepseek_config = r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
experimental_bearer_token = "deepseek-key"
"#;
        let stale_live_config = r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
experimental_bearer_token = "PROXY_MANAGED"
"#;
        crate::codex_config::write_codex_live_atomic(&oauth_auth, Some(stale_live_config))
            .expect("seed stale Codex live config");

        let mut provider = Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "deepseek-key"
                },
                "config": r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#
            }),
            None,
        );
        provider.category = Some("official".to_string());
        db.save_provider("codex", &provider)
            .expect("save misclassified DeepSeek provider");
        db.set_current_provider("codex", "deepseek")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("deepseek"))
            .expect("set local current provider");
        db.save_live_backup(
            "codex",
            &serde_json::to_string(&json!({
                "auth": oauth_auth,
                "config": original_deepseek_config
            }))
            .expect("serialize original backup"),
        )
        .await
        .expect("seed original live backup");
        let mut proxy_config = db
            .get_proxy_config_for_app("codex")
            .await
            .expect("get Codex proxy config");
        proxy_config.takeover_enabled = true;
        proxy_config.route_mode = RouteMode::Proxy;
        db.update_proxy_config_for_app(proxy_config)
            .await
            .expect("mark Codex takeover enabled");

        service
            .set_takeover_for_app("codex", true, RouteMode::Proxy)
            .await
            .expect("rebuild Codex takeover");

        let live_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read live auth");
        assert_eq!(
            live_auth, oauth_auth,
            "repairing stale takeover must restore the preserved OAuth auth from backup"
        );

        let live_config = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read live config");
        let expected_base_url = running_codex_base_url(&service).await;
        assert!(
            live_config.contains(&expected_base_url),
            "stale enabled takeover must be rebuilt to the current proxy base_url"
        );
        assert!(
            live_config.contains(PROXY_TOKEN_PLACEHOLDER),
            "rebuilt takeover should keep the proxy bearer placeholder"
        );
        assert!(
            service
                .live_takeover_matches_current_proxy(&AppType::Codex)
                .await
                .expect("detect rebuilt Codex takeover"),
            "rebuilt Codex live config should match the active proxy address"
        );

        let backup = db
            .get_live_backup("codex")
            .await
            .expect("get Codex live backup")
            .expect("backup exists");
        let backup_value: Value =
            serde_json::from_str(&backup.original_config).expect("parse backup");
        assert_eq!(
            backup_value.get("auth"),
            Some(&oauth_auth),
            "rebuilding stale takeover must not overwrite the original OAuth backup"
        );
        assert!(
            backup_value
                .get("config")
                .and_then(|value| value.as_str())
                .is_some_and(|config| config.contains("deepseek-key")
                    && !config.contains("http://127.0.0.1")),
            "backup should remain the restorable DeepSeek config, not the proxy config"
        );

        service
            .set_takeover_for_app("codex", false, RouteMode::Proxy)
            .await
            .expect("disable Codex takeover");
        crate::settings::update_settings(crate::settings::AppSettings::default())
            .expect("reset settings");
    }

    #[tokio::test]
    #[serial]
    async fn codex_takeover_preserve_disabled_uses_legacy_auth_write_path() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        crate::settings::update_settings(crate::settings::AppSettings {
            preserve_codex_official_auth_on_switch: false,
            ..Default::default()
        })
        .expect("disable Codex official auth preservation");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());
        let oauth_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "oauth-id",
                "access_token": "oauth-access"
            }
        });
        let deepseek_live_config = r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#;
        crate::codex_config::write_codex_live_atomic(&oauth_auth, Some(deepseek_live_config))
            .expect("seed live OAuth auth with DeepSeek config");

        let mut provider = Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "deepseek-key"
                },
                "config": r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#
            }),
            None,
        );
        provider.category = Some("cn_official".to_string());
        db.save_provider("codex", &provider)
            .expect("save DeepSeek provider");
        db.set_current_provider("codex", "deepseek")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("deepseek"))
            .expect("set local current provider");

        service
            .takeover_live_config_strict(&AppType::Codex)
            .await
            .expect("take over Codex live config");

        let live_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read live auth");
        assert_eq!(
            live_auth
                .get("OPENAI_API_KEY")
                .and_then(|value| value.as_str()),
            Some(PROXY_TOKEN_PLACEHOLDER),
            "disabled preservation should keep the legacy auth.json takeover placeholder"
        );
        assert_eq!(
            live_auth
                .get("tokens")
                .and_then(|tokens| tokens.get("access_token"))
                .and_then(|value| value.as_str()),
            Some("oauth-access"),
            "the new config-only takeover branch must not run when preservation is disabled"
        );

        let live_config = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read live config");
        assert!(
            !live_config.contains(PROXY_TOKEN_PLACEHOLDER),
            "disabled preservation should not move the takeover placeholder into config.toml"
        );

        crate::settings::update_settings(crate::settings::AppSettings::default())
            .expect("reset settings");
    }

    #[test]
    #[serial]
    fn codex_takeover_cleanup_removes_config_placeholder_without_touching_oauth_auth() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db);
        let oauth_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "oauth-id",
                "access_token": "oauth-access"
            }
        });
        crate::codex_config::write_codex_live_atomic(
            &oauth_auth,
            Some(
                r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "http://127.0.0.1:42567/v1"
wire_api = "responses"
experimental_bearer_token = "PROXY_MANAGED"
"#,
            ),
        )
        .expect("seed taken-over Codex live config");

        assert!(
            service.detect_takeover_in_live_config_for_app(&AppType::Codex),
            "config.toml placeholder should be detected before cleanup"
        );

        service
            .cleanup_codex_takeover_placeholders_in_live()
            .expect("cleanup Codex takeover placeholders");

        let live_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read live auth");
        assert_eq!(
            live_auth, oauth_auth,
            "cleanup should preserve ChatGPT OAuth auth"
        );

        let live_config = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read live config");
        assert!(
            !live_config.contains(PROXY_TOKEN_PLACEHOLDER),
            "cleanup should remove config.toml proxy bearer placeholder"
        );
        assert!(
            !live_config.contains("http://127.0.0.1:42567"),
            "cleanup should remove local proxy base_url"
        );
    }

    #[test]
    #[serial]
    fn codex_custom_provider_live_write_can_overwrite_auth_when_preserve_disabled() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        crate::settings::update_settings(crate::settings::AppSettings {
            preserve_codex_official_auth_on_switch: false,
            ..Default::default()
        })
        .expect("disable Codex official auth preservation");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db);
        let oauth_auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "oauth-id",
                "access_token": "oauth-access"
            }
        });
        crate::codex_config::write_codex_live_atomic(
            &oauth_auth,
            Some(
                r#"model_provider = "openai"
model = "gpt-5-codex"
"#,
            ),
        )
        .expect("seed live OAuth auth");

        let mut provider = Provider::with_id(
            "rightcode".to_string(),
            "RightCode".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "rightcode-key"
                },
                "config": r#"model_provider = "rightcode"
model = "gpt-5-codex"

[model_providers.rightcode]
name = "RightCode"
base_url = "https://rightcode.example/v1"
wire_api = "responses"
"#
            }),
            None,
        );
        provider.category = Some("custom".to_string());
        let takeover_auth = json!({
            "OPENAI_API_KEY": PROXY_TOKEN_PLACEHOLDER
        });
        let takeover_settings = json!({
            "auth": takeover_auth,
            "config": r#"model_provider = "rightcode"
model = "gpt-5-codex"

[model_providers.rightcode]
name = "RightCode"
base_url = "http://127.0.0.1:42567/v1"
wire_api = "responses"
"#
        });

        service
            .write_codex_live_for_provider(&takeover_settings, Some(&provider))
            .expect("write provider-driven Codex live config");

        let live_auth: Value =
            crate::config::read_json_file(&crate::codex_config::get_codex_auth_path())
                .expect("read live auth");
        assert_eq!(
            live_auth,
            json!({
                "OPENAI_API_KEY": PROXY_TOKEN_PLACEHOLDER
            }),
            "disabled preservation should let third-party switches overwrite auth.json"
        );

        let live_config = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read live config");
        assert!(
            !live_config.contains("experimental_bearer_token"),
            "provider token should stay in auth.json when preservation is disabled"
        );

        crate::settings::update_settings(crate::settings::AppSettings::default())
            .expect("reset settings");
    }

    #[test]
    fn update_toml_base_url_updates_active_model_provider_base_url() {
        let input = r#"
model_provider = "any"
model = "gpt-5.1-codex"
disable_response_storage = true

[model_providers.any]
name = "any"
base_url = "https://anyrouter.top/v1"
wire_api = "responses"
requires_openai_auth = true
"#;

        let new_url = "http://127.0.0.1:5000/v1";
        let output = ProxyService::update_toml_base_url(input, new_url);

        let parsed: toml::Value =
            toml::from_str(&output).expect("updated config should be valid TOML");

        let base_url = parsed
            .get("model_providers")
            .and_then(|v| v.get("any"))
            .and_then(|v| v.get("base_url"))
            .and_then(|v| v.as_str())
            .expect("model_providers.any.base_url should exist");

        assert_eq!(base_url, new_url);
        assert!(
            parsed.get("base_url").is_none(),
            "should not write top-level base_url"
        );

        let wire_api = parsed
            .get("model_providers")
            .and_then(|v| v.get("any"))
            .and_then(|v| v.get("wire_api"))
            .and_then(|v| v.as_str())
            .expect("model_providers.any.wire_api should exist");
        assert_eq!(wire_api, "responses");
    }

    #[test]
    fn apply_codex_proxy_toml_config_forces_local_responses_wire_api() {
        let input = r#"
model_provider = "chat_only"
model = "gpt-5.1-codex"

[model_providers.chat_only]
name = "Chat Only"
base_url = "https://chat-only.example/v1"
wire_api = "chat"
"#;

        let proxy_url = "http://127.0.0.1:5000/v1";
        let output =
            ProxyService::apply_codex_proxy_toml_config_for_provider(input, proxy_url, None);
        let parsed: toml::Value =
            toml::from_str(&output).expect("updated config should be valid TOML");

        let provider = parsed
            .get("model_providers")
            .and_then(|v| v.get("chat_only"))
            .expect("model_providers.chat_only should exist");

        assert_eq!(
            provider.get("base_url").and_then(|v| v.as_str()),
            Some(proxy_url)
        );
        assert_eq!(
            provider.get("wire_api").and_then(|v| v.as_str()),
            Some("responses")
        );
    }

    #[test]
    fn apply_codex_proxy_toml_config_keeps_upstream_model_for_chat_provider() {
        let input = r#"
model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#;
        let mut provider = Provider::with_id(
            "deepseek".to_string(),
            "DeepSeek".to_string(),
            json!({
                "config": input
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..Default::default()
        });

        let proxy_url = "http://127.0.0.1:5000/v1";
        let output = ProxyService::apply_codex_proxy_toml_config_for_provider(
            input,
            proxy_url,
            Some(&provider),
        );
        let parsed: toml::Value =
            toml::from_str(&output).expect("updated config should be valid TOML");

        assert_eq!(
            parsed.get("model").and_then(|v| v.as_str()),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            parsed
                .get("model_providers")
                .and_then(|v| v.get("deepseek"))
                .and_then(|v| v.get("base_url"))
                .and_then(|v| v.as_str()),
            Some(proxy_url)
        );
    }

    #[test]
    fn apply_codex_proxy_toml_config_preserves_model_for_responses_provider() {
        let input = r#"
model_provider = "responses"
model = "upstream-responses-model"

[model_providers.responses]
name = "Responses"
base_url = "https://responses.example/v1"
wire_api = "responses"
"#;
        let mut provider = Provider::with_id(
            "responses".to_string(),
            "Responses".to_string(),
            json!({
                "config": input
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            api_format: Some("openai_responses".to_string()),
            ..Default::default()
        });

        let output = ProxyService::apply_codex_proxy_toml_config_for_provider(
            input,
            "http://127.0.0.1:5000/v1",
            Some(&provider),
        );
        let parsed: toml::Value =
            toml::from_str(&output).expect("updated config should be valid TOML");

        assert_eq!(
            parsed.get("model").and_then(|v| v.as_str()),
            Some("upstream-responses-model")
        );
    }

    #[test]
    fn apply_codex_proxy_toml_config_restores_upstream_model_for_responses_provider() {
        let input = r#"
model_provider = "responses"
model = "gpt-5.4"

[model_providers.responses]
name = "Responses"
base_url = "http://127.0.0.1:5000/v1"
wire_api = "responses"
"#;
        let mut provider = Provider::with_id(
            "responses".to_string(),
            "Responses".to_string(),
            json!({
                "config": r#"model_provider = "responses"
model = "upstream-responses-model"

[model_providers.responses]
name = "Responses"
base_url = "https://responses.example/v1"
wire_api = "responses"
"#
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            api_format: Some("openai_responses".to_string()),
            ..Default::default()
        });

        let output = ProxyService::apply_codex_proxy_toml_config_for_provider(
            input,
            "http://127.0.0.1:5000/v1",
            Some(&provider),
        );
        let parsed: toml::Value =
            toml::from_str(&output).expect("updated config should be valid TOML");

        assert_eq!(
            parsed.get("model").and_then(|v| v.as_str()),
            Some("upstream-responses-model")
        );
    }

    #[test]
    fn update_toml_base_url_falls_back_to_top_level_base_url() {
        let input = r#"
model = "gpt-5.1-codex"
"#;

        let new_url = "http://127.0.0.1:5000/v1";
        let output = ProxyService::update_toml_base_url(input, new_url);

        let parsed: toml::Value =
            toml::from_str(&output).expect("updated config should be valid TOML");

        let base_url = parsed
            .get("base_url")
            .and_then(|v| v.as_str())
            .expect("base_url should exist");

        assert_eq!(base_url, new_url);
    }

    #[tokio::test]
    #[serial]
    async fn sync_claude_token_does_not_add_anthropic_api_key() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.anthropic.com",
                    "ANTHROPIC_AUTH_TOKEN": "stale"
                }
            }),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set current provider");

        let live_config = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "fresh"
            }
        });

        service
            .sync_live_config_to_provider(&AppType::Claude, &live_config)
            .await
            .expect("sync");

        let updated = db
            .get_provider_by_id("p1", "claude")
            .expect("get provider")
            .expect("provider exists");
        let env = updated
            .settings_config
            .get("env")
            .and_then(|v| v.as_object())
            .expect("env object");

        assert_eq!(
            env.get("ANTHROPIC_AUTH_TOKEN").and_then(|v| v.as_str()),
            Some("fresh")
        );
        assert!(
            !env.contains_key("ANTHROPIC_API_KEY"),
            "should not add ANTHROPIC_API_KEY when absent"
        );
    }

    #[tokio::test]
    #[serial]
    async fn sync_claude_token_respects_existing_api_key_field() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.anthropic.com",
                    "ANTHROPIC_API_KEY": "stale"
                }
            }),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set current provider");

        let live_config = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "fresh"
            }
        });

        service
            .sync_live_config_to_provider(&AppType::Claude, &live_config)
            .await
            .expect("sync");

        let updated = db
            .get_provider_by_id("p1", "claude")
            .expect("get provider")
            .expect("provider exists");
        let env = updated
            .settings_config
            .get("env")
            .and_then(|v| v.as_object())
            .expect("env object");

        assert_eq!(
            env.get("ANTHROPIC_API_KEY").and_then(|v| v.as_str()),
            Some("fresh")
        );
        assert!(
            !env.contains_key("ANTHROPIC_AUTH_TOKEN"),
            "should not add ANTHROPIC_AUTH_TOKEN when absent"
        );
    }

    #[tokio::test]
    #[serial]
    async fn switch_proxy_target_updates_live_backup_when_taken_over() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "a-key"
                }
            }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "B".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "b-key"
                }
            }),
            None,
        );
        db.save_provider("claude", &provider_a)
            .expect("save provider a");
        db.save_provider("claude", &provider_b)
            .expect("save provider b");
        db.set_current_provider("claude", "a")
            .expect("set current provider");

        // 模拟"已接管"状态：存在 Live 备份（内容不重要，会被热切换更新）
        db.save_live_backup("claude", "{\"env\":{}}")
            .await
            .expect("seed live backup");

        service
            .switch_proxy_target("claude", "b")
            .await
            .expect("switch proxy target");

        // 断言：本地 settings 的 current provider 已同步
        assert_eq!(
            crate::settings::get_current_provider(&AppType::Claude).as_deref(),
            Some("b")
        );

        // Option A: hot-switch updates current provider but NEVER rewrites the
        // immutable first-open restore snapshot. Managed-expected baseline is C3.
        let backup = db
            .get_live_backup("claude")
            .await
            .expect("get live backup")
            .expect("backup exists");
        assert_eq!(
            backup.original_config, "{\"env\":{}}",
            "proxy target switch must preserve the first-open restore snapshot"
        );
    }

    #[tokio::test]
    #[serial]
    async fn hot_switch_provider_updates_claude_live_while_preserving_takeover_fields() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "a-key",
                    "ANTHROPIC_BASE_URL": "https://api.a.example",
                    "ANTHROPIC_MODEL": "claude-old"
                },
                "permissions": { "allow": ["Bash"] }
            }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "B".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "b-key",
                    "ANTHROPIC_BASE_URL": "https://api.b.example",
                    "ANTHROPIC_MODEL": "claude-new",
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "deepseek-v4-flash",
                    "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME": "DeepSeek V4 Flash",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL": "deepseek-v4-pro[1M]",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "DeepSeek V4 Pro",
                    "ANTHROPIC_DEFAULT_OPUS_MODEL": "deepseek-v4-ultra [1m]",
                    "CLAUDE_CODE_SUBAGENT_MODEL": "deepseek-v4-pro[1M]"
                },
                "permissions": { "allow": ["Read"] }
            }),
            None,
        );

        db.save_provider("claude", &provider_a)
            .expect("save provider a");
        db.save_provider("claude", &provider_b)
            .expect("save provider b");
        db.set_current_provider("claude", "a")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("a"))
            .expect("set local current provider");
        db.save_live_backup(
            "claude",
            &serde_json::to_string(&provider_a.settings_config).expect("serialize provider a"),
        )
        .await
        .expect("seed live backup");
        service
            .write_claude_live(&json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:42567",
                    "ANTHROPIC_API_KEY": PROXY_TOKEN_PLACEHOLDER,
                    "ANTHROPIC_MODEL": "stale-model",
                    "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "Stale Sonnet",
                    "CLAUDE_CODE_SUBAGENT_MODEL": "stale-subagent"
                },
                "permissions": { "allow": ["Bash"] }
            }))
            .expect("seed taken-over live file");

        service
            .hot_switch_provider("claude", "b")
            .await
            .expect("hot switch provider");

        let live = service.read_claude_live().expect("read live config");
        assert_eq!(
            live.get("permissions"),
            provider_b.settings_config.get("permissions"),
            "provider-derived live settings should be refreshed"
        );
        assert_eq!(
            live.get("env")
                .and_then(|env| env.get("ANTHROPIC_API_KEY"))
                .and_then(|v| v.as_str()),
            Some(PROXY_TOKEN_PLACEHOLDER),
            "takeover token placeholder should be preserved"
        );
        assert_eq!(
            live.get("env")
                .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
                .and_then(|v| v.as_str()),
            Some("http://127.0.0.1:42567"),
            "takeover proxy URL should remain active"
        );
        assert!(
            live.get("env")
                .and_then(|env| env.get("ANTHROPIC_MODEL"))
                .is_none(),
            "fallback model override should be removed in takeover mode"
        );
        let live_env = live
            .get("env")
            .and_then(|env| env.as_object())
            .expect("live env");
        assert_eq!(
            live_env
                .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
                .and_then(|v| v.as_str()),
            Some("claude-haiku-4-5"),
            "takeover mode should expose a stable Haiku role model"
        );
        assert_eq!(
            live_env
                .get("ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME")
                .and_then(|v| v.as_str()),
            Some("DeepSeek V4 Flash"),
            "model menu should show the current provider Haiku display name"
        );
        assert_eq!(
            live_env
                .get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                .and_then(|v| v.as_str()),
            Some("claude-sonnet-4-6[1M]"),
            "Sonnet role should carry the local 1M declaration for Claude Code"
        );
        assert_eq!(
            live_env
                .get("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME")
                .and_then(|v| v.as_str()),
            Some("DeepSeek V4 Pro"),
            "stale model display names should be replaced during hot switch"
        );
        assert_eq!(
            live_env
                .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
                .and_then(|v| v.as_str()),
            Some("claude-opus-4-8[1M]"),
            "Opus role should preserve the current provider 1M capability marker"
        );
        assert_eq!(
            live_env
                .get("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME")
                .and_then(|v| v.as_str()),
            Some("deepseek-v4-ultra"),
            "implicit display names should strip the local 1M marker"
        );
        assert_eq!(
            live_env
                .get("CLAUDE_CODE_SUBAGENT_MODEL")
                .and_then(|v| v.as_str()),
            Some("deepseek-v4-pro[1M]"),
            "subagent model should follow the target provider during hot switch"
        );

        // Option A: proxy-safe live refresh follows provider B, while the
        // immutable first-open snapshot remains provider A.
        let backup = db
            .get_live_backup("claude")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let expected = serde_json::to_string(&provider_a.settings_config).expect("serialize");
        assert_eq!(backup.original_config, expected);
    }

    #[tokio::test]
    #[serial]
    async fn hot_switch_provider_serializes_same_app_switches() {
        use tokio::time::{sleep, Duration};

        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "a-key" } }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "B".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "b-key" } }),
            None,
        );
        let provider_c = Provider::with_id(
            "c".to_string(),
            "C".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "c-key" } }),
            None,
        );

        db.save_provider("claude", &provider_a)
            .expect("save provider a");
        db.save_provider("claude", &provider_b)
            .expect("save provider b");
        db.save_provider("claude", &provider_c)
            .expect("save provider c");
        db.set_current_provider("claude", "a")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("a"))
            .expect("set local current provider");
        db.save_live_backup("claude", "{\"env\":{}}")
            .await
            .expect("seed live backup");

        let guard = service.lock_switch_for_test("claude").await;
        let service_for_b = service.clone();
        let service_for_c = service.clone();

        let switch_b = tokio::spawn(async move {
            service_for_b
                .hot_switch_provider("claude", "b")
                .await
                .expect("switch to b")
        });
        sleep(Duration::from_millis(20)).await;
        let switch_c = tokio::spawn(async move {
            service_for_c
                .hot_switch_provider("claude", "c")
                .await
                .expect("switch to c")
        });

        sleep(Duration::from_millis(20)).await;
        drop(guard);

        let outcome_b = switch_b.await.expect("join switch b");
        let outcome_c = switch_c.await.expect("join switch c");
        assert!(outcome_b.logical_target_changed);
        assert!(outcome_c.logical_target_changed);

        assert_eq!(
            crate::settings::get_effective_current_provider(&db, &AppType::Claude)
                .expect("effective current"),
            Some("c".to_string())
        );
        assert_eq!(
            crate::settings::get_current_provider(&AppType::Claude).as_deref(),
            Some("c")
        );
        assert_eq!(
            db.get_current_provider("claude").expect("db current"),
            Some("c".to_string())
        );

        // Option A: concurrent hot-switches still serialize current-provider
        // updates, but they never rewrite the first-open restore snapshot.
        let backup = db
            .get_live_backup("claude")
            .await
            .expect("get live backup")
            .expect("backup exists");
        assert_eq!(
            backup.original_config, "{\"env\":{}}",
            "serialized hot-switches must preserve the first-open restore snapshot"
        );
    }

    #[tokio::test]
    #[serial]
    async fn restore_waits_for_hot_switch_and_restores_first_open_snapshot() {
        use tokio::time::{sleep, Duration};

        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "a-key" } }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "B".to_string(),
            json!({ "env": { "ANTHROPIC_API_KEY": "b-key" } }),
            None,
        );

        db.save_provider("claude", &provider_a)
            .expect("save provider a");
        db.save_provider("claude", &provider_b)
            .expect("save provider b");
        db.set_current_provider("claude", "a")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("a"))
            .expect("set local current provider");
        db.save_live_backup(
            "claude",
            &serde_json::to_string(&provider_a.settings_config).expect("serialize provider a"),
        )
        .await
        .expect("seed live backup");
        service
            .write_claude_live(&json!({ "env": { "ANTHROPIC_API_KEY": "stale" } }))
            .expect("seed live file");

        let guard = service.lock_switch_for_test("claude").await;
        let service_for_switch = service.clone();
        let service_for_restore = service.clone();

        let switch_to_b = tokio::spawn(async move {
            service_for_switch
                .hot_switch_provider("claude", "b")
                .await
                .expect("switch to b")
        });
        sleep(Duration::from_millis(20)).await;
        let restore = tokio::spawn(async move {
            service_for_restore
                .restore_live_config_for_app_with_fallback(&AppType::Claude)
                .await
                .expect("restore claude live")
        });

        sleep(Duration::from_millis(20)).await;
        drop(guard);

        let outcome = switch_to_b.await.expect("join switch");
        restore.await.expect("join restore");
        assert!(outcome.logical_target_changed);

        assert_eq!(
            crate::settings::get_effective_current_provider(&db, &AppType::Claude)
                .expect("effective current"),
            Some("b".to_string())
        );

        // Option A: restore waits for the in-flight hot-switch (serialized by the
        // per-app switch lock), then restores the IMMUTABLE first-open snapshot
        // (provider A). The hot-switch's current-provider update to B stands, but
        // the restore-snapshot source is provider A, not the latest hot-switch.
        let backup = db
            .get_live_backup("claude")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let expected = serde_json::to_string(&provider_a.settings_config).expect("serialize");
        assert_eq!(backup.original_config, expected);
        assert_eq!(
            service.read_claude_live().expect("read live"),
            provider_a.settings_config
        );
    }

    #[tokio::test]
    #[serial]
    async fn update_live_backup_from_provider_applies_claude_common_config() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        db.set_config_snippet(
            "claude",
            Some(
                serde_json::json!({
                    "includeCoAuthoredBy": false
                })
                .to_string(),
            ),
        )
        .expect("set common config snippet");

        let service = ProxyService::new(db.clone());

        let mut provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "token",
                    "ANTHROPIC_BASE_URL": "https://claude.example"
                }
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            common_config_enabled: Some(true),
            ..Default::default()
        });

        service
            .update_live_backup_from_provider("claude", &provider)
            .await
            .expect("update live backup");

        let backup = db
            .get_live_backup("claude")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let stored: Value =
            serde_json::from_str(&backup.original_config).expect("parse backup json");

        assert_eq!(
            stored.get("includeCoAuthoredBy").and_then(|v| v.as_bool()),
            Some(false),
            "common config should be applied into Claude restore backup"
        );
    }

    #[tokio::test]
    #[serial]
    async fn update_live_backup_from_provider_applies_codex_common_config() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        db.set_config_snippet(
            "codex",
            Some("disable_response_storage = true\n".to_string()),
        )
        .expect("set common config snippet");

        let service = ProxyService::new(db.clone());

        let mut provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "token"
                },
                "config": r#"model_provider = "any"
model = "gpt-5"

[model_providers.any]
base_url = "https://codex.example/v1"
"#
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            common_config_enabled: Some(true),
            ..Default::default()
        });

        service
            .update_live_backup_from_provider("codex", &provider)
            .await
            .expect("update live backup");

        let backup = db
            .get_live_backup("codex")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let stored: Value =
            serde_json::from_str(&backup.original_config).expect("parse backup json");
        let config = stored
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config string");

        assert!(
            config.contains("disable_response_storage = true"),
            "common config should be applied into Codex restore backup"
        );
    }

    #[tokio::test]
    #[serial]
    async fn update_live_backup_from_provider_preserves_codex_mcp_servers() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        db.save_live_backup(
            "codex",
            &serde_json::to_string(&json!({
                "auth": {
                    "OPENAI_API_KEY": "old-token"
                },
                "config": r#"model_provider = "any"
model = "gpt-4"

[model_providers.any]
base_url = "https://old.example/v1"

[mcp_servers.echo]
command = "npx"
args = ["echo-server"]
"#
            }))
            .expect("serialize seed backup"),
        )
        .await
        .expect("seed live backup");

        let provider = Provider::with_id(
            "p2".to_string(),
            "P2".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "new-token"
                },
                "config": r#"model_provider = "any"
model = "gpt-5"

[model_providers.any]
base_url = "https://new.example/v1"
"#
            }),
            None,
        );

        service
            .update_live_backup_from_provider("codex", &provider)
            .await
            .expect("update live backup");

        let backup = db
            .get_live_backup("codex")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let stored: Value =
            serde_json::from_str(&backup.original_config).expect("parse backup json");
        let config = stored
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config string");

        assert!(
            config.contains("[mcp_servers.echo]"),
            "existing Codex MCP section should survive proxy hot-switch backup update"
        );
        assert!(
            config.contains("https://new.example/v1"),
            "provider-specific base_url should still update to the new provider"
        );
    }

    #[tokio::test]
    #[serial]
    async fn hot_switch_codex_provider_preserves_first_open_backup_and_proxy_label() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "RightCode".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "rightcode-key"
                },
                "config": r#"model_provider = "rightcode"
model = "gpt-5.4"

[model_providers.rightcode]
name = "RightCode"
base_url = "https://rightcode.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#
            }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "AiHubMix".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "aihubmix-key"
                },
                "config": r#"model_provider = "aihubmix"
model = "gpt-5.4"

[model_providers.aihubmix]
name = "AiHubMix"
base_url = "https://aihubmix.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#
            }),
            None,
        );

        db.save_provider("codex", &provider_a)
            .expect("save provider a");
        db.save_provider("codex", &provider_b)
            .expect("save provider b");
        db.set_current_provider("codex", "a")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("a"))
            .expect("set local current provider");
        db.save_live_backup(
            "codex",
            &serde_json::to_string(&provider_a.settings_config).expect("serialize provider a"),
        )
        .await
        .expect("seed live backup");
        service
            .write_codex_live(&json!({
                "auth": {
                    "OPENAI_API_KEY": PROXY_TOKEN_PLACEHOLDER
                },
                "config": r#"model_provider = "rightcode"
model = "gpt-5.4"

[model_providers.rightcode]
name = "RightCode"
base_url = "http://127.0.0.1:42567/v1"
wire_api = "responses"
requires_openai_auth = true
"#
            }))
            .expect("seed taken-over Codex live config");

        service
            .hot_switch_provider("codex", "b")
            .await
            .expect("hot switch Codex provider");

        let backup = db
            .get_live_backup("codex")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let stored: Value =
            serde_json::from_str(&backup.original_config).expect("parse backup json");
        let backup_config = stored
            .get("config")
            .and_then(|v| v.as_str())
            .expect("backup config string");
        let parsed_backup: toml::Value =
            toml::from_str(backup_config).expect("parse backup config");
        // Option A: hot-switch must not rewrite the immutable first-open backup
        // (provider A / RightCode). Only the proxy-safe live label follows B.
        assert_eq!(
            parsed_backup.get("model_provider").and_then(|v| v.as_str()),
            Some("rightcode"),
            "first-open restore backup should remain provider A"
        );
        let backup_model_providers = parsed_backup
            .get("model_providers")
            .and_then(|v| v.as_table())
            .expect("backup model_providers");
        assert!(backup_model_providers.get("custom").is_none());
        assert_eq!(
            backup_model_providers
                .get("rightcode")
                .and_then(|v| v.get("base_url"))
                .and_then(|v| v.as_str()),
            Some("https://rightcode.example/v1"),
            "first-open backup should retain provider A endpoint"
        );

        let live = service.read_codex_live().expect("read Codex live config");
        let live_config = live
            .get("config")
            .and_then(|v| v.as_str())
            .expect("live config string");
        let parsed_live: toml::Value = toml::from_str(live_config).expect("parse live config");
        assert_eq!(
            parsed_live.get("model_provider").and_then(|v| v.as_str()),
            Some("aihubmix"),
            "hot-switched Codex live config should expose the selected provider"
        );
        assert_eq!(
            parsed_live
                .get("model_providers")
                .and_then(|v| v.get("aihubmix"))
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str()),
            Some("AiHubMix"),
            "Codex app provider label should follow the selected provider"
        );
        assert_eq!(
            parsed_live
                .get("model_providers")
                .and_then(|v| v.get("aihubmix"))
                .and_then(|v| v.get("base_url"))
                .and_then(|v| v.as_str()),
            Some("http://127.0.0.1:42567/v1"),
            "taken-over live config should stay pointed at the local proxy"
        );

        service
            .restore_live_config_for_app_with_fallback(&AppType::Codex)
            .await
            .expect("restore Codex live config");

        let live = service.read_codex_live().expect("read Codex live config");
        let live_config = live
            .get("config")
            .and_then(|v| v.as_str())
            .expect("live config string");
        let parsed_live: toml::Value = toml::from_str(live_config).expect("parse live config");
        assert_eq!(
            parsed_live.get("model_provider").and_then(|v| v.as_str()),
            Some("rightcode"),
            "restore should return to the immutable first-open provider A config"
        );
        assert_eq!(
            live.get("auth")
                .and_then(|auth| auth.get("OPENAI_API_KEY"))
                .and_then(|v| v.as_str()),
            Some("rightcode-key"),
            "restore should use the first-open provider A auth"
        );
    }

    #[tokio::test]
    #[serial]
    async fn hot_switch_codex_chat_provider_updates_live_provider_display() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let provider_a = Provider::with_id(
            "a".to_string(),
            "Responses".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "responses-key"
                },
                "config": r#"model_provider = "stable"
model = "responses-model"

[model_providers.stable]
name = "Stable"
base_url = "https://responses.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#
            }),
            None,
        );
        let mut provider_b = Provider::with_id(
            "b".to_string(),
            "DeepSeek".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "deepseek-key"
                },
                "config": r#"model_provider = "deepseek"
model = "deepseek-v4-flash"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
requires_openai_auth = true
"#
            }),
            None,
        );
        provider_b.meta = Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..Default::default()
        });

        db.save_provider("codex", &provider_a)
            .expect("save provider a");
        db.save_provider("codex", &provider_b)
            .expect("save provider b");
        db.set_current_provider("codex", "a")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("a"))
            .expect("set local current provider");
        db.save_live_backup(
            "codex",
            &serde_json::to_string(&provider_a.settings_config).expect("serialize provider a"),
        )
        .await
        .expect("seed live backup");
        service
            .write_codex_live(&json!({
                "auth": {
                    "OPENAI_API_KEY": PROXY_TOKEN_PLACEHOLDER
                },
                "config": r#"model_provider = "stable"
model = "responses-model"

[model_providers.stable]
name = "Stable"
base_url = "http://127.0.0.1:42567/v1"
wire_api = "responses"
requires_openai_auth = true
"#
            }))
            .expect("seed taken-over Codex live config");

        service
            .hot_switch_provider("codex", "b")
            .await
            .expect("hot switch Codex provider");

        let live = service.read_codex_live().expect("read Codex live config");
        let live_config = live
            .get("config")
            .and_then(|v| v.as_str())
            .expect("live config string");
        let parsed_live: toml::Value = toml::from_str(live_config).expect("parse live config");

        assert_eq!(
            parsed_live.get("model_provider").and_then(|v| v.as_str()),
            Some("deepseek")
        );
        assert_eq!(
            parsed_live
                .get("model_providers")
                .and_then(|v| v.get("deepseek"))
                .and_then(|v| v.get("name"))
                .and_then(|v| v.as_str()),
            Some("DeepSeek")
        );
        assert_eq!(
            parsed_live
                .get("model_providers")
                .and_then(|v| v.get("deepseek"))
                .and_then(|v| v.get("base_url"))
                .and_then(|v| v.as_str()),
            Some("http://127.0.0.1:42567/v1")
        );
        assert_eq!(
            parsed_live.get("model").and_then(|v| v.as_str()),
            Some("deepseek-v4-flash")
        );
        assert_eq!(
            live.get("auth")
                .and_then(|auth| auth.get("OPENAI_API_KEY"))
                .and_then(|v| v.as_str()),
            Some(PROXY_TOKEN_PLACEHOLDER)
        );
    }

    #[tokio::test]
    #[serial]
    async fn update_live_backup_from_provider_keeps_new_codex_mcp_entries_on_conflict() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        db.save_live_backup(
            "codex",
            &serde_json::to_string(&json!({
                "auth": {
                    "OPENAI_API_KEY": "old-token"
                },
                "config": r#"[mcp_servers.shared]
command = "old-command"

[mcp_servers.legacy]
command = "legacy-command"
"#
            }))
            .expect("serialize seed backup"),
        )
        .await
        .expect("seed live backup");

        let provider = Provider::with_id(
            "p2".to_string(),
            "P2".to_string(),
            json!({
                "auth": {
                    "OPENAI_API_KEY": "new-token"
                },
                "config": r#"[mcp_servers.shared]
command = "new-command"

[mcp_servers.latest]
command = "latest-command"
"#
            }),
            None,
        );

        service
            .update_live_backup_from_provider("codex", &provider)
            .await
            .expect("update live backup");

        let backup = db
            .get_live_backup("codex")
            .await
            .expect("get live backup")
            .expect("backup exists");
        let stored: Value =
            serde_json::from_str(&backup.original_config).expect("parse backup json");
        let config = stored
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config string");
        let parsed: toml::Value = toml::from_str(config).expect("parse merged codex config");

        let mcp_servers = parsed
            .get("mcp_servers")
            .expect("mcp_servers should be present");
        assert_eq!(
            mcp_servers
                .get("shared")
                .and_then(|v| v.get("command"))
                .and_then(|v| v.as_str()),
            Some("new-command"),
            "new provider/common-config MCP definition should win on conflict"
        );
        assert_eq!(
            mcp_servers
                .get("legacy")
                .and_then(|v| v.get("command"))
                .and_then(|v| v.as_str()),
            Some("legacy-command"),
            "backup-only MCP entries should still be preserved"
        );
        assert_eq!(
            mcp_servers
                .get("latest")
                .and_then(|v| v.get("command"))
                .and_then(|v| v.as_str()),
            Some("latest-command"),
            "new MCP entries should remain in the restore backup"
        );
    }

    #[tokio::test]
    #[serial]
    async fn provider_switch_with_restored_codex_backup_refreshes_catalog_and_common_config() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        seed_codex_model_template();

        let db = Arc::new(Database::memory().expect("init db"));
        let state = crate::store::AppState::new(db.clone());

        db.set_config_snippet(
            "codex",
            Some(
                r#"[mcp_servers.shared]
command = "shared-command"
"#
                .to_string(),
            ),
        )
        .expect("set common config snippet");

        let proxy_config = ProxyConfig {
            listen_port: 0,
            ..Default::default()
        };
        db.update_proxy_config(proxy_config)
            .await
            .expect("set test proxy config");
        state
            .proxy_service
            .start()
            .await
            .expect("start proxy server");

        let config_a = r#"model_provider = "provider-a"
model = "model-a"

[model_providers.provider-a]
name = "ProviderA"
base_url = "https://provider-a.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#;
        let config_b = r#"model_provider = "provider-b"
model = "model-b"

[model_providers.provider-b]
name = "ProviderB"
base_url = "https://provider-b.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#;

        let provider_a = Provider::with_id(
            "a".to_string(),
            "ProviderA".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "key-a" },
                "config": config_a,
                "modelCatalog": { "models": [{ "model": "model-a" }] }
            }),
            None,
        );
        let mut provider_b = Provider::with_id(
            "b".to_string(),
            "ProviderB".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "key-b" },
                "config": config_b,
                "modelCatalog": { "models": [{ "model": "model-b" }] }
            }),
            None,
        );
        provider_b.meta = Some(ProviderMeta {
            common_config_enabled: Some(true),
            ..Default::default()
        });

        db.save_provider("codex", &provider_a)
            .expect("save provider a");
        db.save_provider("codex", &provider_b)
            .expect("save provider b");
        db.set_current_provider("codex", "a")
            .expect("set current provider a");
        crate::settings::set_current_provider(&AppType::Codex, Some("a"))
            .expect("set local current provider a");

        state
            .proxy_service
            .write_codex_live_for_provider(&provider_a.settings_config, Some(&provider_a))
            .expect("seed live codex config");
        assert!(
            !state
                .proxy_service
                .detect_takeover_in_live_config_for_app(&AppType::Codex),
            "seeded live config should not be proxy-taken-over"
        );

        db.save_live_backup(
            "codex",
            &serde_json::to_string(&provider_a.settings_config).expect("serialize backup"),
        )
        .await
        .expect("seed restored backup");

        // C2a: the truth source for whether a switch writes live is proxy_config,
        // not backup presence. This test's intent is "AGS owns Codex live and a
        // switch rewrites the selected provider's real upstream config (with
        // catalog)" — that is takeover_enabled=true + route_mode=direct.
        let mut codex_cfg = db
            .get_proxy_config_for_app("codex")
            .await
            .expect("get codex proxy config");
        codex_cfg.takeover_enabled = true;
        codex_cfg.route_mode = RouteMode::Direct;
        db.update_proxy_config_for_app(codex_cfg)
            .await
            .expect("enable codex direct takeover");

        crate::services::provider::ProviderService::switch(&state, AppType::Codex, "b")
            .expect("provider switch to provider b");
        state.proxy_service.stop().await.expect("stop proxy server");

        let catalog_path = crate::codex_config::get_codex_model_catalog_path();
        assert!(
            catalog_path.exists(),
            "agent-switch-model-catalog.json must be created on provider switch"
        );
        let catalog_text = std::fs::read_to_string(&catalog_path).expect("read catalog json");
        let catalog: serde_json::Value =
            serde_json::from_str(&catalog_text).expect("parse catalog json");
        let slugs: Vec<&str> = catalog
            .get("models")
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| e.get("slug").and_then(|s| s.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            slugs.contains(&"model-b"),
            "catalog must contain provider B's model after switch; got: {slugs:?}"
        );
        assert!(
            !slugs.contains(&"model-a"),
            "catalog must not contain stale provider A model after switch; got: {slugs:?}"
        );

        let config_path = crate::codex_config::get_codex_config_path();
        let config_text = std::fs::read_to_string(&config_path).expect("read config.toml");
        assert!(
            config_text.contains("model_catalog_json"),
            "config.toml must reference model_catalog_json after switch"
        );
        assert!(
            config_text.contains("[mcp_servers.shared]"),
            "config.toml must keep common config after switch"
        );
        assert!(
            config_text.contains(r#"command = "shared-command""#),
            "config.toml must include common config content after switch"
        );
    }

    #[tokio::test]
    #[serial]
    async fn provider_switch_with_restored_codex_backup_propagates_catalog_write_errors() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        seed_codex_model_template();

        let db = Arc::new(Database::memory().expect("init db"));
        let state = crate::store::AppState::new(db.clone());

        let proxy_config = ProxyConfig {
            listen_port: 0,
            ..Default::default()
        };
        db.update_proxy_config(proxy_config)
            .await
            .expect("set test proxy config");
        state
            .proxy_service
            .start()
            .await
            .expect("start proxy server");

        let config_a = r#"model_provider = "provider-a"
model = "model-a"

[model_providers.provider-a]
name = "ProviderA"
base_url = "https://provider-a.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#;
        let config_b = r#"model_provider = "provider-b"
model = "model-b"

[model_providers.provider-b]
name = "ProviderB"
base_url = "https://provider-b.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#;

        let provider_a = Provider::with_id(
            "a".to_string(),
            "ProviderA".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "key-a" },
                "config": config_a,
                "modelCatalog": { "models": [{ "model": "model-a" }] }
            }),
            None,
        );
        let provider_b = Provider::with_id(
            "b".to_string(),
            "ProviderB".to_string(),
            serde_json::json!({
                "auth": { "OPENAI_API_KEY": "key-b" },
                "config": config_b,
                "modelCatalog": { "models": [{ "model": "model-b" }] }
            }),
            None,
        );

        db.save_provider("codex", &provider_a)
            .expect("save provider a");
        db.save_provider("codex", &provider_b)
            .expect("save provider b");
        db.set_current_provider("codex", "a")
            .expect("set current provider a");
        crate::settings::set_current_provider(&AppType::Codex, Some("a"))
            .expect("set local current provider a");

        state
            .proxy_service
            .write_codex_live_for_provider(&provider_a.settings_config, Some(&provider_a))
            .expect("seed live codex config");
        assert!(
            !state
                .proxy_service
                .detect_takeover_in_live_config_for_app(&AppType::Codex),
            "seeded live config should not be proxy-taken-over"
        );

        db.save_live_backup(
            "codex",
            &serde_json::to_string(&provider_a.settings_config).expect("serialize backup"),
        )
        .await
        .expect("seed restored backup");

        // C2a: the truth source for whether a provider switch writes live is
        // proxy_config, not backup presence. This scenario (switch writes the new
        // provider's real upstream catalog to live) is direct-mode takeover.
        let mut codex_config = db
            .get_proxy_config_for_app("codex")
            .await
            .expect("get codex proxy config");
        codex_config.takeover_enabled = true;
        codex_config.route_mode = RouteMode::Direct;
        db.update_proxy_config_for_app(codex_config)
            .await
            .expect("enable direct takeover for codex");

        let catalog_path = crate::codex_config::get_codex_model_catalog_path();
        if catalog_path.exists() {
            std::fs::remove_file(&catalog_path).expect("remove catalog file");
        }
        std::fs::create_dir_all(&catalog_path).expect("turn catalog path into directory");

        let err = crate::services::provider::ProviderService::switch(&state, AppType::Codex, "b")
            .expect_err("provider switch should fail when catalog cannot be written");
        state.proxy_service.stop().await.expect("stop proxy server");

        let message = err.to_string();
        assert!(
            message.contains("写入 Codex 配置失败") || message.contains("原子替换失败"),
            "switch should surface catalog write failure, got: {message}"
        );
    }

    /// Regression: turning proxy takeover off restores Live from the backup. The
    /// backup snapshot is `read_codex_live_settings()` output (`{auth, config}`,
    /// never an inline `modelCatalog`). The restore must NOT route the config
    /// through catalog projection, which would see no specs and strip the
    /// `model_catalog_json` pointer — silently dropping the user's Codex model
    /// mapping from Live even though the DB SSOT still holds it.
    #[tokio::test]
    #[serial]
    async fn codex_restore_from_backup_preserves_model_catalog_pointer() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        // Pre-takeover Live state: config.toml points at the cc-switch generated
        // catalog file, and that file exists on disk (takeover never touches it).
        let catalog_path = crate::codex_config::get_codex_model_catalog_path();
        if let Some(parent) = catalog_path.parent() {
            std::fs::create_dir_all(parent).expect("create codex dir");
        }
        std::fs::write(
            &catalog_path,
            r#"{"models":[{"slug":"deepseek-v4-flash"}]}"#,
        )
        .expect("seed generated catalog file");

        let pointer = catalog_path.to_string_lossy().replace('\\', "/");
        let backup_config = format!(
            "model_provider = \"custom\"\n\
             model = \"deepseek-v4-flash\"\n\
             model_catalog_json = \"{pointer}\"\n\n\
             [model_providers.custom]\n\
             name = \"DeepSeek\"\n\
             base_url = \"https://api.deepseek.example/v1\"\n\
             wire_api = \"responses\"\n"
        );
        let backup_json = serde_json::to_string(&json!({
            "auth": { "OPENAI_API_KEY": "deepseek-key" },
            "config": backup_config,
        }))
        .expect("serialize backup");
        db.save_live_backup("codex", &backup_json)
            .await
            .expect("seed live backup");

        // Turning takeover off restores Live from this backup.
        service
            .restore_live_config_for_app_with_fallback(&AppType::Codex)
            .await
            .expect("restore codex live from backup");

        let restored = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read restored config.toml");
        assert!(
            restored.contains("model_catalog_json"),
            "restore must preserve the model_catalog_json pointer, got:\n{restored}"
        );
        assert!(
            restored.contains(pointer.as_str()),
            "restored pointer must still reference the cc-switch generated catalog file"
        );
    }

    /// Regression: a hot-switch during takeover rebuilds the backup from the DB
    /// provider (`update_live_backup_from_provider`), so the backup carries an
    /// inline `modelCatalog` (DB SSOT) but a `config.toml` text WITHOUT a
    /// `model_catalog_json` pointer. Restoring that backup must project the
    /// inline catalog — (re)generating both the catalog file and the pointer —
    /// or the Codex model mapping vanishes from Live after takeover-off.
    #[tokio::test]
    #[serial]
    async fn codex_restore_from_backup_projects_inline_model_catalog() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        // Catalog projection needs a model template; seed `models_cache.json`
        // with the template slug so we don't depend on the `codex` CLI.
        let codex_dir = crate::codex_config::get_codex_config_dir();
        std::fs::create_dir_all(&codex_dir).expect("create codex dir");
        std::fs::write(
            codex_dir.join("models_cache.json"),
            r#"{"models":[{"slug":"gpt-5.5"}]}"#,
        )
        .expect("seed models_cache template");

        // Provider-rebuilt backup shape: inline modelCatalog, pointer-less config.
        let backup_json = serde_json::to_string(&json!({
            "auth": { "OPENAI_API_KEY": "deepseek-key" },
            "config": "model_provider = \"custom\"\nmodel = \"deepseek-v4-flash\"\n\n[model_providers.custom]\nname = \"DeepSeek\"\nbase_url = \"https://api.deepseek.example/v1\"\nwire_api = \"responses\"\n",
            "modelCatalog": {
                "models": [
                    { "model": "deepseek-v4-flash", "displayName": "DeepSeek V4 Flash", "contextWindow": 1_000_000 }
                ]
            }
        }))
        .expect("serialize backup");
        db.save_live_backup("codex", &backup_json)
            .await
            .expect("seed live backup");

        service
            .restore_live_config_for_app_with_fallback(&AppType::Codex)
            .await
            .expect("restore codex live from backup");

        let restored = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read restored config.toml");
        let catalog_path = crate::codex_config::get_codex_model_catalog_path();
        assert!(
            restored.contains("model_catalog_json"),
            "restore must (re)generate the model_catalog_json pointer from inline catalog, got:\n{restored}"
        );
        assert!(
            catalog_path.exists(),
            "restore must generate the cc-switch catalog file on disk"
        );
        let catalog: Value = serde_json::from_str(
            &std::fs::read_to_string(&catalog_path).expect("read generated catalog"),
        )
        .expect("parse generated catalog");
        let slugs: Vec<&str> = catalog
            .get("models")
            .and_then(|m| m.as_array())
            .expect("catalog models")
            .iter()
            .filter_map(|m| m.get("slug").and_then(|s| s.as_str()))
            .collect();
        assert!(
            slugs.contains(&"deepseek-v4-flash"),
            "generated catalog must contain the inline model, got slugs: {slugs:?}"
        );
    }

    /// Regression: a provider-rebuilt backup can pair an inline `modelCatalog`
    /// with EMPTY `auth.json` (`{}`) — the bearer-token / Mobile-compat shape
    /// where the API key lives in the config's `experimental_bearer_token`. The
    /// empty-auth restore branch deletes `auth.json` and writes config raw; it
    /// must still project the inline catalog (decision is orthogonal to auth), or
    /// the model mapping vanishes on takeover-off for this provider shape.
    #[tokio::test]
    #[serial]
    async fn codex_restore_empty_auth_backup_still_projects_inline_catalog() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let codex_dir = crate::codex_config::get_codex_config_dir();
        std::fs::create_dir_all(&codex_dir).expect("create codex dir");
        std::fs::write(
            codex_dir.join("models_cache.json"),
            r#"{"models":[{"slug":"gpt-5.5"}]}"#,
        )
        .expect("seed models_cache template");

        // Empty auth.json + key carried in config.toml's experimental_bearer_token,
        // plus the inline modelCatalog (DB SSOT).
        let backup_json = serde_json::to_string(&json!({
            "auth": {},
            "config": "model_provider = \"custom\"\nmodel = \"deepseek-v4-flash\"\n\n[model_providers.custom]\nname = \"DeepSeek\"\nbase_url = \"https://api.deepseek.example/v1\"\nwire_api = \"responses\"\nexperimental_bearer_token = \"sk-deepseek\"\n",
            "modelCatalog": {
                "models": [ { "model": "deepseek-v4-flash", "displayName": "DeepSeek V4 Flash" } ]
            }
        }))
        .expect("serialize backup");
        db.save_live_backup("codex", &backup_json)
            .await
            .expect("seed live backup");

        service
            .restore_live_config_for_app_with_fallback(&AppType::Codex)
            .await
            .expect("restore codex live from backup");

        let restored = std::fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read restored config.toml");
        assert!(
            restored.contains("model_catalog_json"),
            "empty-auth restore must still project the inline catalog pointer, got:\n{restored}"
        );
        assert!(
            crate::codex_config::get_codex_model_catalog_path().exists(),
            "empty-auth restore must generate the cc-switch catalog file"
        );
        assert!(
            !crate::codex_config::get_codex_auth_path().exists(),
            "empty-auth restore must delete auth.json rather than write an empty one"
        );
    }

    /// Regression: when the backup row itself contains the proxy placeholder
    /// (a corrupted state where previous start/stop cycles saved the proxy
    /// config as the "original Live"), restore must NOT write it back to Live.
    /// It should fall through to the SSOT (current provider) path and rebuild
    /// Live from the provider DB instead.
    #[tokio::test]
    #[serial]
    async fn restore_falls_through_to_ssot_when_backup_is_proxy_placeholder() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        // Seed DB with a current provider that has a real API key
        let provider = Provider::with_id(
            "p1".to_string(),
            "P1".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.minimaxi.com/anthropic",
                    "ANTHROPIC_API_KEY": "real-key-from-db"
                }
            }),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set current provider");

        // Seed backup with proxy placeholder (the corrupted state)
        let corrupted_backup = serde_json::to_string(&json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": PROXY_TOKEN_PLACEHOLDER,
                "ANTHROPIC_BASE_URL": "http://127.0.0.1:42567"
            }
        }))
        .expect("serialize corrupted backup");
        db.save_live_backup("claude", &corrupted_backup)
            .await
            .expect("seed corrupted backup");

        // Seed Live with the same proxy placeholder (matches the corrupted state)
        service
            .write_claude_live(&json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": PROXY_TOKEN_PLACEHOLDER,
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:42567"
                }
            }))
            .expect("seed taken-over live file");

        // Restore: must NOT use the corrupted backup
        service
            .restore_live_config_for_app_with_fallback(&AppType::Claude)
            .await
            .expect("restore should succeed via SSOT");

        // The backup should still be the corrupted one (we didn't touch it on this path)
        let backup_after = db
            .get_live_backup("claude")
            .await
            .expect("get backup")
            .expect("backup still exists");
        assert_eq!(
            backup_after.original_config, corrupted_backup,
            "restore must NOT overwrite the corrupted backup"
        );

        // Live should now reflect the SSOT (provider DB), NOT the proxy URL
        let restored_live = service.read_claude_live().expect("read live");
        let restored_url = restored_live
            .get("env")
            .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
            .and_then(|v| v.as_str());
        assert_eq!(
            restored_url,
            Some("https://api.minimaxi.com/anthropic"),
            "Live must be rebuilt from SSOT, not from the corrupted backup"
        );
        let restored_key = restored_live
            .get("env")
            .and_then(|env| env.get("ANTHROPIC_API_KEY"))
            .and_then(|v| v.as_str());
        assert_eq!(
            restored_key,
            Some("real-key-from-db"),
            "Live must carry the real API key from the provider DB"
        );
        assert_ne!(
            restored_live
                .get("env")
                .and_then(|env| env.get("ANTHROPIC_AUTH_TOKEN"))
                .and_then(|v| v.as_str()),
            Some(PROXY_TOKEN_PLACEHOLDER),
            "Live must not still carry the proxy placeholder"
        );
    }

    /// Regression: when Live is already a proxy placeholder (a corrupted state
    /// where previous stop failed to restore), backup must NOT overwrite a
    /// previously-good backup with the proxy config. This prevents the bug
    /// where stop-then-start cycles permanently corrupt the backup.
    #[tokio::test]
    #[serial]
    async fn backup_skips_when_live_is_already_proxy_placeholder() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        // Seed a GOOD backup (the "real" original Live)
        let good_backup = serde_json::to_string(&json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.minimaxi.com/anthropic",
                "ANTHROPIC_AUTH_TOKEN": "real-token"
            }
        }))
        .expect("serialize good backup");
        db.save_live_backup("claude", &good_backup)
            .await
            .expect("seed good backup");

        // Seed Live with proxy placeholder (the corrupted state)
        service
            .write_claude_live(&json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": PROXY_TOKEN_PLACEHOLDER,
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:42567"
                }
            }))
            .expect("seed taken-over live file");

        // Call backup_live_config_strict: must skip
        service
            .backup_live_config_strict(&AppType::Claude)
            .await
            .expect("backup should succeed (no-op when live is placeholder)");

        // The good backup must still be intact
        let backup_after = db
            .get_live_backup("claude")
            .await
            .expect("get backup")
            .expect("backup still exists");
        assert_eq!(
            backup_after.original_config, good_backup,
            "must not overwrite a good backup with a proxy placeholder"
        );
    }

    /// Regression: when ALL apps have Live=proxy-placeholder (worst-case
    /// corrupted state), the bulk `backup_live_configs` path used by
    /// `start_with_takeover` must skip every save — instead of overwriting
    /// good backups with the proxy config.
    #[tokio::test]
    #[serial]
    async fn bulk_backup_skips_all_when_live_is_proxy_placeholder() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        // Seed good backups for all three apps
        let good_backup = serde_json::to_string(&json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "real-token"
            }
        }))
        .expect("serialize good backup");
        db.save_live_backup("claude", &good_backup)
            .await
            .expect("seed claude backup");

        let codex_good_backup = serde_json::to_string(&json!({
            "auth": { "OPENAI_API_KEY": "real-codex-token" }
        }))
        .expect("serialize codex good backup");
        db.save_live_backup("codex", &codex_good_backup)
            .await
            .expect("seed codex backup");

        let gemini_good_backup = serde_json::to_string(&json!({
            "env": { "GEMINI_API_KEY": "real-gemini-key" }
        }))
        .expect("serialize gemini good backup");
        db.save_live_backup("gemini", &gemini_good_backup)
            .await
            .expect("seed gemini backup");

        // Seed all three Live files with proxy placeholders
        service
            .write_claude_live(&json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": PROXY_TOKEN_PLACEHOLDER,
                    "ANTHROPIC_BASE_URL": "http://127.0.0.1:42567"
                }
            }))
            .expect("seed claude live");
        let codex_dir = crate::codex_config::get_codex_config_dir();
        std::fs::create_dir_all(&codex_dir).expect("create codex dir");
        std::fs::write(
            crate::codex_config::get_codex_config_path(),
            r#"model_provider = "custom"

[model_providers.custom]
name = "Custom"
base_url = "http://127.0.0.1:42567/v1"
wire_api = "chat"
experimental_bearer_token = "PROXY_MANAGED"
"#,
        )
        .expect("seed codex config.toml");
        std::fs::write(
            crate::codex_config::get_codex_auth_path(),
            r#"{"OPENAI_API_KEY":"PROXY_MANAGED"}"#,
        )
        .expect("seed codex auth.json");
        let gemini_env_path = crate::gemini_config::get_gemini_env_path();
        if let Some(parent) = gemini_env_path.parent() {
            std::fs::create_dir_all(parent).expect("create gemini dir");
        }
        std::fs::write(&gemini_env_path, "GEMINI_API_KEY=PROXY_MANAGED\n")
            .expect("seed gemini env");

        // Call bulk backup: must skip all three apps
        service
            .backup_live_configs()
            .await
            .expect("bulk backup should succeed (no-op when all live are placeholders)");

        // All three good backups must still be intact
        for (app_type, original) in [
            ("claude", good_backup.as_str()),
            ("codex", codex_good_backup.as_str()),
            ("gemini", gemini_good_backup.as_str()),
        ] {
            let backup_after = db
                .get_live_backup(app_type)
                .await
                .expect("get backup")
                .expect("backup still exists");
            assert_eq!(
                backup_after.original_config, original,
                "must not overwrite good backup for {app_type} with proxy placeholder"
            );
        }
    }

    /// C1：启动路径只回收所有权，不自动重新接管；direct 保留 live，proxy 有备份则恢复。
    #[tokio::test]
    #[serial]
    async fn recover_from_crash_clears_ownership_without_retakeover() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        // direct 模块：写 live，标记接管，但启动后应只清 ownership，不改 live。
        let claude_live = serde_json::json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.anthropic.example",
                "ANTHROPIC_AUTH_TOKEN": "direct-token"
            }
        });
        service
            .write_claude_live(&claude_live)
            .expect("seed claude live");
        let mut direct_cfg = db
            .get_proxy_config_for_app("claude")
            .await
            .expect("get claude config");
        direct_cfg.takeover_enabled = true;
        direct_cfg.route_mode = RouteMode::Direct;
        db.update_proxy_config_for_app(direct_cfg)
            .await
            .expect("mark claude direct takeover");
        db.save_live_backup(
            "claude",
            r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"stale-snapshot"}}"#,
        )
        .await
        .expect("seed direct snapshot that must be abandoned");

        // proxy 模块：live 是占位，backup 为真实上游；恢复后 ownership 清空。
        let codex_backup = serde_json::json!({
            "auth": { "OPENAI_API_KEY": "real-key" },
            "config": "model_provider = \"custom\"\nmodel = \"gpt-test\"\n"
        });
        db.save_live_backup(
            "codex",
            &serde_json::to_string(&codex_backup).expect("serialize codex backup"),
        )
        .await
        .expect("seed codex backup");
        service
            .write_codex_live(&serde_json::json!({
                "auth": { "OPENAI_API_KEY": "PROXY_MANAGED" },
                "config": "model_provider = \"cc-switch-proxy\"\n"
            }))
            .expect("seed codex proxy residue");
        let mut proxy_cfg = db
            .get_proxy_config_for_app("codex")
            .await
            .expect("get codex config");
        proxy_cfg.takeover_enabled = true;
        proxy_cfg.route_mode = RouteMode::Proxy;
        db.update_proxy_config_for_app(proxy_cfg)
            .await
            .expect("mark codex proxy takeover");

        // 残留运行镜像必须可被启动路径归零。
        let mut global = db
            .get_global_proxy_config()
            .await
            .expect("get global proxy config");
        global.proxy_enabled = true;
        db.update_global_proxy_config(global)
            .await
            .expect("seed crash proxy_enabled mirror");

        db.reset_proxy_runtime_mirror()
            .await
            .expect("reset runtime mirror");
        service
            .recover_from_crash()
            .await
            .expect("recover leftover ownership");

        let claude_after = db
            .get_proxy_config_for_app("claude")
            .await
            .expect("claude after");
        assert!(
            !claude_after.takeover_enabled,
            "direct ownership must clear"
        );
        assert!(
            db.get_live_backup("claude")
                .await
                .expect("claude backup")
                .is_none(),
            "direct must abandon snapshot"
        );
        let claude_live_after = service.read_claude_live().expect("read claude live");
        assert_eq!(
            claude_live_after
                .get("env")
                .and_then(|env| env.get("ANTHROPIC_AUTH_TOKEN"))
                .and_then(|v| v.as_str()),
            Some("direct-token"),
            "direct recover must not rewrite live"
        );

        let codex_after = db
            .get_proxy_config_for_app("codex")
            .await
            .expect("codex after");
        assert!(!codex_after.takeover_enabled, "proxy ownership must clear");
        assert!(
            db.get_live_backup("codex")
                .await
                .expect("codex backup")
                .is_none(),
            "proxy snapshot must be deleted after successful restore"
        );
        let codex_live_after = service.read_codex_live().expect("read codex live");
        assert_eq!(
            codex_live_after
                .get("auth")
                .and_then(|auth| auth.get("OPENAI_API_KEY"))
                .and_then(|v| v.as_str()),
            Some("real-key"),
            "proxy residue must restore from backup"
        );

        let global_after = db.get_global_proxy_config().await.expect("global after");
        assert!(
            !global_after.proxy_enabled,
            "crash proxy_enabled mirror must reset to false"
        );

        // 启动不得重接管：recover 后 set_takeover(..., true) 不应被自动调用；
        // 这里断言所有模块 takeover_enabled=false，且网关未运行。
        let status = service
            .get_takeover_status()
            .await
            .expect("takeover status");
        for app in AppType::all() {
            assert!(
                !status.for_app(&app).takeover_enabled,
                "{} must remain disabled after startup recovery",
                app.as_str()
            );
        }
        assert!(!service.is_running().await, "gateway must not auto-start");
    }

    /// C1：仅 proxy 路由接管阻止用户停网关；direct 不阻止。
    #[tokio::test]
    async fn proxy_route_takeovers_blocks_only_proxy_modules() {
        let db = Arc::new(Database::memory().expect("init db"));
        let service = ProxyService::new(db.clone());

        let mut proxy = db.get_proxy_config_for_app("gemini").await.expect("gemini");
        proxy.takeover_enabled = true;
        proxy.route_mode = RouteMode::Proxy;
        db.update_proxy_config_for_app(proxy)
            .await
            .expect("enable gemini proxy");

        let mut direct = db.get_proxy_config_for_app("claude").await.expect("claude");
        direct.takeover_enabled = true;
        direct.route_mode = RouteMode::Direct;
        db.update_proxy_config_for_app(direct)
            .await
            .expect("enable claude direct");

        let blocked = service
            .proxy_route_takeovers()
            .await
            .expect("list blocked modules");
        assert_eq!(blocked, vec!["gemini".to_string()]);

        let status = service.get_takeover_status().await.expect("status");
        assert!(status.claude.takeover_enabled);
        assert_eq!(status.claude.route_mode, RouteMode::Direct);
        assert!(status.gemini.takeover_enabled);
        assert_eq!(status.gemini.route_mode, RouteMode::Proxy);
        assert!(!status.hermes.takeover_enabled);
        assert!(!status.claude_desktop.takeover_enabled);
        assert!(!status.opencode.takeover_enabled);
        assert!(!status.openclaw.takeover_enabled);
    }

    /// C1：七模块状态 wire 键固定为 camelCase，不再输出歧义 enabled。
    #[test]
    fn proxy_takeover_status_wire_keys_are_stable() {
        let mut status = ProxyTakeoverStatus::default();
        status.set_for_app(
            &AppType::ClaudeDesktop,
            ProxyModuleTakeoverStatus {
                takeover_enabled: true,
                route_mode: RouteMode::Proxy,
            },
        );
        let value = serde_json::to_value(&status).expect("serialize status");
        assert!(value.get("claudeDesktop").is_some());
        assert!(value.get("claude-desktop").is_none());
        assert!(value.get("hermes").is_some());
        let desktop = value.get("claudeDesktop").expect("claudeDesktop field");
        assert_eq!(
            desktop.get("takeoverEnabled").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            desktop.get("routeMode").and_then(|v| v.as_str()),
            Some("proxy")
        );
        assert!(desktop.get("enabled").is_none());
    }
}
