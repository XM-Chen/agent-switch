//! 旧 Codex Provider 原文的纯解析器。
//!
//! 仅解析 Agent Switch 自有数据库 `providers.settings_config` 中保存的历史 JSON/TOML
//! 原文，供 v17 provenance 迁移和现有 Codex adapter 使用。这里绝不解析路径、
//! 不读取或写入 `~/.codex`，也不探测 Codex 客户端是否安装。

use serde_json::Value;
use toml_edit::DocumentMut;

const CODEX_RESERVED_MODEL_PROVIDER_IDS: &[&str] = &[
    "amazon-bedrock",
    "openai",
    "ollama",
    "lmstudio",
    "oss",
    "ollama-chat",
];

fn active_model_provider_id(doc: &DocumentMut) -> Option<String> {
    doc.get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

fn is_custom_model_provider_id(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty()
        && !CODEX_RESERVED_MODEL_PROVIDER_IDS
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(id))
}

pub(crate) fn extract_auth_api_key(auth: &Value) -> Option<String> {
    auth.get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

pub(crate) fn extract_api_key(auth: Option<&Value>, config_text: Option<&str>) -> Option<String> {
    auth.and_then(extract_auth_api_key)
        .or_else(|| config_text.and_then(extract_experimental_bearer_token))
}

/// 从旧 Codex `config.toml` 原文提取当前激活上游的 base URL。
pub(crate) fn extract_base_url(config_text: &str) -> Option<String> {
    let doc = config_text.parse::<toml::Value>().ok()?;

    if let Some(active_provider) = doc.get("model_provider").and_then(toml::Value::as_str) {
        if let Some(base_url) = doc
            .get("model_providers")
            .and_then(|providers| providers.get(active_provider))
            .and_then(|provider| provider.get("base_url"))
            .and_then(toml::Value::as_str)
        {
            return Some(base_url.to_string());
        }
    }

    doc.get("base_url")
        .and_then(toml::Value::as_str)
        .map(ToString::to_string)
}

/// 从旧 Codex `config.toml` 原文提取 provider-scoped bearer token。
pub(crate) fn extract_experimental_bearer_token(config_text: &str) -> Option<String> {
    if !config_text.contains("experimental_bearer_token") {
        return None;
    }
    let doc = config_text.parse::<DocumentMut>().ok()?;
    let provider_id = active_model_provider_id(&doc);

    let top_level_token = || {
        doc.get("experimental_bearer_token")
            .and_then(|item| item.as_str())
    };
    let token = match provider_id.as_deref() {
        Some(id) if is_custom_model_provider_id(id) => doc
            .get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|table| table.get(id))
            .and_then(|item| item.as_table())
            .and_then(|table| table.get("experimental_bearer_token"))
            .and_then(|item| item.as_str())
            .or_else(top_level_token),
        Some(_) | None => top_level_token(),
    };

    token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_active_provider_values_without_file_io() {
        let config = r#"
model_provider = "relay"
[model_providers.relay]
base_url = "https://relay.example/v1"
experimental_bearer_token = "secret-token"
"#;
        assert_eq!(
            extract_base_url(config).as_deref(),
            Some("https://relay.example/v1")
        );
        assert_eq!(
            extract_experimental_bearer_token(config).as_deref(),
            Some("secret-token")
        );
        assert_eq!(
            extract_api_key(Some(&json!({"OPENAI_API_KEY": " auth-key "})), Some(config))
                .as_deref(),
            Some("auth-key")
        );
    }

    #[test]
    fn reserved_provider_uses_top_level_token() {
        let config = r#"
model_provider = "openai"
experimental_bearer_token = "top-level"
[model_providers.openai]
experimental_bearer_token = "ignored"
"#;
        assert_eq!(
            extract_experimental_bearer_token(config).as_deref(),
            Some("top-level")
        );
    }
}
