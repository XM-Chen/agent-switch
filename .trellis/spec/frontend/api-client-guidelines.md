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
