//! C3 外部配置冲突的精确路由判定。
//!
//! 该解析器只消费防抖后 capture 的原始字节，不读取 live 文件，也不复用 crash residue
//! 的宽松 bool detector。当前网关 URL 来自 `ProxyService` 的运行时/持久化端口；C2b
//! 累加式配置只查看 Agent-Switch 当前 provider 对应的 fragment。

use crate::app_config::AppType;
use crate::database::Database;
use crate::proxy::types::RouteMode;
use crate::services::external_config_monitor::{ManagedExpected, ManagedTarget, ManagedTargetKind};
use crate::services::proxy::ProxyService;
use serde_json::Value;
use std::collections::BTreeMap;
use url::{Host, Url};

const PROXY_TOKEN_PLACEHOLDER: &str = "PROXY_MANAGED";
const GATEWAY_TOKEN_SETTING_KEY: &str = "claude_desktop_gateway_token";

#[derive(Debug, Clone)]
pub(crate) struct RouteParseContext {
    expected_gateway_url: Url,
    current_provider_id: Option<String>,
    gateway_token: Option<String>,
}

impl RouteParseContext {
    #[cfg(test)]
    fn new(
        expected_gateway_url: &str,
        current_provider_id: Option<&str>,
        gateway_token: Option<&str>,
    ) -> Self {
        Self {
            expected_gateway_url: Url::parse(expected_gateway_url).expect("valid expected URL"),
            current_provider_id: current_provider_id.map(str::to_string),
            gateway_token: gateway_token.map(str::to_string),
        }
    }
}

/// 精确判定 observed capture 实际写向当前模块网关还是非本地真实上游。
pub(crate) async fn parse_actual_route(
    app_type: &AppType,
    capture: &ManagedExpected,
    proxy_service: &ProxyService,
    db: &Database,
) -> Result<RouteMode, String> {
    let expected_gateway_url = proxy_service.expected_gateway_url(app_type).await?;
    let current_provider_id = if matches!(
        app_type,
        AppType::OpenCode | AppType::OpenClaw | AppType::Hermes
    ) {
        Some(
            crate::settings::get_effective_current_provider(db, app_type)
                .map_err(|error| format!("读取 {} 当前供应商失败: {error}", app_type.as_str()))?
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| format!("{} 当前供应商不存在", app_type.as_str()))?,
        )
    } else {
        None
    };
    let gateway_token = db
        .get_setting(GATEWAY_TOKEN_SETTING_KEY)
        .map_err(|error| format!("读取本地网关 token 失败: {error}"))?
        .filter(|token| !token.trim().is_empty());
    let context = RouteParseContext {
        expected_gateway_url: Url::parse(&expected_gateway_url)
            .map_err(|error| format!("当前网关 URL 无效: {error}"))?,
        current_provider_id,
        gateway_token,
    };
    parse_actual_route_with_context(app_type, capture, &context)
}

pub(crate) fn parse_actual_route_with_context(
    app_type: &AppType,
    capture: &ManagedExpected,
    context: &RouteParseContext,
) -> Result<RouteMode, String> {
    let targets = validated_targets(app_type, capture)?;
    match app_type {
        AppType::Claude => parse_claude(&targets, context),
        AppType::ClaudeDesktop => parse_claude_desktop(&targets, context),
        AppType::Codex => parse_codex(&targets, context),
        AppType::Gemini => parse_gemini(&targets, context),
        AppType::OpenCode => parse_opencode(&targets, context),
        AppType::OpenClaw => parse_openclaw(&targets, context),
        AppType::Hermes => parse_hermes(&targets, context),
    }
}

fn expected_target_ids(app_type: &AppType) -> &'static [&'static str] {
    match app_type {
        AppType::Claude => &["settings"],
        AppType::ClaudeDesktop => &["normal_config", "threep_config", "profile", "meta"],
        AppType::Codex => &["auth", "config", "model_catalog"],
        AppType::Gemini => &[".env"],
        AppType::OpenCode => &["opencode.json"],
        AppType::OpenClaw => &["openclaw.json"],
        AppType::Hermes => &["config.yaml"],
    }
}

fn validated_targets<'a>(
    app_type: &AppType,
    capture: &'a ManagedExpected,
) -> Result<BTreeMap<&'a str, &'a ManagedTarget>, String> {
    let expected_ids = expected_target_ids(app_type);
    if capture.targets.len() != expected_ids.len() {
        return Err(format!(
            "{} capture 目标数量不完整：期望 {}，实际 {}",
            app_type.as_str(),
            expected_ids.len(),
            capture.targets.len()
        ));
    }

    let mut targets = BTreeMap::new();
    for target in &capture.targets {
        if target.kind != ManagedTargetKind::FileBytes {
            return Err(format!(
                "{} target {} 不是 file_bytes",
                app_type.as_str(),
                target.id
            ));
        }
        if !expected_ids.contains(&target.id.as_str()) {
            return Err(format!(
                "{} capture 含未知 target {}",
                app_type.as_str(),
                target.id
            ));
        }
        if targets.insert(target.id.as_str(), target).is_some() {
            return Err(format!("{} capture target 重复", app_type.as_str()));
        }
    }
    if expected_ids.iter().any(|id| !targets.contains_key(id)) {
        return Err(format!("{} capture 缺少稳定 target", app_type.as_str()));
    }
    Ok(targets)
}

fn required_text<'a>(
    targets: &BTreeMap<&str, &'a ManagedTarget>,
    id: &str,
) -> Result<&'a str, String> {
    let target = targets
        .get(id)
        .ok_or_else(|| format!("capture 缺少 target {id}"))?;
    if !target.existed {
        return Err(format!("路由 target {id} 不存在"));
    }
    std::str::from_utf8(&target.bytes).map_err(|error| format!("target {id} 不是 UTF-8: {error}"))
}

fn optional_text<'a>(
    targets: &BTreeMap<&str, &'a ManagedTarget>,
    id: &str,
) -> Result<Option<&'a str>, String> {
    let target = targets
        .get(id)
        .ok_or_else(|| format!("capture 缺少 target {id}"))?;
    if !target.existed {
        return Ok(None);
    }
    std::str::from_utf8(&target.bytes)
        .map(Some)
        .map_err(|error| format!("target {id} 不是 UTF-8: {error}"))
}

fn parse_claude(
    targets: &BTreeMap<&str, &ManagedTarget>,
    context: &RouteParseContext,
) -> Result<RouteMode, String> {
    let value: Value = serde_json::from_str(required_text(targets, "settings")?)
        .map_err(|error| format!("解析 Claude settings 失败: {error}"))?;
    let env = value
        .get("env")
        .and_then(Value::as_object)
        .ok_or_else(|| "Claude settings 缺少 env 对象".to_string())?;
    let actual = required_string(env.get("ANTHROPIC_BASE_URL"), "ANTHROPIC_BASE_URL")?;
    let mode = classify_url(actual, &context.expected_gateway_url)?;
    if mode == RouteMode::Proxy {
        let has_placeholder = [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "OPENROUTER_API_KEY",
            "OPENAI_API_KEY",
        ]
        .into_iter()
        .any(|key| env.get(key).and_then(Value::as_str) == Some(PROXY_TOKEN_PLACEHOLDER));
        if !has_placeholder {
            return Err("Claude proxy 配置缺少 AGS token 占位符".to_string());
        }
    }
    Ok(mode)
}

fn parse_codex(
    targets: &BTreeMap<&str, &ManagedTarget>,
    context: &RouteParseContext,
) -> Result<RouteMode, String> {
    let config_text = required_text(targets, "config")?;
    let document: toml::Value = toml::from_str(config_text)
        .map_err(|error| format!("解析 Codex config.toml 失败: {error}"))?;
    if let Some(catalog) = optional_text(targets, "model_catalog")? {
        serde_json::from_str::<Value>(catalog)
            .map_err(|error| format!("解析 Codex model_catalog 失败: {error}"))?;
    }
    let auth = optional_text(targets, "auth")?
        .map(|text| {
            serde_json::from_str::<Value>(text)
                .map_err(|error| format!("解析 Codex auth.json 失败: {error}"))
        })
        .transpose()?;

    let root_url = document.get("base_url").and_then(toml::Value::as_str);
    let active_provider = document
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let provider_url = active_provider
        .map(|provider_id| {
            document
                .get("model_providers")
                .and_then(|providers| providers.get(provider_id))
                .and_then(|provider| provider.get("base_url"))
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("Codex 当前 model_provider {provider_id} 缺少 base_url"))
        })
        .transpose()?;
    let actual = match (root_url, provider_url) {
        (Some(root), Some(provider)) if root != provider => {
            return Err("Codex 顶层与当前 model_provider base_url 混合冲突".to_string())
        }
        (Some(root), Some(_)) | (Some(root), None) => root,
        (None, Some(provider)) => provider,
        (None, None) => return Err("Codex config.toml 缺少可判定的 base_url".to_string()),
    };
    let mode = classify_url(actual, &context.expected_gateway_url)?;
    if mode == RouteMode::Proxy {
        let auth_placeholder = auth
            .as_ref()
            .and_then(|value| value.get("OPENAI_API_KEY"))
            .and_then(Value::as_str)
            == Some(PROXY_TOKEN_PLACEHOLDER);
        let config_placeholder =
            crate::codex_config::extract_codex_experimental_bearer_token(config_text).as_deref()
                == Some(PROXY_TOKEN_PLACEHOLDER);
        if !auth_placeholder && !config_placeholder {
            return Err("Codex proxy 配置缺少 AGS token 占位符".to_string());
        }
    }
    Ok(mode)
}

fn parse_gemini(
    targets: &BTreeMap<&str, &ManagedTarget>,
    context: &RouteParseContext,
) -> Result<RouteMode, String> {
    let env = crate::gemini_config::parse_env_file_strict(required_text(targets, ".env")?)
        .map_err(|error| format!("解析 Gemini .env 失败: {error}"))?;
    let actual = env
        .get("GOOGLE_GEMINI_BASE_URL")
        .map(String::as_str)
        .ok_or_else(|| "Gemini .env 缺少 GOOGLE_GEMINI_BASE_URL".to_string())?;
    let mode = classify_url(actual, &context.expected_gateway_url)?;
    if mode == RouteMode::Proxy
        && env.get("GEMINI_API_KEY").map(String::as_str) != Some(PROXY_TOKEN_PLACEHOLDER)
    {
        return Err("Gemini proxy 配置缺少 AGS token 占位符".to_string());
    }
    Ok(mode)
}

fn parse_claude_desktop(
    targets: &BTreeMap<&str, &ManagedTarget>,
    context: &RouteParseContext,
) -> Result<RouteMode, String> {
    let normal = parse_json_object(required_text(targets, "normal_config")?, "normal_config")?;
    let threep = parse_json_object(required_text(targets, "threep_config")?, "threep_config")?;
    if normal.get("deploymentMode").and_then(Value::as_str) != Some("3p")
        || threep.get("deploymentMode").and_then(Value::as_str) != Some("3p")
    {
        return Err("Claude Desktop 多目标 deploymentMode 不是一致的 3p".to_string());
    }

    let meta = parse_json_object(required_text(targets, "meta")?, "meta")?;
    if meta.get("appliedId").and_then(Value::as_str)
        != Some(crate::claude_desktop_config::PROFILE_ID)
    {
        return Err("Claude Desktop 当前 appliedId 不是 Agent-Switch profile".to_string());
    }
    let matching_entries = meta
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "Claude Desktop meta 缺少 entries".to_string())?
        .iter()
        .filter(|entry| {
            entry.get("id").and_then(Value::as_str)
                == Some(crate::claude_desktop_config::PROFILE_ID)
        })
        .count();
    if matching_entries != 1 {
        return Err("Claude Desktop meta profile 条目缺失或重复".to_string());
    }

    let profile = parse_json_object(required_text(targets, "profile")?, "profile")?;
    if profile.get("inferenceProvider").and_then(Value::as_str) != Some("gateway")
        || profile
            .get("inferenceGatewayAuthScheme")
            .and_then(Value::as_str)
            != Some("bearer")
    {
        return Err("Claude Desktop profile 不是可判定的 gateway/bearer 配置".to_string());
    }
    let actual = required_string(
        profile.get("inferenceGatewayBaseUrl"),
        "inferenceGatewayBaseUrl",
    )?;
    let mode = classify_url(actual, &context.expected_gateway_url)?;
    validate_c2b_gateway_token(
        mode,
        profile.get("inferenceGatewayApiKey"),
        context,
        "Claude Desktop",
    )?;
    Ok(mode)
}

fn parse_opencode(
    targets: &BTreeMap<&str, &ManagedTarget>,
    context: &RouteParseContext,
) -> Result<RouteMode, String> {
    let config =
        crate::opencode_config::parse_opencode_config(required_text(targets, "opencode.json")?)
            .map_err(|error| format!("解析 OpenCode 配置失败: {error}"))?;
    let provider_id = required_current_provider(context, "OpenCode")?;
    let provider = config
        .get("provider")
        .and_then(Value::as_object)
        .and_then(|providers| providers.get(provider_id))
        .ok_or_else(|| format!("OpenCode 当前 provider {provider_id} fragment 缺失"))?;
    let options = provider
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("OpenCode 当前 provider {provider_id} 缺少 options"))?;
    let actual = required_string(options.get("baseURL"), "OpenCode options.baseURL")?;
    let mode = classify_url(actual, &context.expected_gateway_url)?;
    validate_c2b_gateway_token(mode, options.get("apiKey"), context, "OpenCode")?;
    Ok(mode)
}

fn parse_openclaw(
    targets: &BTreeMap<&str, &ManagedTarget>,
    context: &RouteParseContext,
) -> Result<RouteMode, String> {
    let config =
        crate::openclaw_config::parse_openclaw_config(required_text(targets, "openclaw.json")?)
            .map_err(|error| format!("解析 OpenClaw 配置失败: {error}"))?;
    let provider_id = required_current_provider(context, "OpenClaw")?;
    let provider = config
        .pointer("/models/providers")
        .and_then(Value::as_object)
        .and_then(|providers| providers.get(provider_id))
        .ok_or_else(|| format!("OpenClaw 当前 provider {provider_id} fragment 缺失"))?;
    let actual = required_string(provider.get("baseUrl"), "OpenClaw baseUrl")?;
    let mode = classify_url(actual, &context.expected_gateway_url)?;
    validate_c2b_gateway_token(mode, provider.get("apiKey"), context, "OpenClaw")?;
    Ok(mode)
}

fn parse_hermes(
    targets: &BTreeMap<&str, &ManagedTarget>,
    context: &RouteParseContext,
) -> Result<RouteMode, String> {
    let config = crate::hermes_config::parse_hermes_config(required_text(targets, "config.yaml")?)
        .map_err(|error| format!("解析 Hermes 配置失败: {error}"))?;
    let provider_id = required_current_provider(context, "Hermes")?;
    let matches = config
        .get("custom_providers")
        .and_then(serde_yaml::Value::as_sequence)
        .map(|providers| {
            providers
                .iter()
                .filter(|provider| {
                    provider.get("name").and_then(serde_yaml::Value::as_str) == Some(provider_id)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if matches.len() != 1 {
        return Err(format!(
            "Hermes 当前 provider {provider_id} fragment 缺失或重复"
        ));
    }
    let provider = matches[0];
    let actual = provider
        .get("base_url")
        .and_then(serde_yaml::Value::as_str)
        .ok_or_else(|| format!("Hermes 当前 provider {provider_id} 缺少 base_url"))?;
    let mode = classify_url(actual, &context.expected_gateway_url)?;
    let api_key = provider
        .get("api_key")
        .and_then(serde_yaml::Value::as_str)
        .map(|value| Value::String(value.to_string()));
    validate_c2b_gateway_token(mode, api_key.as_ref(), context, "Hermes")?;
    Ok(mode)
}

fn parse_json_object(text: &str, name: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("解析 Claude Desktop {name} 失败: {error}"))?;
    if !value.is_object() {
        return Err(format!("Claude Desktop {name} 根节点不是对象"));
    }
    Ok(value)
}

fn required_string<'a>(value: Option<&'a Value>, field: &str) -> Result<&'a str, String> {
    let value = value
        .and_then(Value::as_str)
        .ok_or_else(|| format!("缺少字符串字段 {field}"))?;
    if value.is_empty() || value != value.trim() {
        return Err(format!("字段 {field} 为空或含首尾空白"));
    }
    Ok(value)
}

fn required_current_provider<'a>(
    context: &'a RouteParseContext,
    app_name: &str,
) -> Result<&'a str, String> {
    context
        .current_provider_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| format!("{app_name} 当前 provider 不存在"))
}

fn validate_c2b_gateway_token(
    mode: RouteMode,
    token: Option<&Value>,
    context: &RouteParseContext,
    app_name: &str,
) -> Result<(), String> {
    if mode != RouteMode::Proxy {
        return Ok(());
    }
    let expected = context
        .gateway_token
        .as_deref()
        .ok_or_else(|| "当前本地网关 token 不存在，无法接受 proxy 路由".to_string())?;
    if token.and_then(Value::as_str) != Some(expected) {
        return Err(format!("{app_name} proxy token 与当前本地网关不一致"));
    }
    Ok(())
}

fn classify_url(actual: &str, expected: &Url) -> Result<RouteMode, String> {
    if actual != actual.trim() {
        return Err("路由 URL 含首尾空白".to_string());
    }
    let actual = Url::parse(actual).map_err(|error| format!("路由 URL 无效: {error}"))?;
    validate_base_url_shape(&actual)?;

    if urls_exactly_equal(&actual, expected) {
        return Ok(RouteMode::Proxy);
    }

    let same_host = actual.host() == expected.host();
    if same_host {
        return Err(format!(
            "路由指向当前网关主机但 scheme/port/path 不匹配：期望 {}",
            expected.as_str()
        ));
    }
    if is_deceptive_gateway_host(&actual) {
        return Err("路由 URL 使用伪装的本地网关主机名".to_string());
    }
    // A real upstream may legitimately be self-hosted on localhost or a private network.
    // Only the exact current gateway URL is Proxy; variants on the same exact gateway host
    // were rejected above. Other well-formed HTTP(S) URLs are reliably Direct.
    Ok(RouteMode::Direct)
}

fn validate_base_url_shape(url: &Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("路由 URL scheme 必须是 http 或 https".to_string());
    }
    if url.host().is_none() {
        return Err("路由 URL 缺少 host".to_string());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("路由 URL 不得包含用户信息、query 或 fragment".to_string());
    }
    Ok(())
}

fn urls_exactly_equal(actual: &Url, expected: &Url) -> bool {
    actual.scheme() == expected.scheme()
        && actual.host() == expected.host()
        && actual.port() == expected.port()
        && actual.path() == expected.path()
        && actual.query().is_none()
        && actual.fragment().is_none()
}

fn is_deceptive_gateway_host(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => {
            let domain = domain.to_ascii_lowercase();
            // `Url` parses localhost.evil as a valid public domain. It is neither the
            // exact expected gateway host nor a reliable user upstream identity, so do
            // not silently accept it as Direct. Numeric-looking domains are likewise
            // rejected instead of bypassing structured IP comparison.
            domain.starts_with("localhost.")
                || domain.ends_with(".localhost")
                || domain
                    .strip_prefix("127.")
                    .is_some_and(|tail| tail.chars().all(|ch| ch.is_ascii_digit() || ch == '.'))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::external_config_monitor::ManagedTarget;
    use serde_json::json;

    fn managed(targets: &[(&str, Option<Vec<u8>>)]) -> ManagedExpected {
        ManagedExpected::new(
            1,
            targets
                .iter()
                .map(|(id, bytes)| ManagedTarget::file_bytes(*id, bytes.as_deref()))
                .collect(),
        )
        .unwrap()
    }

    fn json_bytes(value: Value) -> Vec<u8> {
        serde_json::to_vec(&value).unwrap()
    }

    fn context(app_type: &AppType) -> RouteParseContext {
        let url = match app_type {
            AppType::Claude | AppType::Gemini => "http://127.0.0.1:42567",
            AppType::Codex => "http://127.0.0.1:42567/v1",
            AppType::ClaudeDesktop => "http://127.0.0.1:42567/claude-desktop",
            AppType::OpenCode => "http://127.0.0.1:42567/opencode/v1",
            AppType::OpenClaw => "http://127.0.0.1:42567/openclaw/v1",
            AppType::Hermes => "http://127.0.0.1:42567/hermes/v1",
        };
        RouteParseContext::new(url, Some("current"), Some("gateway-token"))
    }

    fn capture_for(app_type: &AppType, url: &str, proxy_token: bool) -> ManagedExpected {
        match app_type {
            AppType::Claude => managed(&[(
                "settings",
                Some(json_bytes(json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": url,
                        "ANTHROPIC_AUTH_TOKEN": if proxy_token { "PROXY_MANAGED" } else { "real" }
                    }
                }))),
            )]),
            AppType::Codex => managed(&[
                (
                    "auth",
                    Some(json_bytes(json!({
                        "OPENAI_API_KEY": if proxy_token { "PROXY_MANAGED" } else { "real" }
                    }))),
                ),
                ("config", Some(format!("base_url = \"{url}\"\n").into_bytes())),
                ("model_catalog", None),
            ]),
            AppType::Gemini => managed(&[(
                ".env",
                Some(
                    format!(
                        "GOOGLE_GEMINI_BASE_URL={url}\nGEMINI_API_KEY={}\n",
                        if proxy_token { "PROXY_MANAGED" } else { "real" }
                    )
                    .into_bytes(),
                ),
            )]),
            AppType::ClaudeDesktop => managed(&[
                ("normal_config", Some(json_bytes(json!({"deploymentMode":"3p"})))),
                ("threep_config", Some(json_bytes(json!({"deploymentMode":"3p"})))),
                (
                    "profile",
                    Some(json_bytes(json!({
                        "inferenceProvider":"gateway",
                        "inferenceGatewayAuthScheme":"bearer",
                        "inferenceGatewayBaseUrl":url,
                        "inferenceGatewayApiKey":if proxy_token { "gateway-token" } else { "real" }
                    }))),
                ),
                (
                    "meta",
                    Some(json_bytes(json!({
                        "appliedId":crate::claude_desktop_config::PROFILE_ID,
                        "entries":[{"id":crate::claude_desktop_config::PROFILE_ID}]
                    }))),
                ),
            ]),
            AppType::OpenCode => managed(&[(
                "opencode.json",
                Some(json_bytes(json!({
                    "provider":{"current":{"options":{
                        "baseURL":url,
                        "apiKey":if proxy_token { "gateway-token" } else { "real" }
                    }}}
                }))),
            )]),
            AppType::OpenClaw => managed(&[(
                "openclaw.json",
                Some(json_bytes(json!({
                    "models":{"providers":{"current":{
                        "baseUrl":url,
                        "apiKey":if proxy_token { "gateway-token" } else { "real" }
                    }}}
                }))),
            )]),
            AppType::Hermes => managed(&[(
                "config.yaml",
                Some(
                    format!(
                        "custom_providers:\n  - name: current\n    base_url: {url}\n    api_key: {}\n",
                        if proxy_token { "gateway-token" } else { "real" }
                    )
                    .into_bytes(),
                ),
            )]),
        }
    }

    #[test]
    fn parses_direct_and_exact_proxy_for_all_seven_modules() {
        for app_type in AppType::all() {
            let context = context(&app_type);
            let proxy = capture_for(&app_type, context.expected_gateway_url.as_str(), true);
            assert_eq!(
                parse_actual_route_with_context(&app_type, &proxy, &context).unwrap(),
                RouteMode::Proxy,
                "{} proxy",
                app_type.as_str()
            );

            let direct = capture_for(&app_type, "https://relay.example.com/upstream/v1", false);
            assert_eq!(
                parse_actual_route_with_context(&app_type, &direct, &context).unwrap(),
                RouteMode::Direct,
                "{} direct",
                app_type.as_str()
            );
        }
    }

    #[test]
    fn accepts_self_hosted_local_and_private_upstreams_as_direct() {
        let app_type = AppType::OpenCode;
        let context = context(&app_type);
        for upstream_url in [
            "http://localhost:9000/v1",
            "http://10.0.0.5:8080/v1",
            "http://192.168.1.20:11434/v1",
            "http://[::1]:9000/v1",
        ] {
            let capture = capture_for(&app_type, upstream_url, false);
            assert_eq!(
                parse_actual_route_with_context(&app_type, &capture, &context).unwrap(),
                RouteMode::Direct,
                "self-hosted upstream {upstream_url}"
            );
        }
    }

    #[test]
    fn rejects_deceptive_old_port_wrong_namespace_and_malformed_urls() {
        let app_type = AppType::OpenCode;
        let context = context(&app_type);
        for bad_url in [
            "http://localhost.evil:42567/opencode/v1",
            "http://127.0.0.1:42568/opencode/v1",
            "http://127.0.0.1:42567/openclaw/v1",
            "not a url",
        ] {
            let capture = capture_for(&app_type, bad_url, true);
            assert!(
                parse_actual_route_with_context(&app_type, &capture, &context).is_err(),
                "must reject {bad_url}"
            );
        }
    }

    #[test]
    fn rejects_missing_mixed_and_current_provider_mismatch() {
        let codex_context = context(&AppType::Codex);
        let mixed = managed(&[
            ("auth", Some(json_bytes(json!({"OPENAI_API_KEY":"PROXY_MANAGED"})))),
            (
                "config",
                Some(
                    b"base_url = \"http://127.0.0.1:42567/v1\"\nmodel_provider = \"other\"\n[model_providers.other]\nbase_url = \"https://relay.example.com/v1\"\n"
                        .to_vec(),
                ),
            ),
            ("model_catalog", None),
        ]);
        assert!(parse_actual_route_with_context(&AppType::Codex, &mixed, &codex_context).is_err());

        let missing = managed(&[("config", None), ("auth", None), ("model_catalog", None)]);
        assert!(
            parse_actual_route_with_context(&AppType::Codex, &missing, &codex_context).is_err()
        );

        let opencode = managed(&[(
            "opencode.json",
            Some(json_bytes(json!({
                "provider":{"other":{"options":{
                    "baseURL":"http://127.0.0.1:42567/opencode/v1",
                    "apiKey":"gateway-token"
                }}}
            }))),
        )]);
        assert!(parse_actual_route_with_context(
            &AppType::OpenCode,
            &opencode,
            &context(&AppType::OpenCode)
        )
        .is_err());
    }
}
