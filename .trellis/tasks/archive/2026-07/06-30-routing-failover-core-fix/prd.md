# 路由故障转移主循环修复

## 目标

把 `routing-failover-core` 已实现但未接入主转发循环的子模块真正接线，使 `/claude-code/*`、`/codex/*`、`/v1/*` 三条路由的跨协议转换、流式 SSE 转发、模型级锁、端点冷却持久化端到端生效。本任务只做"接线 + 缺陷修复"，不新增功能、不改协议转换器内部算法、不动数据库 schema。

## 背景

`requirement-alignment-audit` 任务审计发现：`http/proxy/mod.rs::RouteProxy::proxy_request` 主循环虽然各子模块（translator 四方向 + Passthrough、StreamGuard、model_locks DAO、failover 状态机、oauth_refresh、endpoints.update cooldown 字段）均已实现并通过单测，但主循环未正确调用它们。详见 `.trellis/spec/guides/app-stack-conventions.md` 第 10 节「路由转发主循环存在未接入的子模块」。

已确认的代码事实（commit `503cecf33`）：

- `services/translator/mod.rs`：`TranslatorRegistry::resolve(from,to)` 返回 `Arc<dyn Translator>`，`from==to` 返回 Passthrough；`Translator` trait 含 `translate_request(&mut Value,&str)`、`translate_response(&mut Value)`、`translate_stream_line(&str,&mut StreamContext)`。四方向转换器 + helpers SSE 工具（`extract_sse_data`/`is_sse_event_end`/`build_error_event` 等）已就位。
- `http/proxy/stream_guard.rs`：`StreamGuard::buffer_first_chunk(stream,status,body_stream)` 返回 `BufferedResult { first_chunk, remaining_stream }`，首块错误返回 `ProxyError`，正常则置 `stream_started=true`。当前 `cargo check` 对其报 "never constructed/used"。
- `db/dao/model_locks.rs`：`get_active_lock(db,endpoint_id,model_name)`、`set_lock(...)`、`clear_expired(db)` 已实现。
- `db/dao/endpoints.rs`：`update(db,id,EndpointUpdate{ cooldown_until: Some(Some(iso)), .. })` 可写 `cooldown_until`。
- `http/proxy/failover.rs`：`record_failure` 返回冷却秒数；`calculate_cooldown_seconds` 已实现 429/529/5xx 策略。
- `http/proxy/mod.rs`：`reqwest::Client` 已建，但只用 `.send()` + `.bytes()`，未用 `bytes_stream()`。

## 需求

### R1 跨协议转换器接入（F1，关键）

- R1.1 主循环跨协议分支（`protocol_from != protocol_to` 且非 passthrough）必须调用 `translator.translate_request(&mut req_body_clone, &upstream_model)` 后再序列化转发，不得丢弃转换器。
- R1.2 非流式成功响应跨协议时，必须把响应体反序列化为 JSON、调用 `translator.translate_response(&mut resp_json)`、再序列化回客户端。
- R1.3 同协议（`protocol_from == protocol_to`）走 Passthrough，body 不做结构转换（仅 model 已由 model_mapper 改写）。
- R1.4 转换失败（`translate_request`/`translate_response` 返回 Err）归类为 `ProtocolError`，按错误分类不切换、回传可解释错误。

### R2 流式 SSE 转发接入（F2，关键）

- R2.1 判断流式：以请求体 `stream==true`（用 `helpers::is_streaming` 或读取 body.stream 字段）为准，替换当前 `is_stream = method == POST` 的错误启发式。
- R2.2 流式成功响应用 `reqwest` 的 `bytes_stream()` + `StreamGuard::buffer_first_chunk` 缓冲首块探测错误。
- R2.3 首块为错误且未发输出 → 走故障转移（可切换）。
- R2.4 首块正常 → 置 `failover.stream_started=true`，把 `remaining_stream` 作为 `Body::from_stream` 流式回传客户端；此后禁止任何 fallback。
- R2.5 跨协议流式：对每个 SSE 行调用 `translator.translate_stream_line(line, &mut ctx)` 转换后再下发；同协议流式直通。
- R2.6 流式中断/异常按目标协议合成终端事件（复用 `helpers::build_error_event`）。

### R3 模型级锁接入（F3）

- R3.1 `selector.next()` 的 `model_lock_check` 回调改为查询 `model_locks::get_active_lock(db, endpoint_id, model_name)`，存在未过期锁返回 false（跳过该端点）。
- R3.2 候选端点的 model_name 取该端点本轮将使用的 upstream_model（由 model_mapper 解析结果决定）。
- R3.3 404/model_not_found 类错误触发 `model_locks::set_lock` 写入模型级锁（长冷却）。

### R4 端点冷却持久化（F4）

- R4.1 `record_failure` 返回的冷却秒数 > 0 时，计算 `cooldown_until = now + secs` 并通过 `endpoints::update` 写入 DB。
- R4.2 冷却写入失败只记日志，不中断故障转移主流程。
- R4.3 测试模式（test_only）不写冷却（沿用 `record_failure` 现有 test_only 短路）。

### R5 死代码与 no-op 清理（F5、F6）

- R5.1 移除 `final_result` 死变量，"全部失败"日志的 `requested_model` 改用循环内跟踪的最后一次 mapping_result。
- R5.2 `body_hash_sync` no-op：模型改写后重算 body hash 用于日志 `request_body_hash`，或在 model_mapper 返回时附带改写后 hash；二选一，去掉空函数。

### R6 质量门与回归

- R6.1 修复后 `cargo check` 不得再出现 StreamGuard/BufferedResult/model_locks 相关 "never used" 警告。
- R6.2 现有单测（translator、migration、portability）全绿；新增的接线逻辑至少补一条主循环跨协议转换的单测或 translator 调用点测试。

## 验收标准

- [ ] AC1：跨协议路由调用转换器——构造 anthropic→openai-chat 路由，断言转发到上游的 body 是 Chat 格式（messages 结构），不是原始 Anthropic 格式。
- [ ] AC2：同协议路由 Passthrough——anthropic→anthropic body 结构不变。
- [ ] AC3：非流式跨协议响应被 `translate_response` 转回入站协议格式。
- [ ] AC4：流式请求用 bytes_stream + StreamGuard，SSE 逐 chunk 下发，不再全量缓冲。
- [ ] AC5：流式首块错误（如 401）→ 未发 header → fallback 到下一候选。
- [ ] AC6：流式已发输出后 → `stream_started=true` → 禁止 fallback。
- [ ] AC7：模型级锁生效——`get_active_lock` 命中的端点被 selector 跳过。
- [ ] AC8：端点冷却写 DB——失败端点 `endpoints.cooldown_until` 被更新，重启后仍生效（冷却期内跳过）。
- [ ] AC9：404/model_not_found 写入 `model_locks`。
- [ ] AC10：`final_result` 死代码移除；`body_hash_sync` no-op 移除或落实。
- [ ] AC11：质量门——`cargo fmt --check`、`cargo check`（0 warning，StreamGuard/model_locks 不再 never-used）、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全绿；`tsc --noEmit` 通过（前端若改动）。

## 暂不纳入范围

- `chain-testing-debugger` 流式调试器前端 UI（独立子任务）。
- 模型刷新并发限流（D3 小缺口，独立任务）。
- 数据库 schema 变更（本任务只用现有表/字段）。
- 协议转换器内部算法的正确性改进（只接线，不改 translator 内部映射逻辑；若发现 translator bug 单独记录）。
- `npm run build` 环境依赖（rolldown binding）修复。
- Round-Robin 粘性会话增强（现状已实现基础轮询，不在本任务扩展）。

## 开放问题

- 无阻塞性开放问题。流式架构（`Body::from_stream` + 逐行转换）属设计决策，在 design.md 展开。
