# 技术设计 — 修复 ChatToAnthropic 流式缺 content_block_stop(P1-2)

## 边界

仅改 `src-tauri/src/services/translator/anthropic_openai.rs` 的 `ChatToAnthropicTranslator::translate_stream_line`(854-1019),可能小幅扩展 `StreamContext`(mod.rs:31-64)加一个 text-block-opened 标志。不动反向 AnthropicToChat(576-579 跳过 stop 是正确的)。

## 当前数据流(translate_stream_line)

输入:OpenAI Chat SSE 行(`data: {...}` 或 `data: [DONE]`)。
输出:Anthropic SSE 事件串(可多行,`\n\n` 分隔)。

关键分支:
1. `[DONE]`(866)→ 只发 `message_stop`。**缺**:此前未闭合的 text/tool_use 块。
2. role=assistant 首块(897-935)→ 发 `message_start`,置 `has_content=true`。
3. content delta(938-947)→ 发 `content_block_delta`(index 0)。**缺**:首次 text 前无 `content_block_start`;流结束无 `content_block_stop`。
4. tool_calls delta(950-997)→ 首个 id 时发 `content_block_start`(972);args 时发 `input_json_delta`(990)。**缺**:无 `content_block_stop`。
5. finish_reason(1000-1012)→ 发 `message_delta`。**缺**:此前未闭合块。

## 状态跟踪设计

`StreamContext` 已有:`has_content: bool`、`tool_calls: HashMap<i32, ToolCallAcc>`、`content_block_index: i32`。

新增一个字段 `text_block_open: bool`(默认 false):
- text delta 首次输出前,若 `!text_block_open` → 发 `content_block_start`(index 0, type text),置 `text_block_open=true`。
- text 与 tool_use 混排时,Anthropic 的 index 是全局序号。但当前实现 text 固定 index 0、tool_use 用 Chat tool_call index(可能 >0),存在 index 冲突隐患(报告 P2-4 提及)。
  - **本任务范围决策**:保持 text index=0 不变(与现有实现一致,不扩大到 P2-4 的 index 重映射)。仅在流结束时补 stop。若一个 tool_use 恰好也用 index 0,会与 text 冲突,但这是 P2-4 的范围,本任务不动 index 分配,只补 stop 闭环。
  - 为安全:发 text 的 stop 时只看 `text_block_open` 标志,不依赖 index 唯一性。

tool_use 块闭合:遍历 `context.tool_calls.keys()`,对每个已打开(id 非空)的 tool_use 发 `content_block_stop`(index = 该 tool_call 的 index)。

## 闭合时机

在 `finish_reason` 分支(1000)发 `message_delta` **之前**,以及在 `[DONE]` 分支(866)发 `message_stop` **之前**,插入闭合逻辑:

```
fn close_open_blocks(context, output) {
    if context.text_block_open {
        output += content_block_stop(index=0)
        context.text_block_open = false
    }
    for tc_index in context.tool_calls.keys() where acc.id 非空 {
        output += content_block_stop(index=tc_index)
    }
    // 标记已闭合,避免重复(可选:clear tool_calls 或加 closed 标志)
}
```

两处调用点:`[DONE]` 分支前、`finish_reason` 分支前。考虑流可能先 finish_reason 后 [DONE],第一次 close 后第二次不应重复发——用 `text_block_open=false` 与对 tool_calls 加 `closed` 标志(或 clear)防重复。

## 事件序列契约(对照 app-stack-conventions.md)

修复后每个块:`content_block_start` → `delta*` → `content_block_stop`。
- text 块:`start(text,index0)` → `text_delta*` → `stop(index0)`。
- tool_use 块:`start(tool_use,id,name,index=N)` → `input_json_delta*` → `stop(index=N)`。
- 流尾:`message_delta`(finish_reason) → `message_stop`([DONE])。

## 兼容性

- 不改 `translate_request`(非流式)。
- 不改 `acc.arguments` 累积、`partial_json` 转义(988)。
- 不改反向 AnthropicToChat。
- `StreamContext::new` 加 `text_block_open: false` 默认;反向 translator 不用此字段(共享 struct,加字段需确认反向不受影响——反向用各自的 context 实例,不共享,安全)。

## 测试设计

新增 `#[test]` in `anthropic_openai.rs` 的 `#[cfg(test)]` mod:
1. `chat_to_anthropic_stream_text_only_closes_block`:构造纯 text Chat SSE 序列(role+content → content deltas → finish_reason → [DONE]),喂 `translate_stream_line`,收集输出,断言含 `content_block_start`(index 0, text)与 `content_block_stop`(index 0),且 stop 在 message_stop 前。
2. `chat_to_anthropic_stream_tool_use_closes_block`:构造含 tool_calls 的流(start+args delta → finish_reason → [DONE]),断言每个 tool_use 块有 `content_block_stop`(index 匹配)在 message_stop 前。
3. `chat_to_anthropic_stream_mixed_text_and_tool`:text + tool_use 混排,断言两类块都闭环、顺序正确。

## 风险/回滚

- 风险:闭合逻辑在 finish_reason 与 [DONE] 两处调用,若防重复不当会发双 stop。用 `text_block_open` + tool_calls closed 标志防之,单测覆盖"finish_reason 后再 [DONE]"场景。
- 回滚点:`git revert` 单 commit;修复局限单文件 + mod.rs 一字段。
