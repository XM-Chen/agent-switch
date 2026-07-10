# 执行计划 — 路由故障转移主循环修复

> 配套 `prd.md` / `design.md`。按依赖顺序：依赖前提 → 易测纯函数抽取 → 主循环接线（请求转换 → 响应转换 → 冷却 → 模型锁 → 流式）→ 清理 → 质量门 → 验证。

## 实现顺序

### 1. 依赖前提
- [ ] `src-tauri/Cargo.toml`：`reqwest` features 加 `"stream"`（→ `["json","rustls-tls","stream"]`）。`cargo check` 确认 `bytes_stream()` 可用。

### 2. 可测纯函数抽取（先抽再接，便于单测）
- [ ] `http/proxy/mod.rs`（或新建 `http/proxy/translate.rs`）：抽 `build_translated_request_body(registry, from, to, body, upstream_model) -> Result<Vec<u8>, ProxyError>`。
- [ ] 抽 `translate_response_body(registry, to, from, resp_bytes) -> Vec<u8>`（失败回退原 bytes）。
- [ ] 新增 `http/proxy/sse.rs`：`SseLineDecoder`（累积字节、按 `\n` 切完整行、暴露 `push(&[u8]) -> Vec<String>` 与 `flush() -> Option<String>`）。

### 3. 请求转换接线（R1.1, R1.3, R1.4）
- [ ] 替换 mod.rs `let _ = translator;` 块为 `build_translated_request_body` 调用；转换失败 → `ProtocolError` + record_failure + continue。

### 4. 响应转换接线（R1.2）
- [ ] 非流式成功分支：跨协议时 `translate_response_body(registry, protocol_to, protocol_from, resp_bytes)`（方向反转）。

### 5. 端点冷却持久化（R4）
- [ ] 三处 `let _ = cooldown_secs;` 替换为 `endpoints::update` 写 `cooldown_until`/`last_failure_at`/`last_error_kind`（非 test_only 且 secs>0）。失败仅 warn。

### 6. 模型级锁接入（R3）
- [ ] selector.next 回调改查 `model_locks::get_active_lock`（锁键 endpoint_id + 原始 model 名，见 design §4）。
- [ ] 404/model_not_found 错误分支 `model_locks::set_lock`（长冷却）。

### 7. 流式 SSE 接入（R2）—— 核心
- [ ] `is_stream` 改用 `helpers::is_streaming(&req_json)`。
- [ ] 成功分支按 `is_stream` 分流：
  - [ ] 流式：`resp.bytes_stream()` → `StreamGuard::buffer_first_chunk` → 首块错误可 fallback → 正常置 `stream_started` → `Body::from_stream` 下发。
  - [ ] 同协议流式直通；跨协议流式经 `SseLineDecoder` + `translate_stream_line` 适配（design §3）。
  - [ ] 流式异常 `helpers::build_error_event(msg, protocol_from)` 合成终端事件。
- [ ] 非流式：保留现有 bytes 路径 + 步骤 4 的响应转换。

### 8. 死代码清理（R5）
- [ ] 删 `final_result`，循环内 `last_mapping` 跟踪。
- [ ] 删 `body_hash_sync`，改写后重算日志 hash。

### 9. 单元测试（R6.2）
- [ ] 跨协议请求转换 1 条、响应转换方向 1 条、`SseLineDecoder` 跨 chunk 半行 1 条。

### 10. 质量门（AC11）
```bash
cd src-tauri
cargo fmt && cargo fmt --check
cargo check                                  # 0 warning，StreamGuard/model_locks 不再 never-used
cargo clippy --all-targets -- -D warnings
cargo test
cd .. && npx tsc --noEmit                     # 前端若有改动
```

### 11. 运行时验证（标注人工 / 需 mock 上游）
- [ ] 同协议非流式：Claude Code → Anthropic 端点 → 正确响应（回归，不应被破坏）。
- [ ] 跨协议非流式：anthropic 入站 → openai-chat 端点 → 上游收到 Chat 格式 body。
- [ ] 同协议流式：SSE 逐 chunk 到达客户端，不再全量缓冲。
- [ ] 流式首块 401 → 未发 header → fallback。
- [ ] 故障转移：首端点 429 → 切下一候选 → `endpoints.cooldown_until` 写入 DB → 重启冷却仍生效。
- [ ] 模型锁：404 → `model_locks` 写入 → 同模型后续跳过该端点。

## 回滚点
- 纯代码 + 1 处 Cargo feature；无 schema 变更。任一步出错 `git checkout` 对应文件。
- 流式风险最高：若跨协议流式打不通，先交付「同协议流式 + StreamGuard 接入」（满足 AC4/AC5/AC6 主路径），跨协议流式转换降级为后续增量并在 PRD 标注。

## 验证命令速查
```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo check && cargo clippy --all-targets -- -D warnings && cargo test
cd .. && npx tsc --noEmit
```
