# 执行计划 — 修复 ChatToAnthropic 流式缺 content_block_stop(P1-2)

## 前置

- [x] PRD / design 已定。
- [x] 代码事实已复核(范围比报告更广:text 块也缺 start/stop)。

## 执行步骤

1. **配置上下文(1.3)** — `implement.jsonl` / `check.jsonl` 加真实 spec 条目:
   - `implement.jsonl`:`.trellis/spec/guides/app-stack-conventions.md`(流式 wire-format 契约节)、`.trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md`(P1-2 详情)。
   - `check.jsonl`:同 app-stack-conventions.md。
   - `task.py validate` 通过。

2. **激活任务(1.4)** — review gate 后 `task.py start 07-03-fix-anthropic-streaming-stop`。

3. **实现(2.1)** — 派 `trellis-implement` sub-agent(或 inline):
   - `src-tauri/src/services/translator/mod.rs`:`StreamContext` 加 `text_block_open: bool` 字段,`new()` 初始化 false。
   - `src-tauri/src/services/translator/anthropic_openai.rs::translate_stream_line`:
     - 首次 text delta 前(938-947)发 `content_block_start`(index 0, type text),置 `text_block_open=true`。
     - 新增 `close_open_blocks` helper:发 text 的 stop(若 open)+ 所有已打开 tool_use 的 stop。
     - `finish_reason` 分支(1000)发 message_delta 前调用 close。
     - `[DONE]` 分支(866)发 message_stop 前调用 close。
     - 防重复:text_block_open 置 false;tool_calls 加 closed 标志或 clear。

4. **单测(2.1)** — 在 `anthropic_openai.rs` 的 `#[cfg(test)]` mod 加 3 个测试(design §测试设计)。

5. **质量检查(2.2)** — 派 `trellis-check`:
   ```bash
   cd src-tauri && cargo test --quiet
   cd src-tauri && cargo clippy --quiet -- -D warnings
   cd src-tauri && cargo fmt --check    # 本任务新增代码须 fmt 干净
   npx tsc --noEmit
   npm run build
   ```

6. **Spec 更新(3.3)** — 用 `trellis-update-spec` 把"text 块也缺 start/stop"的代码事实与完整修复契约回写 `app-stack-conventions.md` 流式 wire-format 节(纠正报告 §6 的证伪描述)。

7. **提交(3.4)** — `fix(translator): emit content_block_start/stop in ChatToAnthropic streaming (P1-2)`。

## 验证命令

见步骤 5。基线:`cargo test` 当前 92 passed,修复后应 ≥ 95(新增 3 测试)。

## 回滚点

- 单文件 + mod.rs 一字段;`git revert` 单 commit 即可回滚。
- 若测试发现 index 冲突(text index 0 与某 tool_use index 0 碰撞),回到 design 重评是否需要 index 重映射(P2-4 范围),但优先保持本任务范围最小。

## 风险文件

- `src-tauri/src/services/translator/anthropic_openai.rs:854-1019`(translate_stream_line)。
- `src-tauri/src/services/translator/mod.rs:31-64`(StreamContext)。
