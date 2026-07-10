# Design: 前端切换器页面

## 架构与边界

新增前端页面,纯消费子任务 3 的 `/api/providers` REST API,不动后端。

```
App.tsx
  └─ <Route path="/providers"> <ProvidersPage/> </Route>
AppShell.tsx
  └─ NAV_ITEMS 插入「切换器」(工具之后)
src/pages/ProvidersPage.tsx          ← 页面容器(状态 + banner)
  ├─ providersUtils.ts               ← 纯函数(分组/排序/边界)
  ├─ components/AppTypeSection.tsx
  │    └─ components/ProviderCard.tsx
  └─ components/ProviderForm.tsx      ← 创建/编辑(模态或行内)
src/lib/api.ts                       ← + Provider interface + providersApi
src/lib/presentation.ts              ← + APP_TYPE_LABELS / MODE_LABELS / MODE_COLORS
```

边界:
- 不复用 `/tools` 页(接管状态页语义不同,保留)。providers 页是"切换面",tools 页是"检测/状态面"。
- 组件按 app_type 分组,不引入虚拟化(列表小)。

## 数据流

- **读**:`useQuery({ queryKey: ['providers'], queryFn: () => providersApi.list('claude-code') })` × 2 app_type,或一次拉两个 app_type 再 `groupByAppType`。选后者更省请求——但后端 `list` 按 app_type 过滤,需两次请求。**决定:两次 `useQuery`(claude-code + codex),共享 queryKey 前缀便于 invalidate。**
- **切换**:`useMutation(providersApi.switch)` → `onSuccess`:若有 warnings 设 banner(warning 色,3s 后自动清);`onSettled`:invalidate `['providers']`。`onError`:设 banner(错误色,常驻)。
- **排序**:上移/下移 → `providersUtils.moveItem` 算新 sort_index 数组 → `useMutation(providersApi.reorder)` → invalidate。
- **CRUD**:create/update/delete 各一个 `useMutation` → invalidate `['providers']`。delete 前本地 `confirm()`。

## 契约

### `lib/api.ts` 新增
```ts
export interface Provider {
  id: string; app_type: string; name: string;
  mode: 'proxy' | 'direct';
  settings_config: unknown; // JSON Value, 前端只透传/编辑文本
  is_current: boolean;
  category: string | null;
  sort_index: number | null;
  notes: string | null;
  meta: unknown;
  created_at: string; updated_at: string;
}
export const providersApi = {
  list: (appType: string) => request<Provider[]>(`/providers?app_type=${encodeURIComponent(appType)}`),
  create: (body: CreateProviderBody) => request<Provider>('/providers', { method: 'POST', body: JSON.stringify(body) }),
  get: (id: string) => request<Provider>(`/providers/${id}`),
  update: (id: string, body: UpdateProviderBody) => request<Provider>(`/providers/${id}`, { method: 'PUT', body: JSON.stringify(body) }),
  remove: (id: string) => request<void>(`/providers/${id}`, { method: 'DELETE' }),
  switch: (id: string) => request<{ warnings: string[] }>(`/providers/${id}/switch`, { method: 'POST' }),
  reorder: (items: { id: string; sort_index: number }[]) => request<void>('/providers/reorder', { method: 'POST', body: JSON.stringify({ items }) }),
};
```
`CreateProviderBody`/`UpdateProviderBody` 字段对齐后端 `CreateProviderRequest`/`UpdateProviderRequest`(name/mode/settings_config/category/notes)。

### `pages/providersUtils.ts` 纯函数
```ts
export function groupByAppType(providers: Provider[]): Record<'claude-code' | 'codex', Provider[]>
// 按 sort_index asc 排序(NULLS last),分到两个桶
export function moveItem(items: Provider[], from: number, to: number): { id: string; sort_index: number }[]
// 计算移动后的 sort_index 数组(从 0 重新连续编号),供 reorder
export const canMoveUp = (i: number) => i > 0;
export const canMoveDown = (i: number, len: number) => i < len - 1;
```

### `lib/presentation.ts` 新增
```ts
export const APP_TYPE_LABELS: Record<string, string> = { 'claude-code': 'Claude Code', codex: 'Codex' };
export const MODE_LABELS: Record<string, string> = { proxy: '代理', direct: '直连' };
export const MODE_COLORS: Record<string, string> = {
  proxy: 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400',
  direct: 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400',
};
```

### Banner 状态(ProvidersPage 本地)
```ts
type Banner = { kind: 'success' | 'warning' | 'error'; text: string } | null;
const [banner, setBanner] = useState<Banner>(null);
// success/warning 3s 自动清,error 常驻
```

## 兼容性

- 路由新增,不替换现有路由。
- NAV_ITEMS 数组追加,不删现有项。
- api.ts/presentation.ts 追加导出,不改现有符号。
- 现有页面零回归。

## 取舍

- **两次 list 请求 vs 一次拉全部**:后端 `list` 按 app_type 过滤,选两次请求换简单契约;app_type 只有 2 个,开销可忽略。
- **排序不做乐观更新**:排序频次低,invalidate 重拉更简单且不易出不一致。
- **settings_config 用 JSON textarea**:direct 的 endpoint_id 选择器是 P1 后深度绑定的事,本期直接编辑 JSON 字符串即可,降低范围。
- **不引 toast 库**:本地 useState banner 够用,避免新增依赖。
- **`ProviderForm` 用模态**:与现有页面表单风格一致(参考 AccountsPage 的 `ApiKeyAccountForm` 行内切换),但 provider 字段少,模态更聚焦。最终实现可按现有页面习惯选行内或模态。

## 回滚

- 纯前端新增,回滚 = 删 `ProvidersPage.tsx`/`AppTypeSection.tsx`/`ProviderCard.tsx`/`ProviderForm.tsx`/`providersUtils.ts(.test.ts)` + 撤销 App.tsx/AppShell.tsx/api.ts/presentation.ts 的追加。无数据迁移、无后端残留。
