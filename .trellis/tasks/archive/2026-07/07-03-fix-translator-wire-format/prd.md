# Translator 流式 wire format 与 max_tokens 映射修复

## Goal

修复 `services/translator/` 层的 7 条 P2 缺陷与 P3 死代码,使跨协议流式工具调用 wire format 正确闭合、max_tokens 双向映射完整、错误事件不丢消息、system prompt 多块不截断、thinking effort 嵌套路径正确解析。

## Background

- 审计报告锚点:`.trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md` §4 P2-1~7、§5 P3 translator 层
- P1-2(content_block_stop 缺失)已在父任务 `07-03-fix-audit-p1-defects` 修复,本任务处理剩余 wire format 与映射缺陷
- 文件:`anthropic_openai.rs`(1667)、`openai_responses.rs`(1186)、`helpers.rs`(337)、`mod.rs`(269)、`native.rs`(93)

## Requirements

### P2 缺陷(7 条)

- **P2-1** `openai_responses.rs:362` ChatToResponses 流式:finish_reason 时发 response.completed 前必须先为每个已打开的 function_call 发 `response.output_item.done`(status=completed)。
- **P2-2** `openai_responses.rs:35` 双向映射 `max_tokens ↔ max_output_tokens`:Chat→Responses 时 max_tokens→max_output_tokens;Responses→Chat 时 max_output_tokens→max_tokens。
- **P2-3** `anthropic_openai.rs:740` ChatToAnthropic:剥离 max_completion_tokens 时映射到 `max_tokens`(Anthropic 必填)。
- **P2-4** `anthropic_openai.rs:540` AnthropicToChat:input_json_delta 在 block_to_tool_index 缺失时回退 tool_index=0 的问题;真实上游总先发 content_block_start,低概率但需防御(记录到 spec 或加防御日志)。
- **P2-5** `helpers.rs:189` anthropic_thinking_to_reasoning_effort:读取嵌套 effort 用 `thinking["output_config"]["effort"]` 而非单点号字面键名 "output_config.effort"。
- **P2-6** `helpers.rs:48` extract_content_text:多块 Anthropic system prompt 被截断;改用 extract_all_text 或拼接全部 text 块。
- **P2-7** `helpers.rs:92` build_error_event openai-chat 分支:丢弃错误消息;改为包含 msg(在 delta 或附加字段中)。

### P3 死代码

- `openai_responses.rs:680` ResponsesToChat 每遇 `event:` 行重置 content_block_index=0(死代码/误导)→ 删除或修正。
- `anthropic_openai.rs:547` AnthropicToChat input_json_delta 累积的 acc.arguments 永不输出(死状态)→ 要么输出,要么删除并注释说明。
- `helpers.rs:16` map_role docstring 承诺 tool→user 但代码 tool→tool,且零生产 call-site → 删除(由 codex-oauth 子任务判断 role_mapping 是否需真正实现;此处先删死代码)。
- `helpers.rs:59` extract_all_text / `helpers.rs:164` extract_delta_text / `helpers.rs:106` is_sse_event_end 三个导出 helper 生产无调用 → 评估:extract_all_text 在修复 P2-6 后会被调用故保留;其余确认无调用则删除。
- `helpers.rs:85` build_error_event anthropic fallback 的 unwrap_or_else 分支不可达(serde_json::to_string 对 &str 不会失败)→ 简化为直接 to_string。

## Design

### max_tokens 映射矩阵

| 方向 | 源字段 | 目标字段 | 说明 |
|------|--------|----------|------|
| Chat → Anthropic | max_completion_tokens / max_tokens | max_tokens | Anthropic 必填 max_tokens;优先 max_completion_tokens,回退 max_tokens,再回退默认值(如 4096) |
| Chat → Responses | max_tokens | max_output_tokens | Responses 用 max_output_tokens |
| Responses → Chat | max_output_tokens | max_tokens | Chat 用 max_tokens |
| Anthropic → Chat | max_tokens | max_tokens | 同名直接保留 |

### ChatToResponses 流式 function_call 闭合时序

```
检测到 finish_reason 时:
  for 每个已打开的 tool_call (context.tool_calls):
    发 response.output_item.done(output_index, item={...status:"completed"})
  发 response.completed(response_id, status, usage)
```

### build_error_event openai-chat 修复

```rust
"openai-chat" => {
    let escaped_msg = serde_json::to_string(msg).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"error\",\"error\":{{\"message\":{}}}}}]}}\n\ndata: [DONE]\n\n",
        escaped_msg
    )
}
```
注:标准 Chat Completions 无 error 字段,但下游客户端(如 agent-switch 调试器)需要消息;加非标准 `error` 字段并在 spec 记录此扩展。

## Acceptance Criteria

- [ ] AC1(P2-1):ChatToResponses 流式含工具调用时,每个 function_call 在 response.completed 前收到 output_item.done
- [ ] AC2(P2-2):max_tokens ↔ max_output_tokens 双向映射,单测验证
- [ ] AC3(P2-3):ChatToAnthropic 请求含 max_completion_tokens 时 max_tokens 被正确设置,单测验证
- [ ] AC4(P2-4):AnthropicToChat 对 block_to_tool_index 缺失的 input_json_delta 有防御(日志或跳过,不串扰工具 0)
- [ ] AC5(P2-5):thinking adaptive+output_config.effort 正确解析嵌套路径,high/medium/low round-trip 单测验证
- [ ] AC6(P2-6):多块 system prompt 不截断,单测验证
- [ ] AC7(P2-7):build_error_event openai-chat 分支包含错误消息
- [ ] AC8(P3):map_role / 不可达 fallback / 死状态等 P3 项处理完成
- [ ] AC9:`cargo check` 0 warning,`cargo clippy --all-targets -- -D warnings` 通过(本子任务范围内)
- [ ] AC10:`cargo test --lib translator` 相关单测通过

## Out of Scope

- map_role 是否需要真正实现角色映射(由 `07-03-fix-codex-oauth-credentials` 子任务决定;本任务只删死代码)
- 跨协议翻译全接线(父任务 out of scope)
