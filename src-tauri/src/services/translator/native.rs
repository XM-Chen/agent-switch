/// Native Passthrough 转换器。
///
/// 同协议直转：请求/响应/流式行均原样返回，不做任何格式转换。
/// 对应 design.md §3 中的 Passthrough 模式 - 零损失高保真直转，效仿 9router 的 Native Passthrough。
///
/// 模型字段由外部 `model_mapper` 改写，本转换器不再重复处理。
use crate::services::translator::{StreamContext, Translator};
use serde_json::Value;

/// Passthrough 转换器。
///
/// 所有方法均为透传：输入原样返回，不做任何格式修改。
/// `key()` 返回 `("", "")` 标记其为通用回退转换器。
/// 注册表在 `from == to` 时自动路由到本转换器。
pub struct PassthroughTranslator;

impl Translator for PassthroughTranslator {
    fn key(&self) -> (&'static str, &'static str) {
        ("", "")
    }

    /// 请求原样返回，不做转换。
    ///
    /// 模型名由外部 `model_mapper` 改写，此处仅验证 body 为合法 JSON。
    fn translate_request(&self, _body: &mut Value, _model: &str) -> Result<(), String> {
        Ok(())
    }

    /// 响应原样返回。
    fn translate_response(&self, _body: &mut Value) -> Result<(), String> {
        Ok(())
    }

    /// 流式行原样返回，追加 `\n`。
    fn translate_stream_line(
        &self,
        line: &str,
        _context: &mut StreamContext,
    ) -> Result<String, String> {
        Ok(format!("{}\n", line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::translator::StreamContext;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn test_passthrough_request() {
        let t = PassthroughTranslator;
        let mut body =
            json!({"model": "test-model", "messages": [{"role": "user", "content": "hi"}]});
        t.translate_request(&mut body, "test-model").unwrap();
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["content"], "hi");
    }

    #[test]
    fn test_passthrough_response() {
        let t = PassthroughTranslator;
        let mut body = json!({"id": "resp_1", "content": [{"type": "text", "text": "ok"}]});
        t.translate_response(&mut body).unwrap();
        assert_eq!(body["id"], "resp_1");
        assert_eq!(body["content"][0]["text"], "ok");
    }

    #[test]
    fn test_passthrough_stream_line() {
        let t = PassthroughTranslator;
        let mut ctx = StreamContext {
            response_id: "test".into(),
            model: "m".into(),
            tool_calls: HashMap::new(),
            has_content: false,
            text_block_index: None,
            next_content_block_index: 0,
            tool_call_to_block_index: HashMap::new(),
            block_to_tool_index: HashMap::new(),
        };
        let result = t
            .translate_stream_line("data: {\"key\":\"val\"}", &mut ctx)
            .unwrap();
        assert_eq!(result, "data: {\"key\":\"val\"}\n");
    }

    #[test]
    fn test_passthrough_key() {
        let t = PassthroughTranslator;
        assert_eq!(t.key(), ("", ""));
    }
}
