# 端点测试与健康检查策略对比研究

## 研究问题

为 agent-switch 第一版设计端点测试功能，需要综合参考：

- `9router`：Web 管理端 provider/account/model 测试。
- `cpa`（CLIProxyAPI）：管理 API 通用请求测试和健康检查。
- `sub2api`：账号测试、UsageLog 和调度状态联动。
- `ccs`：旧版 Stream Check 与最新版轻量可达性探测的演变。

## 四项目对比

| 项目 | 测试方式 | 是否消耗 token | 是否影响调度/冷却 | 关键启发 |
|------|----------|----------------|--------------------|----------|
| `9router` | Web UI 可测试 provider/account/model；OAuth 测试刷新 token 或访问用户信息端点；API Key 测试 `/models` 或最小 ping；模型测试可对模型逐个探测 | 尽量最小化；聊天测试通常 `max_tokens: 1`；某些 Codex 场景可用最小无效请求避免消耗 | 测试会更新 `testStatus`、`lastError`、`lastErrorAt`；真实请求错误才进入账户不可用和冷却 | 可学习“连接测试 + 模型测试 + 错误展示”，但第一版不宜批量真实测所有模型 |
| `cpa` | `/healthz` 只测试本服务；`/v0/management/api-call` 可发任意管理测试请求，支持 `$TOKEN$` 替换 | 若真实打到上游，通常会消耗 token | 真实执行路径可能触发凭据冷却；健康检查不影响调度 | 用户指定第一版端点测试仿造 `cpa`：提供管理端真实链路测试调用能力，而不是仅做轻量可达性探测 |
| `sub2api` | Web 管理端 `AccountTestModal` 发起账号测试，走完整网关生命周期；可能发送 compact 或 hello 测试请求 | 会消耗 token | 会记录 UsageLog；失败可能更新账号状态、限流、冷却和调度可用性 | 可学习“测试经过完整网关链路 + 摘要日志”，但用户确认第一版测试默认不影响生产调度/冷却 |
| `ccs` | 旧版曾用真实流式模型请求做 Stream Check；最新版改为轻量 HTTP 可达性探测，只测 base_url 网络可达性 | 最新版不发真实模型请求，基本不消耗 token | 最新版可达性检查不重置熔断器，故障转移仍只由真实代理流量驱动 | 可作为风险警示：真实测试容易误报、消耗 token 或触发 WAF；即使仿造 `cpa`，也要在 UI 中明确提示 |

## cpa 风格设计要点

`cpa` 的管理测试更接近“受控 API 调试器”：

- 由管理 API 发起测试调用。
- 请求包含 method、url、header、data 等字段。
- 可选择某个认证记录，并在 header 中使用 `$TOKEN$` 之类占位符注入凭据。
- 返回上游状态码、响应头和响应体。
- 如果真实打到上游，可能消耗 token。
- 这种测试可以验证 base URL、API Key/OAuth 凭据、路径、模型和协议是否真的可用。

## sub2api 风格链路启发

`sub2api` 的测试会走完整网关生命周期，包括认证、账号选择、转发、UsageLog 和健康状态更新。agent-switch 第一版采纳其中“真实链路测试”和“摘要日志”思路，但不采纳“测试默认影响生产调度状态”的行为。

## ccs 取消/弱化旧版端点检测的原因

`ccs` 旧版曾提供真实 Stream Check，会发送真实流式模型请求来验证供应商端点。

最新版改为轻量 HTTP 可达性探测，原因包括：

- 许多第三方供应商会对真实流式测试请求返回 401、403 或 WAF 拦截，造成误报。
- 测试请求可能消耗 token 或触发风控。
- 真实模型请求很难兼容所有供应商的路径、协议、模型和 header 差异。
- “可达”与“可用”是两件事：
  - base_url 返回 403 说明网络可达，但凭据或协议不一定可用。
  - 真正的可用性仍应由真实代理流量、错误分类和故障转移策略判断。

## 用户决策

用户明确要求：

- 端点测试仿造 `cpa` 项目设计。
- 测试应像模拟真实完整链路请求一样执行。
- 测试允许用户自定义请求所用 prompt。
- 测试默认不影响生产故障转移/冷却状态，只记录测试结果和测试链路。

## 推荐给 agent-switch 第一版的落地方案

### 1. 测试名称与定位

第一版测试功能命名为“真实链路测试”，而不是单纯“端点可达性测试”。

语义：

```text
用户选择工具/协议/端点/路由
→ 输入自定义 prompt
→ agent-switch 构造真实请求
→ 经过正常路由、认证注入、协议适配和可选 fallback
→ 请求真实上游
→ 展示响应摘要和路由链路
```

### 2. 测试参数

第一版至少支持：

- 测试目标：Claude Code 路由、Codex 路由、`/v1` 路由、指定端点。
- 模型。
- 自定义 prompt。
- 是否流式。
- 最大输出 token。
- 是否允许在测试中执行故障转移。
- 认证来源：关联账号、端点 API Key、Codex OAuth token。

### 3. 默认测试模板

为降低误用风险，第一版应提供协议模板，而不是一开始只暴露完全任意请求：

- OpenAI-compatible：
  - `/v1/chat/completions` 最小请求，允许用户自定义 prompt。
  - 可选 `/v1/models` 非 prompt 测试。
- Anthropic-compatible：
  - `/v1/messages` 最小请求，允许用户自定义 prompt。
- Codex OAuth：
  - token 状态/刷新测试与真实模型请求测试分开。

### 4. token 消耗与隐私

UI 必须提示：

> 真实链路测试会把你输入的 prompt 发送给所选上游，可能消耗 token，也可能触发供应商风控。agent-switch 默认不保存 prompt、messages、完整响应正文、API Key 或 OAuth token。

日志策略仍遵守 `sub2api` 风格：

- 保存请求摘要、状态码、耗时、错误摘要、请求体 hash、测试 fallback 链路。
- 不保存 prompt/messages/完整 body。
- 响应正文只在当前测试结果弹窗中临时显示摘要；默认不持久化完整正文。

### 5. 是否影响调度/冷却

用户已确认：测试请求默认不影响生产故障转移/冷却状态。

可以更新独立测试字段：

- `last_test_status`
- `last_test_at`
- `last_test_latency_ms`
- `last_test_error_kind`
- `last_test_error_summary`
- `last_test_fallback_chain`

生产路由字段仍由真实代理流量驱动：

- `last_success_at`
- `last_failure_at`
- `cooldown_until`

这样既仿造 `cpa` 的真实测试能力，又吸收 `sub2api` 的完整链路视角，并吸收 `ccs` 的教训，避免测试误伤路由调度。

### 6. 后续扩展

后续版本可以增加：

- 任意请求调试器。
- 批量测试。
- 将测试结果应用到健康状态的显式按钮。
- 自动定时测试。
- 更细粒度的模型可用性探测。

## 结论

agent-switch 第一版端点测试采用：

> 仿造 `cpa` 的管理端测试调用能力，支持自定义 prompt 的真实完整链路测试；吸收 `sub2api` 的网关链路与摘要日志思想；吸收 `ccs` 的风险教训，明确提示 token 消耗和隐私边界，并确保测试结果默认不直接影响生产故障转移/冷却状态。
