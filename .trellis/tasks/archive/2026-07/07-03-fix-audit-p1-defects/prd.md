# P1 缺陷修复(codebase-audit)

## Goal

修复 `codebase-audit` 任务(2026-07-03)发现的、经对抗式验证存活的 5 条 P1 缺陷。本任务为**父任务**,拥有缺陷需求集、子任务映射与最终集成验收,**不直接实现**——实际修复在各子任务中独立完成、独立验收、独立归档。

## Background(缺陷来源)

审计报告:`.trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md`。5 条确凿 P1(报告 §3),均经 2 名 verifier 独立确认存活,严重度为 P1(功能性缺陷,无 P0 级数据丢失/凭据泄露):

| 编号 | 标题 | 文件:行 |
|------|------|---------|
| P1-1 | 媒体 passthrough(image/audio)对 multipart/form-data 端到端失败:model_mapper 在 passthrough 前运行并要求 JSON model key | `src-tauri/src/http/proxy/mod.rs:304` |
| P1-2 | ChatToAnthropic 流式不发 content_block_stop,tool_use input JSON 无法 finalize | `src-tauri/src/services/translator/anthropic_openai.rs:968` |
| P1-3 | 重复 OAuth 登录同一 ChatGPT 账号因 PK 冲突返回 500,新 token 丢弃 | `src-tauri/src/services/codex_oauth.rs:294` |
| P1-4 | 流式链路测试前端 `request()` 对 SSE 强解 JSON,所有 stream=true 测试报错 | `src/lib/api.ts:423` |
| P1-5 | Dashboard widget 无 error 状态,API 失败静默回退空数据并误触发首次引导 | `src/pages/DashboardPage.tsx:43` |

## Task Map(子任务映射)

本任务为父任务。子任务可独立规划/实现/检查/归档,无硬依赖(各自触动不同子系统,互不冲突)。**优先级**:子任务按价值/影响排序,先做核心流式场景(P1-2),再做端到端媒体(P1-1)。

| 子任务 | 修复目标 | 状态 |
|--------|----------|------|
| `07-03-fix-anthropic-streaming-stop` | P1-2(最高优先,跨协议流式工具调用核心场景) | planning |
| `07-03-fix-passthrough-multipart` | P1-1(媒体 passthrough 端到端) | planning |

P1-3 / P1-4 / P1-5 **暂不在本次子任务范围**,待上述两子任务完成、用户确认后再追加子任务。

## Requirements

- 每个子任务须修复报告中对应的 P1,且不引入回归(保持 `cargo test` 全绿、`cargo clippy -D warnings` 0、`tsc` 0 error、`npm run build` 成功)。
- 每个子任务须新增/扩展单测覆盖修复点(报告 §9 退化声明要求流式契约必须有断言测试)。
- 修复须对照 `app-stack-conventions.md` 已固化的契约(§10.1 failover、流式 wire-format)落地。

## Cross-Child Acceptance Criteria(集成验收)

- [ ] P1-2:用一个含工具调用的 mock 上游跑 ChatToAnthropic 端到端流式,断言客户端收到的 SSE 事件序列每个 content_block(start→delta*→stop)闭环;`content_block_stop` 出现在 tool_use 块与 text 块结束处。
- [ ] P1-1:对 `/v1/images/edits`、`/v1/audio/transcriptions` 等 passthrough 端点发 multipart 请求,不再返回 502"请求体缺少 model";原始二进制 body 端到端透传成功。
- [ ] 两子任务修复后,全库 `cargo test` + `cargo clippy -D warnings` + `tsc --noEmit` + `npm run build` 全绿。
- [ ] 父任务集成 review:确认两子任务无交叉副作用、无共享文件冲突、整体质量门通过。

## Out of Scope

- P1-3 / P1-4 / P1-5 及所有 P2/P3(本轮不修,留待后续子任务追加)。
- `cargo fmt` 漂移(已知限制,另项收敛)。
- GUI 桌面运行时实测(需桌面环境)。

## Notes

- 每个子任务按复杂任务处理,需各自的 `prd.md` + `design.md` + `implement.md`。
- 子任务间无依赖;若发现实际有交叉(如同一文件),在子任务 `implement.md` 写明顺序。
