# 前端死代码 Dashboard 占位与测试框架修复 - Design

## 1. 范围

本任务修改 React 前端与日志 API 过滤契约。可修改后端 `http/api/logs.rs` 和 `lib/api.ts` 以保证生产/测试日志分页 total 正确。不得修改 proxy 核心、translator、DB portability 或 Codex OAuth。

## 2. 关键文件

- `src/pages/LogsPage.tsx`
- `src/pages/DashboardPage.tsx`
- `src/components/tools/ToolCard.tsx`
- `src/components/layout/PagePlaceholder.tsx`
- `src/lib/api.ts`
- `src/lib/utils.ts`
- `src-tauri/src/http/api/logs.rs`
- `package.json`
- `vite.config.ts`

## 3. 日志类型过滤设计

### 3.1 当前问题

LogsPage 用 `tool=test` 表示测试日志。但选择 `production` 时没有传任何排除条件,导致生产视图包含 test 日志。

### 3.2 目标契约

API 支持显式 `log_type`:

```ts
type LogType = 'production' | 'test'
logsApi.list({ log_type: 'production' })
```

后端语义:

- `log_type=test`: `tool = 'test'`
- `log_type=production`: `tool IS NULL OR tool != 'test'`
- `tool` 过滤和 `log_type` 同时存在时:
  - `log_type=test` 优先使用 test
  - `log_type=production` + tool=claude-code/codex:生产且指定工具

这样分页 total 与 items 都在后端一致。

## 4. 共享展示工具

新增共享模块:

- `src/lib/presentation.ts`
  - `TOOL_LABELS`
  - `CATEGORY_LABELS`
  - `CATEGORY_COLORS`
- `src/lib/format.ts`
  - `formatTime`
  - `formatDuration` 如需要
- `src/pages/dashboardUtils.ts`
  - `aggregateEndpointHealth`
  - `bucketHealth`
  - `countFallbackHops`

DashboardPage 与 ToolCard 不再复制同一份常量。

## 5. Dashboard 占位处理

如果某些统计无法从现有 API 精确得出,不得伪造。策略:

- 能由 accounts/endpoints/models/routes/tools/logs/settings 组合得出的统计直接展示
- 不能得出的高级统计显示"暂无该统计数据"或"待后续版本接入"
- 不影响 P1 已修复的错误态:任何 query error 都不触发 EmptyGuide

## 6. 死代码处理

- `PagePlaceholder.tsx`:若无引用删除文件;若保留则接入未实现页面。但当前 8 页面已实现,优先删除。
- `src/lib/utils.ts` 的 `cn`:若全仓无引用删除;如引入测试/组件需要 classnames 合并再保留并使用。

## 7. 测试框架

引入 Vitest + React Testing Library:

- `vitest`
- `@testing-library/react`
- `@testing-library/jest-dom`
- `jsdom`

package scripts:

```json
"test": "vitest run",
"test:watch": "vitest"
```

测试优先覆盖纯函数与 API 参数构造,避免复杂浏览器 e2e。

## 8. 测试用例

- `dashboardUtils.test.ts`:health bucket、fallback hop count
- `format.test.ts`:formatTime
- `LogsPage` 或 `logsApi` 参数测试:production/test log_type
- `presentation.test.ts`:label map 基本存在

## 9. 非目标

- 不引入 Playwright/e2e
- 不大规模重设计 UI
- 不重写所有页面状态管理
