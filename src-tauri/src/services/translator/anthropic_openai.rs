/// Anthropic ↔ OpenAI Chat 协议转换器。
///
/// 实现两个转换器：
/// - `AnthropicToChatTranslator`：Anthropic Messages API → OpenAI Chat Completions API
/// - `ChatToAnthropicTranslator`：OpenAI Chat Completions API → Anthropic Messages API
///
/// 覆盖请求/响应/流式 SSE 行三套转换。参考 design.md §3 与 cpa 对应方向实现模式。
///
/// # 角色映射
/// | Anthropic | OpenAI Chat |
/// |-----------|-------------|
/// | `user` (含 tool_result) | `user` (含 tool_result) / `tool` |
/// | `assistant` (含 tool_use) | `assistant` (含 tool_calls) |
/// | `system` (顶层数组) | `messages[0].role=system` |
///
/// # 终止原因映射
/// | Anthropic stop_reason | Chat finish_reason |
/// |-----------------------|-------------------|
/// | `end_turn` | `stop` |
/// | `tool_use` | `tool_calls` |
/// | `max_tokens` | `length` |
/// | `stop_sequence` | `stop` |
use crate::services::translator::helpers;
use crate::services::translator::{StreamContext, ToolCallAcc, Translator};
use serde_json::{json, Value};

// =====================================================================
// Anthropic → OpenAI Chat
// =====================================================================

/// Anthropic Messages API 转 OpenAI Chat Completions API 转换器。
pub struct AnthropicToChatTranslator;

impl Translator for AnthropicToChatTranslator {
    fn key(&self) -> (&'static str, &'static str) {
        ("anthropic", "openai-chat")
    }

    /// 将 Anthropic Messages API 请求转换为 OpenAI Chat Completions API 格式。
    fn translate_request(&self, body: &mut Value, model: &str) -> Result<(), String> {
        // 1. 改写 model
        body["model"] = json!(model);

        // 2. 处理 system → messages[0]（system 消息）
        let system = body.get("system");
        let system_text = system.and_then(|s| helpers::extract_content_text(s));
        if let Some(ref sys_text) = system_text {
            // 在 messages 数组开头插入 system 消息
            let sys_msg = json!({"role": "system", "content": sys_text});
            if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
                messages.insert(0, sys_msg);
            }
            // 移除顶层 system 字段
            body.as_object_mut().map(|m| m.remove("system"));
        }

        // 3. 转换 messages 中的内容块
        if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
            for msg in messages.iter_mut() {
                let role = msg
                    .get("role")
                    .and_then(|r| r.as_str())
                    .unwrap_or("user")
                    .to_string();
                // 仅处理非 system 的消息
                if role == "system" {
                    continue;
                }

                let content_val = msg.get("content").cloned();
                if let Some(content) = content_val {
                    if content.is_array() {
                        let blocks = content.as_array().cloned().unwrap_or_default();
                        let mut text_parts: Vec<String> = Vec::new();
                        let mut has_tool_use = false;
                        let mut has_tool_result = false;

                        for block in &blocks {
                            match block.get("type").and_then(|t| t.as_str()) {
                                Some("text") => {
                                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                        text_parts.push(t.to_string());
                                    }
                                }
                                Some("tool_use") => {
                                    has_tool_use = true;
                                }
                                Some("tool_result") => {
                                    has_tool_result = true;
                                }
                                _ => {}
                            }
                        }

                        if role == "assistant" && has_tool_use {
                            // Assistant 消息：content 合并为字符串，tool_use 转为 tool_calls
                            let combined = text_parts.join("");
                            msg["content"] = json!(combined);

                            let mut tool_calls = Vec::new();
                            for block in &blocks {
                                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                                    let tc_id = block
                                        .get("id")
                                        .and_then(|i| i.as_str())
                                        .unwrap_or("toolu_xxx");
                                    let tc_name =
                                        block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                    let empty_obj = json!({});
                                    let tc_input = block.get("input").unwrap_or(&empty_obj);
                                    tool_calls.push(json!({
                                        "id": tc_id,
                                        "type": "function",
                                        "function": {
                                            "name": tc_name,
                                            "arguments": tc_input.to_string()
                                        }
                                    }));
                                }
                            }
                            if !tool_calls.is_empty() {
                                msg["tool_calls"] = json!(tool_calls);
                            }
                        } else if has_tool_result {
                            // tool_result → tool 角色
                            for block in &blocks {
                                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                                {
                                    let tool_use_id = block
                                        .get("tool_use_id")
                                        .and_then(|i| i.as_str())
                                        .unwrap_or("");
                                    let tool_content = block.get("content").cloned();
                                    msg["role"] = json!("tool");
                                    msg["tool_call_id"] = json!(tool_use_id);
                                    msg["content"] = tool_content.unwrap_or(json!(""));
                                }
                            }
                        } else {
                            // 普通消息：合并文本内容
                            msg["content"] = json!(text_parts.join(""));
                        }
                    }
                }
            }
        }

        // 4. 转换 thinking → reasoning_effort
        helpers::anthropic_thinking_to_reasoning_effort(body);

        // 5. 映射 stop_sequences → stop
        if let Some(stop_seq) = body.get("stop_sequences") {
            body["stop"] = stop_seq.clone();
            body.as_object_mut().map(|m| m.remove("stop_sequences"));
        }

        // 6. 转换 tools 格式
        if let Some(tools) = body.get_mut("tools").and_then(|t| t.as_array_mut()) {
            let openai_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    let name = t.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let desc = t.get("description").and_then(|d| d.as_str()).unwrap_or("");
                    let input_schema = t.get("input_schema").cloned().unwrap_or(json!({}));
                    json!({
                        "type": "function",
                        "function": {
                            "name": name,
                            "description": desc,
                            "parameters": input_schema
                        }
                    })
                })
                .collect();
            body["tools"] = json!(openai_tools);
        }

        // 7. 保留 stream 字段
        // （stream 字段在两种协议中同名，已存在于 body）

        // 8. 移除 Anthropic 专有字段
        let anthropic_only = ["metadata", "thinking", "adult_content", "output_config"];
        if let Some(obj) = body.as_object_mut() {
            for field in &anthropic_only {
                obj.remove(*field);
            }
        }

        Ok(())
    }

    /// 将 Anthropic Messages API 非流式响应转换为 OpenAI Chat Completions 格式。
    fn translate_response(&self, body: &mut Value) -> Result<(), String> {
        // 响应格式转换
        let msg_id = body
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("msg_unknown");

        let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");

        let usage = body.get("usage");
        let (prompt_tokens, completion_tokens, total_tokens) = if let Some(u) = usage {
            let input = u.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            let output = u.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            let cache_create = u
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let cache_read = u
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            (
                input + cache_create + cache_read,
                output,
                input + cache_create + cache_read + output,
            )
        } else {
            (0, 0, 0)
        };

        let stop_reason = body
            .get("stop_reason")
            .and_then(|s| s.as_str())
            .map(helpers::anthropic_stop_reason_to_chat)
            .unwrap_or("stop");

        // 转换 content 数组
        let content_blocks = body
            .get("content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        let mut content_text = String::new();
        let mut tool_calls = Vec::new();

        for block in &content_blocks {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                        content_text.push_str(t);
                    }
                }
                Some("tool_use") => {
                    let tc_id = block.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let tc_name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let empty_obj = json!({});
                    let tc_input = block.get("input").unwrap_or(&empty_obj);
                    tool_calls.push(json!({
                        "id": tc_id,
                        "type": "function",
                        "function": {
                            "name": tc_name,
                            "arguments": tc_input.to_string()
                        }
                    }));
                }
                _ => {}
            }
        }

        let mut choice = json!({
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content_text
            },
            "finish_reason": stop_reason
        });

        if !tool_calls.is_empty() {
            choice["message"]["tool_calls"] = json!(tool_calls);
        }

        *body = json!({
            "id": format!("chatcmpl_{}", &msg_id[..msg_id.len().min(28)]),
            "object": "chat.completion",
            "created": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            "model": model,
            "choices": [choice],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": total_tokens
            }
        });

        Ok(())
    }

    /// 将 Anthropic SSE 行转换为 OpenAI Chat SSE 行。
    fn translate_stream_line(
        &self,
        line: &str,
        context: &mut StreamContext,
    ) -> Result<String, String> {
        // 检查 event 类型
        if let Some(_event_name) = helpers::is_sse_event_type(line) {
            context.content_block_index = 0;
            context.has_content = false;
            return Ok("".to_string());
        }

        // 提取 data 负载
        let data = match helpers::extract_sse_data(line) {
            Some(d) => d,
            None => return Ok(line.to_string()),
        };

        // [DONE] 结束标记
        if data == "[DONE]" {
            return Ok("".to_string());
        }

        // 解析 JSON
        let parsed: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return Ok(line.to_string()),
        };

        let event_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            "message_start" => {
                // message_start → 首个 chat chunk（带 role delta）
                if let Some(message) = parsed.get("message") {
                    let msg_id = message
                        .get("id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("msg_unknown");
                    let model_name = message.get("model").and_then(|m| m.as_str()).unwrap_or("");
                    let usage = message.get("usage");
                    let (prompt, _out, total) = if let Some(u) = usage {
                        let inp = u.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let cache_c = u
                            .get("cache_creation_input_tokens")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        let cache_r = u
                            .get("cache_read_input_tokens")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        let out = u.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        (inp + cache_c + cache_r, out, inp + cache_c + cache_r + out)
                    } else {
                        (0, 0, 0)
                    };

                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);

                    context.response_id = format!("chatcmpl_{}", &msg_id[..msg_id.len().min(28)]);
                    context.model = model_name.to_string();
                    context.created_at = now;

                    let chunk = json!({
                        "id": context.response_id,
                        "object": "chat.completion.chunk",
                        "created": now,
                        "model": model_name,
                        "choices": [
                            {
                                "index": 0,
                                "delta": {
                                    "role": "assistant",
                                    "content": ""
                                },
                                "finish_reason": null
                            }
                        ],
                        "usage": {
                            "prompt_tokens": prompt,
                            "completion_tokens": 0,
                            "total_tokens": total
                        }
                    });
                    context.has_content = true;
                    Ok(format!("data: {}\n\n", chunk.to_string()))
                } else {
                    Ok("".to_string())
                }
            }

            "content_block_start" => {
                // content_block_start 可能是 text 或 tool_use
                let block = parsed.get("content_block");
                let block_type = block
                    .and_then(|b| b.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let index = parsed.get("index").and_then(|i| i.as_i64()).unwrap_or(0) as i32;

                match block_type {
                    "text" => {
                        let text = block
                            .and_then(|b| b.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        let chunk = json!({
                            "choices": [
                                {
                                    "index": 0,
                                    "delta": {
                                        "content": text
                                    },
                                    "finish_reason": null
                                }
                            ]
                        });
                        context.has_content = true;
                        Ok(format!("data: {}\n\n", chunk.to_string()))
                    }
                    "tool_use" => {
                        let tc_id = block
                            .and_then(|b| b.get("id"))
                            .and_then(|i| i.as_str())
                            .unwrap_or("");
                        let tc_name = block
                            .and_then(|b| b.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or("");

                        // 保存到累积器
                        context.tool_calls.insert(
                            index,
                            ToolCallAcc {
                                id: tc_id.to_string(),
                                name: tc_name.to_string(),
                                arguments: String::new(),
                            },
                        );

                        // 发送 tool_call delta
                        let chunk = json!({
                            "choices": [
                                {
                                    "index": 0,
                                    "delta": {
                                        "tool_calls": [
                                            {
                                                "index": index,
                                                "id": tc_id,
                                                "type": "function",
                                                "function": {
                                                    "name": tc_name,
                                                    "arguments": ""
                                                }
                                            }
                                        ]
                                    },
                                    "finish_reason": null
                                }
                            ]
                        });
                        context.has_content = true;
                        Ok(format!("data: {}\n\n", chunk.to_string()))
                    }
                    _ => Ok("".to_string()),
                }
            }

            "content_block_delta" => {
                let delta = parsed.get("delta");
                let delta_type = delta
                    .and_then(|d| d.get("type"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                let index = parsed.get("index").and_then(|i| i.as_i64()).unwrap_or(0) as i32;

                match delta_type {
                    "text_delta" => {
                        let text = delta
                            .and_then(|d| d.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        let chunk = json!({
                            "choices": [
                                {
                                    "index": 0,
                                    "delta": {
                                        "content": text
                                    },
                                    "finish_reason": null
                                }
                            ]
                        });
                        context.has_content = true;
                        Ok(format!("data: {}\n\n", chunk.to_string()))
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        // 累积到对应 tool call 的 arguments
                        if let Some(acc) = context.tool_calls.get_mut(&index) {
                            acc.arguments.push_str(partial);
                        }
                        let chunk = json!({
                            "choices": [
                                {
                                    "index": 0,
                                    "delta": {
                                        "tool_calls": [
                                            {
                                                "index": index,
                                                "id": null,
                                                "type": null,
                                                "function": {
                                                    "arguments": partial
                                                }
                                            }
                                        ]
                                    },
                                    "finish_reason": null
                                }
                            ]
                        });
                        Ok(format!("data: {}\n\n", chunk.to_string()))
                    }
                    _ => Ok("".to_string()),
                }
            }

            "content_block_stop" => {
                // content_block_stop 无对应 Chat 事件，跳过
                Ok("".to_string())
            }

            "message_delta" => {
                // message_delta → 最终 delta（finish_reason + 可能 usage）
                let delta = parsed.get("delta");
                let stop_reason = delta
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|s| s.as_str())
                    .map(helpers::anthropic_stop_reason_to_chat)
                    .unwrap_or("stop");

                let usage = parsed.get("usage");
                let completion_tokens = usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);

                let mut chunk = json!({
                    "choices": [
                        {
                            "index": 0,
                            "delta": {},
                            "finish_reason": stop_reason
                        }
                    ]
                });

                if completion_tokens > 0 {
                    chunk["usage"] = json!({
                        "completion_tokens": completion_tokens
                    });
                }

                Ok(format!("data: {}\n\n", chunk.to_string()))
            }

            "message_stop" => {
                // message_stop → [DONE]
                Ok("data: [DONE]\n\n".to_string())
            }

            "ping" => {
                // Anthropic 心跳 → 忽略
                Ok("".to_string())
            }

            "error" => {
                // 错误事件 → 传递为 Chat 错误
                let err_msg = parsed
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("upstream_error");
                Ok(helpers::build_error_event(err_msg, "openai-chat"))
            }

            _ => {
                // 未知事件，原样传递 data 行
                Ok(format!("data: {}\n", data))
            }
        }
    }
}

// =====================================================================
// OpenAI Chat → Anthropic
// =====================================================================

/// OpenAI Chat Completions API 转 Anthropic Messages API 转换器。
pub struct ChatToAnthropicTranslator;

impl Translator for ChatToAnthropicTranslator {
    fn key(&self) -> (&'static str, &'static str) {
        ("openai-chat", "anthropic")
    }

    /// 将 OpenAI Chat Completions API 请求转换为 Anthropic Messages API 格式。
    fn translate_request(&self, body: &mut Value, model: &str) -> Result<(), String> {
        // 1. 改写 model
        body["model"] = json!(model);

        // 2. 处理 system 消息 → 顶层 system 字段
        let mut system_parts: Vec<String> = Vec::new();
        if let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) {
            let mut i = 0;
            while i < messages.len() {
                let role = messages[i]
                    .get("role")
                    .and_then(|r| r.as_str())
                    .unwrap_or("");
                if role == "system" {
                    if let Some(text) = helpers::extract_content_text(&messages[i]["content"]) {
                        system_parts.push(text);
                    }
                    messages.remove(i);
                } else {
                    // 转换 messages 中的内容
                    Self::convert_message_content(&mut messages[i])?;
                    i += 1;
                }
            }
        }

        if !system_parts.is_empty() {
            body["system"] = json!(system_parts.join("\n"));
        }

        // 3. 转换 reasoning_effort → thinking
        helpers::reasoning_effort_to_anthropic_thinking(body);

        // 4. 映射 stop → stop_sequences
        if let Some(stop) = body.get("stop") {
            let stop_seq = if stop.is_array() {
                stop.clone()
            } else {
                json!([stop.as_str().unwrap_or("")])
            };
            body["stop_sequences"] = stop_seq;
            body.as_object_mut().map(|m| m.remove("stop"));
        }

        // 5. 转换 tools 格式
        if let Some(tools) = body.get_mut("tools").and_then(|t| t.as_array_mut()) {
            let anthropic_tools: Vec<Value> = tools
                .iter()
                .filter_map(|t| {
                    if t.get("type").and_then(|tt| tt.as_str()) == Some("function") {
                        let func = t.get("function")?;
                        let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let desc = func
                            .get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or("");
                        let params = func
                            .get("parameters")
                            .or_else(|| func.get("parametersJsonSchema"))
                            .cloned()
                            .unwrap_or(json!({}));
                        Some(json!({
                            "name": name,
                            "description": desc,
                            "input_schema": params
                        }))
                    } else {
                        None
                    }
                })
                .collect();
            if anthropic_tools.is_empty() {
                body.as_object_mut().map(|m| m.remove("tools"));
            } else {
                body["tools"] = json!(anthropic_tools);
            }
        }

        // 6. 移除 Chat 专有字段
        let chat_only = [
            "reasoning_effort",
            "user",
            "n",
            "logprobs",
            "top_logprobs",
            "max_completion_tokens",
            "presence_penalty",
            "frequency_penalty",
            "logit_bias",
            "seed",
        ];
        if let Some(obj) = body.as_object_mut() {
            for field in &chat_only {
                obj.remove(*field);
            }
        }

        Ok(())
    }

    /// 将 OpenAI Chat Completions 非流式响应转换为 Anthropic Messages API 格式。
    fn translate_response(&self, body: &mut Value) -> Result<(), String> {
        let chat_id = body
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("chatcmpl_unknown");

        let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");

        // 提取第一条 choice
        let choice = body
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .cloned()
            .unwrap_or_default();

        let message = choice.get("message");
        let finish_reason = choice
            .get("finish_reason")
            .and_then(|f| f.as_str())
            .map(helpers::chat_finish_reason_to_anthropic)
            .unwrap_or("end_turn");

        let mut content_blocks: Vec<Value> = Vec::new();

        // 转换 content
        if let Some(msg) = message {
            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                if !content.is_empty() {
                    content_blocks.push(json!({"type": "text", "text": content}));
                }
            }

            // 转换 tool_calls → tool_use
            if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    let tc_id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let tc_name = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let tc_args_str = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                        .unwrap_or("{}");
                    let tc_args: Value = serde_json::from_str(tc_args_str).unwrap_or(json!({}));
                    content_blocks.push(json!({
                        "type": "tool_use",
                        "id": tc_id,
                        "name": tc_name,
                        "input": tc_args
                    }));
                }
            }
        }

        // 生成消息 ID
        let msg_id = format!("msg_{}", &chat_id[9..].to_string());

        // 转换 usage
        let usage = body.get("usage");
        let (input_tokens, output_tokens) = if let Some(u) = usage {
            let prompt = u.get("prompt_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            let completion = u
                .get("completion_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            (prompt, completion)
        } else {
            (0, 0)
        };

        *body = json!({
            "id": msg_id,
            "type": "message",
            "role": "assistant",
            "content": content_blocks,
            "model": model,
            "stop_reason": finish_reason,
            "stop_sequence": null,
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0
            }
        });

        Ok(())
    }

    /// 将 OpenAI Chat SSE 行转换为 Anthropic SSE 行。
    fn translate_stream_line(
        &self,
        line: &str,
        context: &mut StreamContext,
    ) -> Result<String, String> {
        // 提取 data 负载
        let data = match helpers::extract_sse_data(line) {
            Some(d) => d,
            None => return Ok(line.to_string()),
        };

        // [DONE] → message_stop
        if data == "[DONE]" {
            return Ok("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n".to_string());
        }

        // 解析 JSON
        let parsed: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return Ok(line.to_string()),
        };

        let choices = parsed.get("choices").and_then(|c| c.as_array());
        let choice = choices.and_then(|arr| arr.first());

        if choice.is_none() {
            return Ok("".to_string());
        }
        let choice = choice.unwrap();

        let delta = choice.get("delta");
        let finish_reason = choice.get("finish_reason").and_then(|f| f.as_str());
        let content = delta
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str());
        let role = delta.and_then(|d| d.get("role")).and_then(|r| r.as_str());

        // 读取 usage（可能在最后一个 chunk 的顶层）
        let usage = parsed.get("usage");

        let mut output = String::new();

        // 首次 chunk（含 role delta → message_start）
        if role == Some("assistant") && !context.has_content {
            let msg_id = parsed
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("chatcmpl_unknown");
            let model_name = parsed.get("model").and_then(|m| m.as_str()).unwrap_or("");

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            let (inp, _out) = if let Some(u) = usage {
                let p = u.get("prompt_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                let c = u
                    .get("completion_tokens")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                (p, c)
            } else {
                (0, 0)
            };

            context.response_id = format!("msg_{}", &msg_id[9..msg_id.len().min(37)]);
            context.model = model_name.to_string();
            context.created_at = now;

            output.push_str(&format!(
                "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"{}\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"{}\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{{\"input_tokens\":{},\"output_tokens\":{}}}}}}}\n\n",
                context.response_id, model_name, inp, 0
            ));

            context.has_content = true;
        }

        // 处理 content delta → content_block_delta
        if let Some(text) = content {
            if !text.is_empty() {
                output.push_str(&format!(
                    "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{}\"}}}}\n\n",
                    text.replace('\\', "\\\\").replace('"', "\\\"")
                ));
            }
        }

        // 处理 tool_calls delta
        if let Some(tool_calls) = delta
            .and_then(|d| d.get("tool_calls"))
            .and_then(|t| t.as_array())
        {
            for tc in tool_calls {
                let tc_index = tc.get("index").and_then(|i| i.as_i64()).unwrap_or(0) as i32;
                let tc_id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                let tc_name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str());
                let tc_args = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str());

                let acc = context
                    .tool_calls
                    .entry(tc_index)
                    .or_insert_with(|| ToolCallAcc::default());

                if !tc_id.is_empty() && acc.id.is_empty() {
                    acc.id = tc_id.to_string();
                    // 发送 content_block_start for tool_use
                    output.push_str(&format!(
                        "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":{},\"content_block\":{{\"type\":\"tool_use\",\"id\":\"{}\",\"name\":\"{}\",\"input\":{{}}}}}}\n\n",
                        tc_index, tc_id, tc_name.unwrap_or("")
                    ));
                }

                if let Some(name) = tc_name {
                    if !name.is_empty() && acc.name.is_empty() {
                        acc.name = name.to_string();
                    }
                }

                if let Some(args) = tc_args {
                    if !args.is_empty() {
                        let escaped = args.replace('\\', "\\\\").replace('"', "\\\"");
                        output.push_str(&format!(
                            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":{},\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}}}\n\n",
                            tc_index, escaped
                        ));
                        acc.arguments.push_str(args);
                    }
                }
            }
        }

        // 处理 finish_reason → message_delta
        if let Some(reason) = finish_reason {
            let anthropic_reason = helpers::chat_finish_reason_to_anthropic(reason);

            let completion_tokens = usage
                .and_then(|u| u.get("completion_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            output.push_str(&format!(
                "event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"{}\",\"stop_sequence\":null}},\"usage\":{{\"output_tokens\":{}}}}}\n\n",
                anthropic_reason, completion_tokens
            ));
        }

        if output.is_empty() {
            Ok("".to_string())
        } else {
            Ok(output)
        }
    }
}

// =====================================================================
// ChatToAnthropicTranslator 辅助方法
// =====================================================================

impl ChatToAnthropicTranslator {
    /// 转换单条消息的 content 字段从 Chat 格式到 Anthropic 块格式。
    fn convert_message_content(msg: &mut Value) -> Result<(), String> {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

        match role {
            "assistant" => {
                // 处理 tool_calls → content 中的 tool_use 块
                let tool_calls = msg.get("tool_calls").and_then(|t| t.as_array()).cloned();
                let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");

                let mut blocks: Vec<Value> = Vec::new();

                // 文本内容
                if !content.is_empty() {
                    blocks.push(json!({"type": "text", "text": content}));
                }

                // tool_calls → tool_use
                if let Some(tcs) = tool_calls {
                    for tc in &tcs {
                        let tc_id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        let tc_name = tc
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(|n| n.as_str())
                            .unwrap_or("");
                        let tc_args_str = tc
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|a| a.as_str())
                            .unwrap_or("{}");
                        let tc_args: Value = serde_json::from_str(tc_args_str).unwrap_or(json!({}));
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": tc_id,
                            "name": tc_name,
                            "input": tc_args
                        }));
                    }
                }

                msg["content"] = json!(blocks);
                msg.as_object_mut().map(|m| m.remove("tool_calls"));
            }
            "tool" => {
                // tool 角色 → user 消息 + tool_result 内容块
                let tool_call_id = msg
                    .get("tool_call_id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string();
                let content = msg.get("content").cloned();
                msg["role"] = json!("user");
                msg["content"] = json!([{
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content.unwrap_or(json!(""))
                }]);
                msg.as_object_mut().map(|m| m.remove("tool_call_id"));
            }
            "user" => {
                // 处理多模态 content（如果 content 已经是 string 则保持）
                if let Some(content) = msg.get("content") {
                    if let Some(arr) = content.as_array() {
                        let anthropic_blocks: Vec<Value> = arr
                            .iter()
                            .map(|part| {
                                match part.get("type").and_then(|t| t.as_str()) {
                                    Some("text") => json!({"type": "text", "text": part.get("text").and_then(|t| t.as_str()).unwrap_or("")}),
                                    Some("image_url") => {
                                        let url = part.get("image_url").and_then(|u| u.get("url")).and_then(|u| u.as_str()).unwrap_or("");
                                        let (media_type, data) = if url.starts_with("data:") {
                                            let parts: Vec<&str> = url.splitn(2, ',').collect();
                                            let header = parts.first().unwrap_or(&"");
                                            let media = header.trim_start_matches("data:").split(';').next().unwrap_or("application/octet-stream");
                                            (media.to_string(), parts.get(1).unwrap_or(&"").to_string())
                                        } else {
                                            ("image/jpeg".to_string(), url.to_string())
                                        };
                                        json!({"type": "image", "source": {"type": if url.starts_with("data:") {"base64"} else {"url"}, "media_type": media_type, "data": data}})
                                    }
                                    _ => json!({"type": "text", "text": part.to_string()}),
                                }
                            })
                            .collect();
                        msg["content"] = json!(anthropic_blocks);
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =====================================================================
    // Anthropic → Chat 转换
    // =====================================================================

    #[test]
    fn test_anthropic_to_chat_request_basic() {
        let t = AnthropicToChatTranslator;
        let mut body = json!({
            "model": "claude-sonnet-4",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": "Hello"}
            ],
            "system": [{"type": "text", "text": "You are helpful"}],
            "stream": true
        });

        t.translate_request(&mut body, "claude-sonnet-4-20250514")
            .unwrap();

        // system 被移出顶层并作为第一条消息
        assert!(body.get("system").is_none());
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "You are helpful");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "Hello");
        assert_eq!(body["model"], "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_anthropic_to_chat_request_with_tool_use() {
        let t = AnthropicToChatTranslator;
        let mut body = json!({
            "model": "claude-3",
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "text", "text": "Let me check"},
                    {"type": "tool_use", "id": "toolu_abc", "name": "get_weather", "input": {"city": "NYC"}}
                ]}
            ]
        });

        t.translate_request(&mut body, "claude-3").unwrap();

        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["content"], "Let me check");
        assert_eq!(msg["tool_calls"][0]["id"], "toolu_abc");
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "get_weather");
    }

    #[test]
    fn test_anthropic_to_chat_request_with_tool_result() {
        let t = AnthropicToChatTranslator;
        let mut body = json!({
            "model": "claude-3",
            "messages": [
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_abc", "content": "{\"temp\": 72}"}
                ]}
            ]
        });

        t.translate_request(&mut body, "claude-3").unwrap();

        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "tool");
        assert_eq!(msg["tool_call_id"], "toolu_abc");
    }

    #[test]
    fn test_anthropic_to_chat_request_thinking() {
        let t = AnthropicToChatTranslator;
        let mut body = json!({
            "model": "claude-3",
            "messages": [{"role": "user", "content": "hi"}],
            "thinking": {"type": "enabled", "budget_tokens": 16000}
        });

        t.translate_request(&mut body, "claude-3").unwrap();
        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn test_anthropic_to_chat_response_basic() {
        let t = AnthropicToChatTranslator;
        let mut body = json!({
            "id": "msg_01",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello world"}],
            "model": "claude-3",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 20,
                "cache_creation_input_tokens": 5,
                "cache_read_input_tokens": 3
            }
        });

        t.translate_response(&mut body).unwrap();

        assert!(body["id"].as_str().unwrap().starts_with("chatcmpl_"));
        assert_eq!(body["object"], "chat.completion");
        assert_eq!(body["choices"][0]["message"]["content"], "Hello world");
        assert_eq!(body["choices"][0]["finish_reason"], "stop");
        assert!(body["usage"]["prompt_tokens"].as_i64().unwrap() >= 18); // 10+5+3
        assert_eq!(body["usage"]["completion_tokens"], 20);
        assert_eq!(body["model"], "claude-3");
    }

    #[test]
    fn test_anthropic_to_chat_response_with_tools() {
        let t = AnthropicToChatTranslator;
        let mut body = json!({
            "id": "msg_02",
            "content": [
                {"type": "text", "text": "Here is the weather"},
                {"type": "tool_use", "id": "toolu_1", "name": "get_weather", "input": {"city": "NYC"}}
            ],
            "model": "claude-3",
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 5, "output_tokens": 15}
        });

        t.translate_response(&mut body).unwrap();

        assert_eq!(
            body["choices"][0]["message"]["content"],
            "Here is the weather"
        );
        assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(
            body["choices"][0]["message"]["tool_calls"][0]["id"],
            "toolu_1"
        );
        assert_eq!(
            body["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "get_weather"
        );
    }

    #[test]
    fn test_anthropic_to_chat_stream_message_start() {
        let t = AnthropicToChatTranslator;
        let mut ctx = StreamContext::new("", "", 0);

        let line = "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-3\",\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}";
        let result = t.translate_stream_line(line, &mut ctx).unwrap();

        assert!(result.contains("object\":\"chat.completion.chunk"));
        assert!(result.contains("\"role\":\"assistant\""));
        assert!(ctx.has_content);
        assert!(ctx.response_id.starts_with("chatcmpl_"));
    }

    #[test]
    fn test_anthropic_to_chat_stream_text_delta() {
        let t = AnthropicToChatTranslator;
        let mut ctx = StreamContext::new("chatcmpl_01", "claude-3", 12345);

        let line = "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}";
        let result = t.translate_stream_line(line, &mut ctx).unwrap();

        assert!(result.contains("\"content\":\"Hello\""));
        assert!(result.contains("\"finish_reason\":null"));
    }

    #[test]
    fn test_anthropic_to_chat_stream_tool_call() {
        let t = AnthropicToChatTranslator;
        let mut ctx = StreamContext::new("chatcmpl_01", "claude-3", 12345);

        // tool_use content_block_start
        let line1 = "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"get_weather\",\"input\":{}}}";
        let result1 = t.translate_stream_line(line1, &mut ctx).unwrap();
        assert!(result1.contains("\"tool_calls\""));
        assert!(result1.contains("\"name\":\"get_weather\""));

        // input_json_delta
        let line2 = "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"NYC\\\"}\"}}";
        let result2 = t.translate_stream_line(line2, &mut ctx).unwrap();
        assert!(result2.contains("\"function\":{\"arguments\":\"{\\\"city\\\":\\\"NYC\\\"}\""));
    }

    #[test]
    fn test_anthropic_to_chat_stream_message_delta() {
        let t = AnthropicToChatTranslator;
        let mut ctx = StreamContext::new("chatcmpl_01", "claude-3", 12345);

        let line = "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\",\"stop_sequence\":null},\"usage\":{\"output_tokens\":25}}";
        let result = t.translate_stream_line(line, &mut ctx).unwrap();
        assert!(result.contains("\"finish_reason\":\"stop\""));
    }

    #[test]
    fn test_anthropic_to_chat_stream_message_stop() {
        let t = AnthropicToChatTranslator;
        let mut ctx = StreamContext::new("chatcmpl_01", "claude-3", 12345);

        let line = "data: {\"type\":\"message_stop\"}";
        let result = t.translate_stream_line(line, &mut ctx).unwrap();
        assert!(result.contains("[DONE]"));
    }

    // =====================================================================
    // Chat → Anthropic 转换
    // =====================================================================

    #[test]
    fn test_chat_to_anthropic_request_basic() {
        let t = ChatToAnthropicTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "Hello"}
            ],
            "stream": true
        });

        t.translate_request(&mut body, "claude-3").unwrap();

        // system 被移出 messages 到顶层
        assert_eq!(body["system"], "You are helpful");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Hello");
        assert_eq!(body["model"], "claude-3");
    }

    #[test]
    fn test_chat_to_anthropic_request_with_tools() {
        let t = ChatToAnthropicTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "user", "content": "Weather?"}
            ],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "description": "Get weather",
                        "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
                    }
                }
            ]
        });

        t.translate_request(&mut body, "claude-3").unwrap();

        assert!(body.get("tools").is_some());
        let tool = &body["tools"][0];
        assert_eq!(tool["name"], "get_weather");
        assert!(tool.get("input_schema").is_some());
    }

    #[test]
    fn test_chat_to_anthropic_request_with_tool_calls() {
        let t = ChatToAnthropicTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "assistant", "content": "Finding weather", "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"NYC\"}"}}
                ]}
            ]
        });

        t.translate_request(&mut body, "claude-3").unwrap();

        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "assistant");
        // Content 应该包含 text + tool_use 块
        let blocks = msg["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[1]["type"], "tool_use");
        assert_eq!(blocks[1]["id"], "call_1");
        assert!(msg.get("tool_calls").is_none());
    }

    #[test]
    fn test_chat_to_anthropic_request_tool_role() {
        let t = ChatToAnthropicTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "tool", "tool_call_id": "call_1", "content": "{\"temp\":72}"}
            ]
        });

        t.translate_request(&mut body, "claude-3").unwrap();

        let msg = &body["messages"][0];
        assert_eq!(msg["role"], "user");
        let blocks = msg["content"].as_array().unwrap();
        assert_eq!(blocks[0]["type"], "tool_result");
        assert_eq!(blocks[0]["tool_use_id"], "call_1");
    }

    #[test]
    fn test_chat_to_anthropic_response_basic() {
        let t = ChatToAnthropicTranslator;
        let mut body = json!({
            "id": "chatcmpl_abc123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "Hello world"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30
            }
        });

        t.translate_response(&mut body).unwrap();

        assert!(body["id"].as_str().unwrap().starts_with("msg_"));
        assert_eq!(body["type"], "message");
        assert_eq!(body["content"][0]["type"], "text");
        assert_eq!(body["content"][0]["text"], "Hello world");
        assert_eq!(body["stop_reason"], "end_turn");
        assert_eq!(body["usage"]["input_tokens"], 10);
        assert_eq!(body["usage"]["output_tokens"], 20);
    }

    #[test]
    fn test_chat_to_anthropic_response_with_tools() {
        let t = ChatToAnthropicTranslator;
        let mut body = json!({
            "id": "chatcmpl_xyz",
            "model": "gpt-4",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [
                            {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"NYC\"}"}}
                        ]
                    },
                    "finish_reason": "tool_calls"
                }
            ],
            "usage": {"prompt_tokens": 5, "completion_tokens": 10, "total_tokens": 15}
        });

        t.translate_response(&mut body).unwrap();

        let content = body["content"].as_array().unwrap();
        let tool_use = content
            .iter()
            .find(|b| b.get("type") == Some(&json!("tool_use")))
            .unwrap();
        assert_eq!(tool_use["id"], "call_1");
        assert_eq!(tool_use["name"], "get_weather");
        assert_eq!(body["stop_reason"], "tool_use");
    }

    #[test]
    fn test_chat_to_anthropic_stream_basic() {
        let t = ChatToAnthropicTranslator;
        let mut ctx = StreamContext::new("", "", 0);

        // 首个 chunk（role delta → message_start）
        let line1 = "data: {\"id\":\"chatcmpl_abc\",\"object\":\"chat.completion.chunk\",\"created\":12345,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}";
        let result1 = t.translate_stream_line(line1, &mut ctx).unwrap();
        assert!(result1.contains("event: message_start"));
        assert!(ctx.has_content);

        // content delta → content_block_delta
        let line2 = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}";
        let result2 = t.translate_stream_line(line2, &mut ctx).unwrap();
        assert!(result2.contains("event: content_block_delta"));
        assert!(result2.contains("\"text\":\"Hello\""));

        // finish_reason → message_delta
        let line3 = "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}";
        let result3 = t.translate_stream_line(line3, &mut ctx).unwrap();
        assert!(result3.contains("event: message_delta"));
        assert!(result3.contains("\"stop_reason\":\"end_turn\""));

        // [DONE] → message_stop
        let line4 = "data: [DONE]";
        let result4 = t.translate_stream_line(line4, &mut ctx).unwrap();
        assert!(result4.contains("event: message_stop"));
    }

    #[test]
    fn test_chat_to_anthropic_stream_reasoning_effort() {
        let t = ChatToAnthropicTranslator;
        let mut body = json!({
            "model": "o1",
            "messages": [{"role": "user", "content": "think"}],
            "reasoning_effort": "high"
        });

        t.translate_request(&mut body, "claude-3").unwrap();
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["output_config"]["effort"], "high");
    }

    #[test]
    fn test_full_roundtrip_basic() {
        // Anthropic → Chat → Anthropic（基本消息往返）
        let a2c = AnthropicToChatTranslator;
        let c2a = ChatToAnthropicTranslator;

        let mut an_body = json!({
            "model": "claude-sonnet-4",
            "messages": [
                {"role": "user", "content": "Hello"}
            ],
            "system": [{"type": "text", "text": "You are helpful"}],
            "max_tokens": 100
        });

        // Convert Anthropic → Chat
        a2c.translate_request(&mut an_body, "claude-sonnet-4-20250514")
            .unwrap();

        // Verify intermediate Chat format
        assert!(an_body.get("system").is_none());
        assert_eq!(an_body["messages"][0]["role"], "system");
        assert_eq!(an_body["messages"][1]["role"], "user");

        // Convert Chat → Anthropic
        c2a.translate_request(&mut an_body, "claude-sonnet-4-20250514")
            .unwrap();

        // Verify roundtrip
        assert_eq!(an_body["system"], "You are helpful");
        assert_eq!(an_body["messages"][0]["role"], "user");
        assert_eq!(an_body["messages"][0]["content"], "Hello");
        assert_eq!(an_body["model"], "claude-sonnet-4-20250514");
        assert_eq!(an_body["max_tokens"], 100);
    }
}
