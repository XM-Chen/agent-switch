# HTTP / Proxy 规范

## 本地服务边界

- 默认绑定 `127.0.0.1:42567`。
- `/api/*` 为管理 API；`/claude-code/*`、`/codex/*`、`/v1/*` 为代理入口；`/health` 为健康检查。
- 第一版本地入口无认证，必须避免日志和错误响应泄漏敏感凭据。

## 故障转移

- 已向客户端输出 stream chunk 后禁止 fallback。
- 未输出前遇到 retryable 错误，优先同端点重试；同端点重试间隔为 `SAME_ACCOUNT_RETRY_DELAY_MS`。
- `AuthError` 冷却为 300s，避免失效凭据端点被短周期反复选中。
- `cooldown_multiplier` 仅作用于通用上游/网络类冷却，不覆盖 AuthError 或上游 Retry-After 固定语义。

## SSE / stream guard

- stream 首块为空必须返回明确 retryable error，不得返回空流让客户端挂起。
- inline SSE error 在未输出前应转换为 `retryable=true, stream_started=false` 的 `ProxyError`。
- 上游 stream error 只发一次协议化错误事件，并设置终止状态，避免重复错误帧或 flush 残留行。

## OAuth refresh

- 预检刷新必须设置连接/总超时。
- refresh 响应缺少新 refresh_token 时保留旧 refresh_token，并记录可诊断日志。

## 响应头

只透传安全诊断头，如 `x-request-id`、rate-limit 系列；继续过滤 hop-by-hop 与敏感头。
