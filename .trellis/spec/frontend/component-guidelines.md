# 前端组件规范

## 语言

第一版 UI 使用中文，包括导航、表单、错误提示、风险提示和空态说明。

## 共享展示常量

工具标签、分类标签、颜色等展示常量统一放到 `src/lib/presentation.ts`。

## 占位与空态

- 不再需要的占位组件应删除。
- 无法由现有 API 得出的统计不能伪造成 0，应显示“暂无该统计数据”或明确待后续接入。

## 死代码

新增组件或 helper 后必须有真实引用或测试引用；没有引用的 `cn`/placeholder 这类工具应删除。

`lib/api.ts` 的 API 方法同理：无页面/测试引用的 `.get`/`.update` 等预置方法应删除，不为"完整 REST 表面"留死代码（见 [API Client 规范](./api-client-guidelines.md)）。

## 操作反馈 banner 模式

页面级操作反馈用本地 `useState<Banner | null>` 管理，不引第三方 toast 库：

- `success` / `warning`：3s 后自动清（`setTimeout` + `useRef` 持有 timer，卸载/新 banner 时 clear）。
- `error`：常驻，直到下次操作或用户手动 ✕ 关闭。
- `mutation.onSuccess` 按"有 warnings → warning，无 warnings → success"分流（见 providersApi switch 契约）；`onError` → error。
- `onSettled` 统一 `invalidateQueries` 刷新列表。

参考实现：`pages/ProvidersPage.tsx` 的 `BannerView` + `showBanner` + 各 `useMutation` 的回调装配。其他页面若需要操作反馈，沿用此模式而非各写一套。
