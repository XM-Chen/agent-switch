# 设计 — 路由故障转移主循环修复

> 配套 `prd.md`。只写技术设计：接线方式、流式架构、依赖前提、错误流转、兼容与回滚。不重复 PRD 的需求条目。

## 0. 依赖前提（必须先处理）

- **reqwest 需启用 `stream` feature**。当前 `Cargo.toml`：`reqwest = { version="0.12", features=["json","rustls-tls"], default-features=false }`，**无 `stream`**，因此 `Response::bytes_stream()` 不可用。修复 R2 前必须改为 `features=["json","rustls-tls","stream"]`。这是本任务唯一的依赖变更，不增删其它 crate。
- `futures = "0.3"`、`bytes = "1"` 已在依赖中，StreamGuard 已基于 `futures::Stream` 实现，无需新增。
- 不改任何 DB schema（v1–v6 不动）。

## 1. 主循环重构总览

`http/proxy/mod.rs::RouteProxy::proxy_request` 当前是一个大 `while failover.can_continue()` 循环，内联了 selector→model_mapper→auth→（伪）translate→forward→log。修复保持这个结构，只在四个点接线：

```
循环体内：
  ① selector.next(failed_ids, model_lock_check)   ← R3：回调查 model_locks
  ② model_mapper.resolve_and_rewrite             （已正确，保留；R5.2 附带 hash）
  ③ auth 注入                                     （已正确，保留）
  ④ translate_request（跨协议）                   ← R1：去掉 let _ = translator
  ⑤ forward：
       非流式 → bytes + (跨协议) translate_response   ← R1.2
       流式   → bytes_stream + StreamGuard + 逐行 translate_stream_line  ← R2
  ⑥ 失败 → record_failure → 写 endpoints.cooldown_until / model_locks   ← R4/R3.3
循环后：
  全部失败日志用循环内 last_mapping，不用 final_result   ← R5.1
```

## 2. 协议转换接线（R1）

### 请求方向

替换当前 `proxy/mod.rs:242-267` 的 `translated_body` 计算块：

```rust
let translated_body: Vec<u8> = if is_passthrough {
    body_bytes.to_vec()                                  // images/audio 二进制直通
} else {
    let mut to_send = req_body_clone.clone();            // 已被 model_mapper 改写过 model
    if protocol_from != protocol_to {
        let upstream_model = mapping_result
            .as_ref().map(|m| m.upstream_model.clone()).unwrap_or_default();
        match self.translator_registry.resolve(&protocol_from, &protocol_to) {
            Ok(t) => {
                if let Err(e) = t.translate_request(&mut to_send, &upstream_model) {
                    failover.record_failure(&endpoint,
                        &ProxyError::new(ProxyErrorKind::ProtocolError, e), elapsed);
                    continue;
                }
            }
            Err(e) => { /* 同上 record_failure ProtocolError + continue */ }
        }
    }
    serde_json::to_vec(&to_send).unwrap_or_default()
};
```

要点：Passthrough（`from==to`）由 `resolve` 自动返回，其 `translate_request` 是 no-op，所以同协议无需特殊分支；但为省一次 clone/序列化，同协议可直接走 `serde_json::to_vec(&req_body_clone)`。

### 响应方向（非流式）

成功且非流式、跨协议时，对响应 JSON 调用 `translate_response`：

```rust
let resp_bytes = resp.bytes().await.unwrap_or_default();
let out_bytes = if !is_passthrough && protocol_from != protocol_to {
    let mut resp_json: Value = serde_json::from_slice(&resp_bytes).unwrap_or(Value::Null);
    if let Ok(t) = self.translator_registry.resolve(&protocol_to, &protocol_from) {
        // 注意方向反转：响应从上游协议(protocol_to)转回入站协议(protocol_from)
        let _ = t.translate_response(&mut resp_json);
    }
    serde_json::to_vec(&resp_json).unwrap_or_else(|_| resp_bytes.to_vec())
} else {
    resp_bytes.to_vec()
};
```

**关键**：响应转换方向是 `protocol_to → protocol_from`（与请求相反）。需确认注册表中有反向转换器（已有：四方向都注册了）。

## 3. 流式架构（R2）—— 本任务核心设计

### 流式判定

用请求体 `stream` 字段而非 HTTP 方法：

```rust
let is_stream = helpers::is_streaming(&req_json);   // 替换 method == POST
```

### 流式转发流程

```
1. resp = client.request(...).send().await        （成功且 status 2xx）
2. let upstream_stream = resp.bytes_stream();       （需 reqwest stream feature）
3. let mut guard = StreamGuard::new();
4. let buffered = guard.buffer_first_chunk(is_stream, status, upstream_stream).await;
   ├─ Err(ProxyError) 且 !stream_started → record_failure + continue（可 fallback）
   └─ Ok(BufferedResult{ remaining_stream, .. })
5. failover.stream_started = guard.is_stream_started();  // 置位后禁切
6. 跨协议：把 remaining_stream 包一层逐行 translate_stream_line 的适配流
   同协议：remaining_stream 直接用
7. Body::from_stream(adapted_stream) 返回客户端
8. 写日志（stream=true，first_token_ms 可选）
```

### 跨协议流式行转换适配器

`remaining_stream` 是 `Stream<Item=Result<Bytes,_>>`，按 SSE 行边界切分后逐行 `translate_stream_line`。第一版用一个有状态的 `StreamContext`，包成 `async_stream` 风格或手写 `futures::stream::unfold`。设计取舍：

- **同协议直通**（绝大多数场景：Claude Code→Anthropic、Codex→responses）：零转换开销，`remaining_stream` 原样下发。
- **跨协议流式**（Claude Code→OpenAI Chat 等）：逐行转换。需维护跨 chunk 的行缓冲（一个 chunk 可能含半行）。第一版实现一个 `SseLineDecoder`：累积字节 → 按 `\n` 切完整行 → 每行 `translate_stream_line` → 重新拼接下发。
- 流式异常：用 `helpers::build_error_event(msg, protocol_from)` 合成终端事件后结束流。

> 复杂度提示：跨协议流式是本任务最易出错处。若实现时间紧，可分两步交付——先接通同协议流式（StreamGuard + 直通，覆盖主要工具链路），跨协议流式转换作为 R2.5 的后续增量。但 PRD AC4/AC5/AC6 要求 StreamGuard 接入，至少同协议流式必须本任务完成。

## 4. 模型级锁接入（R3）

`selector.next()` 的回调当前是 `|eid| { let _ = eid; true }`。改为：

```rust
let db = &self.db;
let upstream_model_for_lock = mapping_result_model.clone(); // 本轮模型
selector.next(&failover.failed_ids, |eid| {
    match crate::db::dao::model_locks::get_active_lock(db, eid, &upstream_model_for_lock) {
        Ok(Some(_)) => false,   // 有未过期锁 → 跳过
        _ => true,
    }
})
```

**顺序问题**：model_mapper 在 selector.next 之后才知道 upstream_model，但锁检查在 selector.next 内。解决：第一版锁检查用「请求体原始 model 名」或对当前候选端点先做一次轻量 alias 解析；或把锁粒度放在 endpoint 级回退。设计决策：**第一版锁键用 `endpoint_id + 原始 model 名`**（model_mapper 改写前），保持回调无副作用；upstream_model 级锁留待 model_mapper 重构后增强。404 写锁时用实际 upstream_model（此时已知）。

404/model_not_found 写锁：

```rust
if status == 404 /* 且 body 含 model_not_found */ {
    let until = now + long_cooldown;
    let _ = model_locks::set_lock(db, &endpoint.id, &upstream_model, &until_iso, Some("model_not_found"));
}
```

## 5. 端点冷却持久化（R4）

`record_failure` 已返回冷却秒数。接线：

```rust
let cooldown_secs = failover.record_failure(&endpoint, &err, elapsed);
if !test_only && cooldown_secs > 0 {
    let until = OffsetDateTime::now_utc() + Duration::seconds(cooldown_secs);
    let until_iso = until.format(&Iso8601::DEFAULT).unwrap_or_default();
    let _ = endpoints::update(&self.db, &endpoint.id, EndpointUpdate {
        cooldown_until: Some(Some(until_iso)),
        last_failure_at: Some(Some(now_iso)),
        last_error_kind: Some(Some(format!("{}", err.kind))),
        ..Default::default()
    });
}
```

冷却写失败只 `tracing::warn`，不中断循环。

## 6. 死代码清理（R5）

- 删除 `let final_result: Option<...> = None;`（mod.rs:180）和末尾对它的引用；改用循环内 `let mut last_mapping: Option<ModelMappingResult> = None;`，每轮赋值。
- 删除 `body_hash_sync` 空函数；R5.2 采用方案 A：model_mapper 改写 model 后，主循环对改写后的 `req_body_clone` 重算 `RequestLogEntry::hash_body(&serde_json::to_vec(&req_body_clone))` 作为日志 hash（仅日志用途，不影响转发）。

## 7. 兼容与回滚

- 行为兼容：同协议非流式路径（当前唯一真正能工作的路径）行为不变。
- 新生效路径：跨协议转换、流式、模型锁、冷却持久化——这些当前是坏的或 no-op，修复不会破坏已有正确行为。
- 回滚：纯代码改动（含 1 处 Cargo.toml feature），`git checkout` 相关文件即可；无 schema 变更，无数据迁移。

## 8. 测试设计

- 单元测试（不需真实上游）：
  - 跨协议请求转换：构造 anthropic body → 经 `resolve("anthropic","openai-chat").translate_request` → 断言出现 `messages` 且无 `system` 顶层字段。（其实是验证主循环会调用，可抽一个 `build_translated_request` 纯函数便于测。）
  - SSE 行解码器 `SseLineDecoder`：喂入跨 chunk 半行，断言正确重组。
  - 冷却时长 → ISO 写入参数构造正确。
- 运行时（需 mock 上游或真实端点，标注人工）：AC1–AC9 的端到端。

## 9. 实现风险

- **跨协议流式逐行转换**：最高风险，跨 chunk 行缓冲 + 有状态 StreamContext 容易漏数据。缓解：先同协议流式打通，跨协议流式单独小步 + 充分单测 SseLineDecoder。
- **响应转换方向**：请求 `from→to`，响应 `to→from`，方向写反会静默产生错误格式。缓解：单测两个方向各一条。
- **reqwest stream feature**：加 feature 后编译期即可发现 API 可用性，低风险。
- **selector 回调借用**：闭包捕获 `&self.db` 与 `failover.failed_ids` 同时存在，注意借用检查；必要时先取出 `failed_ids` 引用。
