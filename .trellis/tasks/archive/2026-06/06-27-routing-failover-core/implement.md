# 执行计划 — 路由与故障转移核心

> 配套 `prd.md` / `design.md`。按实现依赖顺序推进，从后往前：DB → DAO → 服务层(translator + proxy 各子模块) → API → 前端 → 质量门 → 运行验证。

## 实现顺序

### 1. 依赖与迁移（migration v5）

- [x] `src-tauri/Cargo.toml` 增 `sha2 = "0.10"`（已有，确认）；确认 `reqwest` 已启用 `stream` feature（如无则加）。
- [x] `db/migrations.rs` 增 migration v5：`route_settings`、`request_logs`、`model_locks`（见 design §2）。不动 v1-v4。
- [x] `db/dao/mod.rs` 注册三个新模块。

### 2. DAO 层

- [x] `db/dao/route_settings.rs`：`get(id)`、`upsert(id, label, strategy, protocol_type, failover_enabled, max_switches, same_account_retries, cooldown_multiplier)`、`list_all()`。
- [x] `db/dao/request_logs.rs`：`insert(log)`、`get(id)`、`list(filter)`——支持 tool/status/时间范围/limit/offset 筛选。
- [x] `db/dao/model_locks.rs`（或用内存替代方案——先落表 DAO，性能问题可换内存集）：`set_lock(id, endpoint_id, model_name, locked_until, lock_reason)`、`get_active_lock(endpoint_id, model_name)`、`clear_expired()`。
- [x] `db/dao/endpoints.rs` 小改：新增 `list_by_protocol(protocol_type)` 加上以下行的函数，复用 `list_enabled` 的 SQL。

### 3. translator 协议转换器服务层

- [x] `services/translator/mod.rs`：`Translator` trait（`key`、`translate_request`、`translate_response`、`translate_stream_line`）、`TranslatorRegistry`（register/get/resolve）、`PassthroughTranslator`（同协议直转，model 外原样 body，只重写 model 字段）。
- [x] `services/translator/native.rs`：Native Passthrough 实现——request/response/stream_line 均原样返回，只 apply model 改写（外部 model_mapper 已改好 body）。
- [x] `services/translator/anthropic_openai.rs`：Anthropic → OpenAI Chat 与反向转换。
  - Anthropic 请求 → Chat：roles 映射（assistant→assistant，user→user，system→system）、content blocks（text→text，tool_use→tool_calls）、thinking 配置处理、max_tokens/stop_sequences/stream 映射。
  - Chat 响应 → Anthropic：tool_calls→tool_use、stop_reason 映射（stop→end_turn，tool_calls→tool_use）、usage 格式转换。
  - SSE：Anthropic `content_block_delta`/`content_block_start`/`message_delta` ↔ Chat `choices[0].delta`。
- [x] `services/translator/openai_responses.rs`：OpenAI Chat → responses 与反向转换。
  - Chat → Responses：messages→input、model、tools→tools、stream 几种模式的映射。
  - Responses → Chat：output→choices、usage 与 token 计数格式转换。
  - SSE：Responses `response.created`/`response.in_progress`/`response.completed` ↔ Chat `choices[0].delta` 流式事件格式转换。
- [x] `services/translator/helpers.rs`：共享工具函数——内容块转换、token 计数计算的统一入口、流式行解析与缓冲、SSE 终端事件合成。

### 4. proxy 转发层

- [x] `http/proxy/mod.rs`：`RouteProxy` 门面结构体——持有 `TranslatorRegistry`、`reqwest::Client`（连接池）、`FailoverState`。主干方法 `proxy_request(route_id, path, req_body, stream)`。
- [x] `http/proxy/error.rs`：错误分类——`FailoverError` 枚举（`NetworkError`/`Timeout`/`UpstreamError{status,body}`/`ProtocolError`/`AuthError`/`AllExhausted`）+ `should_failover(&self) -> bool` 实现父任务错误分类规则。
- [x] `http/proxy/selector.rs`：`EndpointSelector`——`next()` 实现 Fill-First 和 Round-Robin。参数：候选端点列表、当前 FailoverState（排除 failed_ids、检查 cooldown）。返回 `Option<&EndpointRow>`。
- [x] `http/proxy/model_mapper.rs`：`ModelMapper`——解析请求 body.model，调用 `model_alias::resolve`，角色映射（haiku/sonnet/opus/fable 到上游模型，剥离 `[1M]`），改写 body.model。返回 `(upstream_model, resolved_alias, resolved_scope)`。
- [x] `http/proxy/auth_injector.rs`：`AuthInjector`——检查端点 auth_mode；API Key 方式解密 `api_key_encrypted` → 注入 header；OAuth 方式调用 `oauth_refresh` 后取 `access_token` → `Authorization: Bearer`。返回 `(headers_map, used_credential_type)`。
- [x] `http/proxy/oauth_refresh.rs`：`OAuthTokenRefresher`——检查 `CodexCredentials.expires_at`，若<=60s 过期或已过期则用 `refresh_token` 请求新 token，加密回写 accounts。刷新失败标记端点冷却。
- [x] `http/proxy/stream_guard.rs`：`StreamGuard`——SSE 首块缓冲哨兵。`buffer_first_chunk()` 等待首个上游 chunk（或错误/timout），成功后提交 SSE header + 开始逐行转发到客户端 response body。已发后禁止 failover。
- [x] `http/proxy/failover.rs`：`FailoverState` 实现——while 循环 + excludeSet。`run()` 方法驱动整个转发链路（selector → model_mapper → oauth_refresh(按需) → auth_injector → translator → forwarder → response）。按 `should_failover` 决策循环或退出。
- [x] `http/proxy/logger.rs`：`RequestLogger`——转发成功后/n 次 failover 耗尽后写 `request_logs` 表。不存 prompt/messages/headers/密钥。

### 5. HTTP 入口替换

- [x] 替换 `http/placeholders.rs` 中 `/claude-code/{*path}` 与 `/codex/{*path}` 的 501 占位为真实转发 handler：
  - handler 解析 path，route_id = 'claude-code' 或 'codex'。
  - 按 path 决定 `protocol_from`（`/v1/messages` → anthropic，`/backend-api/codex/responses` → openai-responses 等）。
  - 调用 `RouteProxy::proxy_request(route_id, path, body, is_stream)`。
  - 返回转发响应。
- [x] `http/router.rs` 更新：添加 `/api/routes` 和 `/api/logs` 挂载。

### 6. 管理 API

- [x] `http/api/routes.rs`：`GET /api/routes`（列出 route_settings + 候选端点实时状态）、`PUT /api/routes/{id}`（更新路由设置）。
- [x] `http/api/logs.rs`：`GET /api/logs`（分页过滤查询）、`GET /api/logs/{id}`（单条详情）。
- [x] `http/api/endpoints.rs` 小改：创建/更新端点时校验 `protocol_type` 取值。

### 7. 前端

- [x] `lib/api.ts`：增 `routesApi`（`getRoutes`/`updateRoute`）+ `RouteSettings`/`RouteDetail` 类型；增 `logsApi`（`listLogs`/`getLog`）+ `RequestLogEntry` 类型；queryKey `['routes']` / `['logs']`。
- [x] `pages/RoutesPage.tsx`：替换占位——两条路由卡片，含策略选择（dropdown fill-first/round-robin）、候选端点列表（协议+优先级+实时状态 badge）、冷却/重试参数配置。
- [x] `pages/LogsPage.tsx`：替换占位——日志摘要列表（表格/List）、tool/status 过滤、分页；单条点击展示详情 panel（fallback_chain 可视化、协议转换路径、token 用量）。
- [x] 路由页与日志页添加导航菜单（如 AppShell 侧栏）。

### 8. 协议转换器注册初始化

- [x] `lib.rs` 或 `http/mod.rs`：注册 translator 到 `AppState.translator_registry`。
- [x] `AppState` 加字段：`translator_registry`、`route_proxy`。

### 9. 质量门（AC15）

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo fmt && cargo fmt --check
cargo check                                   # 0 warning
cargo clippy --all-targets -- -D warnings
cd .. && npm run build
```

### 10. 运行验证

- [x] 迁移 v5 成功。
- [x] 启动后，`GET /api/routes` 返回两条默认路由。
- [x] Claude Code 端点已配（Anthropic 协议）→ `/claude-code/v1/messages` 转发成功，返回上游响应。
- [x] Codex 端点已配（responses 协议）→ `/codex/v1/responses` 转发成功。
- [x] 流式请求：SSE 正确转发，首块前错误不发送 header。
- [x] 多端点故障转移：首个端点返回 429 → 自动切到下一候选。
- [x] Round-Robin：连续请求在端点间轮转。
- [x] 冷却：失败端点写入 `cooldown_until`，冷却期内被跳过。
- [x] 模型映射：alias 正确重写 body.model。
- [x] 日志：请求后 `request_logs` 表写入正确字段。
- [x] fallback 链：`request_logs.fallback_chain` 含每次切换记录。
- [x] OAuth 刷新：选中 OAuth 端点前自动刷新（需 Codex 会话）。

## 风险与回滚点

- **风险：协议转换写错导致请求体损坏**。重点测：Anthropic→Chat 的 roles 映射、tool_use/tool_calls 转换、SSE 事件格式。每方向转换器写单元测试覆盖关键映射路径。
- **风险：流式分块转换期间挂起/漏数据**。流式测试必须覆盖完整流和不完整分块（中间截断）场景。
- **风险：OAuth 刷新并发竞态**。`oauth_refresh` 必须互斥（`tokio::sync::Mutex`），避免两个转发请求同时刷新同一过期 token。
- **风险：模型级锁导致候选端点被误跳**。锁键包含 `endpoint_id + model`，不跨端点污染。
- **风险：占位符 token 误传到上游**。`auth_injector` 必须用真实凭据覆盖而非追加；入站 auth header 必须剥离。
- **回滚**：本任务主要是新模块 + 新迁移 + 页面替换；回滚即 `git checkout` 相关文件（迁移 v5 已应用的 DB 在开发期可删库重建 + 手动 `drop table request_logs; drop table route_settings; drop table model_locks; delete from schema_migrations where version=5`）。

## 实现方式

本任务体量大（30+ 新源文件），建议拆为 sub-agent 并行实现：
1. Agent A：DB + DAO + 常量
2. Agent B：translator 四方向转换器（含单元测试）
3. Agent C：proxy 转发层（selector → auth → stream guard → failover → logger）
4. Agent D：HTTP API + 前端
每 Agent 独立实现后由主 session 做集成审阅与调整。

## 验证命令速查

```bash
# 质量门
cd src-tauri && cargo check && cargo clippy --all-targets -- -D warnings && cargo test
cd .. && npm run build

# 启动服务
cargo run  # 或 tauri dev

# 验证路由 API
curl -s http://127.0.0.1:42567/api/routes | jq .

# 验证转发（假设有 Anthropic 端点）
curl -s -X POST http://127.0.0.1:42567/claude-code/v1/messages \
  -H 'Content-Type: application/json' \
  -H 'x-api-key: dummy-placeholder' \
  -d '{"model":"claude-sonnet-4-20250514","max_tokens":100,"messages":[{"role":"user","content":"Hello"}]}'

# 验证日志
curl -s http://127.0.0.1:42567/api/logs?limit=5 | jq .
```
