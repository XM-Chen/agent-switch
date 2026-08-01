//! 网关上游凭据的领域语义。
//!
//! `credential_kind` 决定运行时如何把解密后的 payload 注入协议 adapter。历史 v17 的
//! `api_key` 仅保留兼容读取，不满足 v18 readiness；完成精确重分类后才能清除旧 provider 原文。

use serde_json::Value;

pub(crate) const LEGACY_API_KEY: &str = "api_key";
pub(crate) const BEARER_TOKEN: &str = "bearer_token";
pub(crate) const X_API_KEY: &str = "x_api_key";
pub(crate) const GOOGLE_API_KEY: &str = "google_api_key";
pub(crate) const GOOGLE_OAUTH: &str = "google_oauth";

pub(crate) const READY_CREDENTIAL_KINDS: &[&str] =
    &[BEARER_TOKEN, X_API_KEY, GOOGLE_API_KEY, GOOGLE_OAUTH];

pub(crate) fn is_supported_kind(kind: &str) -> bool {
    kind == LEGACY_API_KEY || READY_CREDENTIAL_KINDS.contains(&kind)
}

pub(crate) fn is_ready_kind(kind: &str) -> bool {
    READY_CREDENTIAL_KINDS.contains(&kind)
}

/// 凭据类型与上游协议/adapter 的静态兼容矩阵。
/// 这里只判断注入语义，不发起网络请求，也不读取任何客户端配置。
pub(crate) fn kind_can_serve(kind: &str, protocol: &str, adapter_type: &str) -> bool {
    match kind {
        BEARER_TOKEN => {
            matches!(
                (protocol, adapter_type),
                (
                    "anthropic" | "anthropic_messages",
                    "claude" | "module_anthropic"
                ) | (
                    "openai_chat" | "openai_responses",
                    "codex" | "module_openai"
                )
            )
        }
        X_API_KEY => matches!(
            (protocol, adapter_type),
            (
                "anthropic" | "anthropic_messages",
                "claude" | "module_anthropic"
            )
        ),
        GOOGLE_API_KEY | GOOGLE_OAUTH => matches!(
            (protocol, adapter_type),
            ("gemini" | "gemini_native", "gemini")
        ),
        _ => false,
    }
}

/// 验证解密后的 payload 是否符合 kind 语义。普通 key/token 只要求非空 UTF-8；
/// Google OAuth 必须含当前可用的 access token。仅有 refresh_token 的 JSON 暂不满足
/// readiness，因为独立网关尚未实现本机 refresh 闭环，不能把它当作 adapter-ready。
pub(crate) fn validate_payload(kind: &str, plaintext: &[u8]) -> bool {
    let Ok(secret) = std::str::from_utf8(plaintext) else {
        return false;
    };
    let secret = secret.trim();
    if secret.is_empty() {
        return false;
    }
    match kind {
        BEARER_TOKEN | X_API_KEY | GOOGLE_API_KEY | LEGACY_API_KEY => true,
        GOOGLE_OAUTH => google_oauth_access_token(secret).is_some(),
        _ => false,
    }
}

pub(crate) fn is_google_oauth_payload(secret: &str) -> bool {
    let secret = secret.trim();
    if secret.starts_with("ya29.") {
        return true;
    }
    let Ok(value) = serde_json::from_str::<Value>(secret) else {
        return false;
    };
    value
        .get("access_token")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        || value
            .get("refresh_token")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
}

/// 返回可直接注入 Google OAuth Bearer header 的当前 access token。
/// refresh-only JSON 仍被识别为 OAuth payload 以便迁移分类，但不会通过 readiness。
pub(crate) fn google_oauth_access_token(secret: &str) -> Option<String> {
    let secret = secret.trim();
    if secret.starts_with("ya29.") {
        return Some(secret.to_string());
    }
    let value = serde_json::from_str::<Value>(secret).ok()?;
    value
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 根据已提取的 secret 与 legacy settings 精确重建凭据注入语义。
/// 多候选由 gateway_migration 的 ambiguity 检查阻断；这里不猜测不匹配的字段。
pub(crate) fn classify_legacy_kind(
    app_type: &str,
    protocol: &str,
    adapter_type: &str,
    settings: &Value,
    secret: &str,
) -> Option<&'static str> {
    let secret = secret.trim();
    if secret.is_empty() {
        return None;
    }

    if app_type == "gemini" || matches!(protocol, "gemini" | "gemini_native") {
        return Some(if is_google_oauth_payload(secret) {
            GOOGLE_OAUTH
        } else {
            GOOGLE_API_KEY
        });
    }
    if app_type == "codex" || matches!(protocol, "openai_chat" | "openai_responses") {
        return Some(BEARER_TOKEN);
    }
    if matches!(adapter_type, "module_openai" | "module_anthropic") {
        return Some(BEARER_TOKEN);
    }

    let equals = |pointer: &str| {
        settings
            .pointer(pointer)
            .and_then(Value::as_str)
            .is_some_and(|value| value.trim() == secret)
    };
    if equals("/env/ANTHROPIC_AUTH_TOKEN")
        || equals("/env/OPENROUTER_API_KEY")
        || equals("/env/OPENAI_API_KEY")
    {
        return Some(BEARER_TOKEN);
    }
    if equals("/env/GOOGLE_API_KEY") || equals("/env/GEMINI_API_KEY") {
        return Some(if is_google_oauth_payload(secret) {
            GOOGLE_OAUTH
        } else {
            GOOGLE_API_KEY
        });
    }
    if equals("/env/ANTHROPIC_API_KEY")
        || equals("/apiKey")
        || equals("/api_key")
        || equals("/options/apiKey")
    {
        return Some(X_API_KEY);
    }

    // OpenClaw/Hermes 的扁平 key 由 module adapter 使用 Bearer 语义。
    if app_type == "openclaw" || app_type == "hermes" || app_type == "opencode" {
        return Some(BEARER_TOKEN);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_codex_toml_token_and_gemini_oauth() {
        assert_eq!(
            classify_legacy_kind(
                "codex",
                "openai_responses",
                "codex",
                &json!({"config": "experimental_bearer_token = \"secret\""}),
                "secret"
            ),
            Some(BEARER_TOKEN)
        );
        let oauth = r#"{"access_token":"ya29.token","refresh_token":"refresh"}"#;
        assert_eq!(
            classify_legacy_kind(
                "gemini",
                "gemini",
                "gemini",
                &json!({"env": {"GEMINI_API_KEY": oauth}}),
                oauth
            ),
            Some(GOOGLE_OAUTH)
        );
        assert!(validate_payload(GOOGLE_OAUTH, oauth.as_bytes()));
    }

    #[test]
    fn rejects_refresh_only_oauth_until_refresh_loop_exists() {
        let refresh_only =
            r#"{"refresh_token":"refresh","client_id":"id","client_secret":"secret"}"#;
        assert!(is_google_oauth_payload(refresh_only));
        assert_eq!(google_oauth_access_token(refresh_only), None);
        assert!(!validate_payload(GOOGLE_OAUTH, refresh_only.as_bytes()));
    }

    #[test]
    fn validates_adapter_credential_matrix() {
        assert!(kind_can_serve(BEARER_TOKEN, "openai_responses", "codex"));
        assert!(kind_can_serve(X_API_KEY, "anthropic", "claude"));
        assert!(kind_can_serve(GOOGLE_API_KEY, "gemini", "gemini"));
        assert!(!kind_can_serve(X_API_KEY, "openai_chat", "module_openai"));
        assert!(!kind_can_serve(GOOGLE_OAUTH, "anthropic", "claude"));
        assert!(!kind_can_serve(LEGACY_API_KEY, "anthropic", "claude"));
    }

    #[test]
    fn distinguishes_anthropic_bearer_and_x_api_key() {
        let settings = json!({
            "env": {
                "ANTHROPIC_AUTH_TOKEN": "bearer",
                "ANTHROPIC_API_KEY": "x-key"
            }
        });
        assert_eq!(
            classify_legacy_kind("claude", "anthropic", "claude", &settings, "bearer"),
            Some(BEARER_TOKEN)
        );
        assert_eq!(
            classify_legacy_kind("claude", "anthropic", "claude", &settings, "x-key"),
            Some(X_API_KEY)
        );
    }
}
