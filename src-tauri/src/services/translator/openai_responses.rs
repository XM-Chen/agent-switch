/// OpenAI Chat ↔ Responses API 协议转换器。
///
/// 实现两个转换器：
/// - `ChatToResponsesTranslator`：Chat Completions API → Responses API
/// - `ResponsesToChatTranslator`：Responses API → Chat Completions API
///
/// 覆盖请求/响应/流式行三套转换。参考 design.md §3 与 9router/cpa 对应方向实现模式。
///
/// # 格式差异
/// | Chat Completions | Responses API |
/// |------------------|---------------|
/// | `messages: [...]` | `input: [...]` |
/// | `choices[0].message` | `output[0]`（item 类型） |
/// | `finish_reason` | `status: "completed" \| "incomplete" \| "failed"` |
/// | SSE `choices[0].delta` | SSE `response.text.delta` / `response.output_item.*` |
/// | `data: [DONE]` | `event: response.done` / `response.completed` |
use crate::services::translator::helpers;
use crate::services::translator::{StreamContext, Translator};
use serde_json::{json, Value};

// =====================================================================
// Chat → Responses
// =====================================================================

/// OpenAI Chat Completions API 转 Responses API 转换器。
pub struct ChatToResponsesTranslator;

impl Translator for ChatToResponsesTranslator {
    fn key(&self) -> (&'static str, &'static str) {
        ("openai-chat", "openai-responses")
    }

    /// 将 Chat Completions 请求转换为 Responses API 格式。
    fn translate_request(&self, body: &mut Value, model: &str) -> Result<(), String> {
        // 1. 改写 model
        body["model"] = json!(model);

        // 2. messages → input
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            let input_items: Vec<Value> = messages
                .iter()
                .flat_map(|msg| {
                    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                    let content = msg.get("content").cloned().unwrap_or(json!(""));
                    match role {
                        "system" => vec![json!({
                            "type": "message",
                            "role": "developer",
                            "content": content
                        })],
                        "user" => vec![json!({
                            "type": "message",
                            "role": "user",
                            "content": content
                        })],
                        "assistant" => {
                            // assistant 消息作为 message item；
                            // tool_calls 作为独立的 function_call item 平铺到 input（而非嵌在 message item 的 output 里）。
                            let mut items: Vec<Value> = vec![json!({
                                "type": "message",
                                "role": "assistant",
                                "content": content
                            })];
                            if let Some(tool_calls) =
                                msg.get("tool_calls").and_then(|t| t.as_array())
                            {
                                for tc in tool_calls {
                                    let tc_id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                                    let tc_name = tc
                                        .get("function")
                                        .and_then(|f| f.get("name"))
                                        .and_then(|n| n.as_str())
                                        .unwrap_or("");
                                    let tc_args = tc
                                        .get("function")
                                        .and_then(|f| f.get("arguments"))
                                        .and_then(|a| a.as_str())
                                        .unwrap_or("{}");
                                    items.push(json!({
                                        "type": "function_call",
                                        "id": format!("fc_{}", tc_id),
                                        "call_id": tc_id,
                                        "name": tc_name,
                                        "arguments": tc_args,
                                        "status": "completed"
                                    }));
                                }
                            }
                            items
                        }
                        "tool" => {
                            let tc_id = msg
                                .get("tool_call_id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("");
                            vec![json!({
                                "type": "function_call_output",
                                "id": format!("fco_{}", tc_id),
                                "call_id": tc_id,
                                "output": content
                            })]
                        }
                        _ => vec![json!({
                            "type": "message",
                            "role": "user",
                            "content": content
                        })],
                    }
                })
                .collect();

            body["input"] = json!(input_items);
            body.as_object_mut().map(|m| m.remove("messages"));
        }

        // 3. 转换 tools 格式
        if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
            let resp_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    if t.get("type").and_then(|tt| tt.as_str()) == Some("function") {
                        let func = t.get("function").unwrap();
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
                        json!({
                            "type": "function",
                            "name": name,
                            "description": desc,
                            "parameters": params,
                            "strict": false
                        })
                    } else {
                        json!({})
                    }
                })
                .filter(|t| t.is_object() && !t.is_null())
                .collect();
            body["tools"] = json!(resp_tools);
        }

        // 4. 处理 stream（保留原字段）
        // 5. 映射 max_tokens → max_output_tokens
        if let Some(obj) = body.as_object_mut() {
            if let Some(max_tokens) = obj.remove("max_tokens") {
                obj.entry("max_output_tokens").or_insert(max_tokens);
            }
        }

        // 6. 移除 Chat 专有字段
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
            "stop",
            "stop_sequences",
        ];
        if let Some(obj) = body.as_object_mut() {
            for field in &chat_only {
                obj.remove(*field);
            }
        }

        Ok(())
    }

    /// 将 Chat Completions 非流式响应转换为 Responses API 格式。
    fn translate_response(&self, body: &mut Value) -> Result<(), String> {
        let chat_id = body
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("chatcmpl_unknown");
        let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");

        let choice = body
            .get("choices")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .cloned()
            .unwrap_or_default();

        let finish_reason = choice
            .get("finish_reason")
            .and_then(|f| f.as_str())
            .unwrap_or("stop");

        let status = match finish_reason {
            "stop" => "completed",
            "tool_calls" => "completed",
            "length" => "incomplete",
            _ => "completed",
        };

        let message = choice.get("message");
        let mut output_items: Vec<Value> = Vec::new();

        if let Some(msg) = message {
            let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");

            // 消息输出
            let msg_item = json!({
                "type": "message",
                "role": "assistant",
                "content": content
            });
            output_items.push(msg_item);

            // tool_calls output
            if let Some(tool_calls) = msg.get("tool_calls").and_then(|t| t.as_array()) {
                for tc in tool_calls {
                    let tc_id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let tc_name = tc
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let tc_args = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|a| a.as_str())
                        .unwrap_or("{}");
                    output_items.push(json!({
                        "type": "function_call",
                        "id": format!("fc_{}", tc_id),
                        "call_id": tc_id,
                        "name": tc_name,
                        "arguments": tc_args,
                        "status": "completed"
                    }));
                }
            }
        }

        // usage
        let usage = body.get("usage");
        let (input_tokens, output_tokens) = if let Some(u) = usage {
            let inp = u.get("prompt_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            let out = u
                .get("completion_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            (inp, out)
        } else {
            (0, 0)
        };

        *body = json!({
            "id": chat_id,
            "object": "response",
            "model": model,
            "status": status,
            "output": output_items,
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "total_tokens": input_tokens + output_tokens
            }
        });

        Ok(())
    }

    /// 将 Chat SSE 行转换为 Responses API SSE 行。
    fn translate_stream_line(
        &self,
        line: &str,
        context: &mut StreamContext,
    ) -> Result<String, String> {
        let data = match helpers::extract_sse_data(line) {
            Some(d) => d,
            None => return Ok(line.to_string()),
        };

        if data == "[DONE]" {
            return Ok("event: response.done\ndata: {\"type\":\"response.done\"}\n\n".to_string());
        }

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

        let mut output = String::new();

        // 首个 chunk（role delta → response.created）
        if role == Some("assistant") && !context.has_content {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            let resp_id = parsed
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("resp_unknown");

            context.response_id = resp_id.to_string();
            context.created_at = now;

            output.push_str(&format!(
                "event: response.created\ndata: {{\"type\":\"response.created\",\"response\":{{\"id\":\"{}\",\"object\":\"response\",\"model\":\"{}\",\"status\":\"in_progress\",\"output\":[]}}}}\n\n",
                resp_id,
                context.model
            ));

            context.has_content = true;
        }

        // content delta → response.text.delta
        if let Some(text) = content {
            if !text.is_empty() {
                // 用 serde_json 正确转义，避免手动 replace 漏处理 \n/\r/\t/控制字符。
                let escaped = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
                output.push_str(&format!(
                    "event: response.text.delta\ndata: {{\"type\":\"response.text.delta\",\"delta\":{}}}\n\n",
                    escaped
                ));
            }
        }

        // tool_calls delta → function_call_arguments.delta
        if let Some(tool_calls) = delta
            .and_then(|d| d.get("tool_calls"))
            .and_then(|t| t.as_array())
        {
            for tc in tool_calls {
                // 用 Chat 的 tool_call index 作为 item 标识，发 output_item.added/done
                // 生命周期事件，并映射到 Responses 的 output_index/item_index。
                let tc_index = tc.get("index").and_then(|i| i.as_i64()).unwrap_or(0) as i32;
                let tc_id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
                let tc_name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("");

                // 首次见到该 tool_call：发 output_item.added 创建 function_call item
                if !tc_id.is_empty() && !context.tool_calls.contains_key(&tc_index) {
                    let item_id = format!("fc_{}", tc_id);
                    output.push_str(&format!(
                        "event: response.output_item.added\ndata: {{\"type\":\"response.output_item.added\",\"output_index\":{},\"item\":{{\"type\":\"function_call\",\"id\":\"{}\",\"call_id\":\"{}\",\"name\":\"{}\",\"arguments\":\"\",\"status\":\"in_progress\"}}}}\n\n",
                        tc_index, item_id, tc_id, tc_name
                    ));
                    context.tool_calls.insert(
                        tc_index,
                        crate::services::translator::ToolCallAcc {
                            id: tc_id.to_string(),
                            name: tc_name.to_string(),
                            arguments: String::new(),
                        },
                    );
                }

                if let Some(args) = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                {
                    if !args.is_empty() {
                        // 用 serde_json 正确转义 args（部分 JSON 字符串），避免双重转义损坏
                        let escaped =
                            serde_json::to_string(args).unwrap_or_else(|_| "\"\"".to_string());
                        output.push_str(&format!(
                            "event: response.function_call_arguments.delta\ndata: {{\"type\":\"response.function_call_arguments.delta\",\"output_index\":{},\"delta\":{}}}\n\n",
                            tc_index, escaped
                        ));
                        if let Some(acc) = context.tool_calls.get_mut(&tc_index) {
                            acc.arguments.push_str(args);
                        }
                    }
                }
            }
        }

        // finish_reason → close function_call items, then response.completed
        if let Some(reason) = finish_reason {
            let mut done_items: Vec<(i32, crate::services::translator::ToolCallAcc)> = context
                .tool_calls
                .iter()
                .map(|(idx, acc)| (*idx, acc.clone()))
                .collect();
            done_items.sort_by_key(|(idx, _)| *idx);
            for (output_index, acc) in done_items {
                let item_id = format!("fc_{}", acc.id);
                let escaped_id =
                    serde_json::to_string(&item_id).unwrap_or_else(|_| "\"\"".to_string());
                let escaped_call_id =
                    serde_json::to_string(&acc.id).unwrap_or_else(|_| "\"\"".to_string());
                let escaped_name =
                    serde_json::to_string(&acc.name).unwrap_or_else(|_| "\"\"".to_string());
                let escaped_args =
                    serde_json::to_string(&acc.arguments).unwrap_or_else(|_| "\"\"".to_string());
                output.push_str(&format!(
                    "event: response.output_item.done\ndata: {{\"type\":\"response.output_item.done\",\"output_index\":{},\"item\":{{\"type\":\"function_call\",\"id\":{},\"call_id\":{},\"name\":{},\"arguments\":{},\"status\":\"completed\"}}}}\n\n",
                    output_index, escaped_id, escaped_call_id, escaped_name, escaped_args
                ));
            }
            context.tool_calls.clear();

            let status = match reason {
                "stop" | "tool_calls" => "completed",
                "length" => "incomplete",
                _ => "failed",
            };

            let usage = parsed.get("usage");
            let input_tokens = usage
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let output_tokens = usage
                .and_then(|u| u.get("completion_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            output.push_str(&format!(
                "event: response.completed\ndata: {{\"type\":\"response.completed\",\"response\":{{\"id\":\"{}\",\"status\":\"{}\",\"usage\":{{\"input_tokens\":{},\"output_tokens\":{}}}}}}}\n\n",
                context.response_id, status, input_tokens, output_tokens
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
// Responses → Chat
// =====================================================================

/// Responses API 转 Chat Completions API 转换器。
pub struct ResponsesToChatTranslator;

impl Translator for ResponsesToChatTranslator {
    fn key(&self) -> (&'static str, &'static str) {
        ("openai-responses", "openai-chat")
    }

    /// 将 Responses API 请求转换为 Chat Completions 格式。
    fn translate_request(&self, body: &mut Value, model: &str) -> Result<(), String> {
        // 1. 改写 model
        body["model"] = json!(model);

        // 2. input → messages
        if let Some(input) = body.get("input").and_then(|i| i.as_array()) {
            let messages: Vec<Value> = input
                .iter()
                .map(|item| {
                    let item_type = item
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("message");
                    match item_type {
                        "message" => {
                            let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                            // 映射 developer → system
                            let chat_role = if role == "developer" { "system" } else { role };
                            json!({
                                "role": chat_role,
                                "content": item.get("content").cloned().unwrap_or(json!(""))
                            })
                        }
                        "function_call" => {
                            let fc_id = item.get("id").and_then(|i| i.as_str()).unwrap_or("");
                            let fc_name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let fc_args = item
                                .get("arguments")
                                .and_then(|a| a.as_str())
                                .unwrap_or("{}");
                            let call_id = fc_id.strip_prefix("fc_").unwrap_or(fc_id);
                            json!({
                                "role": "assistant",
                                "content": "",
                                "tool_calls": [{
                                    "id": call_id,
                                    "type": "function",
                                    "function": {
                                        "name": fc_name,
                                        "arguments": fc_args
                                    }
                                }]
                            })
                        }
                        "function_call_output" => {
                            let call_id =
                                item.get("call_id").and_then(|i| i.as_str()).unwrap_or("");
                            json!({
                                "role": "tool",
                                "tool_call_id": call_id,
                                "content": item.get("output").cloned().unwrap_or(json!(""))
                            })
                        }
                        _ => {
                            json!({
                                "role": "user",
                                "content": item.to_string()
                            })
                        }
                    }
                })
                .collect();

            body["messages"] = json!(messages);
            body.as_object_mut().map(|m| m.remove("input"));
        }

        // 3. 转换 tools 格式
        if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
            let chat_tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    let name = t.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let desc = t.get("description").and_then(|d| d.as_str()).unwrap_or("");
                    let params = t
                        .get("parameters")
                        .or_else(|| t.get("parametersJsonSchema"))
                        .cloned()
                        .unwrap_or(json!({}));
                    json!({
                        "type": "function",
                        "function": {
                            "name": name,
                            "description": desc,
                            "parameters": params
                        }
                    })
                })
                .collect();
            body["tools"] = json!(chat_tools);
        }

        // 4. 映射 max_output_tokens → max_tokens
        if let Some(obj) = body.as_object_mut() {
            if let Some(max_output_tokens) = obj.remove("max_output_tokens") {
                obj.entry("max_tokens").or_insert(max_output_tokens);
            }
        }

        // 5. 移除 Responses 专有字段（保留 temperature/top_p/max_tokens 等通用参数）
        let resp_only = ["previous_response_id", "truncation", "store", "metadata"];
        if let Some(obj) = body.as_object_mut() {
            for field in &resp_only {
                obj.remove(*field);
            }
        }

        Ok(())
    }

    /// 将 Responses API 非流式响应转换为 Chat Completions 格式。
    fn translate_response(&self, body: &mut Value) -> Result<(), String> {
        let resp_id = body
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("resp_unknown");
        let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");
        let status = body
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("completed");

        // status → finish_reason
        let finish_reason = match status {
            "completed" => "stop",
            "incomplete" => "length",
            "failed" => "content_filter",
            _ => "stop",
        };

        // 提取 output 中的消息内容
        let output = body
            .get("output")
            .and_then(|o| o.as_array())
            .cloned()
            .unwrap_or_default();
        let mut content_text = String::new();
        let mut tool_calls = Vec::new();

        for item in &output {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("message") => {
                    // content 可能是字符串或数组（多模态），统一提取文本
                    let item_content = item.get("content");
                    let text = match item_content {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Array(arr)) => {
                            // 提取数组里所有 text 块的文本
                            arr.iter()
                                .filter_map(|b| {
                                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                                        b.get("text").and_then(|t| t.as_str()).map(String::from)
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("")
                        }
                        _ => String::new(),
                    };
                    if !text.is_empty() {
                        if !content_text.is_empty() {
                            content_text.push('\n');
                        }
                        content_text.push_str(&text);
                    }
                }
                Some("function_call") => {
                    let fc_id = item.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let fc_name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let fc_args = item
                        .get("arguments")
                        .and_then(|a| a.as_str())
                        .unwrap_or("{}");
                    let call_id = fc_id.strip_prefix("fc_").unwrap_or(fc_id);
                    tool_calls.push(json!({
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": fc_name,
                            "arguments": fc_args
                        }
                    }));
                }
                _ => {}
            }
        }

        // usage 转换
        let usage = body.get("usage");
        let (prompt_tokens, completion_tokens) = if let Some(u) = usage {
            let inp = u.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            let out = u.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
            (inp, out)
        } else {
            (0, 0)
        };

        let mut choice = json!({
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content_text
            },
            "finish_reason": finish_reason
        });

        if !tool_calls.is_empty() {
            choice["message"]["tool_calls"] = json!(tool_calls);
        }

        *body = json!({
            "id": resp_id,
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
                "total_tokens": prompt_tokens + completion_tokens
            }
        });

        Ok(())
    }

    /// 将 Responses API SSE 行转换为 Chat SSE 行。
    fn translate_stream_line(
        &self,
        line: &str,
        context: &mut StreamContext,
    ) -> Result<String, String> {
        // 检查 event 类型
        if helpers::is_sse_event_type(line).is_some() {
            return Ok("".to_string());
        }

        // 提取 data
        let data = match helpers::extract_sse_data(line) {
            Some(d) => d,
            None => return Ok(line.to_string()),
        };

        // 解析 JSON
        let parsed: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return Ok(line.to_string()),
        };

        let event_type = parsed.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match event_type {
            "response.created" => {
                // response.created → 首个 chat chunk
                if let Some(response) = parsed.get("response") {
                    let resp_id = response
                        .get("id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("resp_unknown");
                    let model_name = response.get("model").and_then(|m| m.as_str()).unwrap_or("");

                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);

                    context.response_id = resp_id.to_string();
                    context.model = model_name.to_string();
                    context.created_at = now;

                    let chunk = json!({
                        "id": resp_id,
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
                        ]
                    });
                    context.has_content = true;
                    Ok(format!("data: {}\n\n", chunk))
                } else {
                    Ok("".to_string())
                }
            }

            "response.text.delta" => {
                // text delta → content delta
                let text = parsed.get("delta").and_then(|d| d.as_str()).unwrap_or("");
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
                Ok(format!("data: {}\n\n", chunk))
            }

            "response.output_item.added" => {
                // 记录 function_call item 的 output_index → Chat tool_call index 映射。
                // Responses 的 output_index 是所有 output item 的序号（message+function_call 混排），
                // 而 Chat 的 tool_calls[].index 只计工具调用，从 0 开始。
                if parsed
                    .get("item")
                    .and_then(|i| i.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("function_call")
                {
                    let output_index = parsed
                        .get("output_index")
                        .and_then(|i| i.as_i64())
                        .unwrap_or(0) as i32;
                    if !context.block_to_tool_index.contains_key(&output_index) {
                        let tool_index = context.block_to_tool_index.len() as i32;
                        context.block_to_tool_index.insert(output_index, tool_index);
                        // 同时初始化累积器，并发出 tool_call 起始 delta（带 id/name）
                        let item = parsed.get("item").unwrap();
                        let fc_id = item.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        let fc_name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let call_id = item
                            .get("call_id")
                            .and_then(|i| i.as_str())
                            .unwrap_or(fc_id);
                        context.tool_calls.insert(
                            tool_index,
                            crate::services::translator::ToolCallAcc {
                                id: call_id.to_string(),
                                name: fc_name.to_string(),
                                arguments: String::new(),
                            },
                        );
                        let chunk = json!({
                            "choices": [{
                                "index": 0,
                                "delta": {
                                    "tool_calls": [{
                                        "index": tool_index,
                                        "id": call_id,
                                        "type": "function",
                                        "function": { "name": fc_name, "arguments": "" }
                                    }]
                                },
                                "finish_reason": null
                            }]
                        });
                        return Ok(format!("data: {}\n\n", chunk));
                    }
                }
                Ok("".to_string())
            }

            "response.output_item.done" => Ok("".to_string()),

            "response.function_call_arguments.delta" => {
                // function call arguments delta
                let text = parsed.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                let output_index = parsed
                    .get("output_index")
                    .and_then(|i| i.as_i64())
                    .unwrap_or(0) as i32;
                // 用映射后的 Chat tool_call index，而非 Responses output_index
                let tool_index = context
                    .block_to_tool_index
                    .get(&output_index)
                    .copied()
                    .unwrap_or(0);
                let chunk = json!({
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": tool_index,
                                        "function": {
                                            "arguments": text
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

            "response.completed" | "response.done" => {
                // completed/done → finish_reason + [DONE]
                let response = parsed.get("response");
                let status = response
                    .and_then(|r| r.get("status"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("completed");
                let finish_reason = match status {
                    "completed" => "stop",
                    "incomplete" => "length",
                    "failed" => "content_filter",
                    _ => "stop",
                };

                let usage = response.and_then(|r| r.get("usage"));
                let prompt_tokens = usage
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let completion_tokens = usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);

                let mut result = String::new();
                result.push_str(&format!(
                    "data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"{}\"}}],\"usage\":{{\"prompt_tokens\":{},\"completion_tokens\":{},\"total_tokens\":{}}}}}\n\n",
                    finish_reason, prompt_tokens, completion_tokens, prompt_tokens + completion_tokens
                ));
                result.push_str("data: [DONE]\n\n");
                Ok(result)
            }

            "response.failed" => {
                let err_msg = parsed
                    .pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("upstream_error");
                Ok(helpers::build_error_event(err_msg, "openai-chat"))
            }

            "error" => {
                let err_msg = parsed
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("upstream_error");
                Ok(helpers::build_error_event(err_msg, "openai-chat"))
            }

            _ => {
                // 未知事件，跳过
                Ok("".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // =====================================================================
    // Chat → Responses 转换
    // =====================================================================

    #[test]
    fn test_chat_to_responses_request_basic() {
        let t = ChatToResponsesTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "You are helpful"},
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there"}
            ],
            "stream": true
        });

        t.translate_request(&mut body, "gpt-4o").unwrap();

        assert!(body.get("messages").is_none());
        assert!(body.get("input").is_some());
        assert_eq!(body["input"].as_array().unwrap().len(), 3);
        assert_eq!(body["input"][0]["type"], "message");
        assert_eq!(body["input"][0]["role"], "developer");
        assert_eq!(body["input"][1]["role"], "user");
        assert_eq!(body["input"][2]["role"], "assistant");
        assert_eq!(body["model"], "gpt-4o");
    }

    #[test]
    fn test_chat_to_responses_request_with_tools() {
        let t = ChatToResponsesTranslator;
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

        t.translate_request(&mut body, "gpt-4").unwrap();

        assert!(body.get("tools").is_some());
        let tool = &body["tools"][0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["name"], "get_weather");
    }

    #[test]
    fn test_chat_to_responses_response_basic() {
        let t = ChatToResponsesTranslator;
        let mut body = json!({
            "id": "chatcmpl_abc",
            "object": "chat.completion",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hello world"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}
        });

        t.translate_response(&mut body).unwrap();

        assert_eq!(body["status"], "completed");
        assert_eq!(body["output"][0]["type"], "message");
        assert_eq!(body["output"][0]["content"], "Hello world");
        assert_eq!(body["usage"]["input_tokens"], 10);
        assert_eq!(body["usage"]["output_tokens"], 20);
    }

    #[test]
    fn test_chat_to_responses_stream_basic() {
        let t = ChatToResponsesTranslator;
        let mut ctx = StreamContext::new("", "gpt-4", 0);

        // 首个 chunk → response.created
        let line1 = "data: {\"id\":\"resp_1\",\"object\":\"chat.completion.chunk\",\"created\":12345,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}";
        let result1 = t.translate_stream_line(line1, &mut ctx).unwrap();
        assert!(result1.contains("event: response.created"));

        // content delta → response.text.delta
        let line2 = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}";
        let result2 = t.translate_stream_line(line2, &mut ctx).unwrap();
        assert!(result2.contains("event: response.text.delta"));

        // finish_reason → response.completed
        let line3 = "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}";
        let result3 = t.translate_stream_line(line3, &mut ctx).unwrap();
        assert!(result3.contains("event: response.completed"));

        // [DONE] → response.done
        let line4 = "data: [DONE]";
        let result4 = t.translate_stream_line(line4, &mut ctx).unwrap();
        assert!(result4.contains("event: response.done"));
    }

    // =====================================================================
    // Responses → Chat 转换
    // =====================================================================

    #[test]
    fn test_responses_to_chat_request_basic() {
        let t = ResponsesToChatTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "input": [
                {"type": "message", "role": "developer", "content": "Be helpful"},
                {"type": "message", "role": "user", "content": "Hello"}
            ]
        });

        t.translate_request(&mut body, "gpt-4").unwrap();

        assert!(body.get("input").is_none());
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "Be helpful");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "Hello");
    }

    #[test]
    fn test_responses_to_chat_request_with_function_call() {
        let t = ResponsesToChatTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "input": [
                {"type": "function_call", "id": "fc_call_1", "call_id": "call_1", "name": "get_weather", "arguments": "{\"city\":\"NYC\"}", "status": "completed"},
                {"type": "function_call_output", "call_id": "call_1", "output": "Sunny"}
            ]
        });

        t.translate_request(&mut body, "gpt-4").unwrap();

        assert_eq!(body["messages"][0]["role"], "assistant");
        assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call_1");
        assert_eq!(body["messages"][1]["role"], "tool");
        assert_eq!(body["messages"][1]["content"], "Sunny");
    }

    #[test]
    fn test_responses_to_chat_response_basic() {
        let t = ResponsesToChatTranslator;
        let mut body = json!({
            "id": "resp_1",
            "object": "response",
            "model": "gpt-4",
            "status": "completed",
            "output": [
                {"type": "message", "role": "assistant", "content": "Hello world"}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 20, "total_tokens": 30}
        });

        t.translate_response(&mut body).unwrap();

        assert_eq!(body["object"], "chat.completion");
        assert_eq!(body["choices"][0]["message"]["content"], "Hello world");
        assert_eq!(body["choices"][0]["finish_reason"], "stop");
        assert_eq!(body["usage"]["prompt_tokens"], 10);
        assert_eq!(body["usage"]["completion_tokens"], 20);
    }

    #[test]
    fn test_responses_to_chat_stream_basic() {
        let t = ResponsesToChatTranslator;
        let mut ctx = StreamContext::new("", "gpt-4", 0);

        // response.created → 首个 chat chunk
        let line1 = "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"object\":\"response\",\"model\":\"gpt-4\",\"status\":\"in_progress\",\"output\":[]}}";
        let result1 = t.translate_stream_line(line1, &mut ctx).unwrap();
        assert!(result1.contains("\"role\":\"assistant\""));
        assert!(ctx.has_content);

        // response.text.delta → content delta
        let line2 = "data: {\"type\":\"response.text.delta\",\"delta\":\"Hello\"}";
        let result2 = t.translate_stream_line(line2, &mut ctx).unwrap();
        assert!(result2.contains("\"content\":\"Hello\""));

        // response.completed → finish_reason + [DONE]
        let line3 = "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20}}}";
        let result3 = t.translate_stream_line(line3, &mut ctx).unwrap();
        assert!(result3.contains("\"finish_reason\":\"stop\""));
        assert!(result3.contains("[DONE]"));
    }

    #[test]
    fn test_full_roundtrip_request() {
        let c2r = ChatToResponsesTranslator;
        let r2c = ResponsesToChatTranslator;

        let mut body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "system", "content": "Be helpful"},
                {"role": "user", "content": "Hi"}
            ],
            "stream": true
        });

        // Chat → Responses
        c2r.translate_request(&mut body, "gpt-4o").unwrap();
        assert!(body.get("messages").is_none());
        assert!(body.get("input").is_some());
        assert_eq!(body["input"][0]["role"], "developer");

        // Responses → Chat
        r2c.translate_request(&mut body, "gpt-4o").unwrap();
        assert!(body.get("input").is_none());
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["model"], "gpt-4o");
    }

    /// 回归 R4：assistant + tool_calls 应平铺为 message item + 独立 function_call items，
    /// 而非把 function_call 嵌在 message item 的 output 字段里。
    #[test]
    fn test_chat_to_responses_request_assistant_tool_calls_flat() {
        let t = ChatToResponsesTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "messages": [
                {"role": "assistant", "content": "Let me check", "tool_calls": [
                    {"id": "call_1", "type": "function", "function": {"name": "get_weather", "arguments": "{\"city\":\"NYC\"}"}}
                ]}
            ]
        });

        t.translate_request(&mut body, "gpt-4").unwrap();

        let input = body["input"].as_array().unwrap();
        assert_eq!(
            input.len(),
            2,
            "应拆成 1 个 message item + 1 个 function_call item"
        );
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "assistant");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[1]["name"], "get_weather");
        // 确保没有错误嵌套的 output 字段
        assert!(input[0].get("output").is_none());
    }

    /// 回归 R5：temperature/top_p 不应被移除。
    #[test]
    fn test_chat_to_responses_request_preserves_sampling_params() {
        let t = ChatToResponsesTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 0.7,
            "top_p": 0.9
        });

        t.translate_request(&mut body, "gpt-4").unwrap();
        assert_eq!(body["temperature"], 0.7);
        assert_eq!(body["top_p"], 0.9);
    }

    #[test]
    fn test_chat_to_responses_request_maps_max_tokens() {
        let t = ChatToResponsesTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 123
        });

        t.translate_request(&mut body, "gpt-4").unwrap();
        assert!(body.get("max_tokens").is_none());
        assert_eq!(body["max_output_tokens"], 123);
    }

    #[test]
    fn test_responses_to_chat_request_maps_max_output_tokens() {
        let t = ResponsesToChatTranslator;
        let mut body = json!({
            "model": "gpt-4",
            "input": [{"type": "message", "role": "user", "content": "hi"}],
            "max_output_tokens": 456
        });

        t.translate_request(&mut body, "gpt-4").unwrap();
        assert!(body.get("max_output_tokens").is_none());
        assert_eq!(body["max_tokens"], 456);
    }

    #[test]
    fn test_chat_to_responses_stream_function_call_done_before_completed() {
        let t = ChatToResponsesTranslator;
        let mut ctx = StreamContext::new("chatcmpl_abc", "gpt-4", 0);
        let _ = t.translate_stream_line(
            "data: {\"id\":\"resp_1\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let _ = t.translate_stream_line(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"city\\\"\"}}]},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let _ = t.translate_stream_line(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\":\\\"NYC\\\"}\"}}]},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let out = t
            .translate_stream_line(
                "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}",
                &mut ctx,
            )
            .unwrap();

        let done_pos = out.find("event: response.output_item.done").unwrap();
        let completed_pos = out.find("event: response.completed").unwrap();
        assert!(done_pos < completed_pos);
        assert!(out.contains("\"arguments\":\"{\\\"city\\\":\\\"NYC\\\"}\""));
        assert!(!out.contains("in_progress"));
    }

    /// 回归 R1：流式 text delta 含特殊字符时 SSE data 必须是合法 JSON。
    #[test]
    fn test_chat_to_responses_stream_text_special_chars() {
        let t = ChatToResponsesTranslator;
        let mut ctx = StreamContext::new("chatcmpl_abc", "gpt-4", 0);
        let _ = t.translate_stream_line(
            "data: {\"id\":\"resp_1\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let out = t.translate_stream_line(
            "data: {\"id\":\"resp_1\",\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"line1\\nline2 \\\"q\\\" \\\\p\"},\"finish_reason\":null}]}",
            &mut ctx,
        ).unwrap();
        let data_line = out.lines().rev().find(|l| l.starts_with("data: ")).unwrap();
        let payload = data_line.trim_start_matches("data: ");
        let v: Value = serde_json::from_str(payload).expect("必须是合法 JSON");
        assert_eq!(v["delta"], "line1\nline2 \"q\" \\p");
    }
}
