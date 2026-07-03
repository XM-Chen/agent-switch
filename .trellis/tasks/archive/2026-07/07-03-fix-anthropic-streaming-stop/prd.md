# 修复 ChatToAnthropic 流式缺 content_block_stop(P1-2)

> 父任务:`07-03-fix-audit-p1-defects`。审计来源:`codebase-audit` 报告 §3 P1-2。

## Goal

修复 `ChatToAnthropicTranslator::translate_stream_line`(入站 openai-chat → 上游 anthropic 流式翻译)不发 `content_block_stop`(以及 text 块不发 `content_block_start`/`stop`)的缺陷,使符合 Anthropic 流式协议的客户端能正确 finalize 每个 content_block,尤其是 tool_use 块的 input JSON。

## Background(代码事实)

审计报告 P1-2(`anthropic_openai.rs:968`)描述:`finish_reason` 路径(1000-1012)只发 `message_delta`,`[DONE]` 路径(866-868)只发 `message_stop`,均不发 `content_block_stop`。

**代码复核发现范围比报告更广**(子任务须扩大修复):
- `anthropic_openai.rs:943` text 块发 `content_block_delta`(index 0)但**从不发 `content_block_start`(index 0)也不发 `content_block_stop`**。
- `anthropic_openai.rs:972` tool_use 块发 `content_block_start`,但同样**从不发 `content_block_stop`**。
- 全仓 `content_block_stop` 仅在反向 AnthropicToChat(576-579)出现且为跳过。

> 报告 §6 把原 943 条"text 无 content_block_start"标为证伪(survives=false),与代码事实有出入:text 无 start 是**真的**。本子任务以代码事实为准,完整修复 text + tool_use 两类块的 start/stop 闭环。此出入将在子任务完成后回写父任务 spec(见 implement.md)。

## Requirements

- 每个 `content_block_start` 必须有配对的 `content_block_stop`(Anthropic 流式协议契约,见 `app-stack-conventions.md` 流式 wire-format 节)。
- text 块:首个 text delta 前发 `content_block_start`(index 0, type text);流结束(text 结束/finish_reason/message_stop)前发 `content_block_stop`(index 0)。
- tool_use 块:已有 start(972);在流结束(finish_reason/message_stop 路径)前为每个已打开的 tool_use 块发 `content_block_stop`。
- `index` 用 Chat 的 tool_call index(已与 `block_to_tool_index` / `acc` 对齐),text 块 index 0。
- 不破坏现有 `acc.arguments` 累积与 `partial_json` 转义逻辑。
- 不引入回归:`cargo test` 全绿、`cargo clippy -D warnings` 0、`tsc` 0 error、`npm run build` 成功。

## Acceptance Criteria

- [ ] 新增单测:用一个含 tool_use + text 混排的 mock Chat SSE 流喂给 `translate_stream_line`,断言产出的 Anthropic SSE 事件序列满足:每个 content_block 有 `content_block_start` → `delta*` → `content_block_stop` 闭环;tool_use 块的 stop 在 message_stop 前出现。
- [ ] 新增单测:纯 text 流(无 tool_use)断言有 content_block_start(index 0)与 content_block_stop。
- [ ] 现有 `anthropic_openai.rs` 单测全绿,无回归。
- [ ] `cargo test` + `cargo clippy -D warnings` + `tsc --noEmit` + `npm run build` 全绿。

## Out of Scope

- P2-2 / P2-3(max_tokens ↔ max_output_tokens ↔ max_completion_tokens 映射)——另立任务。
- P2-4(input_json_delta 缺 block_to_tool_index 回退 tool_index=0)——另立任务。
- 反向 AnthropicToChat(576-579 跳过 content_block_stop 是正确的,不动)。

## Notes

- 修复须对照 `app-stack-conventions.md`「流式翻译 wire-format 契约(Anthropic 方向)」节落地,该节已记录事件序列契约与 index 规则。
