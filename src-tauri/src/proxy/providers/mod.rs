//! Provider Adapters Module
//!
//! 供应商适配器模块，提供统一的接口抽象不同上游供应商的处理逻辑。
//!
//! ## 模块结构
//! - `adapter`: 定义 `ProviderAdapter` trait
//! - `auth`: 认证类型和策略
//! - `claude`: Claude (Anthropic) 适配器
//! - `codex`: Codex (OpenAI) 适配器
//! - `gemini`: Gemini (Google) 适配器
//! - `models`: API 数据模型
//! - `transform`: 格式转换

mod adapter;
mod auth;
mod claude;
mod codex;
pub(crate) mod codex_chat_common;
pub mod codex_chat_history;
pub mod codex_oauth_auth;
pub(crate) mod codex_responses_sse;
pub mod copilot_auth;
pub mod copilot_model_map;
mod gemini;
pub(crate) mod gemini_schema;
pub mod gemini_shadow;
pub mod models;
pub(crate) mod reasoning_bridge;
pub mod streaming;
pub mod streaming_codex_anthropic;
pub mod streaming_codex_chat;
pub mod streaming_gemini;
pub mod streaming_responses;
pub mod transform;
pub mod transform_codex_anthropic;
pub mod transform_codex_chat;
pub mod transform_gemini;
pub mod transform_responses;

use crate::app_config::AppType;
use crate::provider::Provider;
use crate::proxy::error::ProxyError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// 公开导出
pub use adapter::ProviderAdapter;
pub use auth::{AuthInfo, AuthStrategy};
pub use claude::{
    claude_api_format_needs_transform, get_claude_api_format,
    normalize_anthropic_messages_for_provider, transform_claude_request_for_api_format,
    ClaudeAdapter,
};
pub use codex::CodexAdapter;
pub use codex::{
    apply_codex_chat_upstream_model, apply_codex_upstream_model, codex_provider_upstream_model,
    inject_codex_chat_prompt_cache_key, resolve_codex_catalog_tool_profile,
    resolve_codex_chat_reasoning_config, should_convert_codex_responses_to_anthropic,
    should_convert_codex_responses_to_chat,
};
pub use gemini::GeminiAdapter;

/// 供应商类型枚举
///
/// 区分不同供应商的具体实现方式，决定认证和请求处理逻辑。
/// 比 AppType 更细粒度，支持同一 AppType 下的多种变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    /// Anthropic 官方 API (x-api-key + anthropic-version)
    Claude,
    /// Claude 中转服务 (仅 Bearer 认证，无 x-api-key)
    ClaudeAuth,
    /// OpenAI Codex Response API
    Codex,
    /// Google Gemini API (x-goog-api-key)
    Gemini,
    /// Google Gemini CLI (OAuth Bearer)
    GeminiCli,
    /// OpenRouter（已支持 Claude Code 兼容接口，默认透传；保留旧转换逻辑备用）
    OpenRouter,
    /// GitHub Copilot (OAuth + Copilot Token，需要 Anthropic ↔ OpenAI 转换)
    GitHubCopilot,
    /// OpenAI Codex (ChatGPT Plus/Pro OAuth，需要 Anthropic ↔ Responses API 转换)
    CodexOAuth,
    /// 新模块协议无法识别或不在能力矩阵内。
    ///
    /// 这是显式的拒绝态，禁止把未知协议静默解释成 Codex/OpenAI。
    Unsupported,
}

impl ProviderType {
    /// 是否需要格式转换
    ///
    /// 过去 OpenRouter 需要将 Anthropic 格式转换为 OpenAI 格式；
    /// 现在默认关闭转换（因为 OpenRouter 已支持 Claude Code 兼容接口）。
    /// GitHub Copilot 需要转换（Anthropic → OpenAI 格式）。
    #[allow(dead_code)]
    pub fn needs_transform(&self) -> bool {
        match self {
            ProviderType::GitHubCopilot => true,
            ProviderType::CodexOAuth => true,
            ProviderType::OpenRouter => false,
            _ => false,
        }
    }

    /// 获取默认端点
    #[allow(dead_code)]
    pub fn default_endpoint(&self) -> &'static str {
        match self {
            ProviderType::Claude | ProviderType::ClaudeAuth => "https://api.anthropic.com",
            ProviderType::Codex => "https://api.openai.com",
            ProviderType::Gemini | ProviderType::GeminiCli => {
                "https://generativelanguage.googleapis.com"
            }
            ProviderType::OpenRouter => "https://openrouter.ai/api",
            ProviderType::GitHubCopilot => "https://api.githubcopilot.com",
            ProviderType::CodexOAuth => "https://chatgpt.com/backend-api/codex",
            ProviderType::Unsupported => "",
        }
    }

    /// 从 AppType 和 Provider 配置推断供应商类型
    ///
    /// 根据配置中的 base_url、auth_mode、api_key 格式等信息推断具体的供应商类型
    #[allow(dead_code)]
    pub fn from_app_type_and_config(app_type: &AppType, provider: &Provider) -> Self {
        match app_type {
            AppType::Claude | AppType::ClaudeDesktop => {
                if get_claude_api_format(provider) == "gemini_native" {
                    let adapter = ClaudeAdapter::new();
                    return match adapter.extract_auth(provider).map(|auth| auth.strategy) {
                        Some(AuthStrategy::GoogleOAuth) => ProviderType::GeminiCli,
                        _ => ProviderType::Gemini,
                    };
                }

                // 检测是否为 GitHub Copilot
                if let Some(meta) = provider.meta.as_ref() {
                    if meta.provider_type.as_deref() == Some("github_copilot") {
                        return ProviderType::GitHubCopilot;
                    }
                    if meta.provider_type.as_deref() == Some("codex_oauth") {
                        return ProviderType::CodexOAuth;
                    }
                }

                // 检测 base_url 是否为 GitHub Copilot
                let adapter = ClaudeAdapter::new();
                if let Ok(base_url) = adapter.extract_base_url(provider) {
                    if base_url.contains("githubcopilot.com") {
                        return ProviderType::GitHubCopilot;
                    }
                    // 检测是否为 OpenRouter
                    if base_url.contains("openrouter.ai") {
                        return ProviderType::OpenRouter;
                    }
                }
                // 检测是否为中转服务（仅 Bearer 认证）
                // 注意：ProviderMeta 没有直接的 auth_mode 字段，
                // 我们通过检查 settings_config 中的配置来判断
                // 检查 settings_config 中的 auth_mode
                if let Some(auth_mode) = provider
                    .settings_config
                    .get("auth_mode")
                    .and_then(|v| v.as_str())
                {
                    if auth_mode == "bearer_only" {
                        return ProviderType::ClaudeAuth;
                    }
                }
                // 检查 env 中的 auth_mode
                if let Some(env) = provider.settings_config.get("env") {
                    if let Some(auth_mode) = env.get("AUTH_MODE").and_then(|v| v.as_str()) {
                        if auth_mode == "bearer_only" {
                            return ProviderType::ClaudeAuth;
                        }
                    }
                }
                ProviderType::Claude
            }
            AppType::Codex => ProviderType::Codex,
            AppType::Gemini => {
                // 检测是否为 CLI 模式（OAuth）
                let adapter = GeminiAdapter::new();
                if let Some(auth) = adapter.extract_auth(provider) {
                    let key = &auth.api_key;
                    // OAuth access_token 以 ya29. 开头
                    if key.starts_with("ya29.") {
                        return ProviderType::GeminiCli;
                    }
                    // JSON 格式的 OAuth 凭证
                    if key.starts_with('{') {
                        return ProviderType::GeminiCli;
                    }
                }
                ProviderType::Gemini
            }
            // OpenCode/OpenClaw/Hermes 各自的 settings_config schema 与 Codex 不同，
            // 凭据字段位置也不同（见 `provider.rs:resolve_usage_credentials`）。按各模块
            // canonical 协议投影到既有 ProviderType；无法解析时返回显式 Unsupported，
            // 禁止任何未知协议静默 fallback 到 Codex。
            AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => {
                module_canonical_protocol(app_type, provider)
                    .map(|protocol| protocol.provider_type())
                    .unwrap_or(ProviderType::Unsupported)
            }
        }
    }

    /// 转换为字符串表示
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderType::Claude => "claude",
            ProviderType::ClaudeAuth => "claude_auth",
            ProviderType::Codex => "codex",
            ProviderType::Gemini => "gemini",
            ProviderType::GeminiCli => "gemini_cli",
            ProviderType::OpenRouter => "openrouter",
            ProviderType::GitHubCopilot => "github_copilot",
            ProviderType::CodexOAuth => "codex_oauth",
            ProviderType::Unsupported => "unsupported",
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" => Ok(ProviderType::Claude),
            "claude_auth" | "claude-auth" => Ok(ProviderType::ClaudeAuth),
            "codex" => Ok(ProviderType::Codex),
            "gemini" => Ok(ProviderType::Gemini),
            "gemini_cli" | "gemini-cli" => Ok(ProviderType::GeminiCli),
            "openrouter" => Ok(ProviderType::OpenRouter),
            "github_copilot" | "github-copilot" | "githubcopilot" => {
                Ok(ProviderType::GitHubCopilot)
            }
            "codex_oauth" | "codex-oauth" | "codexoauth" => Ok(ProviderType::CodexOAuth),
            "unsupported" => Ok(ProviderType::Unsupported),
            _ => Err(format!("Invalid provider type: {s}")),
        }
    }
}

/// 根据 AppType 获取对应的适配器
///
/// 仅按 AppType 决定 adapter，够用于 Claude/Codex/Gemini/ClaudeDesktop。
/// OpenCode/OpenClaw/Hermes 的 canonical 协议随 provider schema（`api`/`api_mode`/`npm`）
/// 变化，且凭据字段位置与 Codex 不同，需要 provider-aware 解析——它们**不再** fallback 到
/// CodexAdapter，改由 [`get_adapter_for`] 返回按模块协议规范化后的 adapter。
/// 这里保留 app-type-only 版本供 stream_check 等无 provider 上下文的调用方使用；
/// forwarder 请求路径应改用 [`get_adapter_for`]。
pub fn get_adapter(app_type: &AppType) -> Box<dyn ProviderAdapter> {
    match app_type {
        AppType::Claude | AppType::ClaudeDesktop => Box::new(ClaudeAdapter::new()),
        AppType::Codex => Box::new(CodexAdapter::new()),
        AppType::Gemini => Box::new(GeminiAdapter::new()),
        // 没有 provider 上下文就无法判定新模块协议；返回显式拒绝 adapter，
        // 禁止把未知协议静默当成 Codex/OpenAI。
        AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => {
            Box::new(UnsupportedModuleAdapter::new(app_type.clone()))
        }
    }
}

/// provider-aware 的 adapter 解析：Claude/Codex/Gemini/ClaudeDesktop 与
/// [`get_adapter`] 一致；OpenCode/OpenClaw/Hermes 按 provider 的 canonical 协议
/// 选择底层 Claude/Codex 转换链，并用模块专属 schema 规范化投影出底层 adapter
/// 能识别的凭据视图（`extract_base_url`/`extract_auth`）。未知协议返回显式拒绝
/// adapter，绝不 fallback 到 Codex。
pub fn get_adapter_for(app_type: &AppType, provider: &Provider) -> Box<dyn ProviderAdapter> {
    match app_type {
        AppType::Claude | AppType::ClaudeDesktop => Box::new(ClaudeAdapter::new()),
        AppType::Codex => Box::new(CodexAdapter::new()),
        AppType::Gemini => Box::new(GeminiAdapter::new()),
        AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => {
            match module_canonical_protocol(app_type, provider) {
                Some(protocol) => {
                    Box::new(ModuleNormalizingAdapter::new(app_type.clone(), protocol))
                }
                None => Box::new(UnsupportedModuleAdapter::new(app_type.clone())),
            }
        }
    }
}

/// 根据 ProviderType 获取对应的适配器
#[allow(dead_code)]
pub fn get_adapter_for_provider_type(provider_type: &ProviderType) -> Box<dyn ProviderAdapter> {
    match provider_type {
        ProviderType::Claude
        | ProviderType::ClaudeAuth
        | ProviderType::OpenRouter
        | ProviderType::GitHubCopilot
        | ProviderType::CodexOAuth => Box::new(ClaudeAdapter::new()),
        ProviderType::Codex => Box::new(CodexAdapter::new()),
        ProviderType::Gemini | ProviderType::GeminiCli => Box::new(GeminiAdapter::new()),
        ProviderType::Unsupported => Box::new(UnsupportedModuleAdapter::new(AppType::OpenCode)),
    }
}

// ============================================================================
// OpenCode / OpenClaw / Hermes 的 canonical 协议解析与规范化 adapter（C2b）
// ============================================================================

/// 四新模块（OpenCode/OpenClaw/Hermes）proxy 时选定的 canonical 本地协议。
///
/// 只覆盖现有 Claude/Codex/Gemini 转换链能承载的协议家族：
/// - `Anthropic`：Anthropic Messages（复用 ClaudeAdapter/Anthropic 转换链）。
/// - `OpenAiChat`：OpenAI Chat Completions（复用 CodexAdapter/OpenAI 转换链）。
/// - `OpenAiResponses`：OpenAI Responses（复用 CodexAdapter/Responses 转换链）。
///
/// 能力矩阵外协议（如 Hermes `bedrock_converse`）不在此枚举中，由
/// [`validate_module_proxy_capability`] 在写 live/提交 route_mode 前原子拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleProtocol {
    Anthropic,
    OpenAiChat,
    OpenAiResponses,
}

impl ModuleProtocol {
    /// 该 canonical 协议映射到的既有 ProviderType（用于整流器/统计等判定）。
    pub fn provider_type(self) -> ProviderType {
        match self {
            // 中转/兼容网关按 Bearer 语义处理（与其它模块 top-level api_key 一致）。
            ModuleProtocol::Anthropic => ProviderType::ClaudeAuth,
            ModuleProtocol::OpenAiChat | ModuleProtocol::OpenAiResponses => ProviderType::Codex,
        }
    }
}

/// 解析 OpenCode/OpenClaw/Hermes 的 canonical 本地协议。
///
/// 返回 `None` 表示该 provider 的协议在能力矩阵之外（无现成转换链），
/// proxy 启用必须原子拒绝；direct 不受影响。
///
/// 字段事实来源与 `provider.rs:resolve_usage_credentials` 对齐：
/// - OpenCode：`npm`（仅显式 `@ai-sdk/anthropic` / `@ai-sdk/openai` / `@ai-sdk/openai-compatible`）。
/// - OpenClaw：`api`（`anthropic-messages` → Anthropic；`openai-completions` → Chat；`openai-responses` → Responses）。
/// - Hermes：`api_mode`/`apiMode`（`anthropic_messages` → Anthropic；`chat_completions` → Chat；`codex_responses` → Responses；`bedrock_converse` → 矩阵外）。
pub fn module_canonical_protocol(
    app_type: &AppType,
    provider: &Provider,
) -> Option<ModuleProtocol> {
    let settings = &provider.settings_config;
    match app_type {
        AppType::OpenCode => {
            let npm = settings
                .get("npm")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            match npm.as_str() {
                "@ai-sdk/anthropic" => Some(ModuleProtocol::Anthropic),
                "@ai-sdk/openai" | "@ai-sdk/openai-compatible" => Some(ModuleProtocol::OpenAiChat),
                _ => None,
            }
        }
        AppType::OpenClaw => {
            let api = settings.get("api").and_then(Value::as_str).unwrap_or("");
            match api {
                "anthropic-messages" => Some(ModuleProtocol::Anthropic),
                "openai-completions" => Some(ModuleProtocol::OpenAiChat),
                "openai-responses" => Some(ModuleProtocol::OpenAiResponses),
                _ => None,
            }
        }
        AppType::Hermes => {
            let api_mode = settings
                .get("api_mode")
                .or_else(|| settings.get("apiMode"))
                .and_then(Value::as_str)
                .unwrap_or("");
            match api_mode {
                "anthropic_messages" => Some(ModuleProtocol::Anthropic),
                "chat_completions" => Some(ModuleProtocol::OpenAiChat),
                "codex_responses" => Some(ModuleProtocol::OpenAiResponses),
                // bedrock_converse 等无转换链的协议明确落到 None（矩阵外）。
                _ => None,
            }
        }
        // 其它 AppType 不使用本函数。
        _ => None,
    }
}

/// proxy 启用前的能力矩阵校验（fail-fast）。
///
/// 仅当模块 canonical 协议落在现有转换链（Anthropic / OpenAI Chat）内才允许 proxy。
/// 无法解析（矩阵外协议或字段缺失）时返回 `Err`，调用方必须在 capture snapshot /
/// 写 live / 提交 route_mode **之前**拒绝，保持 `takeover_enabled`/`route_mode`/live/
/// snapshot 原样；direct 不受本校验限制（对齐 `gateway-takeover.md` §4、父 AC8-C2b）。
///
/// 目前的消费方是 `services/proxy.rs::set_takeover_for_app` 的 proxy 分支；该分支的
/// 四模块 match 臂依赖 C2a 落地的公共三维 dispatcher，按 C2b-first 顺序尚未合入，
/// 故本函数暂无调用方（rebase 到 C2a 后接入）。保留 `allow(dead_code)` 而非删除，
/// 避免与 C2a 各自发明两套矩阵校验。
#[allow(dead_code)]
pub fn validate_module_proxy_capability(
    app_type: &AppType,
    provider: &Provider,
) -> Result<ModuleProtocol, String> {
    let protocol = module_canonical_protocol(app_type, provider).ok_or_else(|| {
        format!(
            "{} 当前供应商的协议不在网关能力矩阵内（无现成转换链），无法启用 proxy 接管；可改用 direct 或切换到受支持协议的供应商",
            app_type.as_str()
        )
    })?;

    // C2b 的冻结路由表只为 OpenCode 暴露 OpenAI Chat Completions；
    // @ai-sdk/anthropic 会调用 /messages，而本任务明确不新增 /opencode/v1/messages。
    // 在接管写入前拒绝，不能写出一个必然 404 的 live 配置。
    if matches!(app_type, AppType::OpenCode) && protocol != ModuleProtocol::OpenAiChat {
        return Err(
            "opencode 当前仅支持 OpenAI Chat Completions proxy；该供应商协议可继续用于 direct"
                .to_string(),
        );
    }

    Ok(protocol)
}

/// 未知/矩阵外协议的显式拒绝 adapter。
///
/// `get_adapter` 无 provider 上下文，或 `get_adapter_for` 无法解析协议时返回本类型；
/// 第一个可失败步骤 `extract_base_url` 会给出明确错误，确保请求绝不静默走 Codex。
struct UnsupportedModuleAdapter {
    app_type: AppType,
}

impl UnsupportedModuleAdapter {
    fn new(app_type: AppType) -> Self {
        Self { app_type }
    }

    fn error(&self) -> ProxyError {
        ProxyError::ConfigError(format!(
            "{} 当前供应商协议不在网关能力矩阵内",
            self.app_type.as_str()
        ))
    }
}

impl ProviderAdapter for UnsupportedModuleAdapter {
    fn name(&self) -> &'static str {
        "UnsupportedModule"
    }

    fn extract_base_url(&self, _provider: &Provider) -> Result<String, ProxyError> {
        Err(self.error())
    }

    fn extract_auth(&self, _provider: &Provider) -> Option<AuthInfo> {
        None
    }

    fn build_url(&self, base_url: &str, endpoint: &str) -> String {
        format!(
            "{}/{}",
            base_url.trim_end_matches('/'),
            endpoint.trim_start_matches('/')
        )
    }

    fn get_auth_headers(
        &self,
        _auth: &AuthInfo,
    ) -> Result<Vec<(http::HeaderName, http::HeaderValue)>, ProxyError> {
        Err(self.error())
    }
}

/// 把 OpenCode/OpenClaw/Hermes 的 provider schema 规范化为底层 Claude/Codex adapter
/// 能识别的凭据视图后再委托的 adapter。
///
/// 底层 `ClaudeAdapter`/`CodexAdapter` 只认各自 schema（Anthropic env / Codex base_url+auth），
/// 而四新模块凭据字段位置不同（OpenCode `options.{baseURL,apiKey}`、OpenClaw 顶层
/// `baseUrl`/`apiKey`、Hermes 顶层 `base_url`/`api_key`）。本 adapter 用
/// `Provider::resolve_usage_credentials`（单一事实来源）提取 base_url/api_key，
/// 构造一个底层 adapter 能直接消费的规范化 provider 视图，再委托底层 adapter 完成
/// URL 构建、鉴权头、格式转换。彻底取代旧的 Codex fallback 占位。
struct ModuleNormalizingAdapter {
    app_type: AppType,
    protocol: ModuleProtocol,
    inner: Box<dyn ProviderAdapter>,
}

impl ModuleNormalizingAdapter {
    fn new(app_type: AppType, protocol: ModuleProtocol) -> Self {
        let inner: Box<dyn ProviderAdapter> = match protocol {
            ModuleProtocol::Anthropic => Box::new(ClaudeAdapter::new()),
            ModuleProtocol::OpenAiChat | ModuleProtocol::OpenAiResponses => {
                Box::new(CodexAdapter::new())
            }
        };
        Self {
            app_type,
            protocol,
            inner,
        }
    }

    /// 用模块 canonical 凭据构造底层 adapter 能识别的规范化 provider 视图。
    ///
    /// Anthropic 链：底层 ClaudeAdapter 从 `env.ANTHROPIC_BASE_URL` / `env.ANTHROPIC_AUTH_TOKEN` 读取；
    /// OpenAI 链：底层 CodexAdapter 从顶层 `base_url` / `env.OPENAI_API_KEY` 读取。
    /// 保留原 provider 的 `id`/`name`/`meta` 等，仅重写 `settings_config`。
    fn normalized_provider(&self, provider: &Provider) -> Provider {
        let (base_url, api_key) = provider.resolve_usage_credentials(&self.app_type);
        let mut normalized = provider.clone();
        normalized.settings_config = match self.protocol {
            ModuleProtocol::Anthropic => serde_json::json!({
                "env": {
                    "ANTHROPIC_BASE_URL": base_url,
                    "ANTHROPIC_AUTH_TOKEN": api_key,
                }
            }),
            ModuleProtocol::OpenAiChat | ModuleProtocol::OpenAiResponses => serde_json::json!({
                "base_url": base_url,
                "env": {
                    "OPENAI_API_KEY": api_key,
                }
            }),
        };
        normalized
    }
}

impl ProviderAdapter for ModuleNormalizingAdapter {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn extract_base_url(&self, provider: &Provider) -> Result<String, ProxyError> {
        self.inner
            .extract_base_url(&self.normalized_provider(provider))
    }

    fn extract_auth(&self, provider: &Provider) -> Option<AuthInfo> {
        self.inner.extract_auth(&self.normalized_provider(provider))
    }

    fn build_url(&self, base_url: &str, endpoint: &str) -> String {
        self.inner.build_url(base_url, endpoint)
    }

    fn get_auth_headers(
        &self,
        auth: &AuthInfo,
    ) -> Result<Vec<(http::HeaderName, http::HeaderValue)>, ProxyError> {
        self.inner.get_auth_headers(auth)
    }

    fn needs_transform(&self, provider: &Provider) -> bool {
        self.inner
            .needs_transform(&self.normalized_provider(provider))
    }

    fn transform_request(&self, body: Value, provider: &Provider) -> Result<Value, ProxyError> {
        self.inner
            .transform_request(body, &self.normalized_provider(provider))
    }

    fn transform_response(&self, body: Value) -> Result<Value, ProxyError> {
        self.inner.transform_response(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn create_provider(config: serde_json::Value) -> Provider {
        Provider {
            id: "test".to_string(),
            name: "Test Provider".to_string(),
            settings_config: config,
            website_url: None,
            category: None,
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }

    #[test]
    fn test_provider_type_needs_transform() {
        assert!(!ProviderType::Claude.needs_transform());
        assert!(!ProviderType::ClaudeAuth.needs_transform());
        assert!(!ProviderType::Codex.needs_transform());
        assert!(!ProviderType::Gemini.needs_transform());
        assert!(!ProviderType::GeminiCli.needs_transform());
        assert!(!ProviderType::OpenRouter.needs_transform());
        assert!(ProviderType::GitHubCopilot.needs_transform());
    }

    #[test]
    fn test_provider_type_default_endpoint() {
        assert_eq!(
            ProviderType::Claude.default_endpoint(),
            "https://api.anthropic.com"
        );
        assert_eq!(
            ProviderType::ClaudeAuth.default_endpoint(),
            "https://api.anthropic.com"
        );
        assert_eq!(
            ProviderType::Codex.default_endpoint(),
            "https://api.openai.com"
        );
        assert_eq!(
            ProviderType::Gemini.default_endpoint(),
            "https://generativelanguage.googleapis.com"
        );
        assert_eq!(
            ProviderType::GeminiCli.default_endpoint(),
            "https://generativelanguage.googleapis.com"
        );
        assert_eq!(
            ProviderType::OpenRouter.default_endpoint(),
            "https://openrouter.ai/api"
        );
        assert_eq!(
            ProviderType::GitHubCopilot.default_endpoint(),
            "https://api.githubcopilot.com"
        );
    }

    #[test]
    fn test_provider_type_from_str() {
        assert_eq!(
            "claude".parse::<ProviderType>().unwrap(),
            ProviderType::Claude
        );
        assert_eq!(
            "claude_auth".parse::<ProviderType>().unwrap(),
            ProviderType::ClaudeAuth
        );
        assert_eq!(
            "claude-auth".parse::<ProviderType>().unwrap(),
            ProviderType::ClaudeAuth
        );
        assert_eq!(
            "codex".parse::<ProviderType>().unwrap(),
            ProviderType::Codex
        );
        assert_eq!(
            "gemini".parse::<ProviderType>().unwrap(),
            ProviderType::Gemini
        );
        assert_eq!(
            "gemini_cli".parse::<ProviderType>().unwrap(),
            ProviderType::GeminiCli
        );
        assert_eq!(
            "gemini-cli".parse::<ProviderType>().unwrap(),
            ProviderType::GeminiCli
        );
        assert_eq!(
            "openrouter".parse::<ProviderType>().unwrap(),
            ProviderType::OpenRouter
        );
        assert_eq!(
            "github_copilot".parse::<ProviderType>().unwrap(),
            ProviderType::GitHubCopilot
        );
        assert_eq!(
            "github-copilot".parse::<ProviderType>().unwrap(),
            ProviderType::GitHubCopilot
        );
        assert_eq!(
            "githubcopilot".parse::<ProviderType>().unwrap(),
            ProviderType::GitHubCopilot
        );
        assert!("invalid".parse::<ProviderType>().is_err());
    }

    #[test]
    fn test_provider_type_as_str() {
        assert_eq!(ProviderType::Claude.as_str(), "claude");
        assert_eq!(ProviderType::ClaudeAuth.as_str(), "claude_auth");
        assert_eq!(ProviderType::Codex.as_str(), "codex");
        assert_eq!(ProviderType::Gemini.as_str(), "gemini");
        assert_eq!(ProviderType::GeminiCli.as_str(), "gemini_cli");
        assert_eq!(ProviderType::OpenRouter.as_str(), "openrouter");
        assert_eq!(ProviderType::GitHubCopilot.as_str(), "github_copilot");
    }

    #[test]
    fn test_provider_type_serde() {
        // Test serialization
        let claude = ProviderType::Claude;
        let serialized = serde_json::to_string(&claude).unwrap();
        assert_eq!(serialized, "\"claude\"");

        let claude_auth = ProviderType::ClaudeAuth;
        let serialized = serde_json::to_string(&claude_auth).unwrap();
        assert_eq!(serialized, "\"claude_auth\"");

        // Test deserialization
        let deserialized: ProviderType = serde_json::from_str("\"claude\"").unwrap();
        assert_eq!(deserialized, ProviderType::Claude);

        let deserialized: ProviderType = serde_json::from_str("\"gemini_cli\"").unwrap();
        assert_eq!(deserialized, ProviderType::GeminiCli);
    }

    #[test]
    fn test_from_app_type_claude_direct() {
        let provider = create_provider(json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.anthropic.com",
                "ANTHROPIC_AUTH_TOKEN": "sk-ant-test"
            }
        }));

        let provider_type = ProviderType::from_app_type_and_config(&AppType::Claude, &provider);
        assert_eq!(provider_type, ProviderType::Claude);
    }

    #[test]
    fn test_from_app_type_claude_openrouter() {
        let provider = create_provider(json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://openrouter.ai/api",
                "OPENROUTER_API_KEY": "sk-or-test"
            }
        }));

        let provider_type = ProviderType::from_app_type_and_config(&AppType::Claude, &provider);
        assert_eq!(provider_type, ProviderType::OpenRouter);
    }

    #[test]
    fn test_from_app_type_claude_auth() {
        let provider = create_provider(json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://some-proxy.com",
                "ANTHROPIC_AUTH_TOKEN": "sk-test"
            },
            "auth_mode": "bearer_only"
        }));

        let provider_type = ProviderType::from_app_type_and_config(&AppType::Claude, &provider);
        assert_eq!(provider_type, ProviderType::ClaudeAuth);
    }

    #[test]
    fn test_from_app_type_codex() {
        let provider = create_provider(json!({
            "env": {
                "OPENAI_API_KEY": "sk-test"
            }
        }));

        let provider_type = ProviderType::from_app_type_and_config(&AppType::Codex, &provider);
        assert_eq!(provider_type, ProviderType::Codex);
    }

    #[test]
    fn test_from_app_type_gemini_api_key() {
        let provider = create_provider(json!({
            "env": {
                "GEMINI_API_KEY": "AIza-test-key"
            }
        }));

        let provider_type = ProviderType::from_app_type_and_config(&AppType::Gemini, &provider);
        assert_eq!(provider_type, ProviderType::Gemini);
    }

    #[test]
    fn test_from_app_type_gemini_cli_oauth() {
        let provider = create_provider(json!({
            "env": {
                "GEMINI_API_KEY": "ya29.test-access-token"
            }
        }));

        let provider_type = ProviderType::from_app_type_and_config(&AppType::Gemini, &provider);
        assert_eq!(provider_type, ProviderType::GeminiCli);
    }

    #[test]
    fn test_from_app_type_gemini_cli_json() {
        let provider = create_provider(json!({
            "env": {
                "GEMINI_API_KEY": "{\"access_token\":\"ya29.test\",\"refresh_token\":\"1//test\"}"
            }
        }));

        let provider_type = ProviderType::from_app_type_and_config(&AppType::Gemini, &provider);
        assert_eq!(provider_type, ProviderType::GeminiCli);
    }

    #[test]
    fn test_get_adapter_for_provider_type() {
        let adapter = get_adapter_for_provider_type(&ProviderType::Claude);
        assert_eq!(adapter.name(), "Claude");

        let adapter = get_adapter_for_provider_type(&ProviderType::ClaudeAuth);
        assert_eq!(adapter.name(), "Claude");

        let adapter = get_adapter_for_provider_type(&ProviderType::OpenRouter);
        assert_eq!(adapter.name(), "Claude");

        let adapter = get_adapter_for_provider_type(&ProviderType::GitHubCopilot);
        assert_eq!(adapter.name(), "Claude");

        let adapter = get_adapter_for_provider_type(&ProviderType::Codex);
        assert_eq!(adapter.name(), "Codex");

        let adapter = get_adapter_for_provider_type(&ProviderType::Gemini);
        assert_eq!(adapter.name(), "Gemini");

        let adapter = get_adapter_for_provider_type(&ProviderType::GeminiCli);
        assert_eq!(adapter.name(), "Gemini");
    }

    // ---- C2b：四新模块 canonical 协议解析 / 规范化 adapter ----

    #[test]
    fn opencode_npm_selects_canonical_protocol() {
        let openai = create_provider(json!({
            "npm": "@ai-sdk/openai-compatible",
            "options": {"baseURL": "https://relay.example.com/v1", "apiKey": "sk-x"}
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::OpenCode, &openai),
            Some(ModuleProtocol::OpenAiChat)
        );

        let anthropic = create_provider(json!({
            "npm": "@ai-sdk/anthropic",
            "options": {"baseURL": "https://relay.example.com", "apiKey": "sk-x"}
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::OpenCode, &anthropic),
            Some(ModuleProtocol::Anthropic)
        );
        assert!(validate_module_proxy_capability(&AppType::OpenCode, &anthropic).is_err());

        let unknown = create_provider(json!({
            "npm": "@ai-sdk/amazon-bedrock",
            "options": {"baseURL": "https://relay.example.com", "apiKey": "sk-x"}
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::OpenCode, &unknown),
            None
        );

        let lookalike = create_provider(json!({
            "npm": "@vendor/not-openai-compatible",
            "options": {"baseURL": "https://relay.example.com", "apiKey": "sk-x"}
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::OpenCode, &lookalike),
            None,
            "协议识别必须是显式 allowlist，不能按包含 openai 的子串放行"
        );
    }

    #[test]
    fn openclaw_api_field_selects_canonical_protocol() {
        let completions = create_provider(json!({
            "baseUrl": "https://relay.example.com/v1", "apiKey": "sk-x", "api": "openai-completions"
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::OpenClaw, &completions),
            Some(ModuleProtocol::OpenAiChat)
        );

        let responses = create_provider(json!({
            "baseUrl": "https://relay.example.com/v1", "apiKey": "sk-x", "api": "openai-responses"
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::OpenClaw, &responses),
            Some(ModuleProtocol::OpenAiResponses)
        );

        let anthropic = create_provider(json!({
            "baseUrl": "https://relay.example.com", "apiKey": "sk-x", "api": "anthropic-messages"
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::OpenClaw, &anthropic),
            Some(ModuleProtocol::Anthropic)
        );

        let unknown = create_provider(json!({
            "baseUrl": "https://relay.example.com", "apiKey": "sk-x", "api": "bedrock-converse"
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::OpenClaw, &unknown),
            None
        );
    }

    #[test]
    fn hermes_api_mode_selects_canonical_protocol_and_rejects_bedrock() {
        let chat = create_provider(json!({
            "base_url": "https://relay.example.com/v1", "api_key": "sk-x", "api_mode": "chat_completions"
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::Hermes, &chat),
            Some(ModuleProtocol::OpenAiChat)
        );

        let responses = create_provider(json!({
            "base_url": "https://relay.example.com/v1", "api_key": "sk-x", "apiMode": "codex_responses"
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::Hermes, &responses),
            Some(ModuleProtocol::OpenAiResponses)
        );

        let anthropic = create_provider(json!({
            "base_url": "https://relay.example.com", "api_key": "sk-x", "api_mode": "anthropic_messages"
        }));
        assert_eq!(
            module_canonical_protocol(&AppType::Hermes, &anthropic),
            Some(ModuleProtocol::Anthropic)
        );

        // 能力矩阵外协议：bedrock_converse 必须解析为 None，并被 capability 校验拒绝。
        let bedrock = create_provider(json!({
            "base_url": "https://relay.example.com", "api_key": "sk-x", "api_mode": "bedrock_converse"
        }));
        assert_eq!(module_canonical_protocol(&AppType::Hermes, &bedrock), None);
        assert!(validate_module_proxy_capability(&AppType::Hermes, &bedrock).is_err());
        assert!(validate_module_proxy_capability(&AppType::Hermes, &anthropic).is_ok());
    }

    #[test]
    fn module_adapter_normalizes_credentials_and_avoids_codex_fallback() {
        // OpenCode → OpenAI Chat 链：底层 CodexAdapter 必须从 options.{baseURL,apiKey} 拿到凭据。
        let opencode = create_provider(json!({
            "npm": "@ai-sdk/openai-compatible",
            "options": {"baseURL": "https://relay.example.com/v1", "apiKey": "sk-oc"}
        }));
        let adapter = get_adapter_for(&AppType::OpenCode, &opencode);
        assert_eq!(adapter.name(), "Codex");
        assert_eq!(
            adapter.extract_base_url(&opencode).unwrap(),
            "https://relay.example.com/v1"
        );
        assert_eq!(adapter.extract_auth(&opencode).unwrap().api_key, "sk-oc");

        // OpenClaw → Anthropic 链：底层 ClaudeAdapter 从顶层 baseUrl/apiKey 规范化后读取。
        let openclaw = create_provider(json!({
            "baseUrl": "https://relay.example.com", "apiKey": "sk-claw", "api": "anthropic-messages"
        }));
        let adapter = get_adapter_for(&AppType::OpenClaw, &openclaw);
        assert_eq!(adapter.name(), "Claude");
        assert_eq!(
            adapter.extract_base_url(&openclaw).unwrap(),
            "https://relay.example.com"
        );
        assert_eq!(adapter.extract_auth(&openclaw).unwrap().api_key, "sk-claw");

        // Hermes → OpenAI 链：顶层 base_url/api_key。
        let hermes = create_provider(json!({
            "base_url": "https://relay.example.com/v1", "api_key": "sk-h", "api_mode": "chat_completions"
        }));
        let adapter = get_adapter_for(&AppType::Hermes, &hermes);
        assert_eq!(adapter.name(), "Codex");
        assert_eq!(adapter.extract_auth(&hermes).unwrap().api_key, "sk-h");
    }

    #[test]
    fn unknown_module_protocol_is_explicitly_rejected_without_codex_fallback() {
        let bedrock = create_provider(json!({
            "base_url": "https://bedrock.example.com",
            "api_key": "secret",
            "api_mode": "bedrock_converse"
        }));

        assert_eq!(
            ProviderType::from_app_type_and_config(&AppType::Hermes, &bedrock),
            ProviderType::Unsupported
        );
        let adapter = get_adapter_for(&AppType::Hermes, &bedrock);
        assert_eq!(adapter.name(), "UnsupportedModule");
        assert!(adapter.extract_base_url(&bedrock).is_err());
    }
}
