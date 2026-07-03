/// 协议转换器共享工具函数。
///
/// 提供角色映射、内容提取、SSE 行解析、错误事件合成等跨转换器通用能力。
/// 参考 design.md §3 协议转换器契约。
use serde_json::Value;

/// 从消息的 content 字段提取纯文本。
///
/// 处理两种形式：
/// - 直接字符串 `"content": "hello"`
/// - 内容块数组 `"content": [{"type":"text","text":"hello"}, ...]`
///
/// 返回全部文本块拼接后的文本内容，或 None。
pub fn extract_content_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(arr) => {
            let text = arr
                .iter()
                .filter_map(|item| {
                    if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                        item.get("text").and_then(|t| t.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    }
}

/// 构建协议相关的错误终端 SSE 事件。
///
/// - `"anthropic"`: 输出 `event: error\ndata: {"type":"error","error":{"type":"...","message":"..."}}\n\n`
/// - `"openai-chat"`: 输出携带错误消息的 chat chunk 后接 `[DONE]`
/// - `"openai-responses"`: 输出 `event: response.failed\ndata: {"type":"response.failed","error":...}\n\n`
pub fn build_error_event(msg: &str, protocol: &str) -> String {
    match protocol {
        "anthropic" => {
            let escaped =
                serde_json::to_string(msg).unwrap_or_else(|_| "\"upstream_error\"".to_string());
            format!(
                "event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"upstream_error\",\"message\":{}}}}}\n\n",
                escaped
            )
        }
        "openai-chat" => {
            let escaped =
                serde_json::to_string(msg).unwrap_or_else(|_| "\"upstream_error\"".to_string());
            format!(
                "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{}}},\"finish_reason\":\"error\"}}]}}\n\ndata: [DONE]\n\n",
                escaped
            )
        }
        "openai-responses" => {
            let escaped =
                serde_json::to_string(msg).unwrap_or_else(|_| "\"upstream_error\"".to_string());
            format!(
                "event: response.failed\ndata: {{\"type\":\"response.failed\",\"error\":{{\"message\":{}}}}}\n\n",
                escaped
            )
        }
        _ => "data: [DONE]\n\n".to_string(),
    }
}

/// 从 SSE `data: ...` 行中提取 JSON 正文。
///
/// 返回 `data:` 前缀之后的部分（trimmed）。
/// 若行不以 `data:` 开头，返回 None。
pub fn extract_sse_data(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if let Some(body) = trimmed.strip_prefix("data:") {
        Some(body.trim())
    } else {
        None
    }
}

/// 判断一行是否为 SSE `event:` 行。
pub fn is_sse_event_type(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if let Some(event_name) = trimmed.strip_prefix("event:") {
        Some(event_name.trim())
    } else {
        None
    }
}

/// 将 OpenAI Chat 的 finish_reason 映射到 Anthropic 的 stop_reason。
pub fn chat_finish_reason_to_anthropic(reason: &str) -> &'static str {
    match reason {
        "stop" => "end_turn",
        "tool_calls" => "tool_use",
        "length" => "max_tokens",
        "content_filter" => "content_filter",
        _ => "end_turn",
    }
}

/// 将 Anthropic 的 stop_reason 映射到 OpenAI Chat 的 finish_reason。
pub fn anthropic_stop_reason_to_chat(reason: &str) -> &'static str {
    match reason {
        "end_turn" => "stop",
        "tool_use" => "tool_calls",
        "max_tokens" => "length",
        "stop_sequence" => "stop",
        "content_filter" => "content_filter",
        _ => "stop",
    }
}

/// 判断 body 是否为流式请求（`stream` 字段为 true）。
pub fn is_streaming(body: &Value) -> bool {
    body.get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 将 Anthropic 的 `thinking` 配置转换为 OpenAI Chat 的 `reasoning_effort`。
pub fn anthropic_thinking_to_reasoning_effort(body: &mut Value) {
    if let Some(thinking) = body.get("thinking") {
        let effort = match thinking.get("type").and_then(|t| t.as_str()) {
            Some("enabled") => Some("high"),
            Some("disabled") => Some("none"),
            Some("adaptive") => thinking
                .get("output_config")
                .and_then(|oc| oc.get("effort"))
                .and_then(|e| e.as_str())
                .or(Some("medium")),
            _ => None,
        };
        if let Some(e) = effort {
            body["reasoning_effort"] = Value::String(e.to_string());
        }
        // 移除原始 thinking 字段
        body.as_object_mut().map(|m| m.remove("thinking"));
    }
}

/// 将 OpenAI Chat 的 `reasoning_effort` 转换为 Anthropic 的 `thinking`。
pub fn reasoning_effort_to_anthropic_thinking(body: &mut Value) {
    if let Some(effort) = body.get("reasoning_effort").and_then(|v| v.as_str()) {
        match effort {
            "none" => {
                body["thinking"] = serde_json::json!({"type": "disabled"});
            }
            "auto" => {
                body["thinking"] = serde_json::json!({"type": "adaptive"});
            }
            _ => {
                // low, medium, high → adaptive with effort
                body["thinking"] =
                    serde_json::json!({"type": "adaptive", "output_config": {"effort": effort}});
            }
        }
        body.as_object_mut().map(|m| m.remove("reasoning_effort"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_content_text_string() {
        let v = json!("hello");
        assert_eq!(extract_content_text(&v), Some("hello".to_string()));
    }

    #[test]
    fn test_extract_content_text_array() {
        let v = json!([
            {"type": "text", "text": "Hello"},
            {"type": "tool_use", "id": "t1", "name": "x", "input": {}},
            {"type": "text", "text": " world"}
        ]);
        assert_eq!(extract_content_text(&v), Some("Hello world".to_string()));
    }

    #[test]
    fn test_extract_content_text_empty() {
        let v = json!([{"type": "image", "source": {"type": "base64", "data": "..."}}]);
        assert!(extract_content_text(&v).is_none());
    }

    #[test]
    fn test_build_error_event_anthropic() {
        let event = build_error_event("test error", "anthropic");
        assert!(event.contains("event: error"));
        assert!(event.contains("upstream_error"));
        assert!(event.contains("test error"));
    }

    #[test]
    fn test_build_error_event_openai_chat() {
        let event = build_error_event("test", "openai-chat");
        assert!(event.contains("finish_reason"));
        assert!(event.contains("test"));
        assert!(event.contains("[DONE]"));
    }

    #[test]
    fn test_build_error_event_openai_responses() {
        let event = build_error_event("test", "openai-responses");
        assert!(event.contains("response.failed"));
    }

    #[test]
    fn test_extract_sse_data() {
        assert_eq!(
            extract_sse_data("data: {\"key\":\"val\"}"),
            Some("{\"key\":\"val\"}")
        );
        assert_eq!(extract_sse_data(" event: x"), None);
    }

    #[test]
    fn test_anthropic_stop_reason_mapping() {
        assert_eq!(anthropic_stop_reason_to_chat("end_turn"), "stop");
        assert_eq!(anthropic_stop_reason_to_chat("tool_use"), "tool_calls");
        assert_eq!(anthropic_stop_reason_to_chat("max_tokens"), "length");
    }

    #[test]
    fn test_chat_finish_reason_mapping() {
        assert_eq!(chat_finish_reason_to_anthropic("stop"), "end_turn");
        assert_eq!(chat_finish_reason_to_anthropic("tool_calls"), "tool_use");
    }

    #[test]
    fn test_is_streaming() {
        assert!(is_streaming(&json!({"stream": true})));
        assert!(!is_streaming(&json!({"stream": false})));
        assert!(!is_streaming(&json!({"model": "x"})));
    }

    #[test]
    fn test_thinking_conversion() {
        let mut body = json!({
            "model": "test",
            "thinking": {"type": "enabled", "budget_tokens": 16000}
        });
        anthropic_thinking_to_reasoning_effort(&mut body);
        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("thinking").is_none());

        let mut adaptive = json!({
            "model": "test",
            "thinking": {"type": "adaptive", "output_config": {"effort": "low"}}
        });
        anthropic_thinking_to_reasoning_effort(&mut adaptive);
        assert_eq!(adaptive["reasoning_effort"], "low");

        let mut adaptive_default = json!({
            "model": "test",
            "thinking": {"type": "adaptive"}
        });
        anthropic_thinking_to_reasoning_effort(&mut adaptive_default);
        assert_eq!(adaptive_default["reasoning_effort"], "medium");

        let mut body2 = json!({
            "model": "test",
            "reasoning_effort": "medium"
        });
        reasoning_effort_to_anthropic_thinking(&mut body2);
        assert_eq!(body2["thinking"]["type"], "adaptive");
        assert_eq!(body2["thinking"]["output_config"]["effort"], "medium");
        assert!(body2.get("reasoning_effort").is_none());
    }
}
