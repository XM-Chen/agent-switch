//! Claude Desktop 网关路由解析模块
//!
//! 本模块从原 `claude_desktop_config.rs` 中提取网关仍在使用的纯请求模型映射。
//! 客户端供应商校验与文件 I/O（apply / status / snapshot 等）已随接管域删除。

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::provider::Provider;

const MIMO_REDACTED_THINKING_PLACEHOLDER: &str = "[redacted thinking]";
const MIMO_TOOL_CALL_THINKING_PLACEHOLDER: &str = "tool call";

/// Claude Desktop 模型菜单识别的 route ID 前缀。
pub const CLAUDE_ROUTE_PREFIX: &str = "claude-";
/// 替代前缀（与前端 `ANTHROPIC_CLAUDE_ROUTE_PREFIX` 一致）。
pub const ANTHROPIC_CLAUDE_ROUTE_PREFIX: &str = "anthropic/claude-";
/// Claude Code env 中通过 `[1M]` 后缀声明 1M 上下文能力（匹配用 `eq_ignore_ascii_case`）。
/// Claude Desktop schema 不接受此后缀，import 边界翻译为 `supports1m` 字段。
pub const ONE_M_CONTEXT_MARKER: &str = "[1m]";

const CURRENT_OPUS_ROUTE_ID: &str = "claude-opus-4-8";
const LEGACY_OPUS_ROUTE_ID: &str = "claude-opus-4-7";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDesktopDefaultRoute {
    pub route_id: &'static str,
    pub env_key: &'static str,
    #[serde(rename = "supports1m")]
    pub supports_1m: bool,
}

pub const DEFAULT_PROXY_ROUTES: &[ClaudeDesktopDefaultRoute] = &[
    ClaudeDesktopDefaultRoute {
        route_id: "claude-sonnet-5",
        env_key: "ANTHROPIC_DEFAULT_SONNET_MODEL",
        supports_1m: true,
    },
    ClaudeDesktopDefaultRoute {
        route_id: CURRENT_OPUS_ROUTE_ID,
        env_key: "ANTHROPIC_DEFAULT_OPUS_MODEL",
        supports_1m: true,
    },
    ClaudeDesktopDefaultRoute {
        route_id: "claude-haiku-4-5",
        env_key: "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        supports_1m: true,
    },
    // fable 置于末尾：next_catalog_safe_route_id 给非安全品牌 route 借用合法
    // 角色名时仍按 sonnet->opus->haiku 顺序分配（向后兼容既有 catalog），不会把
    // 无关品牌模型借用成 fable 顶配档名。UI 行序由前端 ROLE_ORDER 独立控制为
    // Sonnet/Opus/Fable/Haiku（所有 proxy 路径都经 normalizeProxyRows 重排），
    // 与此处物理顺序无关。
    ClaudeDesktopDefaultRoute {
        route_id: "claude-fable-5",
        env_key: "ANTHROPIC_DEFAULT_FABLE_MODEL",
        supports_1m: true,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelRoute {
    pub route_id: String,
    pub upstream_model: String,
    pub label_override: Option<String>,
    pub supports_1m: bool,
}

pub fn is_claude_safe_model_id(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized.contains(ONE_M_CONTEXT_MARKER) {
        return false;
    }

    let Some(route_tail) = normalized
        .strip_prefix(ANTHROPIC_CLAUDE_ROUTE_PREFIX)
        .or_else(|| normalized.strip_prefix(CLAUDE_ROUTE_PREFIX))
    else {
        return false;
    };

    // 角色前缀后必须还有实际模型标识，拒绝 claude-sonnet- 这类退化值
    // （否则会写入 profile 并触发 Claude Desktop fail-all 拒收整组）。
    // Claude Desktop 1.12603.1+ 的 fail-all validator 角色白名单已纳入 fable
    // （app.asar 内 ["sonnet","opus","haiku","fable","mythos"]），故 claude-fable-*
    // 可安全写入 profile。mythos 官方未公开发布，暂不暴露给用户。
    ["sonnet-", "opus-", "haiku-", "fable-"]
        .iter()
        .any(|prefix| {
            route_tail
                .strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty())
        })
}

pub fn proxy_model_routes(provider: &Provider) -> Result<Vec<ResolvedModelRoute>, AppError> {
    let routes = provider
        .meta
        .as_ref()
        .map(|meta| &meta.claude_desktop_model_routes)
        .ok_or_else(|| {
            AppError::localized(
                "claude_desktop.provider.routes_missing",
                "Claude Desktop 本地路由模式缺少模型路由映射",
                "Claude Desktop proxy mode is missing model route mappings",
            )
        })?;

    let reserved_route_ids = routes
        .keys()
        .map(|route_id| route_id.trim())
        .filter(|route_id| is_claude_safe_model_id(route_id))
        .map(str::to_string)
        .collect::<std::collections::HashSet<_>>();
    let mut result = Vec::new();
    let mut entries = routes.iter().collect::<Vec<_>>();
    entries.sort_by_key(|(left, _)| *left);
    for (route_id, route) in entries {
        let supports_1m = route.supports_1m.unwrap_or(false);
        let route_id = route_id.trim();
        let upstream_model = route.model.trim();
        if route_id.is_empty() || upstream_model.is_empty() {
            continue;
        }
        let repaired_route_id = if is_claude_safe_model_id(route_id) {
            route_id.to_string()
        } else {
            next_catalog_safe_route_id(&result, &reserved_route_ids)
        };
        result.push(ResolvedModelRoute {
            route_id: repaired_route_id,
            upstream_model: upstream_model.to_string(),
            label_override: route
                .label_override
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    (!is_claude_safe_model_id(route_id)).then(|| upstream_model.to_string())
                }),
            supports_1m,
        });
    }

    result.sort_by(|a, b| a.route_id.cmp(&b.route_id));
    result.dedup_by(|a, b| a.route_id == b.route_id);

    if result.is_empty() {
        return Err(AppError::localized(
            "claude_desktop.provider.routes_missing",
            "Claude Desktop 本地路由模式至少需要一个模型路由映射",
            "Claude Desktop proxy mode requires at least one model route mapping",
        ));
    }

    Ok(result)
}

fn next_catalog_safe_route_id(
    existing: &[ResolvedModelRoute],
    reserved: &std::collections::HashSet<String>,
) -> String {
    if let Some(default_route) = DEFAULT_PROXY_ROUTES
        .iter()
        .map(|route| route.route_id)
        .find(|route_id| {
            !reserved.contains(*route_id)
                && !existing.iter().any(|route| route.route_id == *route_id)
        })
    {
        return default_route.to_string();
    }

    let mut index = 2usize;
    loop {
        let route_id = format!("{}-r{index}", DEFAULT_PROXY_ROUTES[0].route_id);
        if !reserved.contains(&route_id) && !existing.iter().any(|route| route.route_id == route_id)
        {
            return route_id;
        }
        index += 1;
    }
}

pub fn map_proxy_request_model(mut body: Value, provider: &Provider) -> Result<Value, AppError> {
    let requested_raw = body
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::localized(
                "claude_desktop.provider.model_missing",
                "Claude Desktop 请求缺少 model 字段",
                "Claude Desktop request is missing the model field",
            )
        })?;
    let requested = strip_one_m_suffix_for_route_lookup(&requested_raw);

    let routes = proxy_model_routes(provider)?;
    let upstream_model = routes
        .iter()
        .find(|r| r.route_id == requested)
        .or_else(|| {
            routes
                .iter()
                .find(|r| is_compatible_opus_route_alias(&r.route_id, requested))
        })
        .map(|route| route.upstream_model.clone())
        .or_else(|| legacy_raw_route_upstream_model(provider, requested))
        .or_else(|| {
            // 角色关键词回落:Claude Desktop 的部分调用(如子 agent)会请求带发布
            // 日期后缀的完整官方名(claude-haiku-4-5-20251001),与 manifest 暴露的
            // 简短 route_id(claude-haiku-4-5)不精确相等。按 opus/haiku/fable/sonnet
            // 归类到同档已配置路由,对齐 Claude Code model_mapper 的宽松匹配。
            // 匹配前已剥离本地 [1m] 标记；这里仍只对 Claude Desktop 认可的
            // 安全模型名回落，避免非 Claude route 被误映射。
            if !is_claude_safe_model_id(requested) {
                return None;
            }
            let role = claude_role_keyword(requested)?;
            routes
                .iter()
                .find(|route| claude_role_keyword(&route.route_id) == Some(role))
                // 老用户只配了 Sonnet/Opus/Haiku 三档时，fable 请求降级到 opus 档，
                // 与官方安全分类器的降级方向一致，避免 route_unknown 硬错误。
                // 用户一旦显式配置 fable 档，上面的精确角色匹配会优先命中。
                .or_else(|| {
                    (role == "fable")
                        .then(|| {
                            routes
                                .iter()
                                .find(|route| claude_role_keyword(&route.route_id) == Some("opus"))
                        })
                        .flatten()
                })
                .map(|route| route.upstream_model.clone())
        })
        .ok_or_else(|| {
            AppError::localized(
                "claude_desktop.provider.route_unknown",
                format!("Claude Desktop 模型路由未配置: {requested_raw}"),
                format!("Claude Desktop model route is not configured: {requested_raw}"),
            )
        })?;

    body["model"] = json!(upstream_model);
    if should_normalize_mimo_anthropic_thinking_history(provider, &upstream_model) {
        normalize_mimo_anthropic_thinking_history(&mut body);
    }
    Ok(body)
}

fn strip_one_m_suffix_for_route_lookup(model: &str) -> &str {
    let trimmed = model.trim();
    let marker = ONE_M_CONTEXT_MARKER.as_bytes();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= marker.len()
        && bytes[bytes.len() - marker.len()..].eq_ignore_ascii_case(marker)
    {
        return trimmed[..trimmed.len() - marker.len()].trim_end();
    }
    trimmed
}

fn legacy_raw_route_upstream_model(provider: &Provider, requested: &str) -> Option<String> {
    provider
        .meta
        .as_ref()?
        .claude_desktop_model_routes
        .iter()
        .find(|(route_id, _)| route_id.trim() == requested)
        .and_then(|(_, route)| {
            let upstream_model = route.model.trim();
            (!upstream_model.is_empty()).then(|| upstream_model.to_string())
        })
}

fn is_compatible_opus_route_alias(route_id: &str, requested: &str) -> bool {
    matches!(
        (route_id, requested),
        (CURRENT_OPUS_ROUTE_ID, LEGACY_OPUS_ROUTE_ID)
            | (LEGACY_OPUS_ROUTE_ID, CURRENT_OPUS_ROUTE_ID)
    )
}

/// 按角色关键词(opus / haiku / fable / sonnet)归类一个 Claude 模型名/route_id。
/// 仅在命中明确角色词时返回 Some,未知模型返回 None(不回落,保持精确报错语义)。
/// 与前端 `routeRoleFromId` 同序(opus -> haiku -> fable -> sonnet)。
fn claude_role_keyword(model: &str) -> Option<&'static str> {
    let normalized = model.to_ascii_lowercase();
    if normalized.contains("opus") {
        Some("opus")
    } else if normalized.contains("haiku") {
        Some("haiku")
    } else if normalized.contains("fable") {
        Some("fable")
    } else if normalized.contains("sonnet") {
        Some("sonnet")
    } else {
        None
    }
}

fn should_normalize_mimo_anthropic_thinking_history(
    provider: &Provider,
    upstream_model: &str,
) -> bool {
    if !provider_uses_anthropic_messages_format(provider) {
        return false;
    }

    is_mimo_identifier(upstream_model) || provider_has_mimo_endpoint(provider)
}

fn provider_uses_anthropic_messages_format(provider: &Provider) -> bool {
    let api_format = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.api_format.as_deref())
        .or_else(|| {
            provider
                .settings_config
                .get("api_format")
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .unwrap_or("anthropic");

    api_format.is_empty() || api_format == "anthropic"
}

fn provider_has_mimo_endpoint(provider: &Provider) -> bool {
    let settings = &provider.settings_config;
    [
        settings
            .get("env")
            .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
            .and_then(Value::as_str),
        settings.get("base_url").and_then(Value::as_str),
        settings.get("baseURL").and_then(Value::as_str),
        settings.get("apiEndpoint").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .any(is_mimo_identifier)
}

fn is_mimo_identifier(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("mimo") || value.contains("xiaomimimo")
}

fn normalize_mimo_anthropic_thinking_history(body: &mut Value) {
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }

        let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        if !content
            .iter()
            .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
        {
            continue;
        }

        let mut has_thinking = false;
        for block in content.iter_mut() {
            match block.get("type").and_then(Value::as_str) {
                Some("thinking") => {
                    let has_non_empty_thinking = block
                        .get("thinking")
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.trim().is_empty());
                    if let Some(obj) = block.as_object_mut() {
                        obj.remove("signature");
                    }
                    if has_non_empty_thinking {
                        has_thinking = true;
                    } else if let Some(obj) = block.as_object_mut() {
                        obj.insert(
                            "thinking".to_string(),
                            json!(MIMO_TOOL_CALL_THINKING_PLACEHOLDER),
                        );
                        has_thinking = true;
                    }
                }
                Some("redacted_thinking") => {
                    *block = json!({
                        "type": "thinking",
                        "thinking": MIMO_REDACTED_THINKING_PLACEHOLDER
                    });
                    has_thinking = true;
                }
                _ => {}
            }
        }

        if !has_thinking {
            content.insert(
                0,
                json!({
                    "type": "thinking",
                    "thinking": MIMO_TOOL_CALL_THINKING_PLACEHOLDER
                }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{ClaudeDesktopMode, ClaudeDesktopModelRoute, ProviderMeta};
    use serde_json::json;

    fn direct_provider(id: &str) -> Provider {
        let mut provider = Provider::with_id(
            id.to_string(),
            "Direct".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://gateway.example.com",
                    "ANTHROPIC_AUTH_TOKEN": "test-token",
                    "ANTHROPIC_MODEL": "ignored-by-desktop"
                }
            }),
            Some("https://example.com".to_string()),
        );
        provider.meta = Some(ProviderMeta {
            api_format: Some("anthropic".to_string()),
            ..Default::default()
        });
        provider
    }

    fn proxy_provider(id: &str) -> Provider {
        let mut provider = direct_provider(id);
        provider.name = "Proxy".to_string();
        provider.meta = Some(ProviderMeta {
            claude_desktop_mode: Some(ClaudeDesktopMode::Proxy),
            api_format: Some("openai_chat".to_string()),
            claude_desktop_model_routes: std::collections::HashMap::from([(
                "claude-sonnet-4-6".to_string(),
                ClaudeDesktopModelRoute {
                    model: "kimi-k2".to_string(),
                    label_override: Some("Kimi K2".to_string()),
                    supports_1m: Some(true),
                },
            )]),
            ..Default::default()
        });
        provider
    }

    fn mimo_anthropic_proxy_provider(id: &str) -> Provider {
        let mut provider = direct_provider(id);
        provider.name = "MiMo Proxy".to_string();
        provider.settings_config = json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.xiaomimimo.com/anthropic",
                "ANTHROPIC_AUTH_TOKEN": "test-token"
            }
        });
        provider.meta = Some(ProviderMeta {
            claude_desktop_mode: Some(ClaudeDesktopMode::Proxy),
            api_format: Some("anthropic".to_string()),
            claude_desktop_model_routes: std::collections::HashMap::from([(
                "claude-sonnet-4-6".to_string(),
                ClaudeDesktopModelRoute {
                    model: "mimo-v2.5-pro".to_string(),
                    label_override: Some("MiMo v2.5 Pro".to_string()),
                    supports_1m: Some(true),
                },
            )]),
            ..Default::default()
        });
        provider
    }

    #[test]
    fn claude_desktop_proxy_maps_known_route_and_rejects_unknown_route() {
        let provider = proxy_provider("proxy");

        let mapped = map_proxy_request_model(
            json!({"model": "claude-sonnet-4-6", "messages": []}),
            &provider,
        )
        .expect("map route");
        assert_eq!(mapped["model"], json!("kimi-k2"));

        let err = map_proxy_request_model(json!({"model": "claude-opus-4-8"}), &provider)
            .expect_err("unknown route should fail");
        assert!(err.to_string().contains("claude-opus-4-8"));
    }

    #[test]
    fn claude_desktop_proxy_maps_dated_role_alias_via_keyword() {
        // 复现反馈：Claude Desktop 子 agent 请求带发布日期后缀的完整官方名
        // （claude-haiku-4-5-20251001），与 manifest 的简短 route_id（claude-haiku-4-5）
        // 不精确相等，旧逻辑会报 route_unknown。角色关键词回落应将其映射到 Haiku 档。
        let mut provider = proxy_provider("proxy");
        provider
            .meta
            .as_mut()
            .expect("meta")
            .claude_desktop_model_routes = std::collections::HashMap::from([
            (
                "claude-sonnet-4-6".to_string(),
                ClaudeDesktopModelRoute {
                    model: "deepseek-v4-pro".to_string(),
                    label_override: None,
                    supports_1m: Some(true),
                },
            ),
            (
                "claude-opus-4-8".to_string(),
                ClaudeDesktopModelRoute {
                    model: "deepseek-v4-pro".to_string(),
                    label_override: None,
                    supports_1m: Some(true),
                },
            ),
            (
                "claude-haiku-4-5".to_string(),
                ClaudeDesktopModelRoute {
                    model: "deepseek-v4-flash".to_string(),
                    label_override: None,
                    supports_1m: Some(true),
                },
            ),
        ]);

        let mapped = map_proxy_request_model(
            json!({"model": "claude-haiku-4-5-20251001", "messages": []}),
            &provider,
        )
        .expect("dated Haiku alias should map via role keyword");
        assert_eq!(mapped["model"], json!("deepseek-v4-flash"));

        let mapped_sonnet = map_proxy_request_model(
            json!({"model": "claude-sonnet-4-5-20250101", "messages": []}),
            &provider,
        )
        .expect("dated Sonnet alias should map via role keyword");
        assert_eq!(mapped_sonnet["model"], json!("deepseek-v4-pro"));

        // 不含任何角色关键词的模型仍然报错，避免被误映射。
        let err = map_proxy_request_model(json!({"model": "gpt-5"}), &provider)
            .expect_err("model without a role keyword should still fail");
        assert!(err.to_string().contains("gpt-5"));
    }

    #[test]
    fn claude_desktop_proxy_maps_fable_to_opus_tier() {
        // issue #4026/#4049：老用户只配 Sonnet/Opus/Haiku 三档、未显式配置
        // fable 档时，fable 请求按官方分类器降级方向回落到 opus 档兜底。
        let mut provider = proxy_provider("proxy");
        provider
            .meta
            .as_mut()
            .expect("meta")
            .claude_desktop_model_routes = std::collections::HashMap::from([
            (
                "claude-opus-4-8".to_string(),
                ClaudeDesktopModelRoute {
                    model: "upstream-opus".to_string(),
                    label_override: None,
                    supports_1m: Some(true),
                },
            ),
            (
                "claude-sonnet-4-6".to_string(),
                ClaudeDesktopModelRoute {
                    model: "upstream-sonnet".to_string(),
                    label_override: None,
                    supports_1m: Some(true),
                },
            ),
        ]);

        let mapped = map_proxy_request_model(
            json!({"model": "claude-fable-5", "messages": []}),
            &provider,
        )
        .expect("fable should fall back to the opus tier");
        assert_eq!(mapped["model"], json!("upstream-opus"));

        // 带 [1m] 标记与日期后缀的形态也应命中同一回落。
        let mapped_one_m = map_proxy_request_model(
            json!({"model": "claude-fable-5[1m]", "messages": []}),
            &provider,
        )
        .expect("fable with [1m] marker should fall back to the opus tier");
        assert_eq!(mapped_one_m["model"], json!("upstream-opus"));

        let mapped_dated = map_proxy_request_model(
            json!({"model": "claude-fable-5-20260609", "messages": []}),
            &provider,
        )
        .expect("dated fable alias should fall back to the opus tier");
        assert_eq!(mapped_dated["model"], json!("upstream-opus"));
    }

    #[test]
    fn claude_desktop_proxy_fable_without_opus_route_still_errors() {
        // 没有 opus 档可回落时保持精确报错语义，不静默落到其他档。
        let mut provider = proxy_provider("proxy");
        provider
            .meta
            .as_mut()
            .expect("meta")
            .claude_desktop_model_routes = std::collections::HashMap::from([(
            "claude-sonnet-4-6".to_string(),
            ClaudeDesktopModelRoute {
                model: "upstream-sonnet".to_string(),
                label_override: None,
                supports_1m: Some(true),
            },
        )]);

        let err = map_proxy_request_model(
            json!({"model": "claude-fable-5", "messages": []}),
            &provider,
        )
        .expect_err("fable without an opus route should fail");
        assert!(err.to_string().contains("claude-fable-5"));
    }

    #[test]
    fn claude_desktop_proxy_maps_fable_to_dedicated_route() {
        // Desktop 1.12603.1+ fail-all 校验已放行 claude-fable-5，用户可显式配置
        // 独立 fable 档；此时 fable 请求精确命中 fable 档，不再降级到 opus。
        let mut provider = proxy_provider("proxy");
        provider
            .meta
            .as_mut()
            .expect("meta")
            .claude_desktop_model_routes = std::collections::HashMap::from([
            (
                "claude-opus-4-8".to_string(),
                ClaudeDesktopModelRoute {
                    model: "upstream-opus".to_string(),
                    label_override: None,
                    supports_1m: Some(true),
                },
            ),
            (
                "claude-fable-5".to_string(),
                ClaudeDesktopModelRoute {
                    model: "upstream-fable".to_string(),
                    label_override: None,
                    supports_1m: Some(true),
                },
            ),
        ]);

        // 精确匹配优先命中 fable 档
        let mapped = map_proxy_request_model(
            json!({"model": "claude-fable-5", "messages": []}),
            &provider,
        )
        .expect("explicit fable route should match");
        assert_eq!(mapped["model"], json!("upstream-fable"));

        // 带日期后缀经角色关键词回落仍归 fable 档，而非降级 opus
        let mapped_dated = map_proxy_request_model(
            json!({"model": "claude-fable-5-20260609", "messages": []}),
            &provider,
        )
        .expect("dated fable alias should map via fable role keyword");
        assert_eq!(mapped_dated["model"], json!("upstream-fable"));
    }

    #[test]
    fn claude_desktop_proxy_accepts_opus_4_7_4_8_alias_during_rollout() {
        let mut provider = proxy_provider("proxy");
        let current_routes = std::collections::HashMap::from([(
            "claude-opus-4-8".to_string(),
            ClaudeDesktopModelRoute {
                model: "upstream-opus-new".to_string(),
                label_override: None,
                supports_1m: Some(true),
            },
        )]);
        provider
            .meta
            .as_mut()
            .expect("meta")
            .claude_desktop_model_routes = current_routes;

        let mapped = map_proxy_request_model(
            json!({"model": "claude-opus-4-7", "messages": []}),
            &provider,
        )
        .expect("legacy Opus route should map to current route");
        assert_eq!(mapped["model"], json!("upstream-opus-new"));

        let legacy_routes = std::collections::HashMap::from([(
            "claude-opus-4-7".to_string(),
            ClaudeDesktopModelRoute {
                model: "upstream-opus-legacy".to_string(),
                label_override: None,
                supports_1m: Some(true),
            },
        )]);
        provider
            .meta
            .as_mut()
            .expect("meta")
            .claude_desktop_model_routes = legacy_routes;

        let mapped = map_proxy_request_model(
            json!({"model": "claude-opus-4-8", "messages": []}),
            &provider,
        )
        .expect("current Opus route should map to legacy saved route");
        assert_eq!(mapped["model"], json!("upstream-opus-legacy"));
    }

    #[test]
    fn claude_desktop_mimo_anthropic_rewrites_redacted_thinking_for_tool_history() {
        let provider = mimo_anthropic_proxy_provider("mimo");

        let mapped = map_proxy_request_model(
            json!({
                "model": "claude-sonnet-4-6",
                "messages": [{
                    "role": "assistant",
                    "content": [
                        {"type": "redacted_thinking", "data": "opaque"},
                        {"type": "tool_use", "id": "call_1", "name": "read_file", "input": {"path": "README.md"}}
                    ]
                }]
            }),
            &provider,
        )
        .expect("map MiMo route");

        assert_eq!(mapped["model"], json!("mimo-v2.5-pro"));
        assert_eq!(
            mapped["messages"][0]["content"][0]["type"],
            json!("thinking")
        );
        assert_eq!(
            mapped["messages"][0]["content"][0]["thinking"],
            json!("[redacted thinking]")
        );
        assert_eq!(
            mapped["messages"][0]["content"][1]["type"],
            json!("tool_use")
        );
    }

    #[test]
    fn claude_desktop_mimo_anthropic_injects_thinking_for_tool_history_without_one() {
        let provider = mimo_anthropic_proxy_provider("mimo");

        let mapped = map_proxy_request_model(
            json!({
                "model": "claude-sonnet-4-6",
                "messages": [{
                    "role": "assistant",
                    "content": [
                        {"type": "tool_use", "id": "call_1", "name": "read_file", "input": {"path": "README.md"}}
                    ]
                }]
            }),
            &provider,
        )
        .expect("map MiMo route");

        assert_eq!(
            mapped["messages"][0]["content"][0]["type"],
            json!("thinking")
        );
        assert_eq!(
            mapped["messages"][0]["content"][0]["thinking"],
            json!("tool call")
        );
        assert_eq!(
            mapped["messages"][0]["content"][1]["type"],
            json!("tool_use")
        );
    }

    #[test]
    fn claude_desktop_mimo_anthropic_keeps_thinking_text_but_drops_signature() {
        let provider = mimo_anthropic_proxy_provider("mimo");

        let mapped = map_proxy_request_model(
            json!({
                "model": "claude-sonnet-4-6",
                "messages": [{
                    "role": "assistant",
                    "content": [
                        {"type": "thinking", "thinking": "Need to inspect the file.", "signature": "anthropic-signature"},
                        {"type": "tool_use", "id": "call_1", "name": "read_file", "input": {"path": "README.md"}}
                    ]
                }]
            }),
            &provider,
        )
        .expect("map MiMo route");

        assert_eq!(
            mapped["messages"][0]["content"][0]["thinking"],
            json!("Need to inspect the file.")
        );
        assert!(mapped["messages"][0]["content"][0]
            .get("signature")
            .is_none());
    }

    #[test]
    fn claude_desktop_proxy_repairs_legacy_unsafe_route_without_colliding() {
        let mut provider = proxy_provider("proxy");
        provider.meta = Some(ProviderMeta {
            claude_desktop_mode: Some(ClaudeDesktopMode::Proxy),
            api_format: Some("openai_chat".to_string()),
            claude_desktop_model_routes: std::collections::HashMap::from([
                (
                    "claude-deepseek-v4-pro".to_string(),
                    ClaudeDesktopModelRoute {
                        model: "deepseek-v4-pro".to_string(),
                        label_override: None,
                        supports_1m: Some(true),
                    },
                ),
                (
                    "claude-old".to_string(),
                    ClaudeDesktopModelRoute {
                        model: "legacy-upstream".to_string(),
                        label_override: None,
                        supports_1m: Some(false),
                    },
                ),
                (
                    "claude-sonnet-5".to_string(),
                    ClaudeDesktopModelRoute {
                        model: "claude-sonnet-5".to_string(),
                        label_override: None,
                        supports_1m: Some(false),
                    },
                ),
            ]),
            ..Default::default()
        });

        let routes = proxy_model_routes(&provider).expect("routes");
        assert_eq!(routes.len(), 3);
        let repaired = routes
            .iter()
            .find(|route| route.upstream_model == "deepseek-v4-pro")
            .expect("repaired route");
        assert_eq!(repaired.route_id, "claude-opus-4-8");
        assert_eq!(repaired.label_override.as_deref(), Some("deepseek-v4-pro"));
        assert!(repaired.supports_1m);
        let repaired_old = routes
            .iter()
            .find(|route| route.upstream_model == "legacy-upstream")
            .expect("legacy route should be repaired");
        assert_eq!(repaired_old.route_id, "claude-haiku-4-5");
        assert_eq!(
            repaired_old.label_override.as_deref(),
            Some("legacy-upstream")
        );

        let mapped = map_proxy_request_model(
            json!({"model": "claude-opus-4-8", "messages": []}),
            &provider,
        )
        .expect("map repaired route");
        assert_eq!(mapped["model"], json!("deepseek-v4-pro"));

        let legacy_mapped =
            map_proxy_request_model(json!({"model": "claude-old", "messages": []}), &provider)
                .expect("map stale profile route");
        assert_eq!(legacy_mapped["model"], json!("legacy-upstream"));
    }

    #[test]
    fn claude_desktop_proxy_strips_1m_suffix_before_route_lookup() {
        let mut provider = proxy_provider("proxy");
        provider
            .meta
            .as_mut()
            .expect("meta")
            .claude_desktop_model_routes = std::collections::HashMap::from([
            (
                "claude-sonnet-4-6".to_string(),
                ClaudeDesktopModelRoute {
                    model: "upstream-sonnet".to_string(),
                    label_override: None,
                    supports_1m: Some(true),
                },
            ),
            (
                "claude-opus-4-8".to_string(),
                ClaudeDesktopModelRoute {
                    model: "upstream-opus".to_string(),
                    label_override: None,
                    supports_1m: Some(true),
                },
            ),
        ]);

        let mapped = map_proxy_request_model(
            json!({"model": "claude-opus-4-8[1m]", "messages": []}),
            &provider,
        )
        .expect("compact 1M suffix should map to Opus route");
        assert_eq!(mapped["model"], json!("upstream-opus"));

        let mapped = map_proxy_request_model(
            json!({"model": "claude-sonnet-4-6 [1M]", "messages": []}),
            &provider,
        )
        .expect("spaced uppercase 1M suffix should map to Sonnet route");
        assert_eq!(mapped["model"], json!("upstream-sonnet"));

        let err = map_proxy_request_model(json!({"model": "gpt-5[1m]", "messages": []}), &provider)
            .expect_err("non-Claude route should still fail after stripping 1M suffix");
        assert!(err.to_string().contains("gpt-5[1m]"));
    }

    #[test]
    fn claude_desktop_rejects_1m_suffix_as_model_id() {
        assert!(!is_claude_safe_model_id("claude-sonnet-4-6 [1m]"));
        assert!(!is_claude_safe_model_id("  claude-sonnet-4-6  [1M]  "));
        assert!(!is_claude_safe_model_id("claude-old"));
        assert!(!is_claude_safe_model_id("claude-3-5-sonnet-20241022"));
        assert!(!is_claude_safe_model_id("claude-deepseek-v4-pro"));
        assert!(!is_claude_safe_model_id("claude-gpt-5-4"));
        assert!(!is_claude_safe_model_id("claude-"));
        assert!(!is_claude_safe_model_id("anthropic/claude-"));
        assert!(!is_claude_safe_model_id("sonnet"));
        assert!(!is_claude_safe_model_id("sonnet-"));
        // 角色前缀后无实际标识的退化值必须拒绝
        assert!(!is_claude_safe_model_id("claude-sonnet-"));
        assert!(!is_claude_safe_model_id("claude-opus-"));
        assert!(!is_claude_safe_model_id("anthropic/claude-haiku-"));
        assert!(is_claude_safe_model_id("  claude-sonnet-4-6  "));
        assert!(is_claude_safe_model_id("anthropic/claude-opus-4-8"));
    }
}
