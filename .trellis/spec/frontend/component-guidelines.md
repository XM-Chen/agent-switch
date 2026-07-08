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

## Claude Code env 行为开关编辑器

`ProviderForm` 的 Claude Code 行为开关分区只服务 `app_type='claude-code'`，读写 `provider.meta.snapshot.env`。它是结构化字段 + 裸 JSON 逃生舱的双向编辑器，不是连接层配置编辑器。

### 1. Scope / Trigger
- Trigger：新增/维护 Claude Code provider 表单中的模型默认值、`API_TIMEOUT_MS`、`CLAUDE_CODE_*`、Bedrock/AWS env、预设模板或「应用到 live」按钮。

### 2. Signatures
- Component state：`ClaudeEnvSwitches`。
- Helpers：`parseClaudeEnv(meta)`、`serializeClaudeEnv(meta, switches)`、`validateApiTimeoutMs(value)`、`strip/set/hasClaudeOneMMarker(model)`。
- Props：`onApplyLive?: () => void` 仅当前激活 provider 显示。

### 3. Contracts
- 仅 `appType === 'claude-code'` 渲染；Codex provider 不显示、不写入这些 env。
- 保存编辑时写 `UpdateProviderBody.meta`；创建 provider 时不写 meta（新建默认无行为 env）。
- 结构化字段编辑必须同步刷新裸 JSON；用户正在编辑裸 JSON 时，用 ref 防止结构化回填覆盖正在输入的文本。
- 裸 JSON 可保留未结构化 env 键；结构化字段只负责已知行为键。
- 「应用到 live」只对 `initial.is_current` provider 显示，且调用 provider switch 重切，不直接写本地文件。

### 4. Validation & Error Matrix
- `API_TIMEOUT_MS` 空串 -> 允许，序列化删除键。
- `API_TIMEOUT_MS` 非空非正整数 -> 就近显示错误并拦截提交。
- 裸 JSON 非对象/非法 JSON -> 显示错误，不回填结构化字段。
- Bedrock checkbox 关闭 -> `CLAUDE_CODE_USE_BEDROCK` 删除；AWS 明文字段可保留/删除按结构化字段空值规则处理。

### 5. Good/Base/Bad Cases
- Good：输入 `60000`，保存 meta；当前 provider 再点「应用到 live」落 live。
- Base：编辑非当前 provider 的 env，只保存 DB，下一次切换自然生效。
- Bad：把 `ANTHROPIC_AUTH_TOKEN` 做成结构化输入项，或在表单里直接调用文件写入 API。

### 6. Tests Required
- Helper 单测：1M 标记、parse/serialize round-trip、空值删键、`API_TIMEOUT_MS` 校验、连接键不被结构化 helper 写入。
- 页面/集成可选：保存按钮 pending guard、「应用到 live」只在 current provider 出现。
- 每次新增 env 结构化字段，都要同时补 helper parse/serialize 和测试。

### 7. Wrong vs Correct

Wrong:
```tsx
<input value={token} onChange={setToken} /> // 连接 token 不属于行为 env 编辑器
```

Correct:
```tsx
const meta = serializeClaudeEnv(initial.meta, switches);
onSubmit({ ...body, meta }, true);
```
