# 前端 API Client 规范

## 入口

所有管理 API 请求通过 `src/lib/api.ts` 发起。页面不直接拼 fetch URL。

## 错误处理

- 非 2xx 响应必须抛出包含状态码/错误消息的 Error。
- 页面不得在 error 状态静默把数据 fallback 成“空资源”并误导用户。

## 参数语义

- 日志筛选使用显式 `log_type=production|test`，不要复用 `tool=test` 表示测试日志。
- `log_type=production` 后端语义为排除 `tool='test'`；分页 total 与 items 必须在后端一致。
- 流式测试不得复用默认 `resp.json()` helper 解析 SSE。

## 测试

API 路径构造逻辑应提取为可测试函数，例如 `buildLogsPath`。

## API 方法保留原则

`lib/api.ts` 只保留**有真实页面或测试引用**的 API 方法。不要为了"完整 REST 表面"预置未调用的 `.get`/`.update` 等方法——它们会积累成死代码（本次精简一次性删掉了 7 个无调用者的 CRUD 方法）。

- 新增 API 方法时，同 PR 内必须有调用点（页面或测试）；否则不合并。
- 删除某个页面前，先确认其依赖的 API 方法是否还有其他引用；若无，一并删除。
- `logsApi.get` 这类"看似通用但实际只有一处用"的方法，保留前在注释里标明调用方，避免被误删。

## providersApi 契约

切换器页面 (`ProvidersPage`) 依赖的两个非标准 REST 语义：

- `switch(id)` → `POST /providers/{id}/switch`，返回 `{ warnings: string[] }`。`warnings` 非空表示切换成功但有非致命提示（如"备份跳过"），前端用 warning banner 展示；非 2xx 抛错走 error banner。**不要**把 warnings 当错误。
- `reorder(items)` → `POST /providers/reorder`，body `{ items: { id, sort_index }[] }`，`sort_index` 为 0 起连续重新编号后的新位置。前端用 `moveItem` 计算新顺序后整体提交，**不做乐观更新**（排序频次低，invalidate 列表即可）。
