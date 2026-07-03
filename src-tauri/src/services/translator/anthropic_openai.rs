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
use crate::services::translator::{StreamContext, Translator};
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
        let system_text = system.and_then(helpers::extract_content_text);
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
            let mut i = 0;
            while i < messages.len() {
                let role = messages[i]
                    .get("role")
                    .and_then(|r| r.as_str())
                    .unwrap_or("user")
                    .to_string();
                // 仅处理非 system 的消息
                if role == "system" {
                    i += 1;
                    continue;
                }

                let content_val = messages[i].get("content").cloned();
                if let Some(content) = content_val {
                    if content.is_array() {
                        let blocks = content.as_array().cloned().unwrap_or_default();
                        let mut text_parts: Vec<String> = Vec::new();
                        let mut has_tool_use = false;
                        let tool_result_count = blocks
                            .iter()
                            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
                            .count();
                        let has_tool_result = tool_result_count > 0;

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
                                _ => {}
                            }
                        }

                        let msg = &mut messages[i];

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
                            // tool_result → 每个块成为一条独立的 role:"tool" 消息。
                            // 原 user 消息可能含多个 tool_result，必须拆成多条，
                            // 否则在原 msg 上原地改 role 会互相覆盖、丢失前面的结果。
                            let mut tool_msgs: Vec<Value> = Vec::new();
                            for block in &blocks {
                                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                                {
                                    let tool_use_id = block
                                        .get("tool_use_id")
                                        .and_then(|i| i.as_str())
                                        .unwrap_or("");
                                    let tool_content = block.get("content").cloned();
                                    tool_msgs.push(json!({
                                        "role": "tool",
                                        "tool_call_id": tool_use_id,
                                        "content": tool_content.unwrap_or(json!(""))
                                    }));
                                }
                            }
                            if tool_msgs.is_empty() {
                                // 退化：无 tool_result 块（不应发生），保留原消息
                                msg["content"] = json!(text_parts.join(""));
                            } else {
                                // 用拆分出的消息替换原位置（1 条变 N 条）
                                let mut new_msgs: Vec<Value> = Vec::new();
                                // 保留非 tool_result 的文本部分作为 user 消息（如果有）
                                let extra_text = text_parts.join("");
                                if !extra_text.is_empty() {
                                    new_msgs.push(json!({"role": "user", "content": extra_text}));
                                }
                                new_msgs.extend(tool_msgs);
                                // 替换
                                let splice: Vec<Value> = new_msgs;
                                let len = splice.len();
                                messages.splice(i..=i, splice);
                                i += len;
                                continue; // 已移动游标，跳过末尾的 i+=1
                            }
                        } else {
                            // 普通消息：合并文本内容
                            msg["content"] = json!(text_parts.join(""));
                        }
                    }
                }
                i += 1;
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
        // 检查 event 类型行（"event: xxx"）：仅作分隔符，不重置累积状态。
        // 此前这里重置 content_block_index=0 / has_content=false 会破坏
        // block_to_tool_index 映射和 has_content 的"首次"语义（Anthropic 每个
        // content_block 前都会发 event 行，反复重置会导致状态错乱）。
        if helpers::is_sse_event_type(line).is_some() {
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
                    Ok(format!("data: {}\n\n", chunk))
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
                        Ok(format!("data: {}\n\n", chunk))
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

                        // 将 Anthropic content_block index 映射为 OpenAI tool_call index。
                        // Anthropic 的 index 是所有 content block 的全局序号（text+tool_use 混排），
                        // 而 OpenAI 的 tool_calls[].index 只计工具调用，从 0 开始。
                        let tool_index = context.block_to_tool_index.len() as i32;
                        context
                            .block_to_tool_index
                            .insert(index, tool_index);

                        // 发送 tool_call delta
                        let chunk = json!({
                            "choices": [
                                {
                                    "index": 0,
                                    "delta": {
                                        "tool_calls": [
                                            {
                                                "index": tool_index,
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
                        Ok(format!("data: {}\n\n", chunk))
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
                        Ok(format!("data: {}\n\n", chunk))
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        // 已在 content_block_start 记录 index 映射；缺失映射时丢弃畸形 delta，避免错误归到 tool 0。
                        let Some(tool_index) = context.block_to_tool_index.get(&index).copied() else {
                            return Ok("".to_string());
                        };
                        let chunk = json!({
                            "choices": [
                                {
                                    "index": 0,
                                    "delta": {
                                        "tool_calls": [
                                            {
                                                "index": tool_index,
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
                        Ok(format!("data: {}\n\n", chunk))
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

                Ok(format!("data: {}\n\n", chunk))
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

        // 6. 映射 max_completion_tokens → max_tokens（Anthropic 请求必需/常用字段）
        if let Some(obj) = body.as_object_mut() {
            if let Some(max_completion_tokens) = obj.remove("max_completion_tokens") {
                obj.entry("max_tokens").or_insert(max_completion_tokens);
            }
        }

        // 7. 移除 Chat 专有字段
        let chat_only = [
            "reasoning_effort",
            "user",
            "n",
            "logprobs",
            "top_logprobs",
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

        // 生成消息 ID（去掉 "chatcmpl_" 前缀，若格式不符则原样使用，避免越界切片）
        let msg_id = if chat_id.starts_with("chatcmpl_") && chat_id.len() > 9 {
            format!("msg_{}", &chat_id[9..])
        } else {
            format!("msg_{}", chat_id)
        };

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

        // [DONE] → close remaining content blocks, then message_stop
        if data == "[DONE]" {
            let mut output = String::new();
            Self::close_open_blocks(context, &mut output);
            output.push_str("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
            return Ok(output);
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

            // 去掉 "chatcmpl_" 前缀（若格式不符则原样使用，避免越界切片 panic）
            context.response_id = if msg_id.starts_with("chatcmpl_") && msg_id.len() > 9 {
                format!("msg_{}", &msg_id[9..msg_id.len().min(37)])
            } else {
                format!("msg_{}", msg_id)
            };
            context.model = model_name.to_string();
            context.created_at = now;

            output.push_str(&format!(
                "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"{}\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"{}\",\"stop_reason\":null,\"stop_sequence\":null,\"usage\":{{\"input_tokens\":{},\"output_tokens\":{}}}}}}}\n\n",
                context.response_id, model_name, inp, 0
            ));

            context.has_content = true;
        }

        // 处理 content delta → content_block_start + content_block_delta
        if let Some(text) = content {
            if !text.is_empty() {
                let text_block_index = match context.text_block_index {
                    Some(index) => index,
                    None => {
                        let index = context.next_content_block_index;
                        context.next_content_block_index += 1;
                        context.text_block_index = Some(index);
                        output.push_str(&format!(
                            "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":{},\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n",
                            index
                        ));
                        index
                    }
                };
                // 用 serde_json 正确转义，避免手动 replace 漏处理 \n/\r/\t/控制字符。
                let escaped = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
                output.push_str(&format!(
                    "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":{},\"delta\":{{\"type\":\"text_delta\",\"text\":{}}}}}\n\n",
                    text_block_index, escaped
                ));
            }
        }

        // 处理 tool_calls delta
        if let Some(tool_calls) = delta
            .and_then(|d| d.get("tool_calls"))
            .and_then(|t| t.as_array())
        {
            // Anthropic content_block 是全局有序块；开始 tool_use 前先闭合已打开的 text 块。
            if let Some(text_idx) = context.text_block_index {
                output.push_str(&format!(
                    "event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":{}}}\n\n",
                    text_idx
                ));
                context.text_block_index = None;
            }

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

                let acc = context.tool_calls.entry(tc_index).or_default();
                let block_index = context
                    .tool_call_to_block_index
                    .get(&tc_index)
                    .copied()
                    .unwrap_or_else(|| {
                        let index = context.next_content_block_index;
                        context.next_content_block_index += 1;
                        context.tool_call_to_block_index.insert(tc_index, index);
                        context.block_to_tool_index.insert(index, tc_index);
                        index
                    });

                if !tc_id.is_empty() && acc.id.is_empty() {
                    acc.id = tc_id.to_string();
                    // 发送 content_block_start for tool_use
                    output.push_str(&format!(
                        "event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":{},\"content_block\":{{\"type\":\"tool_use\",\"id\":\"{}\",\"name\":\"{}\",\"input\":{{}}}}}}\n\n",
                        block_index, tc_id, tc_name.unwrap_or("")
                    ));
                }

                if let Some(name) = tc_name {
                    if !name.is_empty() && acc.name.is_empty() {
                        acc.name = name.to_string();
                    }
                }

                if let Some(args) = tc_args {
                    if !args.is_empty() && !acc.id.is_empty() {
                        // 用 serde_json 正确转义。args 本身是部分 JSON 字符串，
                        // 塞进 "partial_json":"..." 字段必须做 JSON 字符串转义，
                        // 但不能对 args 内部再做反斜杠转义（那会损坏 JSON）。
                        let escaped = serde_json::to_string(args).unwrap_or_else(|_| "\"\"".to_string());
                        output.push_str(&format!(
                            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":{},\"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":{}}}}}\n\n",
                            block_index, escaped
                        ));
                        acc.arguments.push_str(args);
                    }
                }
            }
        }

        // 处理 finish_reason → close open blocks, then message_delta
        if let Some(reason) = finish_reason {
            Self::close_open_blocks(context, &mut output);
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
    /// 为所有打开的 content_block 发送 content_block_stop 事件。
    ///
    /// 在流结束时（finish_reason 或 [DONE]）调用，确保每个已打开的块都正确闭合。
    /// - text 块：若 text_block_index 不为 None，发送对应 index 的 stop。
    /// - tool_use 块：遍历 tool_call_to_block_index，为每个已打开的 tool_call 发送 stop。
    fn close_open_blocks(context: &mut StreamContext, output: &mut String) {
        // 关闭 text 块
        if let Some(text_idx) = context.text_block_index {
            output.push_str(&format!(
                "event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":{}}}\n\n",
                text_idx
            ));
            context.text_block_index = None;
        }

        // 关闭所有已打开的 tool_use 块。按 Anthropic block index 排序，避免 HashMap 迭代顺序导致测试/输出抖动。
        let mut open_tool_blocks: Vec<(i32, i32)> = context
            .tool_call_to_block_index
            .iter()
            .filter_map(|(tc_index, block_index)| {
                context
                    .tool_calls
                    .get(tc_index)
                    .filter(|acc| !acc.id.is_empty())
                    .map(|_| (*tc_index, *block_index))
            })
            .collect();
        open_tool_blocks.sort_by_key(|(_, block_index)| *block_index);
        for (_, block_index) in open_tool_blocks {
            output.push_str(&format!(
                "event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":{}}}\n\n",
                block_index
            ));
        }
        context.tool_call_to_block_index.clear();
    }

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

    /// 回归测试 T2：同一条 user 消息含多个 tool_result 必须拆成多条独立 tool 消息，
    /// 而非在原消息上原地覆盖（否则只保留最后一个）。
    #[test]
    fn test_anthropic_to_chat_request_with_multiple_tool_results() {
        let t = AnthropicToChatTranslator;
        let mut body = json!({
            "model": "claude-3",
            "messages": [
                {"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_1", "content": "result1"},
                    {"type": "tool_result", "tool_use_id": "toolu_2", "content": "result2"}
                ]}
            ]
        });

        t.translate_request(&mut body, "claude-3").unwrap();

        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2, "应拆成 2 条独立 tool 消息");
        assert_eq!(msgs[0]["role"], "tool");
        assert_eq!(msgs[0]["tool_call_id"], "toolu_1");
        assert_eq!(msgs[0]["content"], "result1");
        assert_eq!(msgs[1]["role"], "tool");
        assert_eq!(msgs[1]["tool_call_id"], "toolu_2");
        assert_eq!(msgs[1]["content"], "result2");
    }

    /// 回归测试 T4：流式 content delta 含换行/引号时，转换后的 SSE data 必须是合法 JSON。
    #[test]
    fn test_chat_to_anthropic_stream_text_with_special_chars() {
        let t = ChatToAnthropicTranslator;
        let mut ctx = StreamContext::new("chatcmpl_abc", "gpt-4", 0);
        // 先发首块建立 message_start
        let _ = t.translate_stream_line(
            "data: {\"id\":\"chatcmpl_abc\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        // 含换行、引号、反斜杠的文本
        let out = t.translate_stream_line(
            "data: {\"id\":\"chatcmpl_abc\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"line1\\nline2 \\\"quoted\\\" \\\\path\"},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        // 提取 content_block_delta 的 data 行负载并解析为 JSON，验证 text 字段正确还原
        assert!(out.contains("content_block_start"));
        assert!(out.contains("content_block_delta"));
        let payload = out
            .split("\n\n")
            .filter_map(|event| event.lines().find(|l| l.starts_with("data: ")))
            .map(|line| line.trim_start_matches("data: "))
            .find(|payload| payload.contains("text_delta"))
            .unwrap();
        let v: Value = serde_json::from_str(payload).expect("SSE data 必须是合法 JSON");
        assert_eq!(v["delta"]["text"], "line1\nline2 \"quoted\" \\path");
    }

    /// 回归测试 T1：流式 tool_call arguments delta 必须正确转义，
    /// 客户端解析 partial_json 后应得到原始 JSON 片段（无多余反斜杠）。
    #[test]
    fn test_chat_to_anthropic_stream_tool_args_not_double_escaped() {
        let t = ChatToAnthropicTranslator;
        let mut ctx = StreamContext::new("chatcmpl_abc", "gpt-4", 0);
        let _ = t.translate_stream_line(
            "data: {\"id\":\"chatcmpl_abc\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        // tool_call 参数 delta，含 JSON 字符串值
        let out = t.translate_stream_line(
            "data: {\"id\":\"chatcmpl_abc\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"city\\\":\\\"NYC\\\"}\"}}]},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let data_line = out.lines().rev().find(|l| l.starts_with("data: ")).unwrap();
        let payload = data_line.trim_start_matches("data: ");
        let v: Value = serde_json::from_str(payload).expect("SSE data 必须是合法 JSON");
        // partial_json 应是原始 JSON 片段，不含双重转义
        let partial = v["delta"]["partial_json"].as_str().unwrap();
        assert_eq!(partial, "{\"city\":\"NYC\"}");
    }

    /// 回归测试 T5：非标准短 id 不应 panic。
    #[test]
    fn test_chat_to_anthropic_response_short_id_no_panic() {
        let t = ChatToAnthropicTranslator;
        let mut body = json!({
            "id": "abc",
            "model": "gpt-4",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}]
        });
        t.translate_response(&mut body).unwrap();
        assert!(body["id"].as_str().unwrap().starts_with("msg_"));
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
    fn test_chat_to_anthropic_request_maps_max_completion_tokens() {
        let t = ChatToAnthropicTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "Hello"}],
            "max_completion_tokens": 256
        });

        t.translate_request(&mut body, "claude-3").unwrap();
        assert_eq!(body["max_tokens"], 256);
        assert!(body.get("max_completion_tokens").is_none());
    }

    #[test]
    fn test_chat_to_anthropic_stream_text_block_start_and_stop() {
        let t = ChatToAnthropicTranslator;
        let mut ctx = StreamContext::new("", "", 0);

        let _ = t.translate_stream_line(
            "data: {\"id\":\"chatcmpl_abc\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let delta_out = t.translate_stream_line(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let finish_out = t.translate_stream_line(
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}",
            &mut ctx,
        ).unwrap();
        let done_out = t.translate_stream_line("data: [DONE]", &mut ctx).unwrap();

        assert!(delta_out.contains("event: content_block_start"));
        assert!(delta_out.contains("\"type\":\"text\""));
        assert!(delta_out.contains("event: content_block_delta"));
        assert!(finish_out.contains("event: content_block_stop"));
        assert!(finish_out.contains("event: message_delta"));
        assert!(!done_out.contains("content_block_stop"), "finish_reason 后不应重复 stop");
        assert!(done_out.contains("event: message_stop"));
    }

    #[test]
    fn test_chat_to_anthropic_stream_mixed_text_and_tool_blocks_are_closed_in_order() {
        let t = ChatToAnthropicTranslator;
        let mut ctx = StreamContext::new("", "", 0);

        let _ = t.translate_stream_line(
            "data: {\"id\":\"chatcmpl_abc\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let text_out = t.translate_stream_line(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let tool_out = t.translate_stream_line(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"city\\\":\\\"NYC\\\"}\"}}]},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let finish_out = t.translate_stream_line(
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}",
            &mut ctx,
        ).unwrap();

        assert!(text_out.contains("\"index\":0"));
        assert!(tool_out.contains("content_block_stop"), "tool 开始前应先闭合 text 块");
        assert!(tool_out.contains("\"index\":1"), "tool_use 应拿到全局 block index 1");
        let stop_positions: Vec<_> = finish_out.match_indices("event: content_block_stop").collect();
        assert_eq!(stop_positions.len(), 1, "finish 时只应剩 tool_use 块待关闭");
        assert!(finish_out.contains("\"index\":1"));
        assert!(finish_out.contains("event: message_delta"));
    }

    #[test]
    fn test_anthropic_to_chat_request_combines_all_system_text_blocks() {
        let t = AnthropicToChatTranslator;
        let mut body = json!({
            "model": "claude-sonnet-4",
            "messages": [
                {"role": "user", "content": "Hello"}
            ],
            "system": [
                {"type": "text", "text": "Line 1. "},
                {"type": "tool_use", "id": "ignored", "name": "noop", "input": {}},
                {"type": "text", "text": "Line 2."}
            ]
        });

        t.translate_request(&mut body, "claude-sonnet-4").unwrap();
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "Line 1. Line 2.");
    }

    #[test]
    fn test_anthropic_to_chat_stream_ignores_unmapped_input_json_delta() {
        let t = AnthropicToChatTranslator;
        let mut ctx = StreamContext::new("chatcmpl_01", "claude-3", 12345);

        let line = "data: {\"type\":\"content_block_delta\",\"index\":7,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"NYC\\\"}\"}}";
        let result = t.translate_stream_line(line, &mut ctx).unwrap();
        assert_eq!(result, "");
        assert!(ctx.tool_calls.is_empty());
    }

    #[test]
    fn test_anthropic_to_chat_request_adaptive_thinking_uses_nested_effort() {
        let t = AnthropicToChatTranslator;
        let mut body = json!({
            "model": "claude-3",
            "messages": [{"role": "user", "content": "hi"}],
            "thinking": {"type": "adaptive", "output_config": {"effort": "low"}}
        });

        t.translate_request(&mut body, "claude-3").unwrap();
        assert_eq!(body["reasoning_effort"], "low");
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
