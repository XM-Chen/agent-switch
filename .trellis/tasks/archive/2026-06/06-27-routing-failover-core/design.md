# 设计 — 路由与故障转移核心

> 配套 `prd.md`。本文件只写技术设计：架构分层、数据模型、协议转换契约、故障转移引擎、请求日志、新模块边界与依赖。

## 1. 架构与分层

### 新增模块

```
src-tauri/src/
├── http/
│   └── proxy/                    ⇐ 新增：路由转发核心
│       ├── mod.rs                管道编排：receive → select → inject → convert → forward → log
│       ├── selector.rs           端点筛选 + 选择策略（Fill-First / Round-Robin）
│       ├── auth_injector.rs      凭据解密与注入（API Key / OAuth token）
│       ├── model_mapper.rs       模型名解析与重写（调用 model_alias + 角色映射）
│       ├── stream_guard.rs       SSE 首块缓冲哨兵 + 流态跟踪
│       ├── error.rs              错误分类 + fallback 决策
│       ├── failover.rs           故障转移状态机（while+excludeSet，效仿 sub2api/9router）
│       ├── oauth_refresh.rs      OAuth token 过期检查与自动刷新
│       └── logger.rs             请求摘要日志记录
├── http/api/
│   ├── routes.rs                 ⇐ 新增：路由设置管理 API
│   └── logs.rs                   ⇐ 新增：请求日志查询 API
├── db/dao/
│   ├── route_settings.rs         ⇐ 新增
│   ├── request_logs.rs           ⇐ 新增
│   └── model_locks.rs             ⇐ 新增（或可选内存集）
├── services/
│   └── translator/               ⇐ 新增：协议转换器注册表
│       ├── mod.rs                translator 注册 + 路由
│       ├── native.rs             Native Passthrough 直转（零损失）
│       ├── anthropic_openai.rs   Anthropic ↔ OpenAI Chat 转换
│       ├── openai_responses.rs   OpenAI Chat ↔ responses 转换
│       └── helpers.rs            共享转换工具（内容块、token 计数、SSE 流式转换）
└── db/migrations.rs              migration v5 追加
```

### 数据流（转发链路）

```
工具请求 → /claude-code/{*path} 或 /codex/{*path}
    │
    ▼
┌─────────────────────────────────────────────────────┐
│ 1. stream_guard.rs                                   │
│    创建 StreamGuard，记录初始 Writer.Size()          │
│    如果预计流式：暂不写 SSE header                  │
├─────────────────────────────────────────────────────┤
│ 2. selector.rs                                       │
│    按 route → protocol_type 筛候选端点               │
│    按策略(Fill-First/RR)排序                         │
│    跳过 disabled/cooldown                           │
├─────────────────────────────────────────────────────┤
│ 3. if OAuth endpoint: oauth_refresh.rs               │
│    检查 CodexCredentials.expires_at                  │
│    <= 60s 过期 → refresh_token → 换新 → 回写        │
│    刷新失败 → 此端点冷却 → selector 重选            │
├─────────────────────────────────────────────────────┤
│ 4. model_mapper.rs                                   │
│    解析 body.model                                   │
│    model_alias::resolve(tool) 取候选链               │
│    选最优候选 → 改写 body.model 为 upstream_model    │
│    角色映射(haiku→…, 剥离[1M])                       │
│    失败 → 返回可解释错误                             │
├─────────────────────────────────────────────────────┤
│ 5. selector.choose() → 定 endpoint_id + model        │
│    写入日志上下文上下文: {endpoint_id, model, ...}    │
├─────────────────────────────────────────────────────┤
│ 6. auth_injector.rs                                  │
│    API Key: decrypt api_key_encrypted → auth header   │
│    OAuth: decrypt CodexCredentials → Bearer token     │
│    注入到转发请求 header                             │
├─────────────────────────────────────────────────────┤
│ 7. translator 层                                     │
│    判断同协议 → Native Passthrough (只改 host+model) │
│    跨协议 → translatorRegistry 匹配转换器            │
│    返回 (transformed_request_body, protocol_from,     │
│            protocol_to)                              │
├─────────────────────────────────────────────────────┤
│ 8. forwarder.rs (内核)                               │
│    reqwest::Client 发向上游                          │
│    如果是流式: SSE 分块转换 + 流式哨兵              │
│    非流式: 获取完整响应 + 响应转换                   │
├─────────────────────────────────────────────────────┤
│ 9. failover.rs (外层 while 循环)                      │
│    成功 → break 出循环 → log => response             │
│    失败 → error 分类                                 │
│      可切换且 ↓ 未写出流内容 → mark_candidate_failed │
│                               → failover_state.next()│
│                               → 回到 step 4(重选)    │
│      不可切换 → break 出循环 → error response + log  │
│      已写出流内容 → 不可切换 → 合成错误终端事件      │
│                     → break 出循环 → log             │
├─────────────────────────────────────────────────────┤
│ 10. logger.rs                                        │
│     写入 request_logs 表                             │
│     (不存 prompt/headers/密钥)                       │
└─────────────────────────────────────────────────────┘
    │
    ▼
  响应返回客户端
```

### 故障转移状态机（效仿 sub2api `failover_loop.go`）

```
FailoverState {
    switch_count: u32,              // 已执行跨端点切换次数
    max_switches: u32,              // 最大切换数(默认10)
    failed_ids: HashSet<String>,    // 已失败端点id集合
    same_account_retries: HashMap<String, u32>, // 同端点重试计数
    last_error: Option<FailoverError>,
    stream_started: bool,           // 是否已向客户端输出流内容
    chain: Vec<FallbackHop>,        // 用于日志
}

流程：
while switch_count < max_switches {
    let candidate = selector.next(&failover_state)?  // None → 全部耗尽
    // OAuth 刷新检查
    // 模型映射
    // 凭据注入
    // 协议转换
    // 转发
    match result {
        Ok(resp) → return resp
        Err(e) if should_failover(&e) && !stream_started →
            mark_failed(candidate)
            if same_account_retries[candidate] < 3 {
                sleep(500ms) → retry same
            } else {
                switch_count++
                chain.push(FallbackHop { endpoint, model, status, error })
                
                // 设置冷却
                set_cooldown(endpoint, cooldown_duration(&e))
                // 如果适用，设模型级锁
                set_model_lock(endpoint, model, model_cooldown(&e))
            }
        Err(e) →
            // 不可切换或已发流 → 返回错误
    }
}
// 全部失败 → 502 + 含 fallback_chain 的日志/响应
```

### 冷却逻辑

| 错误类型 | 冷却时长 | 机制 | 参考 |
|----------|----------|------|------|
| 429 `Retry-After` | 按上游值 + 1s buffer | 端点级 `cooldown_until` | sub2api + cpa |
| 429 无 header | 指数退避，max=300s | 端点级 `cooldown_until` | cpa TransientErrorCooldown |
| 529/503 过载 | 指数退避，max=300s | 端点级 + 模型级锁 | 9router modelLock |
| 5xx 非过载 | 退避 30-120s | 端点级 | sub2api |
| 401/403 | 按账号/端点类型定 | 端点级；OAuth 401 → 尝试刷新后仍 401 → keychain 失效 | 9router |
| 404/model_not_found | 模型级锁，永久或长冷却 | `model_lock` 表 | 9router |

## 2. 数据模型（migration v5）

> 新增迁移 v5，**不改动 v1-v4**。

### route_settings

```sql
CREATE TABLE IF NOT EXISTS route_settings (
    id          TEXT PRIMARY KEY,   -- 'claude-code' | 'codex'
    label       TEXT NOT NULL,      -- 显示名
    strategy    TEXT NOT NULL DEFAULT 'fill-first',  -- 'fill-first' | 'round-robin'
    protocol_type TEXT NOT NULL,    -- 筛选依据
    failover_enabled INTEGER NOT NULL DEFAULT 1,
    max_switches INTEGER NOT NULL DEFAULT 10,
    same_account_retries INTEGER NOT NULL DEFAULT 3,
    cooldown_multiplier REAL NOT NULL DEFAULT 1.0,
    updated_at  TEXT NOT NULL
);

INSERT OR IGNORE INTO route_settings (id, label, strategy, protocol_type, updated_at)
VALUES ('claude-code', 'Claude Code', 'fill-first', 'anthropic', datetime('now'));
INSERT OR IGNORE INTO route_settings (id, label, strategy, protocol_type, updated_at)
VALUES ('codex', 'Codex', 'fill-first', 'openai-responses', datetime('now'));
```

### request_logs

```sql
CREATE TABLE IF NOT EXISTS request_logs (
    id                TEXT PRIMARY KEY,
    request_id        TEXT NOT NULL,
    tool              TEXT,                      -- 'claude-code' | 'codex'
    inbound_endpoint  TEXT NOT NULL,             -- '/claude-code/v1/messages'
    requested_model   TEXT,
    resolved_alias    TEXT,
    resolved_scope    TEXT,                       -- 'tool' | 'route' | 'endpoint' | 'global' | 'name_match'
    target_endpoint_id TEXT,
    upstream_model    TEXT,
    upstream_endpoint TEXT,                       -- upstream base_url + path
    protocol_from     TEXT,                       -- 'anthropic' | 'openai-chat'
    protocol_to       TEXT,                       -- 'anthropic' | 'openai-chat' | 'openai-responses'
    status            INTEGER,                    -- 最终 HTTP 状态码
    error_kind        TEXT,                       -- 错误分类，成功时为 NULL
    fallback_chain    TEXT,                       -- JSON 数组 [{endpoint_id, model, status, error, latency_ms}]
    stream            INTEGER NOT NULL DEFAULT 0,
    duration_ms       INTEGER,
    first_token_ms    INTEGER,
    input_tokens      INTEGER,
    output_tokens     INTEGER,
    cache_creation_tokens INTEGER,
    cache_read_tokens     INTEGER,
    request_body_hash TEXT,                       -- SHA256 哈希(仅用于去重,不存正文)
    created_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_request_logs_tool ON request_logs(tool);
CREATE INDEX IF NOT EXISTS idx_request_logs_status ON request_logs(status);
CREATE INDEX IF NOT EXISTS idx_request_logs_created ON request_logs(created_at DESC);
```

### model_locks（可选：用内存集替代则不需此表）

```sql
CREATE TABLE IF NOT EXISTS model_locks (
    id          TEXT PRIMARY KEY,  -- '{endpoint_id}_{model}'      -- composite key 用 id
    endpoint_id TEXT NOT NULL,
    model_name  TEXT NOT NULL,
    locked_until TEXT NOT NULL,     -- ISO8601
    lock_reason TEXT,
    created_at  TEXT NOT NULL,
    UNIQUE(endpoint_id, model_name)
);
```

## 3. 协议转换器契约

### 注册表

```rust
/// 转换器注册表（效仿 cpa translator.Register 模式）。
pub trait Translator: Send + Sync {
    /// 返回支持的 `(protocol_from, protocol_to)`
    fn key(&self) -> (&'static str, &'static str);
    /// 转换请求 body（JSON Value）。
    fn translate_request(&self, body: &mut serde_json::Value, model: &str) -> Result<(), String>;
    /// 转换响应 body（非流式）。
    fn translate_response(&self, body: &mut serde_json::Value) -> Result<(), String>;
    /// 流式 SSE 行转换（每行回调）。
    fn translate_stream_line(&self, line: &str, context: &StreamContext) -> Result<String, String>;
}

/// 全局注册表。
pub struct TranslatorRegistry {
    translators: HashMap<(&'static str, &'static str), Box<dyn Translator>>,
}
impl TranslatorRegistry {
    pub fn register(&mut self, t: Box<dyn Translator>);
    pub fn get(&self, from: &str, to: &str) -> Option<&Box<dyn Translator>>;
    pub fn resolve(&self, from: &str, to: &str) -> Result<&Box<dyn Translator>, String> {
        // Native Passthrough 匹配考虑：from==to 时返回 PassthroughTranslator
        if from == to { return Ok(&self.passthrough); }
        self.get(from, to).ok_or_else(|| format!("无可用转换器: {} → {}", from, to))
    }
}
```

### 第一版注册方向

| from | to | 实现 | 来源 |
|------|-----|------|------|
| `anthropic` | `anthropic` | Passthrough（直转） | 9router Native |
| `openai-responses` | `openai-responses` | Passthrough（直转） | 9router Native |
| `openai-chat` | `openai-chat` | Passthrough（直转） | 9router Native |
| `anthropic` | `openai-chat` | Anthropic→Chat 转换 | cpa / sub2api |
| `openai-chat` | `anthropic` | Chat→Anthropic 转换 | cpa |
| `openai-chat` | `openai-responses` | Chat→Responses 转换 | 9router |
| `openai-responses` | `openai-chat` | Responses→Chat 转换 | 9router |

> Anthropic↔responses 直接转换与 Anthropic↔Chat↔responses 的双跳等价。第一版走双跳（中间经 Chat）还是直跳视复杂度定。

## 4. 常量与路径契约

proxy 层核心常量定义在 `http/proxy/constants.rs`：

```rust
pub const FILL_FIRST: &str = "fill-first";
pub const ROUND_ROBIN: &str = "round-robin";
pub const DEFAULT_MAX_SWITCHES: u32 = 10;
pub const DEFAULT_SAME_ACCOUNT_RETRIES: u32 = 3;
pub const SAME_ACCOUNT_RETRY_DELAY_MS: u64 = 500;
pub const OAUTH_REFRESH_LEAD_TIME_SECS: i64 = 60;
pub const COOLDOWN_429_RETRY_AFTER_BUFFER_SECS: i64 = 1;
pub const COOLDOWN_MAX_EXPONENTIAL_SECS: u64 = 300;
```

协议常量同时在 `protocol_type` 验证中使用：

```rust
pub const PROTOCOL_ANTHROPIC: &str = "anthropic";
pub const PROTOCOL_OPENAI_CHAT: &str = "openai-chat";
pub const PROTOCOL_OPENAI_RESPONSES: &str = "openai-responses";
```

## 5. HTTP API

### 新路由挂载

挂载（在 `http/router.rs`，在 `/api/{*path}` catch-all 之前）：

```rust
.nest("/api/routes", api::routes::routes())
.nest("/api/logs", api::logs::routes())
```

### 路由管理 API

```text
GET    /api/routes
       → [{ id, label, strategy, protocol_type, failover_enabled,
            max_switches, same_account_retries, updated_at,
            candidates: [{ id, name, base_url, priority, enabled,
                           cooldown_until, last_success_at, last_failure_at,
                           last_error_kind, has_api_key }] }]

PUT    /api/routes/{id}
       body: { strategy?, failover_enabled?, max_switches?,
               same_account_retries?, cooldown_multiplier? }
       → 204
```

### 请求日志 API

```text
GET    /api/logs?tool=&status=&from=&to=&limit=50&offset=0
       → [{ id, request_id, tool, inbound_endpoint, requested_model,
            resolved_alias, resolved_scope, target_endpoint_id,
            upstream_model, status, error_kind, fallback_chain,
            stream, duration_ms, first_token_ms,
            input_tokens, output_tokens, created_at }]
       ⟵ 限制最大条目 1000，默认降序

GET    /api/logs/{id}
       → { ...(同 GET /api/logs 字段), upstream_endpoint,
           protocol_from, protocol_to, fallback_chain (full JSON),
           cache_*_tokens, request_body_hash }
```

### 前端

- 「路由」页：替换 PagePlaceholder，展示两条固定路由卡片 + 策略选择 + 候选端点状态列表。
- 「日志」页：替换 PagePlaceholder，展示日志摘要列表 + 过滤 + 详情面板（含 fallback 链路与转换路径）。

## 6. 工具/协议转换注册初始化

在 `lib.rs` 的 setup 或状态初始化时：

```rust
let mut registry = TranslatorRegistry::new();
registry.register(Box::new(native::PassthroughTranslator));            // 同协议直转
registry.register(Box::new(anthropic_openai::AnthropicToChatTranslator));
registry.register(Box::new(anthropic_openai::ChatToAnthropicTranslator));
registry.register(Box::new(openai_responses::ChatToResponsesTranslator));
registry.register(Box::new(openai_responses::ResponsesToChatTranslator));
let registry = Arc::new(registry);
// 存入 AppState
```

`AppState` 加字段：

```rust
pub struct AppState {
    // ...现有字段...
    pub translator_registry: Arc<TranslatorRegistry>,
    pub route_proxy: Arc<RouteProxy>,  // 转发管道门面
}
```

## 7. 关键取舍

1. **Native Passthrough 不走协议转换流水线**：同协议时整个 translator 层短路，request/response 原样透传（只改 host + model + auth），最大化性能，对齐 9router 零损失直转。
2. **跨协议转换走注册表**：不硬编码，便于后续加方向（如 Gemini ↔ Anthropic），对齐 cpa 的 translator.Register 架构。
3. **模型映射与故障转移联动**：每次 failover 到新端点时重新做 `model_alias::resolve`（允许不同端点的 alias 指向不同上游模型），避免 alias 结果缓存导致的错误，对齐 9router while+excludeSet 设计。
4. **日志表默认不自动清理**：第一版不做 TTL 自动删除；用户可通过管理 API 或直接在 SQLite 手动清理。sub2api 同样默认不删。
5. **冷却多维**：端点级 + 模型级锁两层，非互斥。端点级覆盖全局过载/认证错误，模型级锁覆盖 per-model 限流（如 429 specific model），对齐 9router 的 `modelLock_${model}` + cpa 配额冷却。
6. **模型映射依赖同名或 alias**：原名匹配在 `model_alias::resolve` 内已有兜底；若连原名都匹配不到 → 返回可解释错误 + 日志记录。
7. **protocol_type 取值约定**：端点创建/更新时增加 `protocol_type` 校验（仅允许 `anthropic` / `openai-chat` / `openai-responses`），不符合的返回 400。

## 8. 与其它子任务的衔接

- 子任务 `06-27-openai-compatible-v1-endpoints` 可在本任务完成后，用相同的 translator 注册表注册 `anthropic/` → 新协议方向的转换器。
- 子任务 `06-27-chain-testing-debugger` 的测试请求可复用本任务的转发管道，但跳过 failover/冷却影响。
- 子任务 `06-27-import-export-settings` 导入后：`route_settings` 可以导入，但路由不会自动启用（用户手动检查）。`request_logs` 不参与导入导出。
