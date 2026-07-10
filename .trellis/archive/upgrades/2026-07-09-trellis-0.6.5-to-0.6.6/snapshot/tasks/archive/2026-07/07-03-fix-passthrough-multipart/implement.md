# 执行计划 — 修复媒体 passthrough multipart 端到端失败(P1-1)

## 前置

- [x] PRD / design 已定。
- [x] 代码事实已复核(208 无条件调用 resolve_and_rewrite,304 passthrough 不可达)。

## 执行步骤

1. **配置上下文(1.3)** — `implement.jsonl` / `check.jsonl`:
   - `implement.jsonl`:`.trellis/spec/guides/app-stack-conventions.md`(管道顺序 + images/audio 媒体透明流转约定)、`.trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md`(P1-1 详情)。
   - `check.jsonl`:同 app-stack-conventions.md。
   - `task.py validate` 通过。

2. **激活任务(1.4)** — `task.py start 07-03-fix-passthrough-multipart`。

3. **实现(2.1)** — 派 `trellis-implement`(或 inline):
   - `src-tauri/src/http/proxy/mod.rs::proxy_request`(208 区):
     - 加 `if is_passthrough { ... 短路构造 ModelMappingResult ... } else { 原 resolve_and_rewrite }`。
     - passthrough 分支:`loop_body_hash` 用 `body_hash`(line 100 已算)。
   - 模型锁检查(228-243):`upstream_model.is_empty()` 时跳过 `get_active_lock`。
   - 确认 304-306 passthrough body 透传现在可达。

4. **单测(2.1)** — `integration_tests.rs` 加 3 测试(design §测试设计)。

5. **质量检查(2.2)** — 派 `trellis-check`:
   ```bash
   cd src-tauri && cargo test --quiet
   cd src-tauri && cargo clippy --quiet -- -D warnings
   cd src-tauri && cargo fmt --check
   npx tsc --noEmit
   npm run build
   ```

6. **Spec 更新(3.3)** — 用 `trellis-update-spec` 在 `app-stack-conventions.md` 管道顺序节加"passthrough 在 model_mapper 之前短路"的执行契约(防止未来回归)。

7. **提交(3.4)** — `fix(proxy): skip model_mapper for passthrough multipart requests (P1-1)`。

## 验证命令

见步骤 5。基线 92 passed,修复后应 ≥ 95(新增 3 测试)。

## 回滚点

- 单文件 mod.rs;`git revert` 单 commit。

## 风险文件

- `src-tauri/src/http/proxy/mod.rs:200-330`(主循环 model_mapper + passthrough 区)。
- `src-tauri/src/http/proxy/integration_tests.rs`(新增测试)。
