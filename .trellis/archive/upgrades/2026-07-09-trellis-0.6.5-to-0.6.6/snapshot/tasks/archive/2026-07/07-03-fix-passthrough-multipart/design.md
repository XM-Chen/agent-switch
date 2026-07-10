# 技术设计 — 修复媒体 passthrough multipart 端到端失败(P1-1)

## 边界

仅改 `src-tauri/src/http/proxy/mod.rs` 的 `proxy_request` 主循环(约 126-330 区段),在 `model_mapper.resolve_and_rewrite`(208)之前加 `is_passthrough` 短路。可能小幅用 `model_mapper.rs` 的 `ModelMappingResult` 构造(不改其逻辑)。不动 translator、不动 capability.rs。

## 当前数据流(主循环)

1. `mod.rs:96-105`:读 body → `req_json`(multipart 时 `Value::Null`)。
2. `mod.rs:108-111`:`original_model = req_json.get("model")`(multipart → None)。
3. `mod.rs:126`:`is_passthrough = matches!(required_capability, Some("images")|"audio")`。
4. `mod.rs:208`:`model_mapper.resolve_and_rewrite(&mut req_body_clone)` — **无条件**,Null body 报"请求体缺少 model 字段"。
5. `mod.rs:216-224`:Err → record_failure + continue,耗尽候选 → 502。
6. `mod.rs:304-306`:`is_passthrough` 用原始 body — 永远走不到。

## 修复设计

在 208 行前判断 `is_passthrough`,passthrough 时**短路**构造 `ModelMappingResult`,不调用 `resolve_and_rewrite`:

```
let mapping_result = if is_passthrough {
    // passthrough (multipart) 不经过 model_mapper:body 非 JSON,无 model key。
    // upstream_model 取 original_model(若客户端在 query/header 传了)或空,
    // 模型锁检查时 upstream_model 为空则跳过锁检查。
    let model = original_model.clone().unwrap_or_default();
    Some(ModelMappingResult {
        original_model: model.clone(),
        upstream_model: model,
        resolved_alias: String::new(),
        resolved_scope: "passthrough".to_string(),
    })
} else {
    match model_mapper.resolve_and_rewrite(&mut req_body_clone) { ... }  // 原逻辑
};
```

- `loop_body_hash`(211-213):passthrough 路径用 `body_bytes` 的 hash(已有 `body_hash` 在 100 行算过),不依赖 `req_body_clone`。
- 模型锁检查(228-243):`upstream_model` 为空时跳过 `get_active_lock`(passthrough 媒体端点通常无模型锁语义)。
- `translated_body`(304-306):passthrough 用 `body_bytes.to_vec()` — 现在可达。

## upstream_model 来源权衡

multipart 请求 model 通常在 form field 而非 JSON body,本修复**不解析 multipart form**(避免引入 multipart 解析依赖)。取 `original_model`(line 108,JSON body 的 model,multipart 时 None)→ 空。媒体端点上游用 endpoint 配置路由,model 字段对上游非必需(OpenAI images/audio 端点 model 可选或用 endpoint 默认)。若后续需精确 model,另立任务做 multipart form 解析(超出 P1-1 范围)。

## 兼容性

- 不动非 passthrough 路径(JSON body 的 model 映射不变)。
- 不动 translator(304-306 已用 `is_passthrough` 跳过 translator,保持)。
- 不动 capability.rs、model_mapper.rs 逻辑。
- `ModelMappingResult` 直接构造(字段都 pub),无需新 API。

## 测试设计

新增 `integration_tests.rs`:
1. `passthrough_multipart_forwarded_not_502`:构造 multipart body(非 JSON),required_capability=images,mock 上游返回 200,断言代理非 502、上游收到原始 multipart bytes。
2. `passthrough_skips_model_mapper`:passthrough 路径断言 `mapping_result.scope == "passthrough"`、`resolve_and_rewrite` 未被调用(可用日志/副作用断言)。
3. `passthrough_empty_upstream_model_skips_lock`:upstream_model 为空时模型锁检查跳过,不 panic。

## 风险/回滚

- 风险:passthrough 路径若 endpoint 确实需要 model(某些上游 images API 必填 model),空 upstream_model 可能上游 400。这是上游契约问题,非代理 bug;若出现,后续做 multipart form 解析。本任务以"不再 502、原始 body 透传"为验收。
- 回滚点:单文件 mod.rs;`git revert` 单 commit。
