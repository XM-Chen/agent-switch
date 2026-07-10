# 执行计划 — OpenAI-compatible v1 多端点

> 配套 `prd.md` / `design.md`。按实现依赖顺序推进。

## 实现顺序

### 1. Selector 与管道扩展

- [ ] `http/proxy/constants.rs` 新增常量：`PROTOCOL_OPENAI_COMPATIBLE = "openai-compatible"`，capability 列表。
- [ ] `http/proxy/capability.rs`（新增）：`path_to_capability(path) -> Option<&str>` 映射函数、capability 验证工具。
- [ ] `http/proxy/selector.rs` 扩展：新增 `required_capability` 字段 / `set_required_capability()` / `filter_by_capability()`。
- [ ] `http/proxy/model_mapper.rs` 扩展：新增 `set_required_capability()` / `validate_capability()`。
- [ ] `db/dao/endpoint_models.rs` 新增：`has_capable_model(endpoint_id, capability) -> bool` / `list_capable(endpoint_id, capability) -> Vec<EndpointModelRow>`。

### 2. RouteProxy v1 适配

- [ ] `http/proxy/mod.rs` (RouteProxy)：`proxy_request` 新增 `required_capability` 参数（None 表示无限制）。通过 path_to_capability 确定。capability 传入 selector 和 model_mapper。
- [ ] `http/proxy/mod.rs`：capability 为 images/audio 时跳过 translator，设置 passthrough 模式。

### 3. 迁移与路由配置

- [ ] `db/migrations.rs` 增 migration v6：INSERT 默认 route_settings v1 条目（id=v1, protocol_type=openai-compatible, strategy=fill-first）。

### 4. GET /v1/models

- [ ] `http/api/v1_models.rs`（新增）：聚合 handler——查询 endpoint_models，去重，组装 OpenAI Models API 格式。
- [ ] 支持 `?capability=` 过滤参数。

### 5. HTTP 入口替换

- [ ] `http/router.rs`：替换 `/v1/{*path}` 501 占位为 v1 真实 handler：
  - v1_handler：非 `/v1/models` → 解析 path → capability → 调用 RouteProxy::proxy_request
  - v1_models_handler：`GET /v1/models` → 调用 v1_models::get_models
- [ ] `http/placeholders.rs`：移除 `/v1/{*path}` 路由（三条路由均已实现）。

### 6. 日志扩展

- [ ] `http/proxy/logger.rs`：RequestLogEntry 新增可选字段 `media_type`、`content_length`（i64）、`body_sha256_hash`。
- [ ] `db/dao/request_logs.rs`：INSERT 语句适配新增字段。
- [ ] `db/migrations.rs` migration v6：ALTER TABLE request_logs ADD COLUMN（如需要——如果新字段不改变表结构只在代码层处理则跳过）。

### 7. 别名能力校验

- [ ] `http/api/aliases.rs`：创建 alias 时，若请求中指定了 capability 字段，校验目标端点模型是否具备该能力。不具备返回 400。
- [ ] 校验逻辑在 `services/model_alias.rs` 或新增独立工具函数中实现。

### 8. 质量门（AC12）

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cd src-tauri && cargo fmt && cargo fmt --check
cargo check                                   # 0 error
cargo clippy --all-targets                    # 0 error (for v1 code)
cd .. && npm run build
```

### 9. 运行验证

- [ ] 迁移 v6 成功，`GET /api/routes` 返回三条路由。
- [ ] 配置 openai-chat 端点后：`POST /v1/chat/completions` 转发成功。
- [ ] 配置 responses 端点后：`POST /v1/responses` 转发成功。
- [ ] 配置 embeddings 模型后：`POST /v1/embeddings` 返回向量。
- [ ] `GET /v1/models` 返回聚合模型列表（包含各端点模型）。
- [ ] `GET /v1/models?capability=chat` 只返回 chat 模型。
- [ ] Images/Audio 透明流转（配置 images 端点后验证）。
- [ ] 能力过滤：用 chat-only 端点尝试 embeddings 请求 → 502（无可用端点）。
- [ ] alias 创建时能力校验：尝试创建能力不匹配的 alias → 400。
- [ ] 流式 chat/completions：SSE 正常转发。

## 风险与回滚点

- **风险：能力过滤过于严格导致无可用端点**——selector 预筛时如果所有端点都无 capable 模型，返回 AllExhausted 错误。用户应被引导检查端点模型列表是否正确配置了 capabilities。
- **风险：Images/Audio 首块探测误判**——如果上游 images 响应的第一块不是 JSON error，但首块包含 "error" 子串被误判为正文字误匹配。解法：首块探测只匹配 `Content-Type: application/json` + HTTP 非 2xx 状态码，不对二进制首块做字符串关键词匹配。
- **回滚**：本任务主要是 pipeline 扩展 + 新增 v1_models.rs + 路由替换。回滚即 `git checkout` 相关文件。迁移 v6 仅增一行 INSERT，可手动删除。

## 实现方式

体量中等（selector 扩展 + model_mapper 扩展 + v1_models handler + capability 工具 + alias 校验），1 个 worktree sub-agent 可实现。也可进主线 inline 实现。