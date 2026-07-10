# Implement Plan: 前端切换器页面

## 执行清单(按序)

### 1. 数据层 `src/lib/api.ts`
- [ ] 加 `Provider` interface + `CreateProviderBody`/`UpdateProviderBody` 类型。
- [ ] 加 `providersApi`(list/create/get/update/remove/switch/reorder),复用 `request<T>`。
- [ ] 在 `api.test.ts` 加 providersApi 的 fetch mock 测试(参考现有 api.test.ts 模式)。

### 2. 文案 `src/lib/presentation.ts`
- [ ] 加 `APP_TYPE_LABELS` / `MODE_LABELS` / `MODE_COLORS`。
- [ ] 在 `presentation.test.ts` 加映射存在性断言(参考现有 presentation.test.ts)。

### 3. 纯函数 `src/pages/providersUtils.ts`
- [ ] 实现 `groupByAppType`/`moveItem`/`canMoveUp`/`canMoveDown`。
- [ ] `providersUtils.test.ts`(Vitest)覆盖:分组正确、sort_index asc + NULLS last、moveItem 上下移 + 边界、canMoveUp/canMoveDown 边界。

### 4. 组件
- [ ] `src/components/providers/ProviderCard.tsx`:props(provider, onSwitch, onEdit, onDelete, onMoveUp, onMoveDown, canUp, canDown);展示 name + category badge(`CATEGORY_LABELS`/`COLORS`)+ mode 标签(`MODE_LABELS`/`MODE_COLORS`)+ 激活态高亮(is_current 边框/底色)+ 操作按钮。
- [ ] `src/components/providers/AppTypeSection.tsx`:props(appType, providers, 上述回调);标题用 `APP_TYPE_LABELS` + 列表按 sort_index。
- [ ] `src/components/providers/ProviderForm.tsx`:创建/编辑表单(name/mode select/category/notes/settings_config JSON textarea);模态或行内,按现有页面习惯。
- [ ] 组件用 Tailwind class + 暗色模式,风格对齐 AccountsPage/EndpointsPage。

### 5. 页面 `src/pages/ProvidersPage.tsx`
- [ ] 两个 `useQuery`(claude-code + codex)+ `groupByAppType`。
- [ ] `useMutation` for switch(含 banner success/warning/error 处理)、reorder、create、update、delete(本地 confirm)。
- [ ] banner 状态:success/warning 3s 自动清,error 常驻。
- [ ] 顶部标题 + 「添加 provider」按钮 + 错误/加载态。

### 6. 路由与导航
- [ ] `App.tsx` 注册 `/providers` → `<ProvidersPage/>`。
- [ ] `AppShell.tsx` `NAV_ITEMS` 在「工具」后插入 `{ path: '/providers', label: '切换器', icon: '🔀' }`(确认图标不与现有「路由」🔀 冲突;冲突则换图标如 🔄)。

### 7. 门禁
- [ ] `npm run build`(tsc --noEmit + vite build)
- [ ] `npm test`(vitest run,含新增 utils + api + presentation 测试)

## 风险文件 / 回滚点

- `App.tsx` / `AppShell.tsx` — 追加路由/导航项,改动小但位置敏感;回滚撤销追加即可。
- `lib/api.ts` / `lib/presentation.ts` — 追加导出,不改现有符号,低风险。
- 新增组件/页面/utils — 隔离新增,零回归风险。
- 回滚:删新增文件 + 撤销 4 处追加。

## review 门

- 纯函数测试先过,再做组件。
- 组件完成后手测(或 RTL 测试关键交互:切换按钮可点、激活态渲染)。
- 门禁全绿才算完成。

## 依赖与前置

- 依赖子任务 3 的 `/api/providers` API(已合并)。
- 不阻塞子任务 6(spec/gates/archive)。
