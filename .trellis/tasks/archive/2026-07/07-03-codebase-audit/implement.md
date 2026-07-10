# 执行计划 — 全代码库彻底审查

> 本任务的"实现"= 运行审查工作流并生成报告,不写应用代码。

## 前置(已完成)

- [x] `cargo fmt --check` 失败已知(约 10 处漂移)——作为 P3 质量发现纳入报告,不提前修。
- [x] PRD / design 已定:门槛 = 正确性+安全+数据损坏,P3 质量附录。
- [x] 维度划分 10 个 finder(见 design.md)。

## 执行步骤

1. **配置上下文(1.3)** — 在 `implement.jsonl` / `check.jsonl` 写入真实 spec/research 条目(替换 `_example` 种子行):
   - `implement.jsonl`:指向 `guides/app-stack-conventions.md`(应用技术栈/路径隔离契约)、`guides/cross-layer-thinking-guide.md`、`guides/code-reuse-thinking-guide.md`。
   - `check.jsonl`:指向 `guides/quality-guidelines.md`(若适用)或同类。
   - 运行 `task.py validate` 确认通过。

2. **激活任务(1.4)** — review gate 后 `python ./.trellis/scripts/task.py start`,状态 → in_progress。

3. **运行审查工作流** — 调用 `Workflow` 执行 design.md 的三段式:
   - `pipeline(DIMENSIONS, find_stage, verify_stage)` — 10 finder 并行,每条 P0/P1 finding 即时派 2 verifier 对抗验证。
   - Phase 3:synthesize agent 汇总去重、排序、写 `research/audit-report.md`。
   - 工作流脚本内联;finder/verifier/synthesizer 用 `schema` 强制结构化输出。

4. **质量检查(2.2)** — 工作流返回后:
   - 确认报告含全部章节(摘要/主表/附录/已知限制/覆盖矩阵/退化声明)。
   - 确认每条 P0/P1 有验证 verdict。
   - 确认无子系统遗漏(覆盖矩阵齐全)。

5. **Spec 更新(3.3)** — 若审查中发现值得固化的契约(如 failover 错误分类、translator index 映射),用 `trellis-update-spec` 写入 `guides/app-stack-conventions.md` 或新建 spec。

6. **提交(3.4)** — 提交 PRD/design/implement/report 等 planning + 产出文件,conventional commit。

## 验证命令(工作流前后)

```bash
cd src-tauri && cargo test --quiet          # 基线:应保持 92 passed
cd src-tauri && cargo clippy --quiet -- -D warnings
npx tsc --noEmit
npm run build
cd src-tauri && cargo fmt --check           # 预期仍失败(本轮不修)
```

## 回滚点

- 工作流仅读代码 + 写 `research/`,不改应用代码 → 无需应用层回滚。
- 若工作流产出异常(空报告/大量误报),检查 `<transcriptDir>/journal.jsonl` 各 agent 返回值,调整 finder prompt 后用 `Workflow({scriptPath, resumeFromRunId})` 续跑。

## 风险文件(审查重点,非修改)

- `services/translator/anthropic_openai.rs`(1667)、`openai_responses.rs`(1186)——近期密集修复,最易藏回归。
- `http/proxy/oauth_refresh.rs`——凭据解密,安全敏感。
- `db/dao/endpoint_models.rs` + `services/model_sync.rs`——锁与事务,近期改过。
