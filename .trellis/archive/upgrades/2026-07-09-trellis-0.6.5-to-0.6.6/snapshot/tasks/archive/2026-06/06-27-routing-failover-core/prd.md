# 路由与故障转移核心

## 目标

为 agent-switch 实现 `/claude-code/*` 与 `/codex/*` 两条本地路由的真实代理转发与基础自动故障转移：把本机 AI 编程工具（已通过工具接管指向 agent-switch）的请求，按策略选择上游端点、注入真实凭据、转发到上游，并在安全可切换的错误下沿候选链故障转移；同时产出请求摘要/链路日志。完成后工具接管闭环端到端生效。

## 背景与边界

- 父任务：`06-26-agent-switch-web-router-mvp`，本子任务对应子任务拆分第 5 项「路由与故障转移核心」。
- 范围：`/claude-code/*`（Claude Code，优先参考 `ccs`）、`/codex/*`（Codex，优先参考 `9router`）的真实转发 + 策略选择 + 冷却 + 错误分类 + fallback 链路日志。
- 包含 OAuth token 自动刷新（Codex OAuth `access_token` 过期前 60 秒后台刷新）。
- 不含 `/v1/*` OpenAI-compatible 多端点（子任务 `06-27-openai-compatible-v1-endpoints`）。
- 不含真实链路测试器/流式调试器 UI（子任务 `06-27-chain-testing-debugger`）。
- 不含复杂评分/测速型故障转移（父任务确认第一版用简单失败切换）。
- 不含导入导出（子任务 `06-27-import-export-settings`）。

## 已就位基础

- `/claude-code/{*path}`、`/codex/{*path}` 当前返回 501 占位。
- `endpoints` 表已含故障转移状态字段：`cooldown_until`、`last_success_at`、`last_failure_at`、`last_error_kind`、`priority`、`enabled`、`protocol_type`、`auth_mode`、`api_key_encrypted`。
- `endpoints::list_enabled()` 按 `priority ASC, created_at ASC` 返回。
- `CryptoService::decrypt(blob, aad)` 已实现；端点 API Key 加密结构 `{"api_key": "..."}`；account OAuth 凭据 `CodexCredentials`（含 `access_token`/`refresh_token`/`expires_at`）。
- `model_alias::resolve(db, alias, ctx)` 已就位。
- `reqwest`（rustls-tls + json）已是依赖。

## 技术决策

> 决策原则：效仿四个参考项目（9router / cpa / ccs / sub2api）取最优工程实践，取最优解，不惜实现复杂度。

### D1 转发语义 = 9router 式混合

同协议走 **Native Passthrough**（零损失高保真直转）：Claude Code → Anthropic 端点；Codex → responses 端点。
跨协议走 **cpa 式注册转换器**：第一版落地方向 **Anthropic ↔ OpenAI Chat**、**OpenAI Chat ↔ responses**。转换覆盖请求/响应/流式分块三套；转换器以可扩展注册表组织（`translator.Register(from, to, reqFn, respFn)`）。

### D2 端点选择 = 显式上游池 + 选择策略

候选端点先按协议类匹配（`protocol_type`），再按选择策略组织候选链：Fill-First（优先级顺序，默认）/ Round-Robin（粘性轮询）。`protocol_type` 约定取值在本任务定义（`anthropic` / `openai-chat` / `openai-responses`）。

### D3 模型映射 = 转发链路生效

转发前解析请求 body 的 `model` 字段，用 `model_alias` 解析（tool 作用域，含 Claude Code 角色 haiku/sonnet/opus/fable 映射），重写为上游真实模型名后转发。角色映射参考 ccs：剥离 `[1M]` 标记；haiku/sonnet/opus/fable 分别映射到不同上游模型名。

### D4 OAuth token 自动刷新 = 纳入本任务

Codex OAuth 端点选中前检查 `CodexCredentials.expires_at`，临近过期（60 秒）用 `refresh_token` 换新 token，加密回写 accounts。效仿 ccs「过期前 60 秒后台刷新」+ 9router refreshToken 流程。

### D5 路由配置 = 轻量 route_settings

第一版两条固定路由（`claude-code` / `codex`），各自可配选择策略（fill-first / round-robin）、是否启用故障转移、冷却参数。存储用轻量 `route_settings` 表（每路由一行）。不建重型多规则 routes 引擎。`model_alias` 的 `route` 作用域暂不强绑。

### D6 请求摘要日志 = sub2api 风格

建 `request_logs` 表，字段效仿 sub2api：`request_id`、`tool`、`inbound_endpoint`、`requested_model`、`resolved_alias`、`resolved_scope`、`target_endpoint_id`、`upstream_model`、`upstream_endpoint`、`protocol_from`、`protocol_to`、`status`、`error_kind`、`fallback_chain`（JSON 数组）、`stream`、`duration_ms`、`first_token_ms`、`input_tokens`、`output_tokens`、`cache_creation_tokens`、`cache_read_tokens`、`request_body_hash`、`created_at`。**绝不**存 prompt/messages/完整正文/完整 headers/API Key/OAuth token。

### D7 故障转移 = sub2api 状态机 + 9router 模型级锁 + cpa 冷却

错误分类：网络/超时/408/429/529/5xx/容量类 → 切换；400/405/406/413/414/415/422/501/无效请求/上下文超限/本地错误 → 不切；401/403、404/model_not_found、余额不足按账号/端点类型谨慎处理；流式已发输出 → 禁切。
冷却：端点级写 `endpoints.cooldown_until`；模型级锁 `model_lock_${endpoint_id}_${model}`（新表或内存集）；429 优先按 `Retry-After`，否则指数退避。
尝试上限：候选链顺序切换（`maxSwitches=10`）；同端点重试（`maxSameAccountRetries=3`、`retryDelay=500ms`）。
fallback 全程写入 `request_logs.fallback_chain`。

### D8 入站鉴权 = 忽略入站 token，仅注入上游凭据

接管写入的是占位符 token；入站请求的 auth 不校验、不转发。agent-switch 用端点/账号的真实凭据注入上游。仅绑定 `127.0.0.1`；在转发前预留中间件扩展位供后续本地认证。

### D9 流式 SSE = 首块缓冲探测 + 已发禁切

效仿 cpa：发送 SSE header 前缓冲首块探测上游错误（此前可 fallback）。效仿 sub2api：一旦向客户端写出流内容则禁止 fallback。流式中断按协议合成终端事件（如 responses 的 `response.failed` + `[DONE]`）。

## 参考项目对比

- **9router**：同协议 Native Passthrough，跨生态转换；故障转移 while+excludeSet + `modelLock_${model}`；入站 auth 仅校验不转发。第一版混合同理。
- **cpa**：16+ 转换链全矩阵；分层 selector + 配额冷却；SSE 首块缓冲探测错误；模型映射三层（OAuth 全局/APIKey 按 auth/模型池轮询）。本任务借鉴其 selector 架构与注册式转换器模式。
- **ccs**：改工具配置为主；角色模型映射（haiku/sonnet/opus/fable→上游模型，剥离 `[1M]`）。本任务 Claude Code 模型角色映射直接来源。
- **sub2api**：故障转移状态机是本任务直接蓝本——错误分类、`maxSwitches`/`maxRetries`/`retryDelay`、`Writer.Size()` 流式禁切、请求日志极简字段集（不存 prompt/密钥）。

## 需求

### R1 转发层（Rust，新增 `http/proxy/` 模块）

- R1.1 `/claude-code/{*path}` 接收 Claude Code 请求，转发到已配置的 Anthropic 协议端点。
- R1.2 `/codex/{*path}` 接收 Codex 请求，转发到已配置的 responses 协议端点。
- R1.3 同协议走 Native Passthrough：保留请求 body 结构，只改 host、注入凭据、可选的 model 名重写。
- R1.4 跨协议走注册式转换器：第一版支持 Anthropic ↔ OpenAI Chat、OpenAI Chat ↔ responses。
- R1.5 流式 SSE 转发：发送 header 前缓冲首块探测上游错误；已发输出后禁止 fallback。
- R1.6 流式中断/异常时合成协议相关的终端事件。

### R2 上游选择与策略

- R2.1 按 `protocol_type` 筛选候选端点（`anthropic` / `openai-chat` / `openai-responses`），再按策略排序。
- R2.2 Fill-First：按 `priority ASC` 固定顺序选择，失败后递进下一候选。
- R2.3 Round-Robin：在相同优先级内轮询，带粘性会话（同一请求来源粘性到同一端点）。
- R2.4 跳过 `enabled=0` 或 `cooldown_until > now` 的端点。
- R2.5 凭据注入：API Key 端点解密 `api_key_encrypted`；OAuth 端点从关联 account 取 `CodexCredentials.access_token`。
- R2.6 路由设置持久化：`route_settings` 表（claude-code / codex 各一行），含选择策略、故障转移开关、冷却参数。

### R3 故障转移

- R3.1 错误分类直接继承父任务（见 D7）。
- R3.2 端点级冷却：失败后写 `endpoints.cooldown_until`；模型级锁：`model_lock_${endpoint_id}_${model}`。
- R3.3 候选链顺序切换，`maxSwitches=10`；同端点 `maxSameAccountRetries=3`，`retryDelay=500ms`。
- R3.4 fallback 链路由轨迹写入 `request_logs.fallback_chain`。
- R3.5 流式已发输出后禁止任何 fallback。
- R3.6 429 优先按上游 `Retry-After` 做冷却，否则指数退避。
- R3.7 全部候选耗尽返回 502/503 + fallback 链路日志。

### R4 OAuth token 自动刷新

- R4.1 选中 OAuth 端点前检查凭据 `expires_at`，过期前 60 秒内自动刷新。
- R4.2 用 `refresh_token` 换新 `access_token`，加密回写 accounts。
- R4.3 刷新失败：端点临时冷却，fallback 到下一候选。

### R5 请求摘要日志

- R5.1 每次转发写一条 `request_logs`，字段如 D6 定义。
- R5.2 不限 prompt/messages/完整正文/headers/API Key/OAuth token。
- R5.3 日志可持久化查询，日志页展示摘要列表 + 单条详情。
- R5.4 fallback 链路以 JSON 数组记录每次切换的 endpoint_id、model、status、error。

### R6 模型映射

- R6.1 转发前解析请求 body 的 `model` 字段，调用 `model_alias::resolve` 拿到候选链。
- R6.2 选中候选后重写请求 body 的 model 为 `upstream_model`。
- R6.3 Claude Code 角色映射：haiku/sonnet/opus/fable 分别映射到不同上游模型；剥离 `[1M]` 标记。
- R6.4 未配置 alias 且原名匹配失败则返回可解释错误（不转发）。

### R7 路由管理 API 与前端

- R7.1 `GET /api/routes` 列出两条路由 + 设置 + 候选端点实时状态。
- R7.2 `PUT /api/routes/{id}` 更新路由设置。
- R7.3 前端「路由」页替换占位：两条路由卡片 + 策略配置 + 候选端点列表 + 端点健康状态。

### R8 日志页

- R8.1 `GET /api/logs?tool=&status=&limit=&offset=` 分页查询请求日志。
- R8.2 `GET /api/logs/{id}` 单条详情。
- R8.3 前端「日志」页替换占位：摘要列表 + 过滤 + 详情面板（含 fallback 链路与协议转换路径）。

## 验收标准

- [ ] AC1:Anthropic 协议端点上线后，Claude Code 请求经 `/claude-code/v1/messages` 转发到上游，返回正确响应。
- [ ] AC2:OpenAI responses 协议端点上线后，Codex 请求经 `/codex/backend-api/codex/responses`（或 `/codex/v1/responses`）转发到上游。
- [ ] AC3:同一路由配置多个端点时，Fill-First 按优先级顺序使用；端点失败后自动切到下一候选。
- [ ] AC4:Round-Robin 模式下，连续请求在不同端点间轮转。
- [ ] AC5:流式请求首块前探测到上游错误（如 401），不发送 SSE header 且 fallback 到下一候选。
- [ ] AC6:流式已开始输出后上游错误，不 fallback，合成错误终端事件。
- [ ] AC7:错误分类正确：400 不切换、429/5xx 切换、流式已发不切换。
- [ ] AC8:冷却生效：失败端点写入 `cooldown_until`，冷却期内跳过。
- [ ] AC9:OAuth token 过期前自动刷新，刷新后转发成功；刷新失败则冷却+fallback。
- [ ] AC10:模型映射生效：`model_alias` 解析后转发请求 body 的 model 被改写；Claude Code 角色 haiku→正确上游模型。
- [ ] AC11:请求摘要日志记录全部必填字段，不记录禁止字段。
- [ ] AC12:fallback 链路由轨迹在日志中完整可查。
- [ ] AC13:路由管理 API 返回两条路由、设置与候选端点状态。
- [ ] AC14:前端路由页与日志页正常工作，替换占位内容。
- [ ] AC15:质量门通过——`cargo fmt --check`、`cargo check`（0 warning）、`cargo clippy --all-targets -- -D warnings`、`npm run build`。

## 暂不纳入范围

- `/v1/*` OpenAI-compatible 多端点转发。
- 真实链路测试器与流式调试器 UI。
- 导入导出。
- 复杂评分/测速型故障转移。
- 多规则 routes 引擎（企业网关级功能）。
