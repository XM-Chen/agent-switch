# 前端死代码 Dashboard 占位与测试框架修复

## Goal

修复前端层 P2/P3 缺陷和已知限制:生产/测试日志过滤语义、Dashboard 占位/重复代码、死组件/死工具函数、queryKey 形态不一致,并引入基础前端测试框架以覆盖关键纯函数和 API 参数构造。

## Background

- 审计报告锚点:`.trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md` §4 P2-18, §5 Frontend 与 frontend-dashboard
- P1-4/P1-5 已修复,但前端仍缺测试框架,存在死代码/重复逻辑
- 文件:`DashboardPage.tsx`, `LogsPage.tsx`, `RoutesPage.tsx`, `ToolCard.tsx`, `PagePlaceholder.tsx`, `utils.ts`, `vite.config.ts`, `package.json`

## Requirements

### P2

- **P2-18** `LogsPage.tsx:31`:选择"生产"日志时必须排除 `tool='test'` 的测试日志。当前 `effectiveTool=undefined` 导致 GET `/api/logs` 不过滤,生产视图混入测试日志。

### P3 / 前端质量项

- `PagePlaceholder.tsx`:无引用死组件,删除或接回。
- `src/lib/utils.ts` 的 `cn`:无引用死函数,删除或在组件中实际使用。
- Dashboard 与 ToolCard 重复 `TOOL_LABELS` / `CATEGORY_LABELS` / `CATEGORY_COLORS`:提取到共享模块。
- Dashboard 与 LogsPage 重复 `formatTime`:提取到共享模块。
- Dashboard `logs` queryKey `['logs']` 与 LogsPage `['logs', params]` 不一致:改为带参数 key。
- Dashboard 部分统计/可视化为占位:应落地为基于现有 API 数据的真实统计,或明确显示"暂无该统计数据"而不是伪造。
- RoutesPage 连续点击同一测试按钮的低置信观察项:确认是否存在并修复(如 pending guard)。

### 前端测试框架

- 引入 Vitest + React Testing Library 或等价轻量测试框架。
- 最小覆盖:
  - LogsPage 参数构造:生产过滤排除 test(日志 API 参数或前端二次过滤)
  - Dashboard 纯函数:健康聚合、fallback hop 计数、时间格式化
  - 共享 label/format 工具无重复
- `npm test` 或 `npm run test` 可运行。

## Design

### 生产日志过滤

优先方案:后端 API 增加 `exclude_tool=test` 更精确;但这是前端子任务,若后端当前不支持,前端可在 `logType === 'production'` 时请求所有非 test 生产项并本地过滤。若日志量分页下本地过滤会影响 total,更正确是修改后端 `logsApi.list` 支持 `log_type=production/test` 或 `excludeTool` 参数。

推荐实现:
1. `logsApi.list` 增加 `log_type?: 'production' | 'test'`
2. 后端 `/api/logs` 支持 `log_type`:test 等价 `tool='test'`;production 等价 `tool!='test'`
3. LogsPage 传 `log_type`,不再通过 tool 复用测试/生产语义

如为了最小改动先前端过滤,需在 PRD/实现说明记录分页 total 限制。

### 共享前端常量与工具

新增:
- `src/lib/toolLabels.ts` 或 `src/lib/presentation.ts`:工具标签、分类标签、颜色
- `src/lib/format.ts`:formatTime, fallback count 等纯函数
- `src/pages/dashboardUtils.ts`:Dashboard 专属纯函数(便于测试)

## Acceptance Criteria

- [ ] AC1(P2-18):日志页选择"生产"不会显示 `tool='test'` 日志,total/分页语义正确或限制已说明
- [ ] AC2:PagePlaceholder 与 cn 死代码处理完成,无未引用文件/函数
- [ ] AC3:Dashboard/ToolCard 重复标签常量消除
- [ ] AC4:Dashboard/LogsPage 时间格式化重复消除
- [ ] AC5:Dashboard logs queryKey 带参数且不与 LogsPage 语义冲突
- [ ] AC6:Dashboard 占位项落地为真实统计或明确空态说明
- [ ] AC7:前端测试框架引入,`npm run test` 可运行
- [ ] AC8:至少覆盖 LogsPage 过滤与 Dashboard 纯函数
- [ ] AC9:`npm run build` 通过

## Out of Scope

- 大规模 UI 视觉重设计
- e2e 浏览器自动化(可后续单独引入 Playwright)
