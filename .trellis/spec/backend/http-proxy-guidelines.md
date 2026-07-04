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

## 管理 API 路由注册顺序

- `nest` 的具体前缀（如 `/api/providers`）必须注册在 `/api/{*path}` 兜底之前，否则被兜底吞掉。
- 同一 `Router` 内固定段与参数段冲突时（`/reorder` vs `/{id}`），固定段优先由 axum 静态段优先级保证，但仍应把固定段路由写在参数段之前以免误读。

## Provider 切换的原子性（set_current + 接管）

切换要同时改两处状态：DB 的 `is_current`（`providers` 表 partial unique index 保证互斥）与工具接管配置（`tool_takeover`）。二者必须一致，禁止"DB 说 current 是 A 但工具没接管"。

契约（`POST /api/providers/{id}/switch`）：

1. 查目标 provider，不存在 → 404。
2. 解析 `app_type` → `Tool`，`supports_takeover()` 为假（如 opencode）→ 400。
3. 记录切换前的 current（用于回滚）。
4. **先** `set_current`（DB），**再**按 `mode` 接管：`proxy`→`enable`，`direct`→`enable_direct(.., crypto)`。
5. **接管失败必须回滚 `is_current`**：恢复到切换前的 current，若原本无 current 则 `clear_current`。回滚本身再失败时，把两个错误一并返回 500。
6. `direct` 模式 crypto 不可用 → 503，**不静默降级**为 proxy。

因为接管服务把 `tool_takeover` 状态的 `upsert_state` 放在最后一步，接管失败时该状态保持不变，与回滚后的 `is_current` 自然一致。

删除 current provider：先 `clear_current`，若 `tool_takeover.active_provider_id` 仍指向被删 id，用 `set_mode` 复位为 `proxy`/`None`，避免悬空 direct 引用。
