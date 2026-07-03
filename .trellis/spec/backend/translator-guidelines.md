# Translator 协议转换规范

## 支持方向

- Anthropic ↔ OpenAI Chat
- OpenAI Chat ↔ OpenAI Responses
- 同协议 passthrough

## 请求字段映射

- Chat → Responses: `max_tokens` 必须映射为 `max_output_tokens`。
- Responses → Chat: `max_output_tokens` 必须映射为 `max_tokens`。
- Chat → Anthropic: `max_completion_tokens` 优先映射为 `max_tokens`；无该字段时保留/使用 `max_tokens`。
- Anthropic `thinking.output_config.effort` 必须按嵌套路径读取，禁止用字面键名 `output_config.effort`。

## 内容提取

- content 为数组时，文本提取应拼接所有 `type=text` 块，不能只取第一个 text 块。
- system prompt 多块 text 必须保留顺序拼接。

## 流式事件契约

- Chat → Responses 的 function_call 流：`response.output_item.added` → `response.function_call_arguments.delta` → `response.output_item.done` → `response.completed`。
- Chat → Anthropic 的 tool_use/text content block 必须成对 start/stop。
- Anthropic → Chat 收到缺少 block 映射的 `input_json_delta` 时不得回退到 tool index 0，避免参数串扰。

## 错误事件

`build_error_event` 的所有协议分支都必须保留错误消息。OpenAI Chat 分支允许携带非标准 `error.message` 字段以便本地调试器展示原因，同时保留 `[DONE]` 终止帧。

## 死代码约定

不要保留未接入生产调用点的角色映射 stub。未来若需要 role mapping，应在真实 translator call site 设计并配套测试。
