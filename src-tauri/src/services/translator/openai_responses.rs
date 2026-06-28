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
                .map(|msg| {
                    let role = msg
                        .get("role")
                        .and_then(|r| r.as_str())
                        .unwrap_or("user");
                    let content = msg.get("content").cloned().unwrap_or(json!(""));
                    match role {
                        "system" => json!({
                            "type": "message",
                            "role": "developer",
                            "content": content
                        }),
                        "user" => json!({
                            "type": "message",
                            "role": "user",
                            "content": content
                        }),
                        "assistant" => {
                            let mut item = json!({
                                "type": "message",
                                "role": "assistant",
                                "content": content
                            });
                            // tool_calls → output items
                            if let Some(tool_calls) =
                                msg.get("tool_calls").and_then(|t| t.as_array())
                            {
                                let fc_items: Vec<Value> = tool_calls
                                    .iter()
                                    .map(|tc| {
                                        let tc_id = tc
                                            .get("id")
                                            .and_then(|i| i.as_str())
                                            .unwrap_or("");
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
                                        json!({
                                            "type": "function_call",
                                            "id": format!("fc_{}", tc_id),
                                            "name": tc_name,
                                            "arguments": tc_args,
                                            "status": "completed"
                                        })
                                    })
                                    .collect();
                                item["output"] = json!(fc_items);
                            }
                            item
                        }
                        "tool" => {
                            let tc_id = msg
                                .get("tool_call_id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("");
                            json!({
                                "type": "function_call_output",
                                "id": format!("fco_{}", tc_id),
                                "call_id": tc_id,
                                "output": content
                            })
                        }
                        _ => json!({
                            "type": "message",
                            "role": "user",
                            "content": content
                        }),
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
                        let name = func
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("");
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
        // 5. 处理 max_tokens（保留）

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
        let model = body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("");

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
            let content = msg
                .get("content")
                .and_then(|c| c.as_str())
                .unwrap_or("");

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
    fn translate_stream_line(&self, line: &str, context: &mut StreamContext) -> Result<String, String> {
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
        let content = delta.and_then(|d| d.get("content")).and_then(|c| c.as_str());
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
                output.push_str(&format!(
                    "event: response.text.delta\ndata: {{\"type\":\"response.text.delta\",\"delta\":\"{}\"}}\n\n",
                    text.replace('\\', "\\\\").replace('"', "\\\"")
                ));
            }
        }

        // tool_calls delta → function_call_arguments.delta
        if let Some(tool_calls) = delta.and_then(|d| d.get("tool_calls")).and_then(|t| t.as_array()) {
            for tc in tool_calls {
                if let Some(args) = tc
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                {
                    if !args.is_empty() {
                        output.push_str(&format!(
                            "event: response.function_call_arguments.delta\ndata: {{\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{}\"}}\n\n",
                            args.replace('\\', "\\\\").replace('"', "\\\"")
                        ));
                    }
                }
            }
        }

        // finish_reason → response.completed
        if let Some(reason) = finish_reason {
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
                            let role = item
                                .get("role")
                                .and_then(|r| r.as_str())
                                .unwrap_or("user");
                            // 映射 developer → system
                            let chat_role = if role == "developer" {
                                "system"
                            } else {
                                role
                            };
                            json!({
                                "role": chat_role,
                                "content": item.get("content").cloned().unwrap_or(json!(""))
                            })
                        }
                        "function_call" => {
                            let fc_id = item
                                .get("id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("");
                            let fc_name = item
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or("");
                            let fc_args = item
                                .get("arguments")
                                .and_then(|a| a.as_str())
                                .unwrap_or("{}");
                            let call_id = if fc_id.starts_with("fc_") {
                                &fc_id[3..]
                            } else {
                                fc_id
                            };
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
                            let call_id = item
                                .get("call_id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("");
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
                    let desc = t
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("");
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

        // 4. 移除 Responses 专有字段
        let resp_only = [
            "previous_response_id",
            "truncation",
            "temperature",
            "top_p",
            "store",
            "metadata",
        ];
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
        let model = body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("");
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
        let output = body.get("output").and_then(|o| o.as_array()).cloned().unwrap_or_default();
        let mut content_text = String::new();
        let mut tool_calls = Vec::new();

        for item in &output {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("message") => {
                    let item_content = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
                    if !item_content.is_empty() {
                        if !content_text.is_empty() {
                            content_text.push('\n');
                        }
                        content_text.push_str(item_content);
                    }
                }
                Some("function_call") => {
                    let fc_id = item
                        .get("id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("");
                    let fc_name = item
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("");
                    let fc_args = item
                        .get("arguments")
                        .and_then(|a| a.as_str())
                        .unwrap_or("{}");
                    let call_id = if fc_id.starts_with("fc_") {
                        &fc_id[3..]
                    } else {
                        fc_id
                    };
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
            let out = u
                .get("output_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
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
    fn translate_stream_line(&self, line: &str, context: &mut StreamContext) -> Result<String, String> {
        // 检查 event 类型
        if let Some(_event_name) = helpers::is_sse_event_type(line) {
            context.content_block_index = 0;
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
                    let model_name = response
                        .get("model")
                        .and_then(|m| m.as_str())
                        .unwrap_or("");

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
                    Ok(format!("data: {}\n\n", chunk.to_string()))
                } else {
                    Ok("".to_string())
                }
            }

            "response.text.delta" => {
                // text delta → content delta
                let text = parsed
                    .get("delta")
                    .and_then(|d| d.as_str())
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
                Ok(format!("data: {}\n\n", chunk.to_string()))
            }

            "response.output_item.added" | "response.output_item.done" => {
                // 跳过 item 生命周期事件
                Ok("".to_string())
            }

            "response.function_call_arguments.delta" => {
                // function call arguments delta
                let text = parsed
                    .get("delta")
                    .and_then(|d| d.as_str())
                    .unwrap_or("");
                let chunk = json!({
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
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
                Ok(format!("data: {}\n\n", chunk.to_string()))
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
}
