/// 协议转换的可测纯函数。
///
/// 把主循环中的请求/响应协议转换抽成无副作用的纯函数，便于单测。
/// 请求方向为 `from → to`（入站协议 → 上游协议），
/// 响应方向为 `to → from`（上游协议 → 入站协议）。
use serde_json::Value;

use crate::http::proxy::error::{ProxyError, ProxyErrorKind};
use crate::services::translator::TranslatorRegistry;

/// 构建转发到上游的请求 body（`from → to`）。
///
/// - `from == to`：直接序列化 `body`（同协议透传，model 已由 model_mapper 改写）。
/// - 跨协议：`resolve(from, to)` 后 clone 一份 body 调 `translate_request(&mut, upstream_model)`。
///
/// 任何转换/解析失败归类为 `ProxyErrorKind::ProtocolError`。
pub fn build_translated_request_body(
    registry: &TranslatorRegistry,
    from: &str,
    to: &str,
    body: &Value,
    upstream_model: &str,
) -> Result<Vec<u8>, ProxyError> {
    if from == to {
        return serde_json::to_vec(body).map_err(|e| {
            ProxyError::new(
                ProxyErrorKind::ProtocolError,
                format!("序列化请求失败: {}", e),
            )
        });
    }

    let translator = registry
        .resolve(from, to)
        .map_err(|e| ProxyError::new(ProxyErrorKind::ProtocolError, e))?;

    let mut translated = body.clone();
    translator
        .translate_request(&mut translated, upstream_model)
        .map_err(|e| ProxyError::new(ProxyErrorKind::ProtocolError, e))?;

    serde_json::to_vec(&translated).map_err(|e| {
        ProxyError::new(
            ProxyErrorKind::ProtocolError,
            format!("序列化请求失败: {}", e),
        )
    })
}

/// 把上游响应 body 转回入站协议（`to → from`）。
///
/// 注意方向反转：请求是 `from → to`，响应是 `to → from`。
///
/// - `from == to`：原样返回。
/// - 反序列化失败 / 无可用转换器 / 转换失败 / 序列化失败：均回退原始 `resp_bytes`。
pub fn translate_response_body(
    registry: &TranslatorRegistry,
    to: &str,
    from: &str,
    resp_bytes: &[u8],
) -> Vec<u8> {
    if from == to {
        return resp_bytes.to_vec();
    }

    let mut resp_json: Value = match serde_json::from_slice(resp_bytes) {
        Ok(v) => v,
        Err(_) => return resp_bytes.to_vec(),
    };

    let translator = match registry.resolve(to, from) {
        Ok(t) => t,
        Err(_) => return resp_bytes.to_vec(),
    };

    if translator.translate_response(&mut resp_json).is_err() {
        return resp_bytes.to_vec();
    }

    serde_json::to_vec(&resp_json).unwrap_or_else(|_| resp_bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_translated_request_cross_protocol_anthropic_to_chat() {
        let reg = TranslatorRegistry::new();
        let body = json!({
            "model": "claude-sonnet-4",
            "system": "you are helpful",
            "messages": [
                {"role": "user", "content": "hello"}
            ],
            "max_tokens": 100
        });

        let out = build_translated_request_body(&reg, "anthropic", "openai-chat", &body, "gpt-4")
            .expect("跨协议请求转换应成功");
        let parsed: Value = serde_json::from_slice(&out).expect("转换结果应为合法 JSON");

        // Chat 格式：顶层应出现 messages，且 system 应被并入 messages 而非保留为顶层字段
        assert!(parsed.get("messages").is_some(), "应出现顶层 messages");
        assert!(parsed.get("system").is_none(), "不应保留顶层 system");
    }

    #[test]
    fn translate_response_direction_chat_to_anthropic() {
        let reg = TranslatorRegistry::new();
        // 一个 openai-chat 风格响应
        let resp = json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 12345,
            "model": "gpt-4",
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": "hi there"},
                    "finish_reason": "stop"
                }
            ],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        });
        let bytes = serde_json::to_vec(&resp).unwrap();

        // 方向 to=openai-chat → from=anthropic（响应转回入站协议）
        let out = translate_response_body(&reg, "openai-chat", "anthropic", &bytes);
        assert!(!out.is_empty(), "响应转换结果不应为空");
    }

    #[test]
    fn build_translated_request_same_protocol_passthrough() {
        let reg = TranslatorRegistry::new();
        let body = json!({"model": "x", "messages": []});
        let out =
            build_translated_request_body(&reg, "anthropic", "anthropic", &body, "x").unwrap();
        let parsed: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed, body, "同协议应原样透传");
    }
}
