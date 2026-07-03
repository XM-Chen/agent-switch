# 补全 Dashboard 总览页

## 目标

为 `/` 总览页（`src/pages/DashboardPage.tsx`，当前 5 行 PagePlaceholder 占位）填充真实内容：聚合现有各管理 API 的数据，一屏展示账号/端点/模型/路由计数、工具接管状态、模型自动刷新状态、近期请求日志摘要与端点健康。让用户打开应用即看到全局概览，而非占位文案。

## 背景与边界

- 父任务 `06-26-agent-switch-web-router-mvp` 已归档，8 子任务全部落地，集成验收静态层通过。Dashboard 是 8 页 IA 中唯一仍占位的页面。
- 范围：仅前端，复用现有 API，**不新增后端接口**（技术决策 D1）。
- 现有数据源（已确认契约，`src/lib/api.ts`）：
  - 计数：`accountsApi.list()` / `endpointsApi.list()` / `routesApi.list()` / `modelsApi.list()` → 数组长度。
  - 工具接管：`toolsApi.list()` → 每工具 enabled / live_category / supports_takeover。
  - 自动刷新：`settingsApi.getAutoRefresh()` → enabled / last_sync_at / last_sync_error。
  - 近期日志：`logsApi.list({ limit: 10 })` → status / duration_ms / fallback_chain / created_at / tool（后端默认 limit=50，Dashboard 传 10）。
  - 端点健康：`endpointsApi.list()` 含 enabled；`routesApi.list()` 的 candidates 含 cooldown_until / last_success_at / last_failure_at / last_error_kind。

## 技术决策（参考四项目取最优）

> **原则**：取四项目（ccs/sub2api/9router/cpa）最优解，不取最简也不照搬。

### D1 数据获取 = 纯前端组合，不加后端聚合接口

- **sub2api** 用后端 `DashboardService`+`AggregationService` 重度聚合（成本/用量/趋势/多维度分桶），单接口返回所有统计——**弃**：agent-switch 是单机本地工具，无成本/用量/趋势需求，重聚合是过度工程。
- **cli-proxy-api** 的 TUI dashboard 直接 `len(apiKeys)` 组合已有数据，不调聚合接口——**取**：印证本地工具「前端组合已有数据」最优。
- **9router** 首页=endpoint 页无总览、**cc-switch** 无独立总览页（AppCountBar 嵌面板内）——**弃其不设总览**，我们要独立总览页。
- 落地：前端并行 6 个 TanStack Query（accounts/endpoints/models/routes/tools/logs + settings 共 7 个），本地服务（127.0.0.1）毫秒级延迟无性能问题。MVP 不增加后端契约面；未来若性能/UX 需要，可再加 `GET /api/dashboard/summary` 聚合（暂不纳入）。

### D2 布局 = 响应式网格统计卡（取 sub2api）

- sub2api 用 `grid grid-cols-2 lg:grid-cols-4` 响应式网格展示统计卡——**取**此布局范式。
- 落地：计数卡 4 列（lg）/2 列（sm）网格；下方分区块展示工具接管、自动刷新、端点健康、近期日志。

### D3 状态分桶 = 轻量版分桶（取 sub2api 思路，按本地维度）

- sub2api accounts 按 normal/error/ratelimit/overload 分桶——**取其分桶思路**，但用 agent-switch 本地维度：
  - 端点：启用数 / 禁用数；正常 / 冷却中（cooldown_until 未过期）/ 最近失败（last_failure_at 较近）。
  - 路由：failover_enabled 的路由数。
- 不引入 sub2api 的成本/用量维度（单机工具无此需求）。

### D4 近期日志 = 10 条（sub2api 后端返回列表前端全渲染 → 本地前端取 10）

- sub2api RecentUsage 前端 `v-for` 渲染后端返回的列表（后端控制条数）——**取其「概览精简列表」**，但本地无后端聚合，前端 `logsApi.list({ limit: 10 })` 取 10 条。
- 10 条理由：概览非日志页（LogsPage 默认 50/页带分页），10 条够展示近期趋势又不挤压其它卡片；后端已支持 limit 参数，零改动。

## 需求

### R1 计数卡片（D2 响应式网格）
- R1.1 账号总数 / 端点总数（含启用·禁用分桶）/ 模型总数（custom + synced）/ 路由总数。
- R1.2 每个计数卡可点击跳转对应管理页（`/accounts` / `/endpoints` / `/models` / `/routes`）。

### R2 工具接管状态
- R2.1 列出 Claude Code / Codex / OpenCode 的 enabled 状态与 live_category（指向分类）。
- R2.2 OpenCode 标注「仅手动配置」（supports_takeover=false）。

### R3 模型自动刷新状态
- R3.1 显示自动刷新开关状态、last_sync_at、last_sync_error（有则提示）。
- R3.2 点击跳转 `/settings`。

### R4 近期请求日志摘要（D4，10 条）
- R4.1 最近 10 条请求日志：状态码、tool、duration_ms、fallback_chain（有几跳）、时间。
- R4.2 成功（2xx）/失败（非 2xx 或 error_kind 非空）视觉区分；点击跳转 `/logs`。

### R5 端点健康（D3 轻量分桶）
- R5.1 聚合各路由下端点：正常 / 冷却中（cooldown_until 未过期）/ 最近失败（last_failure_at 较近）；端点启用·禁用分桶。
- R5.2 标识异常端点（冷却中或最近失败），点击跳转 `/endpoints` 或 `/routes`。

### R6 空状态与加载
- R6.1 首次无数据时显示中文引导（如「尚未添加账号，前往账号页添加」+ 跳转按钮），不阻塞主界面。
- R6.2 加载中显示骨架/Spinner。

### R7 一致性
- R7.1 复用现有 `*Api` 与 TanStack Query 范式（queryKey 按资源命名：`['accounts']` / `['endpoints']` / `['models']` / `['routes']` / `['tools']` / `['logs']` / `['auto-refresh']`），不新建 API 客户端。
- R7.2 文案中文，样式遵循现有卡片风格（Tailwind + 现有组件）。

## 验收标准

- [ ] AC1：总览页展示账号/端点（含启用·禁用分桶）/模型/路由四类计数，响应式网格（lg 4 列 / sm 2 列），点击跳转对应页。
- [ ] AC2：展示三工具接管状态（enabled + live_category），OpenCode 标注手动配置。
- [ ] AC3：展示自动刷新状态（开关 + last_sync_at + last_sync_error 提示），点击跳 `/settings`。
- [ ] AC4：展示最近 10 条请求日志摘要（状态/tool/耗时/fallback 跳数/时间），成功失败区分，点击跳 `/logs`。
- [ ] AC5：端点健康分桶（正常/冷却中/最近失败 + 启用·禁用），标识异常端点，点击跳转。
- [ ] AC6：无数据时中文引导 + 跳转，不阻塞主界面；加载中骨架/Spinner。
- [ ] AC7：复用现有 API 与 TanStack Query（7 个 query），无新增后端接口。
- [ ] AC8：`npm run build` 0 error；文案中文；样式遵循现有卡片风格。
- [ ] AC9：质量门——`cargo fmt --check` / `cargo check`（后端无改动应仍 0 error）/ `npm run build`。

## 暂不纳入范围

- Dashboard 数据的自动刷新/轮询（首版静态加载，用户可手动刷新或去各管理页）。
- 计数图表/趋势图（首版用数字卡片，不做可视化图表；sub2api 的 charts/trend 不取）。
- 后端聚合接口 `/api/dashboard/summary`（首版前端组合；未来性能/UX 需要再加）。
- 实时请求统计仪表盘（sub2api realtime metrics 不取）。
- 成本/用量/token 统计（单机本地工具无此需求，sub2api 维度不取）。
