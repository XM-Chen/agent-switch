# 设计 — OpenAI-compatible v1 多端点

> 配套 `prd.md`。本文件只写技术设计：架构扩展、能力过滤、模型聚合、媒体透明流转、新增与修改的模块边界。

## 1. 架构扩展

### 核心思路

v1 路由**不引入新模块**，在现有 RouteProxy 管道内做扩展性改造：

1. **route_settings 新增 v1 条目**（id=v1，protcol_type=openai-compatible，非现有三值）。
2. **Selector 扩展为「协议 + 能力」双维度过滤**：按子路径解析 required_capability，作为额外过滤条件。
3. **ModelMapper 扩展能力后校验**：选中模型后检查 `endpoint_models.capabilities` 包含 required_capability。
4. **新增 `/v1/models` handler**：不走管道，直接聚合 DB 返回。
5. **Images/Audio 透明流转**：跳过 JSON 翻译器/格式化。

### 数据流

```
POST /v1/chat/completions          POST /v1/images/generations
        │                                  │
        ▼                                  ▼
 ┌──────────────────────┐     ┌──────────────────────────┐
 │ 1. Handler: 解析 path │     │ 1. Handler: 解析 path     │
 │    → capability=chat  │     │    → capability=images    │
 │    → route="v1"       │     │    → route="v1"          │
 └──────┬───────────────┘     └──────┬───────────────────┘
        │                            │
        ▼                            ▼
 ┌──────────────────────┐     ┌──────────────────────────┐
 │ 2. Selector 预筛    │     │ 2. Selector 预筛        │
 │    protocol_type IN  │     │    同左                    │
 │    (openai-chat,…)   │     │                          │
 │    + 有 chat 模型    │     │    + 有 images 模型      │
 └──────┬───────────────┘     └──────┬───────────────────┘
        │                            │
        ▼                            ▼
 ┌──────────────────────┐     ┌──────────────────────────┐
 │ 3. ModelMapper      │     │ 3. ModelMapper          │
 │    解析 body.model   │     │    同左                   │
 │    alias resolve     │     │                          │
 │    + 后校验 chat     │     │    + 后校验 images       │
 └──────┬───────────────┘     └──────┬───────────────────┘
        │                            │
        ▼                            ▼
 ┌──────────────────────┐     ┌──────────────────────────┐
 │ 4. AuthInjector     │     │ 4. AuthInjector         │
 │    → 凭据注入        │     │    同左                   │
 └──────┬───────────────┘     └──────┬───────────────────┘
        │                            │
        ▼                            ▼
 ┌──────────────────────┐     ┌──────────────────────────┐
 │ 5. Translator       │     │ 5. 跳过 translator      │
 │    (Passthrough,     │     │    上游响应原样流转      │
 │     同协议不转换)    │     │    首块缓冲探测错误      │
 └──────┬───────────────┘     └──────┬───────────────────┘
        │                            │
        ▼                            ▼
 ┌──────────────────────┐     ┌──────────────────────────┐
 │ 6. Forward → Log    │     │ 6. Forward → Log        │
 │    → Response        │     │    (不存媒体内容)       │
 └──────────────────────┘     └──────────────────────────┘
```

### GET /v1/models 数据流（不进入管道）

```
GET /v1/models?capability=chat
    │
    ▼
Handler → query endpoint_models WHERE is_available=1
    → 若 capability 指定则 AND capabilities LIKE
    → 去重 (model_name)
    → 去重 → 组装 OpenAI models API 格式
    → 返回 JSON
```

## 2. 能力过滤实现

### 2.1 Selector 扩展

现有 `EndpointSelector` 增加一个可选过滤字段：

```rust
/// 扩展 Selector：按模型能力过滤。
/// 若 `required_capability` 非空，在 load_candidates 后额外检查：
/// 该 protocol_type 的端点中是否存在至少一个模型包含该能力。
impl EndpointSelector {
    /// 设置必需的模型能力（无则不限制）。
    pub fn set_required_capability(&mut self, capability: &str) {
        self.required_capability = Some(capability.to_string());
    }

    /// 从 DB 加载候选后，过滤无 capable 模型的端点。
    fn filter_by_capability(&mut self, db: &Mutex<Connection>) -> Result<(), String> {
        let Some(ref cap) = self.required_capability else { return Ok(()); };
        let eps: Vec<_> = self.candidates.drain(..).filter(|ep| {
            // 查询端点是否有至少一个模型含该能力
            has_capable_model(db, &ep.id, cap).unwrap_or(false)
        }).collect();
        self.candidates = eps;
        Ok(())
    }
}
```

### 2.2 ModelMapper 扩展

选中模型后添加能力校验：

```rust
impl ModelMapper {
    pub fn set_required_capability(&mut self, capability: &str) {
        self.required_capability = Some(capability.to_string());
    }

    fn validate_capability(&self, endpoint_id: &str, model: &str) -> Result<(), String> {
        let Some(ref cap) = self.required_capability else { return Ok(()); };
        // 查询 endpoint_models WHERE endpoint_id=? AND model_name=? AND capabilities LIKE ?
        // 若 capabilities 不含 cap，返回 Err
    }
}
```

### 2.3 子路径 → capability 映射

在 v1 handler 中统一映射：

```rust
pub fn path_to_capability(path: &str) -> Option<&'static str> {
    // /v1/chat/completions → "chat"
    // /v1/responses → "responses"
    // /v1/embeddings → "embeddings"
    // /v1/images/generations → "images"
    // /v1/audio/speech → "audio"
    // /v1/models → None (skip pipeline)
}
```

## 3. 要求修改的文件

### 修改已有文件

| 文件 | 修改内容 |
|------|---------|
| `http/proxy/constants.rs` | 新增 `PROTOCOL_OPENAI_COMPATIBLE` 常量、capability 常量列表 |
| `http/proxy/mod.rs` (RouteProxy) | `proxy_request` 解析入站 path 确定 required_capability，传给 selector 和 model_mapper |
| `http/proxy/selector.rs` | 新增 required_capability 字段 / `set_required_capability()` / `filter_by_capability()` |
| `http/proxy/model_mapper.rs` | 新增 `set_required_capability()` / `validate_capability()` |
| `http/router.rs` | 替换 `/v1/{*path}` 501 占位为 v1 路由 handler（v1_proxy handler + v1_models handler） |
| `http/placeholders.rs` | 再无占位路由（所有 3 条路径均已实现） |
| `db/dao/endpoint_models.rs` | 新增 `list_capable(endpoint_id, capability)` 函数查询某端点的 capable 模型 |
| `db/dao/model_aliases.rs` | 新增 alias 创建时能力校验 |
| `http/api/aliases.rs` | alias 创建 handler 中调用能力校验 |

### 新增文件

| 文件 | 内容 |
|------|------|
| `http/api/v1_models.rs` | GET /v1/models 聚合 handler |
| `http/proxy/capability.rs` | 路径-能力映射函数、能力验证工具函数 |

## 4. 新增 route_settings 默认行

```sql
INSERT OR IGNORE INTO route_settings (id, label, strategy, protocol_type, failover_enabled, updated_at)
VALUES ('v1', 'OpenAI v1', 'fill-first', 'openai-compatible', 1, datetime('now'));
```

`protocol_type=openai-compatible` 表示该路由在 selector 阶段不按单值 protocol_type 筛选，而是按 path_to_capability 确定的能力对应的实际 protocol_type（`openai-chat` 或 `openai-responses`）筛选。

## 5. GET /v1/models 契约

**请求：**
```
GET /v1/models?capability=chat,images
```

**响应（200）：**
```json
{
  "object": "list",
  "data": [
    {
      "id": "gpt-4",
      "object": "model",
      "created": 1693721698,
      "owned_by": "endpoint_id_xxx"
    }
  ]
}
```

查询 SQL：
```sql
SELECT DISTINCT model_name, endpoint_id
FROM endpoint_models
WHERE is_available = 1
  [AND capabilities LIKE '%chat%']
  [AND capabilities LIKE '%images%']
ORDER BY model_name ASC;
```

## 6. Images/Audio 透明流转

- RouteProxy 检测到 capability=images 或 audio 时：
  - 跳过 translator（设置 `passthrough_body: true`）。
  - StreamGuard 仅做首块缓冲探测（标准 JSON error 可解析），成功后直通。
  - RequestLogger 调用 `log_entry.set_media_log(body_hash, content_type, content_length)`，不存储正文。
- 响应头透传上游的 `Content-Type`（如 `image/png`、`audio/mpeg`）。

## 7. 关键取舍

1. **能力过滤是软约束 vs 硬约束**：本设计采用**硬约束**—若 selector 预筛后无候选端点 → 502；若 model_mapper 后校验不通过 → 故障转移到下一候选。不自动降级为无能力过滤转发。
2. **/v1/models 聚合 vs 代理转发**：用 DB 聚合（父 PRD 安全性要求）。上游模型变更需刷新后才能反映到 /v1/models。
3. **Images/Audio 不经过 translator**：response 格式为二进制非 JSON，不存在跨协议转换意义。首块探测足以区别 JSON error vs 合法媒体数据。
4. **protocol_type=openai-compatible** 不新增 endpoint 管理 UI 的校验值，只在 v1 route_settings 条目中使用。

## 8. 与其它子任务的衔接

- 子任务 `06-27-chain-testing-debugger`：可在本任务完成后通过 /v1/* 端点进行真实链路测试；images/audio 的临时展示 UI 在该子任务中实现。
- 子任务 `06-27-import-export-settings`：v1 route_settings 条目纳入导出范围，但 endpoint_models 不导出（上游同步的模型不应静态导出）。
