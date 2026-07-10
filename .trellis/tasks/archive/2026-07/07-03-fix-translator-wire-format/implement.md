# Translator 流式 wire format 与 max_tokens 映射修复 - Implement

## 执行顺序

### Step 1: 启动前确认

```bash
python ./.trellis/scripts/task.py current
```

确认活动任务为 `.trellis/tasks/07-03-fix-translator-wire-format`。

### Step 2: 阅读当前实现

精读以下文件全量,记录当前 wire-format 行为:

- `src-tauri/src/services/translator/openai_responses.rs`
- `src-tauri/src/services/translator/anthropic_openai.rs`
- `src-tauri/src/services/translator/helpers.rs`
- `src-tauri/src/services/translator/mod.rs`
- `src-tauri/src/services/translator/native.rs`

### Step 3: P2-1 ChatToResponses function_call 闭合

在 finish_reason 处理前,遍历 `context.tool_calls`,为每个 open call 发 `response.output_item.done`,再发 `response.completed`。

验证:新增单测断言输出事件顺序。

### Step 4: P2-2 / P2-3 max token 映射

- ChatToResponses request: `max_tokens` → `max_output_tokens`
- ResponsesToChat request: `max_output_tokens` → `max_tokens`
- ChatToAnthropic request: 优先 `max_completion_tokens`,否则 `max_tokens`,写入 `max_tokens`

验证:三个单测。

### Step 5: P2-5 thinking effort 嵌套路径

`anthropic_thinking_to_reasoning_effort`:

```rust
thinking.get("output_config").and_then(|c| c.get("effort")).and_then(|e| e.as_str())
```

验证:high/medium/low round-trip 单测。

### Step 6: P2-6 system prompt 多块

把 AnthropicToChat request 的 system 提取改为拼接所有 text block。

可使用 `extract_all_text`,或新增专用函数。验证:多块 system 拼接单测。

### Step 7: P2-7 build_error_event openai-chat

在 openai-chat 分支加入 msg 的 JSON 转义消息字段。

验证:单测断言包含错误消息。

### Step 8: P2-4 input_json_delta 防串扰

AnthropicToChat 收到 input_json_delta 且 `block_to_tool_index` 缺失时,跳过该 delta,不要回退 0。

验证:单测构造缺 block_start 的流。

### Step 9: P3 死代码

- 删除 `map_role` 及仅覆盖它的单测
- 评估 `extract_delta_text` / `is_sse_event_end` 生产调用,无调用则删除
- 处理 `acc.arguments` 死状态
- 处理 `content_block_index` 误导重置

### Step 10: 质量门

```bash
cd src-tauri
cargo check
cargo clippy --all-targets -- -D warnings
cargo test --lib translator
```

注意:本子任务不强求 `cargo fmt --check`,留待 `fix-fmt-spec-alignment` 统一收敛。

### Step 11: 自检

对照 PRD AC1~AC10 逐项确认。

## 风险

- 删除 `map_role` 前再次 grep 全仓 `map_role(`,确认无生产调用,避免误删未来需要接入的 stub。
- max token 默认值选择不要过大,避免上游 400;优先复用请求原值。
