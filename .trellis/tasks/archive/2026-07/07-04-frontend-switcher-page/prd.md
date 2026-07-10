# 前端切换器页面

## Goal

新增 `/providers` 页面,把子任务 3 的 `/api/providers` HTTP API 暴露成 ccs 式统一切换面:按 app_type 分组展示 provider 列表,每张卡片显示名称/category/mode/激活态,支持切换、上下移排序、CRUD。插入 AppShell 导航但不动现有 8 页。

## Background(已确认事实)

- 技术栈:React 19 + react-router 7 + TanStack Query 5 + Tailwind 4 + Vitest + Testing Library。
- 现有模式(模板):`AccountsPage.tsx` 用 `useQuery`/`useMutation`/`useQueryClient` + `accountsApi` 调 `request<T>`;`AppShell.tsx` 的 `NAV_ITEMS` 数组驱动侧栏;`presentation.ts` 集中 label/color 映射;`dashboardUtils.ts` + `.test.ts` 是纯函数 + Vitest 模式。
- 后端 `/api/providers` 契约(子任务 3 已落地):`GET ?app_type=`、`POST`、`GET/{id}`、`PUT/{id}`、`DELETE/{id}`、`POST/{id}/switch`(返回 `{ warnings: string[] }`)、`POST/reorder`(批量 sort_index)。
- provider 字段:`id/app_type/name/mode(proxy|direct)/settings_config(JSON)/is_current/category/sort_index/notes/created_at/updated_at`。
- `@dnd-kit` 拖拽排序留 P2,本期用上下移按钮。

## Requirements

### R1 路由与导航
- `src/pages/ProvidersPage.tsx` 新建,在 `App.tsx` 注册 `/providers` 路由。
- `AppShell.tsx` 的 `NAV_ITEMS` 插入一项(如 `{ path: '/providers', label: '切换器', icon: '🔀' }`),不动现有 8 项;插入位置排在「工具」之后(语义贴近)。
- 不替换现有 `/tools` 页(接管状态页保留)。

### R2 数据层 `lib/api.ts`
- 加 `Provider` interface(字段对齐后端 `ProviderResponse`)。
- 加 `providersApi`:`list(appType)`、`create(body)`、`get(id)`、`update(id, body)`、`remove(id)`、`switch(id)` → `Promise<{ warnings: string[] }>`、`reorder(items)`。
- 复用现有 `request<T>` 封装,错误风格一致。

### R3 文案 `lib/presentation.ts`
- `APP_TYPE_LABELS`:`{ 'claude-code': 'Claude Code', codex: 'Codex' }`(opencode 后续)。
- `MODE_LABELS`:`{ proxy: '代理', direct: '直连' }` + `MODE_COLORS`(proxy 蓝/direct 紫,与 category 色区分)。
- 复用现有 `CATEGORY_LABELS`/`CATEGORY_COLORS`。

### R4 页面组件树
- `ProvidersPage`:顶部标题 + 「添加 provider」按钮;按 app_type 分两个 `AppTypeSection`(claude-code / codex)。
- `AppTypeSection`:标题(app_type label)+ provider 列表(按 sort_index)+ 拖占位(本期上下移按钮)。
- `ProviderCard`:名称 + category badge + mode 标签 + 激活态高亮(is_current)+ 操作(切换/编辑/删除/上移/下移)。激活态卡片有视觉区分(边框/底色)。
- `ProviderForm`(创建/编辑):name/mode(select proxy|direct)/category/notes + settings_config(JSON textarea,直接编辑模式,非本期重点可简陋)。direct 模式预留 endpoint_id 选择(本期可只接收 JSON 输入,P1 后深度绑定时再做选择器)。

### R5 交互与状态(已定)
- **切换:不二次确认**——切换器核心价值是"点一下即切",按钮清晰可见 + 激活态明确即可,参考 ccs 同款。调 `providersApi.switch`,成功后 invalidate `['providers']`。
- **warnings 展示:成功后有 warnings 则弹可消失提示条**(如"切换成功,但:备份跳过"),无 warnings 静默;**失败用红色错误条常驻**直到下次操作。用页面级 banner/toast 状态管理(不引第三方 toast 库,本地 useState 即可)。
- **排序:上移/下移**调 `providersApi.reorder` 更新 sort_index,invalidate 列表(不做乐观更新,排序频次低,简单为先)。
- **删除:二次确认**(confirm)后调 remove;若删的是 current,前端无需特殊处理(后端已清 tool_takeover)。

### R6 纯函数 `pages/providersUtils.ts` + Vitest
- `groupByAppType(providers)` → 按 app_type 分组并按 sort_index 排序。
- `moveItem(items, fromIndex, toIndex)` → 计算新 sort_index 数组(供 reorder)。
- `canMoveUp/canMoveDown` 边界判断。
- Vitest 覆盖各函数。

### R7 约束
- 不动后端(子任务 3 已就绪)。
- 不动其他页面;`@dnd-kit` 拖拽留 P2。
- 遵循现有 Tailwind class 风格、暗色模式、错误展示模式。

## Acceptance Criteria

- [ ] `/providers` 路由可访问,侧栏新增「切换器」入口,现有 8 页不受影响。
- [ ] 列表按 app_type 分组、按 sort_index 排序展示。
- [ ] ProviderCard 显示名称/category/mode/激活态;当前 current 卡片有视觉区分。
- [ ] 切换 provider:调 switch API,成功后列表激活态更新;失败显示错误;warnings 可见。
- [ ] 上移/下移调 reorder,列表顺序随之变化。
- [ ] 添加/编辑/删除 provider 正常,删除二次确认。
- [ ] `providersUtils.ts` 纯函数 Vitest 全绿。
- [ ] 门禁:`npm run build`(tsc --noEmit + vite build)/ `npm test` 全绿。

## Out of Scope

- `@dnd-kit` 拖拽排序(P2)。
- direct 模式 endpoint_id 选择器(P1 后深度绑定)。
- 后端任何改动。
- 第三方 toast 库(用本地 useState banner)。
