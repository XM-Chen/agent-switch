/// 协议转换器注册表与 Translator trait。
///
/// 定义 `Translator` trait（四方法：key/translate_request/translate_response/translate_stream_line）
/// 与 `TranslatorRegistry` 注册表（register/get/resolve），以及 `StreamContext` 流式上下文。
/// 参考 design.md §3 协议转换器契约。
///
/// 使用方式：
/// ```ignore
/// let mut registry = TranslatorRegistry::new();
/// registry.register(Box::new(MyTranslator::new()));
/// let t = registry.resolve("anthropic", "openai-chat")?;
/// t.translate_request(&mut body, "gpt-4")?;
/// ```
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

mod anthropic_openai;
pub mod helpers;
mod native;
mod openai_responses;

pub use anthropic_openai::{AnthropicToChatTranslator, ChatToAnthropicTranslator};
pub use native::PassthroughTranslator;
pub use openai_responses::{ChatToResponsesTranslator, ResponsesToChatTranslator};

/// 流式转换上下文。
///
/// 在流式 SSE 行转换期间维护累积状态，如 tool call ID/name/arguments 的累积。
#[derive(Debug, Clone)]
pub struct StreamContext {
    /// 上游响应 ID
    pub response_id: String,
    /// 模型名称
    pub model: String,
    /// 创建时间戳（Unix 秒）
    pub created_at: i64,
    /// Tool call 累积器（index → ToolCallAcc）
    pub tool_calls: HashMap<i32, ToolCallAcc>,
    /// 是否已看到首块内容（用于判断是否已发送初始 delta）
    pub has_content: bool,
    /// 当前内容块索引（用于 Anthropic content_block 追踪）
    pub content_block_index: i32,
}

impl StreamContext {
    pub fn new(response_id: &str, model: &str, created_at: i64) -> Self {
        Self {
            response_id: response_id.to_string(),
            model: model.to_string(),
            created_at,
            tool_calls: HashMap::new(),
            has_content: false,
            content_block_index: 0,
        }
    }
}

/// Tool call 累积器（用于流式 response 中逐步构建 tool call 的 arguments）。
#[derive(Debug, Clone, Default)]
pub struct ToolCallAcc {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// 协议转换器 trait。
///
/// 转换器将请求/响应/流式行从一个协议格式转换为另一个。
/// 所有方法返回 `Result<_, String>`，不允许 panic。
pub trait Translator: Send + Sync {
    /// 返回该转换器支持的 `(protocol_from, protocol_to)` 标识。
    fn key(&self) -> (&'static str, &'static str);

    /// 转换请求 body（可变借用）。
    ///
    /// `model` 是已解析的上游模型名，用于写入目标协议的 model 字段。
    fn translate_request(&self, body: &mut Value, model: &str) -> Result<(), String>;

    /// 转换非流式响应 body（可变借用）。
    fn translate_response(&self, body: &mut Value) -> Result<(), String>;

    /// 转换一行 SSE 流数据。
    ///
    /// `line` 是一行原始 SSE 数据（可能为 `data: {...}`、`event: ...` 或空行）。
    /// `context` 提供累积的流上下文。
    /// 返回转换后的 SSE 行（包含 `\n` 或 `\n\n` 后缀），或空字符串以跳过该行。
    fn translate_stream_line(
        &self,
        line: &str,
        context: &mut StreamContext,
    ) -> Result<String, String>;
}

/// 转换器注册表。
///
/// 按 `(from, to)` 注册 `Box<dyn Translator>`，内部转为 `Arc<dyn Translator>` 以便在流式任务中共享。
/// `resolve` 返回 `Arc<dyn Translator>`，支持 Deref 自动解引用到 Translator。
pub struct TranslatorRegistry {
    translators: HashMap<(String, String), Arc<dyn Translator>>,
    passthrough: Arc<dyn Translator>,
}

impl TranslatorRegistry {
    /// 创建新注册表，并自动注册内置转换器和 Passthrough。
    pub fn new() -> Self {
        let mut registry = Self {
            translators: HashMap::new(),
            passthrough: Arc::new(PassthroughTranslator),
        };
        // 注册内置转换器
        registry.register(Box::new(AnthropicToChatTranslator));
        registry.register(Box::new(ChatToAnthropicTranslator));
        registry.register(Box::new(ChatToResponsesTranslator));
        registry.register(Box::new(ResponsesToChatTranslator));
        registry
    }

    /// 注册一个转换器（Box → Arc 内部转换）。
    pub fn register(&mut self, translator: Box<dyn Translator>) {
        let (from, to) = translator.key();
        self.translators
            .insert((from.to_string(), to.to_string()), translator.into());
    }

    /// 按方向精确查找转换器。
    pub fn get(&self, from: &str, to: &str) -> Option<Arc<dyn Translator>> {
        self.translators
            .get(&(from.to_string(), to.to_string()))
            .cloned()
    }

    /// 解析转换器，自动处理 Passthrough（from == to）。
    ///
    /// - 若 `from == to`，返回 Passthrough（native passthrough）。
    /// - 否则从注册表查找；未找到返回错误。
    pub fn resolve(&self, from: &str, to: &str) -> Result<Arc<dyn Translator>, String> {
        if from == to {
            return Ok(self.passthrough.clone());
        }
        self.get(from, to)
            .ok_or_else(|| format!("无可用转换器: {} → {}", from, to))
    }

    /// 返回当前注册的转换器数量（不含 Passthrough）。
    pub fn len(&self) -> usize {
        self.translators.len()
    }

    /// 注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.translators.is_empty()
    }
}

impl Default for TranslatorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_registry_new_not_empty() {
        let reg = TranslatorRegistry::new();
        assert!(!reg.is_empty());
        assert_eq!(reg.len(), 4);
    }

    #[test]
    fn test_registry_resolve_passthrough() {
        let reg = TranslatorRegistry::new();
        let t = reg.resolve("anthropic", "anthropic").unwrap();
        let mut body = json!({"model": "test"});
        t.translate_request(&mut body, "claude-sonnet-4").unwrap();
        assert_eq!(body["model"], "test");
    }

    #[test]
    fn test_registry_resolve_unknown() {
        let reg = TranslatorRegistry::new();
        assert!(reg.resolve("anthropic", "gemini").is_err());
    }

    #[test]
    fn test_registry_resolve_anthropic_to_chat() {
        let reg = TranslatorRegistry::new();
        assert!(reg.resolve("anthropic", "openai-chat").is_ok());
    }

    #[test]
    fn test_registry_resolve_chat_to_anthropic() {
        let reg = TranslatorRegistry::new();
        assert!(reg.resolve("openai-chat", "anthropic").is_ok());
    }

    #[test]
    fn test_registry_resolve_chat_to_responses() {
        let reg = TranslatorRegistry::new();
        assert!(reg.resolve("openai-chat", "openai-responses").is_ok());
    }

    #[test]
    fn test_registry_resolve_responses_to_chat() {
        let reg = TranslatorRegistry::new();
        assert!(reg.resolve("openai-responses", "openai-chat").is_ok());
    }

    #[test]
    fn test_passthrough_translate_response() {
        let reg = TranslatorRegistry::new();
        let t = reg.resolve("openai-chat", "openai-chat").unwrap();
        let mut body = json!({"id": "test", "choices": []});
        t.translate_response(&mut body).unwrap();
        assert_eq!(body["id"], "test");
    }

    #[test]
    fn test_passthrough_translate_stream_line() {
        let t = PassthroughTranslator;
        let mut ctx = StreamContext::new("msg_1", "model", 12345);
        let result = t
            .translate_stream_line("data: {\"key\":\"val\"}", &mut ctx)
            .unwrap();
        assert_eq!(result, "data: {\"key\":\"val\"}\n");
    }

    #[test]
    fn test_custom_translator() {
        struct Dummy;
        impl Translator for Dummy {
            fn key(&self) -> (&'static str, &'static str) {
                ("test-a", "test-b")
            }
            fn translate_request(&self, body: &mut Value, _model: &str) -> Result<(), String> {
                body["custom"] = json!(true);
                Ok(())
            }
            fn translate_response(&self, body: &mut Value) -> Result<(), String> {
                body["custom_resp"] = json!(true);
                Ok(())
            }
            fn translate_stream_line(
                &self,
                line: &str,
                _ctx: &mut StreamContext,
            ) -> Result<String, String> {
                Ok(line.to_string())
            }
        }

        let mut reg = TranslatorRegistry::new();
        reg.register(Box::new(Dummy));

        let t = reg.resolve("test-a", "test-b").unwrap();
        let mut body = json!({});
        t.translate_request(&mut body, "x").unwrap();
        assert_eq!(body["custom"], true);
    }
}
