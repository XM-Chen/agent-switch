//! 独立网关的纯路由领域。
//!
//! 本模块不得依赖 Tauri、SQLite、客户端路径或文件系统。它只负责精确模型解析、
//! 候选顺序、协议兼容过滤与可用性判断。

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IngressProtocol {
    AnthropicMessages,
    OpenAiChatCompletions,
    OpenAiResponses,
    OpenAiResponsesCompact,
    GeminiGenerateContent,
    GeminiStreamGenerateContent,
    GeminiCountTokens,
}

impl IngressProtocol {
    pub fn family(self) -> ProtocolFamily {
        match self {
            Self::AnthropicMessages => ProtocolFamily::Anthropic,
            Self::OpenAiChatCompletions | Self::OpenAiResponses | Self::OpenAiResponsesCompact => {
                ProtocolFamily::OpenAi
            }
            Self::GeminiGenerateContent
            | Self::GeminiStreamGenerateContent
            | Self::GeminiCountTokens => ProtocolFamily::Gemini,
        }
    }
}

impl fmt::Display for IngressProtocol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::AnthropicMessages => "anthropic_messages",
            Self::OpenAiChatCompletions => "openai_chat_completions",
            Self::OpenAiResponses => "openai_responses",
            Self::OpenAiResponsesCompact => "openai_responses_compact",
            Self::GeminiGenerateContent => "gemini_generate_content",
            Self::GeminiStreamGenerateContent => "gemini_stream_generate_content",
            Self::GeminiCountTokens => "gemini_count_tokens",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolFamily {
    Anthropic,
    OpenAi,
    Gemini,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayModelRoute {
    pub gateway_model_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteTarget {
    pub route_target_id: String,
    pub gateway_model_id: String,
    pub upstream_id: String,
    pub target_model: String,
    pub position: i64,
    pub upstream_protocol: UpstreamProtocol,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamProtocol {
    Anthropic,
    OpenAiChat,
    OpenAiResponses,
    Gemini,
}

impl UpstreamProtocol {
    pub fn can_serve(self, ingress: IngressProtocol) -> bool {
        match ingress.family() {
            // 现有 adapter 能在 Anthropic 与 OpenAI 两个家族之间双向转换。
            ProtocolFamily::Anthropic | ProtocolFamily::OpenAi => matches!(
                self,
                Self::Anthropic | Self::OpenAiChat | Self::OpenAiResponses
            ),
            // Gemini 原生 handler/adapter 只允许 Gemini 上游，避免无合同的静默转换。
            ProtocolFamily::Gemini => self == Self::Gemini,
        }
    }
}

impl FromStr for UpstreamProtocol {
    type Err = UnsupportedProtocol;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "anthropic" | "anthropic_messages" => Ok(Self::Anthropic),
            "openai" | "openai_chat" | "openai_chat_completions" => Ok(Self::OpenAiChat),
            "openai_responses" | "responses" => Ok(Self::OpenAiResponses),
            "gemini" | "gemini_native" => Ok(Self::Gemini),
            other => Err(UnsupportedProtocol(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedProtocol(pub String);

impl fmt::Display for UnsupportedProtocol {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "不支持的上游协议: {}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteResolutionError {
    ModelNotFound { requested_model: String },
    NoAvailableTarget { gateway_model_id: String },
}

impl fmt::Display for RouteResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelNotFound { requested_model } => {
                write!(formatter, "网关模型不存在: {requested_model}")
            }
            Self::NoAvailableTarget { gateway_model_id } => {
                write!(formatter, "网关模型没有可用路由候选: {gateway_model_id}")
            }
        }
    }
}

pub trait RouteCatalog {
    fn resolve_model_exact(&self, requested_model: &str) -> Option<GatewayModelRoute>;
    fn resolve_alias_exact(&self, alias: &str) -> Option<GatewayModelRoute>;
    fn list_targets(&self, gateway_model_id: &str) -> Vec<RouteTarget>;
}

pub trait TargetHealth {
    fn is_available(&self, route_target_id: &str) -> bool;
}

pub struct ModelRouter;

impl ModelRouter {
    pub fn resolve<C, H>(
        catalog: &C,
        health: &H,
        ingress: IngressProtocol,
        requested_model: &str,
    ) -> Result<(GatewayModelRoute, Vec<RouteTarget>), RouteResolutionError>
    where
        C: RouteCatalog,
        H: TargetHealth,
    {
        let model = catalog
            .resolve_model_exact(requested_model)
            .or_else(|| catalog.resolve_alias_exact(requested_model))
            .ok_or_else(|| RouteResolutionError::ModelNotFound {
                requested_model: requested_model.to_string(),
            })?;

        let mut targets = catalog.list_targets(&model.gateway_model_id);
        targets.sort_by(|left, right| {
            left.position
                .cmp(&right.position)
                .then_with(|| left.route_target_id.cmp(&right.route_target_id))
        });
        targets.retain(|target| {
            target.upstream_protocol.can_serve(ingress)
                && health.is_available(&target.route_target_id)
        });

        if targets.is_empty() {
            return Err(RouteResolutionError::NoAvailableTarget {
                gateway_model_id: model.gateway_model_id,
            });
        }

        Ok((model, targets))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;

    struct Catalog {
        models: HashMap<String, GatewayModelRoute>,
        aliases: HashMap<String, String>,
        targets: HashMap<String, Vec<RouteTarget>>,
    }

    impl RouteCatalog for Catalog {
        fn resolve_model_exact(&self, requested_model: &str) -> Option<GatewayModelRoute> {
            self.models.get(requested_model).cloned()
        }

        fn resolve_alias_exact(&self, alias: &str) -> Option<GatewayModelRoute> {
            self.aliases
                .get(alias)
                .and_then(|model| self.models.get(model))
                .cloned()
        }

        fn list_targets(&self, gateway_model_id: &str) -> Vec<RouteTarget> {
            self.targets
                .get(gateway_model_id)
                .cloned()
                .unwrap_or_default()
        }
    }

    struct Health(HashSet<String>);

    impl TargetHealth for Health {
        fn is_available(&self, route_target_id: &str) -> bool {
            self.0.contains(route_target_id)
        }
    }

    fn catalog() -> Catalog {
        let route = GatewayModelRoute {
            gateway_model_id: "gm-1".into(),
            model_id: "stable-model".into(),
        };
        Catalog {
            models: HashMap::from([(route.model_id.clone(), route)]),
            aliases: HashMap::from([("alias-model".into(), "stable-model".into())]),
            targets: HashMap::from([(
                "gm-1".into(),
                vec![
                    RouteTarget {
                        route_target_id: "target-late".into(),
                        gateway_model_id: "gm-1".into(),
                        upstream_id: "upstream-2".into(),
                        target_model: "vendor-model-b".into(),
                        position: 2,
                        upstream_protocol: UpstreamProtocol::OpenAiResponses,
                    },
                    RouteTarget {
                        route_target_id: "target-first".into(),
                        gateway_model_id: "gm-1".into(),
                        upstream_id: "upstream-1".into(),
                        target_model: "vendor-model-a".into(),
                        position: 1,
                        upstream_protocol: UpstreamProtocol::Anthropic,
                    },
                    RouteTarget {
                        route_target_id: "target-gemini".into(),
                        gateway_model_id: "gm-1".into(),
                        upstream_id: "upstream-3".into(),
                        target_model: "gemini-vendor-model".into(),
                        position: 0,
                        upstream_protocol: UpstreamProtocol::Gemini,
                    },
                ],
            )]),
        }
    }

    #[test]
    fn exact_model_wins_before_exact_alias_and_keeps_target_order() {
        let mut catalog = catalog();
        catalog
            .aliases
            .insert("stable-model".into(), "missing".into());
        let health = Health(HashSet::from([
            "target-first".into(),
            "target-late".into(),
            "target-gemini".into(),
        ]));

        let (model, targets) = ModelRouter::resolve(
            &catalog,
            &health,
            IngressProtocol::AnthropicMessages,
            "stable-model",
        )
        .expect("resolve exact model");

        assert_eq!(model.gateway_model_id, "gm-1");
        assert_eq!(
            targets
                .iter()
                .map(|target| target.route_target_id.as_str())
                .collect::<Vec<_>>(),
            vec!["target-first", "target-late"]
        );
    }

    #[test]
    fn exact_alias_resolves_without_normalization() {
        let catalog = catalog();
        let health = Health(HashSet::from(["target-first".into()]));
        assert!(ModelRouter::resolve(
            &catalog,
            &health,
            IngressProtocol::OpenAiChatCompletions,
            "alias-model",
        )
        .is_ok());
        assert_eq!(
            ModelRouter::resolve(
                &catalog,
                &health,
                IngressProtocol::OpenAiChatCompletions,
                "Alias-Model",
            ),
            Err(RouteResolutionError::ModelNotFound {
                requested_model: "Alias-Model".into()
            })
        );
    }

    #[test]
    fn protocol_and_target_health_filter_candidates() {
        let catalog = catalog();
        let health = Health(HashSet::from([
            "target-late".into(),
            "target-gemini".into(),
        ]));
        let (_, targets) = ModelRouter::resolve(
            &catalog,
            &health,
            IngressProtocol::OpenAiResponses,
            "stable-model",
        )
        .expect("resolve filtered targets");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].route_target_id, "target-late");
    }

    #[test]
    fn reports_model_not_found_and_no_available_target_separately() {
        let catalog = catalog();
        let no_health = Health(HashSet::new());
        assert_eq!(
            ModelRouter::resolve(
                &catalog,
                &no_health,
                IngressProtocol::AnthropicMessages,
                "missing",
            ),
            Err(RouteResolutionError::ModelNotFound {
                requested_model: "missing".into()
            })
        );
        assert_eq!(
            ModelRouter::resolve(
                &catalog,
                &no_health,
                IngressProtocol::AnthropicMessages,
                "stable-model",
            ),
            Err(RouteResolutionError::NoAvailableTarget {
                gateway_model_id: "gm-1".into()
            })
        );
    }
}
