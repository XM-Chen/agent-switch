# 代理、安全与 Claude 托管上游

## 当前代理

ccs 使用 Axum，本地默认 `127.0.0.1:15721`（`src-tauri/src/proxy/types.rs:45-46`），路由在 `proxy/server.rs:291-356` 组装，包含 Claude messages、OpenAI chat/responses、Gemini、models、health/status 等。

保留能力：

- Claude messages 直连与格式转换；
- hot switch、健康检查、熔断/故障转移；
- streaming、请求日志、Token/成本统计；
- OpenAI chat/responses 等 Claude 上游协议适配；
- Claude Desktop gateway（即使最终不暴露 Desktop 客户端，需在裁剪时根据实际 Claude 主链引用闭包判断）。

## 非 loopback 鉴权目标

ccs 当前只有 Claude Desktop gateway handler 调用 `validate_claude_desktop_gateway_auth`（`src-tauri/src/proxy/handlers.rs:122-138,250-269`），其他转发路由没有统一 token middleware。

Agent Switch 必须实现：

- `127.0.0.1`、`localhost`、`::1` 视为 loopback，默认无鉴权；
- 其他地址（包括 `0.0.0.0`、LAN IP、非 loopback IPv6）对**所有转发端点**统一校验 `Authorization: Bearer <local-token>`；
- 中间件位于 router 外层，不允许逐 handler 漏挂；
- health/status 是否公开必须单独定义，默认不泄露 provider/token/路由细节；
- token 安全生成、持久化、常量时间比较，不写日志/错误；
- 前端风险确认是辅助门，不能代替后端鉴权。

必测：每类路由在 loopback 放行；非 loopback 的缺失/错误 token 401、正确 token 进入 handler；新增路由自动受 middleware 保护。

## 凭据边界

- Provider 的 API token 只在后端读取并注入上游请求；不返回前端、不输出日志。
- 转发前删除用户来路认证头，按选中 Provider 重建，避免凭据串线。
- OAuth refresh 使用账号级互斥、过期 double-check 和超时；失败不把旧 token 持久化回错误账号。
- 请求/响应日志默认脱敏 Authorization、cookie、API key 和完整 Provider JSON。

## GitHub Copilot 与 Codex OAuth

二者是 Claude Provider 的托管上游，不是独立客户端：

必须保留：

- `ProviderType::GitHubCopilot`、`ProviderType::CodexOAuth`、`ProviderType::OpenRouter`；
- `commands/copilot.rs`、`commands/codex_oauth.rs`；
- 登录/device flow、账号绑定、token refresh、配额和模型发现；
- `proxy/providers/copilot_auth.rs`、`copilot_model_map.rs`、`codex_oauth_auth.rs`；
- Claude Provider 表单的认证区；
- `openai_chat`/`openai_responses` 及 Claude 实际调用的 streaming/transform 闭包。

应删除的是独立 Codex 客户端的 live config、session、升级命令和 UI，不是这些上游模块。每次删除前用 Rust 引用和行为测试确认闭包。

## Failover 与 streaming

- 只有在响应尚未向客户端提交时才能切换上游；流首块发出后禁止透明 failover，避免拼接两个供应商的流。
- 区分认证失败、限流、可重试 5xx、客户端错误；不要对所有错误盲目重试。
- 每个 provider 的健康/熔断状态隔离；成功请求恢复规则可测试。
- 格式转换必须保留 stop reason、usage、tool calls、error 和 SSE 终止语义。

## 安全测试

- token 不出现在 UI payload、数据库外日志、proxy request log、panic/crash log；
- 非 loopback 全路由鉴权覆盖；
- OAuth refresh 并发与账号隔离；
- Provider 切换/故障转移不串 token；
- malformed JSON/SSE 返回稳定错误而不 panic；
- 任何测试 fixture 只用假 token。
