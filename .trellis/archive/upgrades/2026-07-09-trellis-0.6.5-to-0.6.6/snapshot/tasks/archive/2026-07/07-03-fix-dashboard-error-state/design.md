# 技术设计 — 修复 Dashboard 无 error 态误触发引导(P1-5)

## 边界

主要改 `src/pages/DashboardPage.tsx`:
- `DashboardPage`(39-272):7 个 useQuery 解构 + anyError 聚合 + EmptyGuide 守卫。
- `CountCard`(277-300):props 加 `error?`,渲染逻辑加错误态分支。
- `SectionCard`(304-340):props 加 `error?`,渲染逻辑加错误态分支(替代 children)。

不动:
- `src/main.tsx`(retry:1 保留,error 态触发条件不变)。
- `src/lib/api.ts`(throws 契约不变)。
- `src/components/layout/AppShell.tsx`(不引 banner,理由见下)。
- 其它页面(已按 per-widget 错误渲染约定实现)。

## 错误 UI 方案选择:per-widget vs banner

**选 per-widget 错误渲染(每卡/每区块内联错误指征),不引顶部 banner**。

理由(对照代码事实):
1. **既有约定压倒性一致**:8 处其它 useQuery(Acc/End/Mod/Route/Log/Setting/Tools/Alias)全用 `{error && <p className="text-red-500">加载失败: {error.message}</p>}` 内联渲染,无任何页用顶部 banner、无任何 banner 组件存在。引入 banner 需在 AppShell 加槽位或 DashboardPage 顶部新加聚合组件, departing from 全仓既有模式。
2. **per-widget 更精确**:Dashboard 8 个 widget 各对应单资源 GET(accounts/endpoints/models/routes/tools/logs/autoRefresh,端点健康共享 endpoints+routes),某单个 GET 500 时仅该卡显示错误其余正常,banner 聚合反而模糊「哪一个 GET 坏了」。
3. **最小修复面**:per-widget 只改 DashboardPage.tsx 单文件 + 两个子组件签名;banner 需引入新组件 + AppShell 改动,放大改动面。

补充约定(防 banner 引入诉求):empty 引导由 `anyError` 门加现成 `allLoaded && totalResources === 0` 守,任一 error 即不渲染「请先添加上游账号」误导——这已达成「不掩盖后端失败」的核心修复目标,banner 是可选增强非必需。若后续 review 强诉求再议。

## 当前数据流(DashboardPage)

7 个 useQuery 并行:accounts/endpoints/models/routes/tools/logsResp/autoRefresh。

各 query 当前解构(全部仅 `data` + `isLoading`):

| query | line | queryKey | 消费 widget |
|-------|------|----------|-------------|
| accounts | 43 | `['accounts']` | CountCard「账号」(114) |
| endpoints | 47 | `['endpoints']` | CountCard「端点」(121) + SectionCard「端点健康」(209,共享 routes) |
| models | 51 | `['models']` | CountCard「模型」(128) |
| routes | 55 | `['routes']` | CountCard「路由」(135) + SectionCard「端点健康」(209,共享 endpoints) |
| tools | 59 | `['tools']` | SectionCard「工具接管状态」(146) |
| logsResp | 63 | `['logs']` | SectionCard「近期请求日志」(247),`logs = logsResp?.items ?? []` |
| autoRefresh | 67 | `['auto-refresh']` | SectionCard「模型自动刷新」(163) |

聚合派生:
- `enabledCount` / `disabledCount`(75-76,依赖 endpoints)。
- `customModels` / `syncedModels`(79-80,依赖 models)。
- `failoverRoutes`(83,依赖 routes)。
- `health = aggregateEndpointHealth(endpoints, routes)`(86,依赖 endpoints + routes)。
- `hasAbnormalEndpoint`(87)。
- `totalResources`(90-91,accounts + endpoints + models + routes)。
- `allLoaded`(92-96,**仅检 accounts/endpoints/models/routes 4 个 loading**)。

EmptyGuide 渲染条件(`:108`):`allLoaded && totalResources === 0`。

**error 态下行为(当前 bug)**:某 query error → data fallback `[]`(或 undefined)→ isLoading=false → 4 个 allLoaded 查询若全 non-loading 则 allLoaded=true → totalResources=0(因数组空)→ 渲染 EmptyGuide,误导。CountCard 显示 `value=0` 无错误指征。

## 修复设计

### 1. 7 个 useQuery 补 error 解构

每行解构加 error(命名沿用 `<资源>Error`):

```tsx
const { data: accounts = [], isLoading: accountsLoading, error: accountsError } = useQuery({...});
// 7 个 query 同样补 error
```

### 2. anyError 聚合

```tsx
const anyError = !!(accountsError || endpointsError || modelsError || routesError || toolsError || logsError || autoRefreshError);
```

放在 allLoaded 之后(`:96` 后)。

### 3. EmptyGuide 守卫加 !anyError

`:108` 改为:
```tsx
{!anyError && allLoaded && totalResources === 0 && (
  <EmptyGuide navigate={navigate} />
)}
```

任一 query error 时不渲染欢迎引导。

### 4. CountCard 加 error 态

`CountCardProps` 加 `error?: unknown`(可选):
```tsx
interface CountCardProps {
  title: string;
  value: number;
  loading: boolean;
  sub: string;
  onClick: () => void;
  error?: unknown;
}
```

渲染逻辑(`285-300`)在 `loading` 分支后、children 前加 error 分支:
```tsx
{loading ? (
  <div className="mt-2 h-8 w-12 animate-pulse rounded bg-gray-100 dark:bg-gray-800" />
) : error ? (
  <p className="mt-1 text-sm font-semibold text-red-500" title={String((error as Error)?.message ?? error)}>加载失败</p>
) : (
  <p className="mt-1 text-3xl font-bold">{value}</p>
)}
```

4 个 CountCard 调用处各传 `error={accountsError}` / `endpointsError` / `modelsError` / `routesError`。

### 5. SectionCard 加 error 态

`SectionCardProps` 加 `error?: unknown`:
```tsx
interface SectionCardProps {
  title: string;
  loading: boolean;
  error?: unknown;
  onTitleClick: () => void;
  children: React.ReactNode;
}
```

渲染逻辑(`329-336`):loading → 骨架;error → 错误文本(替代 children);否则 children:
```tsx
{loading ? (
  <div className="space-y-2">...骨架...</div>
) : error ? (
  <p className="text-sm text-red-500">加载失败: {String((error as Error)?.message ?? error)}</p>
) : (
  children
)}
```

4 个 SectionCard 调用处传对应 error:
- 工具接管(`:146`):`error={toolsError}`。
- 模型自动刷新(`:163`):`error={autoRefreshError}`。
- 端点健康(`:209`):`error={endpointsError || routesError}`(共享两 query,任一 error 即该区块不可信)。
- 近期请求日志(`:247`):`error={logsError}`。

### 6. 不变项(明确)

- `logs = logsResp?.items ?? []`(`:72`)。error 态 logsResp 为 undefined → logs=[] 与既有行为一致;SectionCard 由 error 态接管渲染,不再走 children 空数组分支。
- `aggregateEndpointHealth`、派生计数(enabledCount 等)在 error 态下基于空数组计算得 0,但端点健康 SectionCard 由 error 态渲染错误文本,不渲染 children,故 health 的 0 值不展示——避免增加额外守卫。
- 不改 main.tsx retry、不改 api.ts throws。

## 兼容性

- **loading 态不变**:CountCard/SectionCard 的 loading 骨架行为保留。
- **empty 态(后端健康+真无资源)不变**:`anyError=false`、`allLoaded=true`、`totalResources=0` → EmptyGuide 正常渲染(与既有行为一致)。
- **有资源态不变**:`anyError=false`,各 widget 渲染数据。
- **单 query error(其余 ok)**:仅该 widget 显示错误,其余正常,EmptyGuide 不渲染(因 anyError=true,即使 allLoaded 与 totalResources 满足也不渲染——正确,因任何局部失败不应被视为「全空首次引导」)。

## 测试设计

前端无测试运行器(`package.json` 无 `test` script),以静态门 + 桌面 GUI 手动验证为准。

### 静态门(2.2 自动)

```bash
npx tsc --noEmit        # 0 error
npm run build           # tsc --noEmit && vite build 成功
```

### 手动验证(2.2 / 3 验收,桌面 GUI 环境)

1. **场景 A — 后端不可达**:启动 `npm run tauri dev`(后端 42567 启动)+ 手动 kill 后端进程(或改 API_BASE 端口制造 500/连接拒)→ 浏览器刷新 `/`。
   - 期望:4 个 CountCard 显示「加载失败」(非 `0`);4 个 SectionCard 显示「加载失败: {msg}」;**无 EmptyGuide**(无「请先添加上游账号」「前往账号页」按钮)。
2. **场景 B — 后端健康+真无资源**(全新数据库):刷新 `/`。
   - 期望:EmptyGuide 正常渲染「欢迎使用 Agent-Switch / 尚未添加任何账号…前往账号页」(empty 引导未被错误态误触发也未被新守卫误屏蔽)。
3. **场景 C — 后端健康+有资源**:刷新 `/`。
   - 期望:7 个 widget 全部渲染真实数据,无错误指征,无 EmptyGuide。
4. **场景 D — 单 query 500(模拟某 GET 返回 500)**:较难单独造,可在 dev 环境临时改某 api 方法 throw → 仅对应该 widget 显示错误,其余正常,EmptyGuide 不渲染。可选。

### 回归检查

- 其它 8 处 useQuery 页面(已按既有 per-widget 约定)不受本改动影响(本任务只改 DashboardPage.tsx,无共享组件改动——CountCard/SectionCard 是 DashboardPage 内部局部组件,不导出)。
- `npm run build` 全绿保证类型与构建无回归。

## 风险/回滚

- **风险 1(anyError 聚合遗漏 query)**:`allLoaded` 现仅查 4 个 query(accounts/endpoints/models/routes),但 anyError 须聚合全部 7 个(含 tools/logs/autoRefresh),否则 tools/logs/autoRefresh 单独 error 时仍可能误触 EmptyGuide。已在 anyError 定义中包含全部 7 个,review 时校对。
- **风险 2(per-widget error 显示掩盖 children 简化)**:SectionCard 在 error 态跳过 children,可能跳过 children 内的派生计算(如端点健康 health 的空数组)。已分析:派生在 error 态基于空数组得 0,不渲染 children 即不展示,无副作用。
- **风险 3(既有约定偏离)**:若实现者倾向引入 banner(本设计选 per-widget),需重新评估是否破坏 8 处既有 per-widget 一致性。本设计明确选 per-widget,banner 留作可选增强非必需。
- **回滚点**:单文件(DashboardPage.tsx)+ 两局部组件签名;`git revert` 单 commit 即可回滚。
