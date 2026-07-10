# OpenAI-compatible v1 多端点

## 目标

为 agent-switch 实现 `/v1/*` 作为 OpenAI-compatible 多协议聚合入口，支持六个子端点的真实转发，扩展 selector 按子路径+模型能力过滤候选端点。完成后 3 条路由路径全部真实转发，路径隔离契约兑现。

## 背景与边界

- 父任务：`06-26-agent-switch-web-router-mvp`，子任务拆分第 6 项。
- 范围：`/v1/chat/completions`、`/v1/responses`、`/v1/embeddings`、`/v1/images`、`/v1/audio`、`/v1/models` 的真实转发或聚合。
- 不含 images/audio 媒体文件的持久化存储或预览 UI（属子任务 `06-27-chain-testing-debugger`）。
- 不含复杂评分/测速型端点选择（属父任务暂不纳入）。

### 已就位基础

- 代理管道完全就绪：RouteProxy、EndpointSelector、AuthInjector、FailoverState、StreamGuard、RequestLogger、TranslatorRegistry。
- Translator：Anthropic↔Chat、Chat↔Responses 四方向。
- model_alias 按 tool/v1 作用域解析。
- endpoint_models 表 `capabilities` 字段（逗号分隔字符串）。
- route_settings 表可新增 v1 条目。

### 父任务已确认的约束

- 六个子端点为真实转发目标或聚合查询；各子端点独立可验收。
- 模型能力过滤：入口只选具备对应能力的模型；alias 创建时校验能力。
- Images/Audio 日志不持久化媒体内容，仅记录元数据。

## 技术决策

> 决策原则：效仿四个参考项目（9router / cpa / ccs / sub2api）取最优解。

### D1 v1 路由架构 = 扩展现有管道

- route_settings 新增 v1 路由条目。
- selector 按子路径解析 `required_capability`：`/chat/completions→chat`、`/responses→responses`、`/embeddings→embeddings`、`/images→images`、`/audio→audio`。
- 端点按 protocol_type 筛选（`openai-chat` 覆盖 chat/embeddings/images/audio；`openai-responses` 覆盖 responses）。
- `/v1/models` 不走代理管道，静态聚合 DB 返回。

### D2 GET /v1/models = 静态聚合

- 从 `endpoint_models` 查询所有 enabled 端点的可用模型（`is_available=1`），同 `model_name` 去重。
- 返回 OpenAI 标准格式：`{ object: "list", data: [{ id, object: "model", created, owned_by }] }`。
- 支持 `?capability=` 过滤。不转发上游。

### D3 Images/Audio = 透明流转

- 响应体不解析、不缓冲、不翻译——上游返回什么就原样流转。
- 首块缓冲探测错误（JSON error wrapper 可探测并触发 failover），非错误则直通。
- 日志仅记录元数据：endpoint_id、model、media_type、content_length、body_sha256_hash、latency、status、error_kind、fallback_chain。

### D4 能力过滤 = 两端双重过滤

- **selector 预筛**：加载候选后，排除没有任何 capable 模型的端点。
- **model_mapper 后校验**：选中模型后检查 capabilities 是否包含 required_capability，否则继续候选链。
- **alias 创建时校验**：POST /api/models/aliases 时检查目标模型能力。
- 能力标签：`chat`、`responses`、`embeddings`、`images`、`audio`、`streaming`、`tool_calling`、`vision_input`。

## 需求

### R1 v1 路由接入

- R1.1 route_settings 新增 v1 条目（id=v1，protocol_type 扩展为按子路径映射）。
- R1.2 `/v1/{*path}` 由 RouteProxy 驱动，子路径自动解析为 required_capability。
- R1.3 `POST /v1/chat/completions` → 选 protocol_type=openai-chat + 模型能力含 chat 的端点。
- R1.4 `POST /v1/responses` → 选 protocol_type=openai-responses + 模型能力含 responses 的端点。
- R1.5 `POST /v1/embeddings` → 选 protocol_type=openai-chat + 模型能力含 embeddings 的端点。
- R1.6 `POST /v1/images/generations` → 选 protocol_type=openai-chat + 模型能力含 images 的端点，透明流转。
- R1.7 `POST /v1/audio/speech`（及 `/v1/audio/transcriptions`、`/v1/audio/translations`）→ 选 protocol_type=openai-chat + 模型能力含 audio 的端点，透明流转。
- R1.8 v1 路由复用 claude-code/codex 同源的故障转移、冷却、日志管道。

### R2 GET /v1/models

- R2.1 返回聚合 models 列表，数据源为 endpoint_models 表。
- R2.2 `?capability=chat` 过滤；多值 `?capability=chat,images`。
- R2.3 返回字段：id、object="model"、created、owned_by（取 endpoint_id）。

### R3 能力过滤

- R3.1 selector 加载候选后，过滤掉无对应能力的端点。
- R3.2 model_mapper 解析模型后校验 capabilities 包含 required_capability。
- R3.3 alias 创建时校验目标模型能力（仅限明确指定了 capability scope 的 alias，非全局 alias 不做全量校验）。

### R4 日志扩展

- R4.1 images/audio 日志额外字段：media_type、content_length、body_sha256_hash。
- R4.2 不存储媒体内容（不存 base64、文件路径、响应正文）。

### R5 端点管理扩展

- R5.1 端点创建/更新时提示 capabilities 元数据（可选），UI 展示端点模型能力。
- R5.2 protocol_type 校验扩展：现有 3 值不变，v1 路由内部映射子路径 → protocol_type。

## 验收标准

- [ ] AC1: 配置 openai-chat 端点后，`POST /v1/chat/completions` 转发到正确上游并返回标准 chat completion 响应。
- [ ] AC2: 配置 responses-capable 端点后，`POST /v1/responses` 转发成功。
- [ ] AC3: 配置 embeddings-capable 端点后，`POST /v1/embeddings` 返回 embeddings 向量。
- [ ] AC4: 配置 images-capable 端点后，`POST /v1/images/generations` 返回图片（透明流转，日志不存图片）。
- [ ] AC5: 配置 audio-capable 端点后，`POST /v1/audio/speech` 返回音频流（透明流转）。
- [ ] AC6: `GET /v1/models` 聚合所有 enabled 端点模型，格式为 OpenAI Models API 标准。
- [ ] AC7: 模型能力过滤生效——embeddings 端点的 chat-only 模型不被选为 chat/completions 入口候选。
- [ ] AC8: 能力不匹配时故障转移到下一候选端点，日志记录 resolved_alias 与 capability mismatch。
- [ ] AC9: alias 创建时拒绝能力不匹配的映射（如把 chat-only 模型 alias 到 tools 作用域并设 capability=images）。
- [ ] AC10: images/audio 请求日志仅存元数据，不存媒体内容。
- [ ] AC11: 流式 chat/completions 和 responses 请求经 SSE 转发与 failover 保护正确。
- [ ] AC12: 质量门通过——`cargo check`（0 error）、`cargo clippy --all-targets`（0 error for v1 code）、`npm run build`。

## 暂不纳入范围

- images/audio 的缩略图/播放器 UI 预览（属链测调试器子任务）。
- `/v1/models` 实时并行聚合上游（第一版用 DB 聚合）。
- Gemini/Anthropic 协议兼容 v1 入口（后续版本独立入口）。