//! OpenAI v1 子路径 → 模型能力映射工具。
//!
//! 将 `/v1/chat/completions`、`/v1/images/generations` 等路径
//! 映射为对应的模型能力标签，供 selector 和 model_mapper 做双重过滤。
//! 同时提供能力→协议类型解析，用于 v1 路由的协议类型推导。
#![allow(dead_code)]
use crate::http::proxy::constants;

/// 子路径 → 模型能力映射。
///
/// - `/v1/chat/completions` → `"chat"`
/// - `/v1/responses` → `"responses"`
/// - `/v1/embeddings` → `"embeddings"`
/// - `/v1/images/*`（及 `/v1/images/generations`、`/v1/images/edits`、`/v1/images/variations`）→ `"images"`
/// - `/v1/audio/*`（及 `/v1/audio/speech`、`/v1/audio/transcriptions`、`/v1/audio/translations`）→ `"audio"`
/// - `/v1/models` → `None`（不走代理管道）
pub fn path_to_capability(path: &str) -> Option<&'static str> {
    // 去掉前缀 /v1/
    let rel = path.strip_prefix("/v1/").unwrap_or(path);
    // 再去掉前导斜杠（兼容 /v1 不带斜杠的写法）
    let rel = rel.strip_prefix('/').unwrap_or(rel);

    // /v1/models → 不走管道
    if rel == "models" || rel.starts_with("models/") || rel.starts_with("models?") {
        return None;
    }

    if rel.starts_with("chat/") || rel == "chat" {
        Some("chat")
    } else if rel == "responses" || rel.starts_with("responses/") {
        Some("responses")
    } else if rel == "embeddings" || rel.starts_with("embeddings/") {
        Some("embeddings")
    } else if rel.starts_with("images/") || rel == "images" {
        Some("images")
    } else if rel.starts_with("audio/") || rel == "audio" {
        Some("audio")
    } else {
        // 未知子路径，回退为 chat（常见默认）
        Some("chat")
    }
}

/// 能力标签 → 对应的实际协议类型。
///
/// - `chat`、`embeddings`、`images`、`audio` → `openai-chat`
/// - `responses` → `openai-responses`
pub fn capability_to_protocol(cap: &str) -> Option<&'static str> {
    match cap {
        "chat" | "embeddings" | "images" | "audio" => Some(constants::PROTOCOL_OPENAI_CHAT),
        "responses" => Some(constants::PROTOCOL_OPENAI_RESPONSES),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_completions() {
        assert_eq!(path_to_capability("/v1/chat/completions"), Some("chat"));
    }

    #[test]
    fn test_responses() {
        assert_eq!(path_to_capability("/v1/responses"), Some("responses"));
    }

    #[test]
    fn test_embeddings() {
        assert_eq!(path_to_capability("/v1/embeddings"), Some("embeddings"));
    }

    #[test]
    fn test_images_generations() {
        assert_eq!(path_to_capability("/v1/images/generations"), Some("images"));
    }

    #[test]
    fn test_audio_speech() {
        assert_eq!(path_to_capability("/v1/audio/speech"), Some("audio"));
    }

    #[test]
    fn test_audio_transcriptions() {
        assert_eq!(
            path_to_capability("/v1/audio/transcriptions"),
            Some("audio")
        );
    }

    #[test]
    fn test_models_returns_none() {
        assert_eq!(path_to_capability("/v1/models"), None);
    }

    #[test]
    fn test_cap_to_protocol_chat() {
        assert_eq!(
            capability_to_protocol("chat"),
            Some(constants::PROTOCOL_OPENAI_CHAT)
        );
    }

    #[test]
    fn test_cap_to_protocol_responses() {
        assert_eq!(
            capability_to_protocol("responses"),
            Some(constants::PROTOCOL_OPENAI_RESPONSES)
        );
    }
}
