# 前端死代码 Dashboard 占位与测试框架修复 - Implement

## 执行顺序

### Step 1: 启动前确认

```bash
python ./.trellis/scripts/task.py current
```

确认活动任务为 `.trellis/tasks/07-03-fix-frontend-deadcode-tests`。

### Step 2: 阅读当前实现

精读:

- `src/pages/LogsPage.tsx`
- `src/pages/DashboardPage.tsx`
- `src/components/tools/ToolCard.tsx`
- `src/components/layout/PagePlaceholder.tsx`
- `src/lib/api.ts`
- `src/lib/utils.ts`
- `src-tauri/src/http/api/logs.rs`

### Step 3: P2-18 日志类型过滤

后端 `http/api/logs.rs` 支持 `log_type=production|test`。

`lib/api.ts` 的 `logsApi.list` 增加 `log_type` 参数。

`LogsPage.tsx` 把"生产/测试"选择映射到 `log_type`,与 `tool` 选择正交组合。

### Step 4: 共享展示工具

新增 `src/lib/presentation.ts`、`src/lib/format.ts`、`src/pages/dashboardUtils.ts`。

DashboardPage 与 ToolCard 改为 import 共享常量/函数。

### Step 5: 死代码

- 删除 `PagePlaceholder.tsx`(先 grep 确认无引用)
- 评估 `cn` 使用;无引用删除

### Step 6: Dashboard queryKey

`['logs']` 改为 `['logs', { dashboard: true, limit: 10 }]` 或等价带参数 key,与 LogsPage 不冲突。

### Step 7: Dashboard 占位

把无法由现有 API 得出的统计改为明确空态说明,不伪造数字。

### Step 8: 测试框架

`package.json` 增加 vitest + testing-library + jsdom。

`vite.config.ts` 增加 test 配置(或新建 `vitest.config.ts`)。

新增测试:

- `src/lib/format.test.ts`
- `src/pages/dashboardUtils.test.ts`
- `src/lib/presentation.test.ts`
- LogsPage/LogsApi 过滤参数测试

### Step 9: 质量门

```bash
npm run build
npm run test
```

### Step 10: 自检

对照 PRD AC1~AC9。

## 风险

- 后端 log_type 实现要保证 total 与 items 一致,避免前端本地过滤破坏分页。
- 引入 vitest 注意 Tauri/Vite 版本兼容,优先用 vitest 1.x。
- 删除 PagePlaceholder 前确认无路由引用,避免运行时空引用。
