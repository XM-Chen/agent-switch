# 修复媒体 passthrough multipart 端到端失败(P1-1)

> 父任务:`07-03-fix-audit-p1-defects`。审计来源:`codebase-audit` 报告 §3 P1-1。

## Goal

修复媒体 passthrough(`/v1/images/edits`、`/v1/images/variations`、`/v1/audio/transcriptions`、`/v1/audio/translations` 等)对 multipart/form-data 请求体恒返回 502"请求体缺少 model 字段"的缺陷,使原始二进制 body 端到端透传成功。

## Background(代码事实)

审计报告 P1-1(`proxy/mod.rs:304`)根因:`model_mapper.resolve_and_rewrite` 在 `is_passthrough` 判断**之前**无条件运行(208),而 multipart body 在 102 行被 `serde_json::from_slice` 解析失败成 `Value::Null`,传给 model_mapper 后因缺 `model` 字段报错(216-223),`continue` 跳过该端点,所有候选耗尽后 502。`is_passthrough` 的原始二进制转发分支(304-306)在 model 映射之后,本路径永远不可达。

代码复核确认:
- `mod.rs:101-105`:`req_json` 对 multipart 为 `Value::Null`。
- `mod.rs:108-111`:`original_model` 从 `req_json.get("model")` 取,multipart 时为 None。
- `mod.rs:208`:`model_mapper.resolve_and_rewrite(&mut req_body_clone)` 无条件调用,Null body 报"请求体缺少 model 字段"。
- `mod.rs:304-306`:`is_passthrough` 时用 `body_bytes.to_vec()`,但永远走不到。

## Requirements

- `is_passthrough` 为 true 时,跳过 `model_mapper.resolve_and_rewrite`(不要求 JSON body 含 model key)。
- passthrough 路径仍需 `upstream_model`(用于模型锁检查、日志、failover 记录):从 `original_model`(若客户端在 multipart 字段外仍传了 model query/header)或 endpoint 默认模型推导,不依赖 JSON body。
- 保持 `is_passthrough` 的原始二进制 body 透传(304-306)不变。
- 不引入回归:`cargo test` 全绿、`cargo clippy -D warnings` 0、`tsc` 0 error、`npm run build` 成功。

## Acceptance Criteria

- [ ] 新增单测/集成测试:对 passthrough 能力(`images`/`audio`)发 multipart 请求,mock 上游返回 200,断言代理转发成功(非 502)、上游收到原始 multipart body。
- [ ] 新增单测:passthrough 路径下 `mapping_result` 仍可取到 `upstream_model`(来自 endpoint 默认或 original_model),模型锁检查不因缺 model 崩。
- [ ] 现有 `proxy/mod.rs` / `integration_tests.rs` 全绿,无回归(同协议 JSON 路径、跨协议翻译路径仍正常)。
- [ ] `cargo test` + `cargo clippy -D warnings` + `tsc --noEmit` + `npm run build` 全绿。

## Out of Scope

- passthrough 响应的 SHA256/content-length 记录(已实现,不动)。
- 非 passthrough 的 JSON 路径 model 映射(不动)。
- P1-2 / P1-3 / P1-4 / P1-5(各自独立子任务)。

## Notes

- 修复须对照 `app-stack-conventions.md`「管道顺序」与「images/audio 媒体透明流转」约定:passthrough 使用原始二进制 body,不经过 translator。本修复让该约定在 model_mapper 之前生效。
- `model_mapper` 当前对 Null body 报错是合理防御;修复点是**绕过调用**,不是让 model_mapper 接受 Null。
