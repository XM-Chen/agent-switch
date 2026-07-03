# Proxy failover OAuth 刷新与冷却修复 - Implement

## 执行顺序

### Step 1: 启动前确认

```bash
python ./.trellis/scripts/task.py current
```

确认活动任务为 `.trellis/tasks/07-03-fix-proxy-oauth-failover`。

### Step 2: 阅读当前实现

精读:

- `src-tauri/src/http/proxy/mod.rs`
- `src-tauri/src/http/proxy/failover.rs`
- `src-tauri/src/http/proxy/oauth_refresh.rs`
- `src-tauri/src/http/proxy/sse.rs`
- `src-tauri/src/http/proxy/stream_guard.rs`
- `src-tauri/src/http/proxy/constants.rs`

### Step 3: P2-8 sse.rs errored

`Some(Err(e))` 分支设置 `state.errored = true`,返回一次错误事件;poll 顶部看到 errored 立即返回 None,不再 flush。

### Step 4: P2-9 retry backoff

主循环 retryable 重试路径在 `continue` 前 `tokio::time::sleep(Duration::from_millis(SAME_ACCOUNT_RETRY_DELAY_MS))`。

### Step 5: P2-10 stream_guard inline error retryable

首块 inline error 无 status 时构造 retryable=true、stream_started=false 的 ProxyError。

### Step 6: P2-11 AuthError cooldown

`calculate_cooldown_seconds` 增加 `ProxyErrorKind::AuthError => constants::COOLDOWN_AUTH_SECS` 之类,值为 300。

若常量不存在则新增。

### Step 7: P2-12 oauth_refresh timeout

`Client::builder().timeout(Duration::from_secs(30)).build()`。

### Step 8: P2-13 refresh_token 缺失

未返回新 refresh_token 时保留旧值并 tracing::warn 说明上游未轮换。

### Step 9: P3 项

- stream_guard 空首块返回明确错误或终止事件
- 响应头白名单保留调试头
- cooldown_multiplier 接入冷却计算
- REFRESH_LOCKS 视情况记录或清理
- 失败响应体读取错误记录摘要

### Step 10: 质量门

```bash
cd src-tauri
cargo check
cargo clippy --all-targets -- -D warnings
cargo test --lib proxy
```

不强制 cargo fmt。

### Step 11: 自检

对照 PRD AC1~AC9。

## 风险

- backoff 引入 sleep,确认不会在 stream_started=true 后被触发。
- cooldown_multiplier 接入需把 route setting 传入 FailoverContext;注意不要破坏现有 cooldown 计算。
