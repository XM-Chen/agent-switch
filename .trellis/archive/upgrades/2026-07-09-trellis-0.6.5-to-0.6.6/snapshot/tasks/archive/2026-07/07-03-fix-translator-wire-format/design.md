# Translator 流式 wire format 与 max_tokens 映射修复 - Design

## 1. 范围

本任务仅修改 `src-tauri/src/services/translator/` 及其单元测试,修复审计报告 P2-1~P2-7 与 translator P3 死代码。不得触碰 Codex OAuth 登录链路、proxy failover、DB portability 或前端。

## 2. 关键文件

- `src-tauri/src/services/translator/openai_responses.rs`
- `src-tauri/src/services/translator/anthropic_openai.rs`
- `src-tauri/src/services/translator/helpers.rs`
- `src-tauri/src/services/translator/mod.rs`
- `src-tauri/src/services/translator/native.rs`

## 3. 协议契约

### 3.1 Chat → Responses 流式 function_call 生命周期

当 Chat upstream delta 包含 `tool_calls` 时,Responses SSE 必须遵循:

1. 首次看到 tool call index: `response.output_item.added` with `status:"in_progress"`
2. 参数片段: `response.function_call_arguments.delta`
3. finish_reason 到达前: 对每个 open function_call 发 `response.output_item.done` with `status:"completed"` 和完整 arguments
4. 最后发 `response.completed`

不得在 function_call 未 done 时直接 `response.completed`。

### 3.2 max token 映射

| 方向 | 输入字段 | 输出字段 |
|------|----------|----------|
| Chat → Responses | `max_tokens` | `max_output_tokens` |
| Responses → Chat | `max_output_tokens` | `max_tokens` |
| Chat → Anthropic | `max_completion_tokens` 优先,否则 `max_tokens` | `max_tokens` |

转换后应删除上游不认识的旧字段,避免 400 或限制失效。

### 3.3 Anthropic system prompt 多块

`extract_content_text` 不得只返回第一个 text block。用于 system prompt 的提取应拼接所有 text block,保留原顺序。非 text block 仍忽略。

### 3.4 错误事件

`build_error_event(msg, "openai-chat")` 必须包含 msg。可使用非标准 `error.message` 字段,同时保留 `[DONE]` 终止帧。Anthropic / Responses 分支继续使用 JSON 转义后的 message。

### 3.5 input_json_delta 防串扰

AnthropicToChat 收到 `input_json_delta` 时,如果找不到 `block_to_tool_index` 映射,不得回退到 tool index 0。应跳过并返回空输出或返回可诊断错误,避免把畸形流参数串到第一个工具。

## 4. P3 处理策略

- `map_role`:当前零生产调用,删除函数和仅覆盖该函数的测试。Codex OAuth 子任务若需要角色映射应重新设计并接入真实调用点。
- `extract_all_text`:为 P2-6 保留/使用。
- `extract_delta_text` / `is_sse_event_end`:若仍无生产调用则删除及测试。
- `anthropic_openai.rs` 中累积但不输出的 `acc.arguments`:如果 stop/done 需要完整 arguments 则输出;否则删除死状态。
- `openai_responses.rs` 中误导性 event 行重置 content_block_index:删除或改为真实事件状态机。

## 5. 测试设计

至少新增/调整单元测试:

1. ChatToResponses 含 tool_calls 的流式 finish 输出顺序:added → arguments.delta → output_item.done → response.completed
2. ChatToResponses request max_tokens → max_output_tokens
3. ResponsesToChat request max_output_tokens → max_tokens
4. ChatToAnthropic max_completion_tokens → max_tokens
5. thinking adaptive output_config.effort 读取嵌套路径
6. 多块 system prompt 拼接
7. build_error_event openai-chat 包含错误消息
8. input_json_delta 缺 block mapping 不串扰 index 0

## 6. 非目标

- 不实现完整跨协议翻译全接线
- 不改 proxy 层如何选择 translator
- 不改 Codex OAuth role/account 语义
