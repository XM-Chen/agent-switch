# Proxy failover OAuth 刷新与冷却修复

## Goal

修复 `http/proxy/` 层的 SSE 错误传播、同端点重试、AuthError 冷却、OAuth refresh 超时与 refresh_token 轮换语义等 P2 缺陷,并处理 proxy/failover 相关 P3 死代码与轻微契约偏离。

## Background

- 审计报告锚点:`.trellis/tasks/archive/2026-07/07-03-codebase-audit/research/audit-report.md` §4 P2-8~13, §5 Proxy/Stream/Failover 层
- 关键文件:`proxy/sse.rs`, `proxy/stream_guard.rs`, `proxy/failover.rs`, `proxy/oauth_refresh.rs`, `proxy/mod.rs`, `proxy/constants.rs`

## Requirements

### P2 缺陷

- **P2-8** `sse.rs:74`:上游 stream error 分支必须设置 `errored=true`,避免错误事件每块重复发出并在尾部 flush 残留行。
- **P2-9** `proxy/mod.rs:455`:同端点重试必须应用 `SAME_ACCOUNT_RETRY_DELAY_MS=500ms` backoff,不再立即 `continue` 重发。
- **P2-10** `stream_guard.rs:116`:stream-guard inline error 在无数值 status 时也应生成 retryable=true 的 ProxyError,并允许同账号重试优先于切换端点;首块错误不能跳过 `same_account_retries`。
- **P2-11** `failover.rs:182`:AuthError 预检刷新失败冷却应为 300s(5 分钟),符合 PRD auth 类冷却契约,而非默认 30s。
- **P2-12** `oauth_refresh.rs:168`:OAuth 刷新 HTTP client 必须设置明确超时,避免 `send().await` 无限挂起阻塞故障转移主循环。
- **P2-13** `oauth_refresh.rs:206`:刷新响应未携带新 refresh_token 时,不得盲目保留旧 refresh_token 并假设其有效;必须记录轮换语义并选择安全策略:保留旧值但标记来源/日志,或在 spec 中确认 OpenAI 未返回时按 OAuth 标准保留旧值。若选择保留,需补充 timeout + error handling 并在 PRD 中说明该条实际为上游协议语义风险而非代码 bug。

### P3 / 轻微项

- `stream_guard.rs:94`:首块为空时 `stream_started=false` 但返回空 SSE 流可能挂起;应返回错误或终止事件。
- `sse.rs:561`:不可达的跨协议 fallback passthrough 逻辑如确为死路径则删除或加断言。
- `proxy/mod.rs:445`:上游响应头被全部丢弃;至少保留安全且有价值的调试头(`x-request-id`, rate-limit 系列),避免泄漏敏感 headers。
- `stream_guard.rs:148`:extract_error_code 忽略 Retry-After/overload type;应尝试从 error 对象提取 retry_after/status/type。
- `proxy/mod.rs:570`:跨协议流式丢弃上游 keep-alive/ping;可透传兼容 ping 或记录为 intentionally dropped。
- `failover.rs:142`:route_settings.cooldown_multiplier 死配置;要么接入冷却计算,要么从 DB/API/UI 移除(优先接入)。
- `oauth_refresh.rs:22`:REFRESH_LOCKS 静态 HashMap 不清理;当前有界影响轻微,可接受则记录上限语义,否则实现清理。
- `codex_oauth.rs:363`:同类 HTTP Client::new() 无超时由 `fix-codex-oauth-credentials` 子任务处理,本任务只处理 `oauth_refresh.rs`。
- `failover.rs:367`:上游非成功响应 `.bytes().await.unwrap_or_default()` 吞读取错误,应记录错误摘要。

## Design

### Retry / fallback 语义

1. `stream_started=true`:禁止 fallback,只向客户端发错误终止帧。
2. `stream_started=false && error.retryable && !max_retries`:同端点重试,先 sleep `SAME_ACCOUNT_RETRY_DELAY_MS`。
3. 同端点重试耗尽后:记录 failure + cooldown + 切换候选端点。
4. `AuthError`:冷却 300s;若 OAuth refresh 超时/失败,不进入无限挂起。

### OAuth refresh timeout

- 使用 `reqwest::Client::builder().timeout(Duration::from_secs(30)).build()` 或项目常量。
- 连接/请求超时映射为 `NetworkError` 或 `AuthError` 需保持一致:请求级超时更像 NetworkError;上游 4xx refresh 失败为 AuthError。

### route_settings.cooldown_multiplier

- 若 `RouteSettings` 已暴露该字段,冷却秒数应乘以 multiplier 并 clamp 到合理上限。
- 如果 multiplier 为 0 或无效,按 1.0 处理。

## Acceptance Criteria

- [ ] AC1(P2-8):上游 stream error 后只发送一次错误事件,不会 flush 残留行
- [ ] AC2(P2-9):同端点 retry 前有 500ms backoff,单测或代码路径验证
- [ ] AC3(P2-10):stream 首块 inline error 支持 retryable=true 与 same-account retry
- [ ] AC4(P2-11):AuthError 冷却为 300s,测试覆盖
- [ ] AC5(P2-12):OAuth refresh HTTP client 有超时,超时错误不会无限挂起
- [ ] AC6(P2-13):refresh_token 缺失响应处理策略明确并实现/记录
- [ ] AC7(P3):cooldown_multiplier 接入或移除;响应头保留策略明确;空首块处理不挂起
- [ ] AC8:`cargo check` 0 warning,`cargo clippy --all-targets -- -D warnings` 通过(本子任务范围内)
- [ ] AC9:新增/更新 proxy/failover/oauth_refresh 单元测试或集成测试

## Out of Scope

- Codex OAuth 登录回调与初始登录缺陷(P2-21~24),由 `07-03-fix-codex-oauth-credentials` 处理
- 跨协议翻译全接线
