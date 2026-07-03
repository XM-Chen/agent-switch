# Proxy failover OAuth 刷新与冷却修复 - Design

## 1. 范围

本任务修改 `src-tauri/src/http/proxy/` 下 failover、stream guard、SSE translate、OAuth refresh 与代理主循环相关代码。不得修改 Codex OAuth callback 登录流程、DB portability 或前端页面。

## 2. 关键文件

- `src-tauri/src/http/proxy/mod.rs`
- `src-tauri/src/http/proxy/failover.rs`
- `src-tauri/src/http/proxy/oauth_refresh.rs`
- `src-tauri/src/http/proxy/sse.rs`
- `src-tauri/src/http/proxy/stream_guard.rs`
- `src-tauri/src/http/proxy/constants.rs`

## 3. Retry/fallback 状态机

### 3.1 基本不变量

1. 已向客户端输出 stream chunk 后(`stream_started=true`),禁止 fallback。
2. 未输出前遇到 retryable 错误,优先同端点重试,直到 `same_account_retries` 耗尽。
3. 同端点重试之间必须 sleep `SAME_ACCOUNT_RETRY_DELAY_MS`。
4. 同端点重试耗尽后才 record failure、计算 cooldown、切换候选端点。
5. `AuthError` 冷却应为 300s,避免无效凭据短间隔重复尝试。

### 3.2 stream_guard inline error

首块 SSE 错误如果不含可解析 HTTP status,仍应生成:

```rust
ProxyError {
  kind: ProxyErrorKind::UpstreamError(502),
  status: 502,
  retryable: true,
  stream_started: false,
  ...
}
```

不要通过 `ProxyError::new` 的默认 retryable=false 路径。

### 3.3 sse.rs 上游 stream error

`Some(Err(e))` 分支必须设置 `state.errored = true`,并返回一次协议化错误事件。后续 poll 看到 `errored` 时返回 None,不得继续 flush decoder 残留。

### 3.4 OAuth refresh timeout

`oauth_refresh.rs` 使用带 timeout 的 reqwest client:

```rust
Client::builder()
  .timeout(Duration::from_secs(30))
  .build()
```

请求超时映射为 `NetworkError`;上游 4xx/invalid_grant 映射为 `AuthError`。

### 3.5 refresh_token 缺失响应

OAuth 标准允许 refresh 响应不返回新 refresh_token。第一版策略:

- 若返回新 refresh_token:覆盖保存
- 若未返回:保留旧 refresh_token,但添加 tracing debug/warn 说明上游未轮换返回

该策略比删除旧 token 更安全,避免立即使账号不可刷新。P2-13 的风险通过 timeout、错误可见性与日志说明降低。

### 3.6 cooldown_multiplier

`route_settings.cooldown_multiplier` 已暴露为配置,应接入冷却秒数计算:

```rust
cooldown = (base_cooldown as f64 * multiplier).round()
```

并 clamp 到 `[1, COOLDOWN_MAX_EXPONENTIAL_SECS]` 或合理上限。若当前 `FailoverContext` 拿不到 route setting,最小实现是在创建 context 时传入 multiplier。

## 4. 响应头保留策略

允许透传安全调试头:

- `x-request-id`
- `x-ratelimit-*`
- `openai-processing-ms`
- `anthropic-ratelimit-*`

继续过滤 hop-by-hop、authorization、set-cookie 等敏感/连接相关头。

## 5. 测试设计

- failover AuthError cooldown = 300s
- same endpoint retry 调用路径包含 backoff
- stream_guard inline error 无 status 仍 retryable=true
- sse upstream error 只发送一次,不 flush 残留
- oauth_refresh client builder 有 timeout(可通过抽函数测试或代码审查)
- cooldown_multiplier 影响最终 cooldown

## 6. 非目标

- 不处理 Codex OAuth callback / account_id / expires_at
- 不处理 model_sync fetch timeout(归 db/portability 或 codex 子任务)
- 不做完整 header passthrough 白名单之外的大范围行为改变
