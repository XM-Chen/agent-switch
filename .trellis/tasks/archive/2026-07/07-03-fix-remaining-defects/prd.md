# 修复审计剩余缺陷(P2/P3/有界项)

## Goal

将 2026-07-03 代码库审计报告中存活的所有有界缺陷(P2×24 + P3×40)以及 cargo fmt、Dashboard 占位、role_mapping stub、spec 错位、前端测试框架缺失等已知限制全部修复收敛,使第一版 MVP 达到可发布质量。跨协议翻译全接线作为独立特性任务,不在本父任务范围。

## Background

- 审计报告:`.trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md`
- P1(5 条)已在 `07-03-fix-audit-p1-defects` 父任务中全部修复并归档
- 本父任务处理剩余 P2/P3 与有界项,覆盖 6 个子系统
- 用户决策(2026-07-03):有界项全修,跨协议翻译全接线单独规划;采用父任务+子系统子任务结构

## 范围划分(6 个子任务)

| 子任务 | 覆盖缺陷 | 主要锚点文件 |
|--------|----------|-------------|
| `07-03-fix-translator-wire-format` | P2-1~7, P3 translator 死代码 | `anthropic_openai.rs`, `openai_responses.rs`, `helpers.rs`, `mod.rs`, `native.rs` |
| `07-03-fix-proxy-oauth-failover` | P2-8~13, P3 proxy 死代码 | `proxy/mod.rs`, `failover.rs`, `oauth_refresh.rs`, `sse.rs`, `stream_guard.rs` |
| `07-03-fix-db-portability` | P2-14~19, P3 db/portability 死代码 | `endpoint_models.rs`, `request_logs.rs`, `portability/{mod,apply,crypto_box}.rs`, `model_sync.rs` |
| `07-03-fix-codex-oauth-credentials` | P2-21~24, role_mapping stub, P3 codex 死代码 | `codex_oauth.rs`, `http/api/auth.rs`, `config/paths.rs`, `helpers.rs:16 map_role` |
| `07-03-fix-frontend-deadcode-tests` | P2-18, P3 frontend 死代码, Dashboard 占位, 前端测试框架 | `DashboardPage.tsx`, `LogsPage.tsx`, `RoutesPage.tsx`, `PagePlaceholder.tsx`, `utils.ts` |
| `07-03-fix-fmt-spec-alignment` | cargo fmt, spec 错位, P2-9/10/20 | `src-tauri/**`, `.trellis/spec/{backend,frontend,guides}/**` |

## 跨子任务约束

- **cargo fmt 收敛**统一在 `07-03-fix-fmt-spec-alignment` 中执行,其他子任务修改 Rust 代码后只需保证自身 `cargo check`/`clippy` 通过,不强制 `cargo fmt --check`(避免互相覆盖);最终由 fmt 子任务统一收敛。
- **spec 错位修正**涉及 spec 树重构(backend/frontend index 当前描述 Trellis 运行时/平台适配层,与 agent-switch 应用本体混淆),由 `07-03-fix-fmt-spec-alignment` 统一重构,其他子任务如有新学到的约定,先记入 `guides/app-stack-conventions.md`,最后由 spec 子任务归位。
- **role_mapping stub** 在 `helpers.rs:16`,与 translator 子任务 P3 死代码修复区域重叠,但属"简化 stub"而非死代码,故归入 `07-03-fix-codex-oauth-credentials`(凭据/账号语义层),由该子任务判断是否需要真正实现还是保持 stub 并补充文档。
- 子任务间允许并行,但修改同一文件时需注意:`helpers.rs` 同时被 translator 和 codex-oauth 子任务触及,后者只动 map_role。

## Requirements

- R1: 审计报告 §3 P2 表(24 条)所列每条缺陷必须修复或明确记录"不修复理由"并在 PRD 中说明。
- R2: 审计报告 §5 P3 附录(40 条)所列死代码必须删除或接回生产调用,不得保留无主引用。
- R3: `cargo fmt --check` 在 `src-tauri/` 全量通过。
- R4: `cargo clippy --all-targets -- -D warnings` 通过。
- R5: `cargo check` 0 warning。
- R6: `npm run build`(含 `tsc --noEmit`)通过。
- R7: `.trellis/spec/backend/index.md` 与 `.trellis/spec/frontend/index.md` 描述 agent-switch 应用本体(Rust HTTP 服务 + React 前端),Trellis 运行时/平台适配层规范迁移到独立位置或明确分区。
- R8: Dashboard 占位功能落地或明确标注"占位待实现"且不误导运维。
- R9: 前端引入测试框架(如 Vitest)或明确记录"无测试框架"决策与手动回归清单。
- R10: role_mapping 要么实现完整角色映射,要么在 spec 中记录"简化 stub"的边界与触发条件。

## Acceptance Criteria

- [ ] AC1: 审计报告 24 条 P2 全部修复(每条在对应子任务 PRD 的 AC 勾选)
- [ ] AC2: 审计报告 40 条 P3 全部处理(删除或接回)
- [ ] AC3: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo check` 三连通过,0 warning
- [ ] AC4: `npm run build` 通过
- [ ] AC5: spec 树错位修正完成,backend/frontend index 描述应用本体
- [ ] AC6: Dashboard 占位项落地或标注
- [ ] AC7: 前端测试框架决策落地
- [ ] AC8: role_mapping 处理落地
- [ ] AC9: 全部 6 个子任务归档后,父任务归档

## Out of Scope

- 跨协议翻译全接线(特性级工作,范围未定,单独规划)
- 自动测速、成本优化、智能调度(第一版不纳入)
- 多用户登录与云端同步
- 订阅源导入与批量转换

## Open Questions

- 无(范围与方式已由用户决策确定)
