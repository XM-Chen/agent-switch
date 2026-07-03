# 修复 Dashboard 无 error 态误触发引导(P1-5)

> 父任务:`07-03-fix-audit-p1-defects`。审计来源:`codebase-audit` 报告 §3 P1-5。

## Goal

修复 `src/pages/DashboardPage.tsx` 中 7 个 `useQuery` 全部仅解构 `data`/`isLoading`、从不解构 `error`/`isError` 的缺陷:后端不可达或某 GET 返回 500 时,Dashboard 静默回退为空数据,所有统计卡渲染 `value=0`,并误触发「首次无数据引导」(EmptyGuide)误导运维。修复后每个 widget 显式渲染错误态,EmptyGuide 在任一查询 error 时不再触发,后端失败对运维可见。

## Background(代码事实)

审计报告 P1-5(`DashboardPage.tsx:43`)描述:7 个 useQuery 仅解构 `data: X = []` 与 `isLoading`,从不解构 `error`/`isError`;CountCard(285-300)仅判 `loading`;EmptyGuide 守卫(90-96, 108)为 `allLoaded && totalResources === 0`,无 `!anyError` 门。

**代码逐行复核(全部锚点确认存活)**:

- **7 个 useQuery**(`DashboardPage.tsx:43-70`):
  - `:43` accounts(`['accounts']`, `accountsApi.list`)
  - `:47` endpoints(`['endpoints']`, `endpointsApi.list`)
  - `:51` models(`['models']`, `() => modelsApi.list()`)
  - `:55` routes(`['routes']`, `routesApi.list`)
  - `:59` tools(`['tools']`, `toolsApi.list`)
  - `:63` logsResp(`['logs']`, `() => logsApi.list({ limit: 10 })`)
  - `:67` autoRefresh(`['auto-refresh']`, `settingsApi.getAutoRefresh`)
  - 全部仅 `data: X = []` + `isLoading: xLoading`,无 `error`/`isError`。**确认**。
- **CountCard**(`:277-300`):props `{ title, value, loading, sub, onClick }`,无 `error` prop;`loading` 时渲染骨架、否则渲染 `value`(错误态下 `value=0`)。**确认**。
- **SectionCard**(`:304-340`):props `{ title, loading, onTitleClick, children }`,无 `error` prop;`loading` 时骨架、否则渲染 children。**确认**。Dashboard 用 4 个 SectionCard:工具接管(`:146`)、模型自动刷新(`:163`)、端点健康(`:209`)、近期请求日志(`:247`)。
- **EmptyGuide 守卫**(`:90-96, 108`):
  - `:90-91` `totalResources = accounts.length + endpoints.length + models.length + routes.length`。
  - `:92-96` `allLoaded = !accountsLoading && !endpointsLoading && !modelsLoading && !routesLoading`(**只查 4 个查询**;tools/logs/autoRefresh 不计入 allLoaded)。
  - `:108` `{allLoaded && totalResources === 0 && <EmptyGuide navigate={navigate} />}`。无 `!anyError` 门。**确认**。
- **全局配置**:`main.tsx:11` `retry: 1` → 重试 1 次后 `status='error'`;`api.ts:50-52` 非 2xx 抛 `Error("${status}: ${body}")`,网络拒收 fetch 直接 reject。

**与审计报告一致,无锚点出入**。

**既有约定(代码事实)**:除 DashboardPage 外,`AccountsPage.tsx:7,54` / `EndpointsPage.tsx:7,43` / `ModelsPage.tsx:9,67` / `RoutesPage.tsx:19,34` / `LogsPage.tsx:42,123` / `SettingsPage.tsx:8,27` / `ToolsPage.tsx:7,20` / `AliasPanel.tsx:9,57` 共 8 处 useQuery 全部解构 `{ data, isLoading, error }` 并渲染:

```tsx
{isLoading && <p className="text-gray-500">加载中...</p>}
{error && <p className="text-red-500">加载失败: {error.message}</p>}
```

DashboardPage 是唯一违反此约定的页面(漏 `error` 解构与渲染)。本任务**沿用既有 per-widget 错误渲染约定**,不新引入 banner 组件(理由见 design.md)。

## Requirements

- 每个 useQuery 显式解构 `error`(与既有约定一致),DashboardPage 内部聚合 `anyError = accountsError || endpointsError || modelsError || routesError || toolsError || logsError || autoRefreshError`。
- CountCard 新增错误态:传入 `error?: unknown` prop,`error` truthy 时渲染错误指征(如 `加载失败` 或 `—`),不再渲染 `value=0` 误导。
- SectionCard 新增错误态:传入 `error?: unknown` prop,`error` truthy 时渲染「加载失败: {error.message}」而非 children 骨架/空。
- EmptyGuide 守卫增加 `!anyError` 与 `allLoaded && totalResources === 0` 并列(`:108`):`{!anyError && allLoaded && totalResources === 0 && <EmptyGuide />}`。任一查询 error 时不渲染欢迎引导,避免误触发「请先添加上游账号」。
- 后端健康 + 真实无资源时,empty 态与 loading 态行为不变(不引入回归)。
- 不改 `main.tsx` retry 配置(retry:1 保留)、不改 `api.ts` 错误抛出契约。
- 不引入新的 banner 组件或全局错误 HOC(保持既有 per-widget 约定,见 design.md)。
- 不掩盖后端失败:任一 GET 失败时,对应 widget 显示错误信息,而非静默回退为空数据。

## Acceptance Criteria

- [ ] AC1:停止后端(或断网)后刷新 Dashboard,7 个 query 进入 error 态,**不渲染 EmptyGuide**(无「请先添加上游账号」「前往账号页」误导)。
- [ ] AC2:4 个 CountCard(账号/端点/模型/路由)在对应 query error 时渲染错误指征(如 `加载失败`/`—`),而非 `value=0`。
- [ ] AC3:4 个 SectionCard(工具接管/模型自动刷新/端点健康/近期日志)在对应 query error 时渲染「加载失败: {error.message}」,而非 children 空骨架。
- [ ] AC4:后端健康且真实无资源时,EmptyGuide 仍正常渲染(不误屏蔽empty 引导)。
- [ ] AC5:后端健康且有资源时,所有 widget 正常渲染数据(loading 短暂/正常态不变)。
- [ ] AC6:`npx tsc --noEmit` 0 error;`npm run build`(`tsc --noEmit && vite build`)成功。
- [ ] AC7(手动,前端无测试框架):桌面 GUI 环境 `npm run tauri dev`,停止后端 42567 或断网后刷新 `/`,目视确认以上 AC1-AC3;恢复后端后目视确认 AC4-AC5。

## Out of Scope

- **Dashboard 占位/半成品不修(journal Session 7 + 审计 §7 已记录)**:Dashboard 总览页设计来自 `06-29-dashboard-overview` 任务,Session 7 注「部分统计/可视化尚未落地」,属已知占位项:无自动刷新/轮询(首版静态加载)、无计数图表/趋势图、无后端 `/api/dashboard/summary` 聚合接口、无实时请求统计仪表盘、无成本/用量/token 统计。本任务仅修「错误态静默误导」这一**真实 bug**,不补占位功能。
- 审计 §5 P3 项(DashboardPage.tsx:17 TOOL_LABELS/CATEGORY_LABELS/CATEGORY_COLORS 重复、DashboardPage.tsx:595 formatTime 与 LogsPage 重复、DashboardPage.tsx:53 queryFn 包裹不一致、DashboardPage.tsx:64 logs queryKey 形态不一致)——代码质量观察项,不在本任务修复范围。
- 其它 P1(P1-1 媒体 passthrough / P1-2 ChatToAnthropic 流式 / P1-3 重复 OAuth / P1-4 流式测试强解 JSON)——各独立子任务。
- 其它 P2(如 P2-18 LogsPage 生产日志过滤)——另立任务。
- 不引入全局 error boundary 或 toast/banner 系统(框架未已有,新引超出修复范围)。

## Notes

- 对照 `.trellis/spec/guides/app-stack-conventions.md`「前端 API 客户端约定」节:queryKey 按资源命名(`['accounts']`/`['endpoints']`/`['models']`/`['routes']`/`['tools']`/`['logs']`/`['auto-refresh']`),错误抛 `Error("${status}: ${body}")`。本任务不新增 queryKey、不改抛错契约,仅在消费侧补 `error` 解构与渲染。
- TanStack Query v5 错误态约定:`useQuery` 返回 `{ status, data, error, isError, isLoading, isPending, isSuccess }`;`retry:1` 重试 1 次失败后 `status='error'`、`isError=true`、`error` 为 thrown 对象、`data` 保持上一次值(首屏无缓存即 fallback 到组件内 `= []` 默认)。Dashboard 现存代码 `data: X = []` 的默认值 fallback 在 error 态下正是把错误掩盖成「空数组」的根因,需靠 isError 判断显式区分「真无数据」与「错误态空」。
- 前端无测试运行器(`package.json` scripts 仅 `dev`/`build`/`preview`/`tauri`,无 `test`):验收以 `tsc --noEmit` + `npm run build` 全绿 + 桌面 GUI 手动目视为准。不新增单元测试。
