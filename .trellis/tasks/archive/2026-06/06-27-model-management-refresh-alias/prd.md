# 模型管理刷新与别名

## 目标

建立 agent-switch 的模型管理能力：端点模型列表自动刷新、自定义模型、模型能力类型、模型别名与解析，为路由子任务提供模型级别的路由依据。

## 父任务约束

父任务：`.trellis/tasks/06-26-agent-switch-web-router-mvp`

本子任务必须遵守父任务已确认的跨模块约束：

- 第一版需要模型管理/模型映射能力；模型列表需支持根据上游渠道自动刷新。
- 模型刷新采用手动刷新、应用启动时自动刷新、定时刷新三种方式；只刷新启用端点。
- 自动刷新上游模型列表必须有总开关，默认关闭；只有用户开启后才执行启动刷新和定时刷新。
- 提供"一键全局刷新上游渠道模型"按钮。
- 定时刷新以 6 小时为基准周期 + 抖动机制（避免过于规律）。
- 不同上游渠道可并发刷新；同一 host 默认最多 1 个刷新任务，同一凭据默认最多 1 个。
- 刷新失败不能阻塞应用启动或影响已有配置，需记录 `last_model_sync_at` 和 `last_model_sync_error`。
- 上游列表覆盖语义：本次刷新未返回的旧模型视为已下线并从该端点删除。
- 第一版允许手动添加自定义模型，标记 `source=custom`，不受上游刷新影响。
- 自定义模型必须绑定到具体端点。
- 模型必须引入能力类型：`chat` `responses` `embeddings` `images` `audio` `streaming` `tool_calling` `vision_input`。
- 路由前必须按入口过滤：`/v1/chat/completions` 只能选 `chat` 等。
- 模型别名支持分作用域：全局、工具级、路由级、端点级。
- 别名解析优先级：no-map/直通 > 显式端点前缀 > 工具角色映射 > 路由级 alias > 端点级 alias > 全局 alias > 原名匹配 > 可解释失败。
- 别名可映射到多个目标，必须显式排序形成候选链。
- 删除 synced 模型时 alias 不自动删除，标记失效并在 UI 提示重新选择。
- 首页中文导航已包含"模型"页面入口。

## 已确认技术决策

### 数据模型

参考来源：`9router` 的内置/自定义模型分离 + `sub2api` 的同步/映射字段 + `ccs` 的角色模型映射。

**endpoint_models 表**：

```sql
id TEXT PRIMARY KEY,
endpoint_id TEXT NOT NULL REFERENCES endpoints(id) ON DELETE CASCADE,
model_name TEXT NOT NULL,        -- 上游模型名
display_name TEXT NOT NULL,      -- 显示名
source TEXT NOT NULL,            -- 'synced' | 'custom'
capabilities TEXT,               -- JSON 数组：["chat","streaming","tool_calling"]
context_window INTEGER,          -- 上下文窗口大小（可选）
is_available INTEGER NOT NULL DEFAULT 1,
last_seen_at TEXT,               -- 最后在刷新中出现的时刻
created_at TEXT NOT NULL,
updated_at TEXT NOT NULL,
UNIQUE(endpoint_id, model_name)
```

**model_aliases 表**：

```sql
id TEXT PRIMARY KEY,
scope_type TEXT NOT NULL,        -- 'global' | 'tool' | 'route' | 'endpoint'
scope_id TEXT,                   -- 工具/路由/端点 ID；global 时空
alias_name TEXT NOT NULL,        -- 本地别名：如 "sonnet"、"my-fast-model"
target_endpoint_id TEXT REFERENCES endpoints(id) ON DELETE SET NULL,
target_model_name TEXT NOT NULL, -- 具体模型名
priority INTEGER NOT NULL DEFAULT 0,
enabled INTEGER NOT NULL DEFAULT 1,
invalid_reason TEXT,             -- 模型被删除后标记失效原因
created_at TEXT NOT NULL,
updated_at TEXT NOT NULL
```

### 模型刷新架构

参考来源：`sub2api` 的 syncUpstreamModels 思路 + `ccs` 的手动触发刷新 + agent-switch 独立定时器。

- 刷新总开关（`auto_model_refresh_enabled`）在 settings 表中持久化，默认 false。
- 启用时：启动时执行一次全量刷新 + 每 6 小时（±随机 0~30 分钟抖动）定时刷新。
- 刷新只处理 `enabled=true` 的端点。
- 并发控制：同一 host 最多 1 个任务，同一凭据最多 1 个任务。
- 刷新失败只更新 `last_model_sync_error`，不阻塞应用。
- 手动刷新按钮触发全量并发刷新。
- 定时刷新同样受 host/凭据限流。

### 模型能力类型

参考来源：`9router` 的 modelKind 字段 + agent-switch 对 `/v1` 多端点的入口过滤需求。

能力类型枚举：

```rust
pub enum ModelCapability {
    Chat,           // /v1/chat/completions
    Responses,      // /v1/responses
    Embeddings,     // /v1/embeddings
    Images,         // /v1/images/generations etc.
    Audio,          // /v1/audio/*
    Streaming,      // stream: true 支持
    ToolCalling,    // tools/function calling
    VisionInput,    // 图片输入
}
```

路由过滤规则：`／v1/chat/completions` → 仅 `Chat`；`/v1/responses` → `Responses`；`/v1/embeddings` → `Embeddings`；`/v1/images/*` → `Images`；`/v1/audio/*` → `Audio`。

### 别名解析引擎

参考来源：`cpa` 的 `resolveOAuthUpstreamModel` + `ccs` 的 `map_model` + `9router` 的 `normalizeModel`。 

- 解析函数在 `services/model_alias.rs` 中实现。
- 不修改 alias 表，只读解析。
- 返回 `ResolvedAlias { alias_name, matched_scope, candidates: Vec<(endpoint_id, model_name, priority)> }`。
- 外部调用者根据故障转移策略遍历 candidates。

### 角色映射（扩展别名）

参考来源：`ccs` 的 `ModelMapping { haiku, sonnet, opus, fable }`。

Claude Code 角色名 `sonnet`/`opus`/`haiku`/`fable` 作为 `scope_type='tool'` + `scope_id='claude-code'` 的特殊别名，由别名引擎解析。

## 需求

- 在 accounts-endpoints 子任务基础上新增 `endpoint_models` 和 `model_aliases` 表（迁移 v3）。
- 实现 DAO 层：endpoint_models CRUD、model_aliases CRUD、按 scope 查询 alias。
- 实现模型刷新服务：HTTP 调用上游 `/v1/models` 端点解析模型列表，写入 endpoint_models。
- 实现定时刷新任务（6h + jitter）。
- 实现别名解析引擎。
- 实现管理 API：
  - `GET /api/models` — 列表模型（支持按 endpoint/capability 过滤）
  - `POST /api/models/sync` — 手动刷新
  - `POST /api/models/custom` — 添加自定义模型
  - `DELETE /api/models/{id}` — 删除模型
  - `GET /api/models/aliases` — 列表别名
  - `POST /api/models/aliases` — 创建/更新别名
  - `DELETE /api/models/aliases/{id}` — 删除别名
  - `GET /api/settings/auto-model-refresh` — 查询刷新开关
  - `PUT /api/settings/auto-model-refresh` — 设置刷新开关
- 实现前端模型页面：列表、能力标签、刷新按钮、别名配置。
- UI 文案中文；模型删除/别名失效提示。

## 验收标准

- [ ] `endpoint_models` 和 `model_aliases` 表通过迁移 v3 创建。
- [ ] 手动刷新：调用 `POST /api/models/sync`，所有启用端点 concurrent 刷新（host/credential 限流），更新模型列表。
- [ ] 启动刷新：总开关开启时，启动后自动执行一次刷新；关闭时不执行。
- [ ] 定时刷新：开启后每 6h ± 随机 30min 执行。
- [ ] 刷新失败不影响已有模型，记录 `last_model_sync_error`。
- [ ] 自定义模型：`POST /api/models/custom` 创建，`source='custom'`，绑定 endpoint，不受刷新删除。
- [ ] 别名创建/解析：创建 `scope_type='global'` alias，解析引擎按优先级返回候选链。
- [ ] synced 模型删除后，关联 alias 标记失效，不自动删除。
- [ ] 能力过滤：模型列表 `capabilities` 正确返回，API 支持按 capability 参数过滤。
- [ ] 前端模型页面显示模型列表（名称、端点、来源、能力、可用状态）。
- [ ] 前端可触发手动刷新、添加自定义模型、配置别名。

## 暂不纳入本子任务

- 真实路由转发与故障转移（routing-failover-core 子任务）。
- `/v1/models` 真实 API 响应生成（openai-compatible-v1-endpoints 子任务）。
- 模型上传/下载/共享。
- 模型测速与评分。
- 自动发现未配置端点的上游模型。

## 开放问题

当前子任务不再保留阻塞性开放问题。技术栈和工程默认项基于已有约束与参考项目自行确定。

