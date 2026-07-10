# 模型管理刷新与别名设计

## 设计目标

在已有 accounts/endpoints 数据层基础上新增端点的模型管理、上游刷新、别名解析能力，不重写既有分层。

## 总体架构

```text
agent-switch 进程
├── SQLite（已有）
│   ├── accounts, endpoints（已有）
│   ├── endpoint_models（新增）
│   └── model_aliases（新增）
├── 模型刷新服务（新增）
│   ├── SyncService — 管理并发、限流、定时
│   ├── UpstreamFetcher — HTTP 调用上游 /v1/models
│   └── JitterScheduler — 6h ± 随机 30min
├── 别名引擎（新增）
│   └── ModelAliasEngine — 解析优先级链
├── 管理 API（新增 /api/models/*, /api/models/aliases/*）
├── 前端 /models 页面（更新）
└── 前端 /routes, /tools 页面（占位，后续子任务使用别名）
```

## SQLite 迁移 v3

详见 `prd.md` 中的 `endpoint_models` 和 `model_aliases` 表定义。

设计要点：
- `endpoint_models` 的 UNIQUE(endpoint_id, model_name) 防止同一端点重复模型名。
- `model_aliases` 不设 UNIQUE 约束，允许同一个 alias 指向多个目标。
- `target_endpoint_id` 可空，允许别名指向未来才存在的端点。
- `invalid_reason` 在删除 synced 模型时填充；别名本身不自动删除。

## 模型刷新服务

### SyncService

```rust
pub struct SyncService {
    pub settings: Arc<Mutex<SyncSettings>>,
    pub is_running: Arc<AtomicBool>,
    pub scheduler: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

pub struct SyncSettings {
    pub auto_refresh_enabled: bool,
    pub last_sync_at: Option<OffsetDateTime>,
    pub last_sync_error: Option<String>,
}
```

设计要点：
- 并发：`tokio::sync::Semaphore` 控制 host-level 并发（每 host 1 个）。
- 刷新实现：调用端点的 `/v1/models`，解析 JSON 模型名称列表。
- 覆盖逻辑：刷新返回的模型 → upsert 进 endpoint_models（更新 `last_seen_at`）；
  该端点已有但刷新未返回的模型 → `source='synced'` 且 `last_seen_at < 刷新开始时间` → `is_available=0`。
- `source='custom'` 或 `source='synced'` 且 `last_seen_at >= 刷新开始时间` 不受影响。
- 启动刷新异步执行，不阻塞 Tauri setup。
- 定时刷新在 `auto_refresh_enabled` 开启时由 JitterScheduler 驱动。

### UpstreamFetcher

```rust
pub async fn fetch_models(base_url: &str, api_key: Option<&str>)
    -> Result<Vec<FetchedModel>, String>
```

- GET `{base_url}/v1/models`。
- 如果该端点不支持 `/v1/models`，允许手动建模或跳过。

### 并发限流

```rust
struct RefreshThrottle {
    host_slots: HashMap<String, usize>,   // 每 host 当前任务数
    credential_slots: HashMap<String, usize>, // 每凭据当前任务数
    max_per_host: usize,     // = 1
    max_per_credential: usize, // = 1
}
```

- 手动刷新和定时刷新共享 throttle。
- 刷新完成后释放 slot。

## 别名引擎

```rust
pub struct AliasEngine {
    pub db: Arc<Mutex<Connection>>,
}

pub struct ResolvedAlias {
    pub alias_name: String,
    pub matched_scope: String,  // 'explicit_prefix' | 'tool' | 'route' | 'endpoint' | 'global' | 'name_match'
    pub candidates: Vec<AliasCandidate>,
}

pub struct AliasCandidate {
    pub endpoint_id: String,
    pub model_name: String,
    pub priority: i64,
    pub is_valid: bool,
    pub invalid_reason: Option<String>,
}

impl AliasEngine {
    pub async fn resolve(&self, alias: &str, context: &ResolutionContext) -> ResolvedAlias { ... }
}
```

### ResolutionContext

```rust
pub struct ResolutionContext {
    pub tool: Option<String>,       // 'claude-code' | 'codex'
    pub route_id: Option<String>,   // 当前路由 ID
    pub endpoint_id: Option<String>, // 请求来源端点
}
```

### 解析流程

1. 检查 no-map 列表 → 匹配则直通。
2. 检查 `{prefix}/{model}` 格式 → 匹配则优先解析到该 endpoint 的模型。
3. 检查 tool 级 alias → `scope_type='tool', scope_id=tool`。
4. 检查 route 级 alias → `scope_type='route', scope_id=route_id`。
5. 检查 endpoint 级 alias → `scope_type='endpoint', scope_id=endpoint_id`。
6. 检查 global 级 alias → `scope_type='global'`。
7. 原名匹配 → 在所有可用端点模型查找原名。
8. 全部失败 → 返回 `ResolvedAlias { matched_scope: 'not_found', candidates: [] }`。

## 管理 API 契约

### 模型

```text
GET    /api/models                列表（query: endpoint_id, capability, source）
POST   /api/models/sync           手动全量刷新
POST   /api/models/custom         添加自定义模型
DELETE /api/models/{id}           删除模型（同时标记关联 alias 失效）
```

### 别名

```text
GET    /api/models/aliases        列表别名（query: scope_type, scope_id）
POST   /api/models/aliases        创建/更新别名额（批量覆盖同一别名的 targets）
DELETE /api/models/aliases/{id}   删除别名条目
GET    /api/models/resolve/{alias}  解析别名（query: tool, route_id, endpoint_id）
```

### 刷新设置

```text
GET    /api/settings/auto-model-refresh   查询刷新开关
PUT    /api/settings/auto-model-refresh   设置刷新开关
```

## Rust 模块边界

```text
src-tauri/src/
├── db/
│   ├── migrations.rs         新增 v3 迁移
│   └── dao/
│       ├── endpoints_models.rs  新增
│       └── model_aliases.rs     新增
├── services/
│   ├── model_sync.rs          刷新服务（SyncService, UpstreamFetcher, Throttle）
│   ├── model_alias.rs         别名引擎（AliasEngine, ResolutionContext）
│   └── mod.rs                 更新
├── http/
│   ├── api/
│   │   ├── models.rs          新增：/api/models endpoints
│   │   ├── aliases.rs         新增：/api/models/aliases endpoints
│   │   └── mod.rs             更新
│   └── router.rs              更新：挂载 /api/models
└── app_state.rs               更新：持有 SyncService（可选）
```

设计原则：
- ModelAliasEngine 只读别名表，不修改。
- SyncService 为独立 tokio task，不需要 AppState 引用（通过 AppState clone 获取 db 和端点列表）。
- DAO 只做 SQL，不做模型解析或刷新逻辑。

## 前端设计

### 模型页面

```text
src/pages/ModelsPage.tsx    ← 更新为真实页面
src/components/models/
├── ModelList.tsx           模型表格（名称、端点、来源、能力标签、可用状态）
├── RefreshBar.tsx          刷新按钮 + 刷新状态 + 开关
├── CustomModelForm.tsx     添加自定义模型表单
├── AliasPanel.tsx          别名配置面板
└── AliasForm.tsx           别名编辑（scope_type, alias_name, target, priority）
```

模型表格按端点分组，能力标签使用彩色 Badge。

刷新按钮在运行时显示 loading 状态，完成后自动刷新列表。

别名面板在单独的 section，支持按 scope 过滤。

## 后续扩展点

- `/v1/models` API 响应：本子任务只处理模型数据，不生成 `/v1/models` HTTP 响应（openai-compatible-v1-endpoints 子任务负责）。
- 路由别名集成：routing-failover-core 子任务使用 AliasEngine 做模型路由选择。
- 角色映射编辑器：tool-takeover-claude-code-codex 子任务在工具页面中集成 Claude Code 角色映射编辑。

## 重要取舍

- 选择 `capabilities` 为 JSON 文本字段而非关系表：查询方便，更新原子，第一版不需要按能力做复杂关联查询。
- 选择 `model_aliases` 为行模型（一行一个 target）而非单行 JSON 数组：便于 UI 编辑单个 target、标记失效、按 priority 排序。
- 不与 `settings` 表合并：`auto_model_refresh_enabled` 暂存 `app_metadata` 表（key-value）或 settings 表。
- 刷新错误存储方式：`app_metadata` key-value，避免增加新表。
