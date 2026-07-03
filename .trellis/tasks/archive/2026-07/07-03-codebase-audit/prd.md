# 全代码库彻底审查

## Goal

对 agent-switch 应用(Rust 后端 `src-tauri/` + React 前端 `src/`)做一次彻底的代码审查,产出按严重度排序的**缺陷报告(仅报告,不改代码)**,覆盖所有子系统与跨层契约,为后续修复提供高置信度清单。

## Background(上轮验证已确认的事实)

- 自动化门当前状态:`cargo test` 92 passed / 0 failed;`cargo clippy -D warnings` 0 warning;`tsc --noEmit` 0 error;`npm run build` 成功;`cargo fmt --check` **失败**(约 10 处漂移,集中在 translator/db/proxy 近期修复 commit)。
- 前端**无测试框架**(`package.json` 未配置任何测试运行器),仅 `tsc` + `vite build`。
- GUI 端到端运行时实测从未做过(journal 原文"留给桌面环境")。
- journal 已记录的**已知限制**:Dashboard 占位、跨协议翻译未接线、`role_mapping` 简化 stub。
- **spec 层错位**:`.trellis/spec/backend`、`/frontend` 的 index 描述的是 Trellis 工具层(`.trellis/scripts/`),不是 agent-switch 应用本体;仅 `guides/app-stack-conventions.md` 是应用相关。

## Scope / 子系统

**Rust 后端(~14k LOC):**
- `services/translator/`:`anthropic_openai.rs`(1667)、`openai_responses.rs`(1186)、`native.rs`、`helpers.rs`、`mod.rs` —— 双向流式 SSE 翻译、工具调用 index 映射、JSON 转义(近期密集修复区)。
- `http/proxy/`:`mod.rs`(823)、`failover.rs`、`oauth_refresh.rs`、`sse.rs`、`capability.rs`、`translate.rs`、`stream_guard.rs`、`integration_tests.rs`。
- `services/`:`codex_oauth.rs`(444)、`model_sync.rs`(266)、`portability/`(mod 479 + apply 444 + collect 246 + crypto_box 184)、`tool_takeover/`(310)。
- `db/`:`migrations.rs`(332)、`dao/`(endpoints、endpoint_models、accounts、request_logs)。
- `http/api/`:`routes.rs`、`endpoints.rs` 等;`commands/`、`config/`、`app_state.rs`、`lib.rs`、`main.rs`。

**React 前端(~3.4k LOC):**
- `pages/`:`DashboardPage`(605)、`RoutesPage`(401)、`SettingsPage`(339)、`LogsPage`(339)、`EndpointsPage`(230)、`AccountsPage`(196)、`ModelsPage`(151)、`ToolsPage`。
- `lib/api.ts`(428)、`components/`(models、tools、layout)。

**跨层契约:** 前端 `api.ts` ↔ Tauri `commands` ↔ Rust `services`/`db`;translator 双向流式 SSE;portability 导入导出格式;failover 错误分类(`should_failover`)。

## Requirements

- 按"先扇出各子系统审查员 → 对每条发现做对抗式验证剔除误报 → 汇总排序"的多智能体工作流执行。
- **发现门槛(主报告)**:只报经对抗式验证的确凿缺陷——逻辑错误/崩溃、安全与凭据问题、数据损坏/丢失、资源泄漏/并发竞争。每条含:标题、严重度(P0/P1/P2/P3)、`file:line`、触发/复现条件、建议修复方向。
- **代码质量附录(次级,不进主排序、不需对抗验证)**:可简化、重复模式、效率问题、死代码——单独成节,供后续 `/simplify` 处理。
- P0/P1 发现须经独立验证者投票(对抗式,默认为"误报"直到证实)。
- 跨层契约检查内置进相关子系统审查员 prompt(前端 api.ts ↔ commands ↔ services/db、translator 双向 SSE、portability 格式、failover 错误分类),不单独做 spec 合规层(因 spec index 多数错位)。
- 已知限制(journal 记录的 3 项 + spec 错位)单独成节,不与新发现混排。
- 报告持久化到任务 `research/audit-report.md`。

## Acceptance Criteria

- [ ] 产出按严重度排序的缺陷报告,每条含标题/严重度/`file:line`/触发条件/修复方向。
- [ ] P0/P1 发现均经对抗式验证,误报已剔除。
- [ ] 覆盖 Scope 中列出的全部子系统;若某子系统因规模被采样,需在报告中标注覆盖比例。
- [ ] 已知限制与新发现分节呈现。
- [ ] 报告写入 `research/audit-report.md`(或同目录)。

## Out of Scope

- 修复代码(本轮仅报告;修复另建 Trellis 实现任务)。
- GUI 桌面运行时实测(需桌面环境)。
- `.trellis/` 工具层(Python runtime)自身代码审查。
