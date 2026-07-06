# 应用层技术栈约定

> 适用范围：agent-switch 桌面应用本体（Tauri + Rust + Web 前端），不是 Trellis 工具层。
> Trellis Python 运行时的规范仍在 `.trellis/spec/backend/` 与 `.trellis/spec/frontend/`，不在此处。

---

## 技术栈

| 层 | 技术 | 来源 |
|----|------|------|
| 桌面壳 | Tauri v2 | 参考 `ccs` 桌面应用形态 |
| 前端 | React + TypeScript + Vite | 参考 `ccs`，结合 Tauri Vite 集成 |
| 样式/组件 | Tailwind CSS v4 + shadcn/ui + Radix UI | 参考 `ccs`、`9router` |
| 本地 HTTP 服务 | Axum 0.8 + Tokio + tower-http | Rust/Tauri 约束下的独立最优解 |
| 数据库 | SQLite + rusqlite（bundled） | 参考 `ccs` 的 Tauri/Rust 本地存储 |
| 前端请求状态 | TanStack Query | 参考 `ccs` 前端分层 |
| 页面路由 | React Router | 独立轻量选择 |
| 日志 | tracing + tracing-subscriber + tower-http TraceLayer | Rust/Axum 常规生态 |

---

## Rust 后端目录结构

```text
src-tauri/src/
├── lib.rs              Tauri Builder、setup、managed state、服务启停
├── main.rs             桌面入口
├── app_state.rs        AppState：共享状态
├── config/paths.rs     应用数据目录、数据库路径
├── db/
│   ├── connection.rs   SQLite 连接（Arc<Mutex<Connection>>）
│   ├── migrations.rs   schema_migrations 与迁移执行
│   └── dao/            DAO 层
│       ├── accounts.rs
│       ├── endpoints.rs
│       ├── endpoint_models.rs
│       ├── model_aliases.rs
│       ├── app_metadata.rs
│       └── tool_takeover.rs  工具接管状态与备份记录
├── http/
│   ├── mod.rs          Axum 服务启动 + graceful shutdown
│   ├── router.rs       路径隔离路由 + 静态资源 fallback
│   ├── health.rs       /health
│   ├── error.rs        统一占位错误
│   └── placeholders.rs 未实现入口占位
├── services/           业务逻辑层（按模块拆分子目录或单文件）
│   ├── mod.rs
│   ├── crypto.rs
│   ├── keychain.rs
│   ├── codex_oauth.rs
│   ├── model_sync.rs
│   ├── model_alias.rs
│   └── tool_takeover/  工具接管子模块
│       ├── mod.rs      编排（enable/disable/reapply/status）+ 常量 + 备份逻辑
│       ├── claude_code.rs  Claude Code settings.json 检测与写入
│       └── codex.rs       Codex config.toml + auth.json 检测与写入
└── commands/
    └── app.rs          Tauri command
```

设计原则：

- Tauri command 不直接拼 SQL，不直接构造 HTTP 响应。
- Axum handler 不直接访问 WebView 或窗口对象。
- 数据库访问集中在 DAO/Repository 层。
- 后续子任务在现有分层内增加模块，不重写骨架。

---

## 关键陷阱与约定

### 1. rusqlite::Connection 不是 Send

`rusqlite::Connection` 内部含 `RefCell`，不能直接放进 Axum/Tauri 共享状态。必须用 `Arc<Mutex<Connection>>` 包裹。

### 2. Axum 0.8 通配路由语法

Axum 0.8 使用 `/{*path}` 风格通配路由，旧式 `/*path` 不再适用：

```rust
.route("/api/{*path}", any(placeholders::not_implemented))
```

### 3. Tauri 生命周期与 Axum graceful shutdown

- 在 `tauri::Builder::setup` 中创建 `oneshot::channel::<()>()`。
- `shutdown_tx` 存入 `AppState`，`shutdown_rx` 传给 `http::start_server`。
- `axum::serve(listener, app).with_graceful_shutdown(...)` 等待 `shutdown_rx`。
- 窗口 `on_window_event(CloseRequested)` 中触发 `shutdown_tx.send(())`。
- 关闭窗口即停止本地服务，第一版不做托盘/后台常驻。

### 3.1 Tauri 2 插件接入：注册 + capabilities 双接线

接入 `tauri-plugin-*`（如 updater / process / shell / dialog / fs）必须**两处都接**，缺一不可：

1. **后端注册**（`src-tauri/src/lib.rs`）：`tauri::Builder::default().plugin(tauri_plugin_xxx::init())` 或 `.plugin(tauri_plugin_xxx::Builder::new().build())`（按插件文档，updater 用 Builder 形式）。`Cargo.toml` 加 `tauri-plugin-xxx = "2"`（与 `tauri = "2"` 主版本对齐）。
2. **capabilities 授权**（`src-tauri/capabilities/default.json`）：在 `permissions` 数组加插件需要的权限集（如 `updater:default`、`process:default` + `process:allow-restart`），绑定到 `windows: ["main"]`。**漏掉这步 → 前端 IPC 调用被拒**（前端 `@tauri-apps/plugin-xxx` 的 JS 函数报权限错误），后端插件装了也没用。

前端对应装 `@tauri-apps/plugin-xxx`（`package.json`），在 `src/lib/` 封装一个模块（如 `updater.ts`）隔离插件 API，UI 层不直接 import 插件。参考 `updater.ts` 的 `checkForUpdate()`/`downloadAndInstall(onProgress)` 封装：错误向上抛由 UI 转中文，进度事件转 `DownloadProgress`，签名校验失败由插件内部 reject（不会半安装）。

### 4. 固定端口，不自动换端口

`127.0.0.1:42567` 绑定失败时直接启动失败，不自动寻找下一个可用端口。原因：服务地址被 Claude Code / Codex / OpenCode 配置和自动接管依赖。

### 5. 前端静态资源服务

- `tower-http` 需启用 `fs` feature。
- 使用 `ServeDir::new(dist).fallback(ServeFile::new(dist/index.html))` 作为 `fallback_service`。
- 这样 `/` 返回 `index.html`，`/assets/*` 返回静态资源，`/settings` 等前端路由回退到 `index.html`（SPA）。
- `web_dist_dir` 当前用 `env!("CARGO_MANIFEST_DIR")/../dist` 定位；后续若改为 Tauri resource 打包，需更新此处。

### 6. Tailwind CSS v4 + Vite 集成

- 使用 `@tailwindcss/vite` 插件，不需要 `tailwind.config.js`。
- `src/styles/globals.css` 中 `@import "tailwindcss";`。
- `vite.config.ts` 同时加载 `react()` 和 `tailwindcss()` 插件。

### 7. Vite 8 构建目标

Vite 8 默认 `safari13` 目标会导致 modern 破坏性解构语法转换失败。使用 `build.target: 'esnext'`，因为 Tauri WebView 自带现代浏览器内核。

### 8. Windows 构建

- 需要 `icons/icon.ico` 用于生成 Windows Resource 文件，否则 `tauri-build` 报错。
- 需要 MSVC 生成工具（Visual Studio 2022 或 Build Tools）。
- `cargo`/`rustc` 必须在 PATH 中（`~/.cargo/bin`）。

### 9. SQLite 迁移数组顺序 = 执行顺序（致命陷阱）

`db/migrations.rs` 的 `MIGRATIONS: &[Migration]` 是有序数组。`run_migrations` **按数组顺序**执行 pending 迁移（用 `applied.contains(version)` 过滤已应用的），数组顺序即全新数据库的执行顺序，与 `version` 数值大小无关。

#### 签名

```rust
struct Migration { version: i64, name: &'static str, sql: &'static str }
const MIGRATIONS: &[Migration] = &[ /* 按依赖顺序排列 */ ];
pub fn run_migrations(conn: &Mutex<Connection>) -> Result<(), String>
```

#### 契约 / 错误矩阵

| 条件 | 结果 |
|------|------|
| 数组顺序满足依赖关系（被依赖的迁移排在前面） | 全新库首启正常 |
| 一条迁移的 SQL 引用了尚未由**前面**迁移创建的表/列 | `execute_batch` 失败 → `panic` → **应用无法启动** |
| 已部署数据库（全部迁移已记录） | `pending` 为空，数组顺序无影响 |
| 仅改 `version` 数值但不调数组位置 | 顺序不变，行为不变 |
| 把已部署的 `version` 改成新值 | 重复执行 `ALTER TABLE ADD COLUMN` → 报错（不可改已部署版本号） |

#### Wrong vs Correct

依赖关系：v5 创建 `route_settings` / `request_logs` 表；v6 对这两张表 `INSERT` / `ALTER TABLE ADD COLUMN`。v6 必须排在 v5 之后。

```rust
// Wrong：版本号 6 排在 5 之前，全新库首启 v6 先执行 → no such table → panic
const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, ... },
    Migration { version: 2, ... },
    Migration { version: 3, ... },
    Migration { version: 4, ... },
    Migration { version: 6, name: "add_v1_route_and_media_log_fields", sql: "INSERT OR IGNORE INTO route_settings ...; ALTER TABLE request_logs ...", },
    Migration { version: 5, name: "create_route_settings_request_logs_model_locks", sql: "CREATE TABLE route_settings ...; CREATE TABLE request_logs ...", },
];

// Correct：按依赖顺序排列，版本号单调递增。已部署库 pending 为空，调整数组位置零风险。
const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, ... },
    Migration { version: 2, ... },
    Migration { version: 3, ... },
    Migration { version: 4, ... },
    Migration { version: 5, name: "create_route_settings_request_logs_model_locks", ... }, // 先建表
    Migration { version: 6, name: "add_v1_route_and_media_log_fields", ... },              // 再 INSERT/ALTER
];
```

#### 约定

- 新迁移**追加到数组末尾**，版本号**单调递增**，且**按依赖顺序排列**（被依赖的表/列所属迁移必须排在前）。
- 已部署迁移**不可重排、不可删除、不可改版本号**（`run_migrations` 用版本号判幂等，改号会重复执行 DDL 报错）。
- 需要改已部署迁移时，只能新增一条迁移做补救，不能改原条目。

#### 测试要求（已在 `db::migrations::tests`）

- `fresh_db_runs_all_migrations_in_order`：用 `Connection::open_in_memory()` 全新库跑 `run_migrations`，断言全部迁移成功、v5 表（`route_settings`/`request_logs`）已建、v6 的 ALTER 列（`media_type`/`body_sha256_hash`）存在、`schema_migrations` 记录数 == `MIGRATIONS.len()`。
- `migration_versions_are_ascending`：遍历 `MIGRATIONS` 断言 `version` 严格单调递增，防止顺序错乱回归。

> 本陷阱源自一次真实事故：v6 曾以 `version=6` 排在 `version=5` 之前提交，已部署环境因 `pending` 为空未暴露，全新机器首启必然 panic。补上述两条测试后封堵。

### 10. 路由转发主循环子模块接线（缺口已关闭）

> 来源：`requirement-alignment-audit` 任务（2026-06-30）发现缺口，`routing-failover-core-fix` 任务（2026-06-30）修复关闭。以下表格保留缺口原貌作为**反面教材**：`routing-failover-core` 各子模块当时均已实现并通过单测，但主转发循环 `http/proxy/mod.rs::RouteProxy::proxy_request` **未正确调用它们**。永远不要把"子模块代码存在 + 单测通过"当作"端到端功能已工作"。

**修复前**主循环的实际行为（commit `503cecf33`，现已全部接线）：

| 子模块 | 实现状态 | 主循环是否接入 | 修复前后果 | 修复方式 |
|--------|----------|----------------|------------|----------|
| `services/translator/*`（Anthropic↔Chat、Chat↔Responses 四方向 + Passthrough） | ✅ 完整 + 单测 | ❌→✅ | `proxy/mod.rs` 跨协议分支 `let _ = translator;` 丢弃转换器，body 原样透传，跨协议路由以错误格式发上游 | 新增 `proxy/translate.rs`，请求 `build_translated_request_body`（from→to）、响应 `translate_response_body`（to→from） |
| `http/proxy/stream_guard.rs`（SSE 首块缓冲探测） | ✅ 完整 | ❌→✅ | 主循环用 `resp.bytes().await` 全量缓冲，流式 SSE 不逐 chunk 转发；`failover.stream_started` 永远为 false | 新增 `proxy/sse.rs`（`SseLineDecoder` + `translate_stream`），检测 `text/event-stream` 走 StreamGuard 首块探测 + bytes_stream 逐块转发 |
| `db/dao/model_locks.rs`（模型级锁表 + `get_active_lock`） | ✅ 完整 | ❌→✅ | `selector.next()` 的 `model_lock_check` 回调恒返回 `true`，模型锁从不查询 | 回调改为实查 DB；404/model_not_found 走 `model_locks::set_lock` 写锁 |
| `endpoints.cooldown_until` 冷却写回 | 字段已建 | ❌→✅ | `failover.record_failure` 返回的冷却时长被 `let _ = cooldown_secs;` 丢弃，重启后丢失 | 新增 `persist_cooldown` helper，写 `cooldown_until`/`last_failure_at`/`last_error_kind` |

修复中一并清理的死代码 / no-op：

- `proxy/mod.rs` 的 `final_result`（恒 `None`）→ 替换为 `last_mapping: Option<ModelMappingResult>`。
- `proxy/mod.rs` 的 `body_hash_sync`（`let _ = (body, hash);`）→ 删除，模型改写后实际重算 hash。
- `is_stream = method == Method::POST` → 改为 `helpers::is_streaming(&req_json)`。

**附带修复的真实逻辑 bug（非 lint）**：`http/proxy/error.rs::should_failover` 中 `501` 分支被上方 `500..=599 => true` 区间吞掉，导致 501 被误判为可退避（与 prd.md「501 非退避」契约相反）。已将显式非退避分支上移到 5xx 区间之前。教训：`unreachable_pattern` 这类 clippy warning 可能是掩盖逻辑 bug 的信号，不要无脑 `#[allow]`。

#### 验证这类"实现存在但未接入"缺口的方法

`cargo check` 通过、`cargo clippy` 通过、子模块单测全绿，**都不能**证明主循环接入了子模块——因为未使用的代码在 release 构建里就是 dead code warning（`cargo check` 当前对 `StreamGuard`/`BufferedResult` 等报 "is never constructed/used" 警告，正是这个信号）。要确认接入，必须：

1. grep 主循环是否出现对子模块入口函数的**调用**（如 `translator.translate_request(`、`StreamGuard::new`、`model_locks::get_active_lock`），而不只是 `use` 或 `resolve`。
2. 用 mock 上游跑一次真实转发（同协议 / 跨协议 / 流式各一条），观察行为而非只看编译。

#### 约定

- 新增子模块后，必须在主循环或其调用方**实际调用**才算交付完成；`cargo check` 通过 ≠ 功能可用。
- 凡是 `let _ = <已解析的可用对象>;` 这种"拿到就丢弃"的写法，几乎一定是未完成的接线，review 时应当拦截。

### 10.1 故障转移错误分类与冷却契约（should_failover / calculate_cooldown_seconds）

> 来源：`codebase-audit` 任务（2026-07-03）P2-11 / P2-12 / P2-13 发现。PRD 规定"默认不切换端点"，只有特定错误码/类型才触发 failover；但 `calculate_cooldown_seconds` 对 `AuthError` 走 `_ => 30` 默认分支返回 30s，与 PRD「auth 类冷却 5 分钟」不符。spec 之前只记录了 `should_failover` 的 501 教训，未固化完整分类表与冷却时长，导致 30s 这个值漂出 PRD 契约无人察觉。

**契约（PRD 对齐，写代码时必须遵守）**：

| 错误类型 / 状态码 | `should_failover` 切换端点? | `calculate_cooldown_seconds` 冷却时长 | 说明 |
|------------------|------------------------------|----------------------------------------|------|
| 429 / 529（限流） | 是（可重试，同账号先重试） | PRD 指定值（含 Retry-After） | 同账号重试有 `SAME_ACCOUNT_RETRY_DELAY_MS=500ms` 间隔 |
| 5xx（500/502/503/504，**不含 501**） | 是 | 默认冷却值 | 501 必须在 5xx 区间前显式排除为非退避 |
| 501 | **否**（非退避） | — | 显式非退避分支须在 `500..=599` 区间之前 |
| 401 / 403（凭据失效） | 触发 OAuth 预检刷新 | — | 刷新失败转 AuthError |
| `AuthError`（刷新失败） | 是 | **300s（5 分钟）** | PRD 第 92 行；30s 会反复无效重试放大上游错误率 |
| 4xx（400/404/...，除 401/403/429） | **否** | — | 客户端错误不退避 |
| 网络错误 / 连接重置 | 是 | 默认冷却值 | — |

**常见错误**：

- `calculate_cooldown_seconds` 用 `_ => 30` 兜底所有未显式列出的错误类型 → `AuthError` 漏进默认分支拿到 30s。**必须**为 `AuthError` 显式匹配 300s，并在默认分支注释"仅用于未分类的瞬时错误"。
- 把 `AuthError` 的冷却写短了，会导致 selector 在 30s 后重选同一已失效凭据端点，反复触发刷新失败，放大对上游的错误率。
- OAuth 刷新用无超时的 `reqwest::Client`（`oauth_refresh.rs`）→ auth.openai.com 半挂时 `send().await` 永久阻塞主循环，故障转移无法触发。**刷新 client 必须设超时**（建议 connect+response 总 ≤ 30s）。
- 刷新响应未携带新 `refresh_token` 时直接保留旧值 → 若服务端已轮换 refresh_token，旧值已作废，下次刷新必 400 `invalid_grant`。**应**：响应无 refresh_token 时记 warn 但可继续用旧值；下次刷新失败需明确清理账号凭据而非静默重试。

**约定**：

- 任何新增错误类型都必须在 `should_failover` 与 `calculate_cooldown_seconds` **两处**同时显式分类，禁止依赖默认分支 `_ =>`。新增类型后跑 `http/proxy/error.rs` 与 `failover.rs` 的对应单测。
- 冷却时长是 PRD 契约常量，不要在代码里硬编码 `30` / `300` 散值；集中到 `constants.rs` 并以 PRD 行号注释。
- `should_failover` 的 match 顺序敏感：非退避的显式分支（501、4xx）必须在宽区间（`500..=599`）之前，否则被吞。`unreachable_pattern` clippy warning 常是顺序 bug 信号，不要 `#[allow]`。

### DAO 不做加密

DAO 层（`db/dao/*`）只做 SQL，原样存取 `credentials_encrypted` / `api_key_encrypted` BLOB。加密在 `services/crypto.rs` 完成，HTTP handler 调用加密服务后把 BLOB 传给 DAO。

### AES-256-GCM 加密结构

- 主密钥：32 字节，存系统 Keychain（`keyring` crate，service=`agent-switch`，account=`master-key`）。
- Nonce：12 字节随机。
- AAD：记录 ID（`account_id` / `endpoint_id`），防止密文挪用。
- BLOB 结构：`nonce(12) || ciphertext || tag`。

### Keychain 降级

Keychain 不可用时 `AppState.crypto = None`，应用仍可启动，但凭据相关功能进入降级：
- 创建/更新账号/端点含凭据时返回 503。
- UI 提示"系统凭据管理器不可用"。
- 不静默改明文存储。

### 凭据脱敏

HTTP 响应永不返回 `credentials_encrypted` / `api_key_encrypted` BLOB，只返回 `has_credentials` / `has_api_key` 布尔。OAuth token 不返回值，只返回 email/plan_type/expires_at。

---

## Codex OAuth 约定

### 元数据（参考 9router / cpa）

- 授权端点：`https://auth.openai.com/oauth/authorize`
- Token 端点：`https://auth.openai.com/oauth/token`
- client_id：`app_EMoamEEZ73f0CkXaXp7hrann`
- redirect_uri：`http://localhost:1455/auth/callback`
- scope：`openid profile email offline_access`
- code_challenge_method：`S256`

### 临时回调服务

- 独立于主 Axum 服务（42567），单独 `TcpListener` 绑定 127.0.0.1:1455。
- 登录完成后立即 graceful shutdown 释放端口。
- 同一时刻只允许一个登录会话（`tokio::sync::Mutex`）。
- 1455 被占用时登录失败，不自动换端口。

### JWT 解析

解析 id_token payload（不验签，仅取字段）：
- `email`
- `chatgptAccountId`（9router）或 `accountID`（cpa）
- `chatgptPlanType`

### 后续扩展点

- Device Code 流程（`ccs` 风格）：预留，不在当前子任务实现。
- token 自动刷新：父任务已确认参考 `ccs`（过期前 60 秒后台刷新），后续路由子任务实现。

---

## 管理 API 分层约定

```text
src-tauri/src/http/api/
├── accounts.rs    /api/accounts CRUD + 脱敏
├── endpoints.rs   /api/endpoints CRUD + toggle + 脱敏
├── auth.rs        /api/auth/codex/login + status
├── models.rs      /api/models：list / sync / custom / delete / resolve
├── aliases.rs     /api/models/aliases：list / create / delete
├── settings.rs    /api/settings/auto-model-refresh + export/import（portability，见下节）
├── tools.rs       /api/tools：工具接管状态、开关、重新应用、备份查询
├── routes.rs      /api/routes：路由设置列表/更新（failover_enabled/max_switches 等）
└── logs.rs        /api/logs：请求摘要日志列表/过滤（按 tool=生产/测试、route 过滤）
```

挂载方式（在 `http/router.rs`，按顺序 nest，在 `/api/{*path}` catch-all 之前）：

```rust
.nest("/api/accounts", api::accounts::routes())
.nest("/api/endpoints", api::endpoints::routes())
.nest("/api/auth", api::auth::routes())
.nest("/api/models", api::models::routes())
.nest("/api/models/aliases", api::aliases::routes())
.nest("/api/settings", api::settings::routes())
.nest("/api/tools", api::tools::routes())
.nest("/api/routes", api::routes::routes())
.nest("/api/logs", api::logs::routes())
.route("/api/{*path}", any(placeholders::not_implemented))  // 其余 /api/* 占位
```

HTTP handler 只做参数校验和响应脱敏，不直接拼 SQL 或加密。

---

## 模型管理与别名约定

> 来源：`model-management-refresh-alias` 子任务。数据模型参考 `9router` 内置/自定义模型分离、`sub2api` 同步字段、`ccs` 角色映射；别名解析参考 `cpa` resolveOAuthUpstreamModel、`ccs` map_model。

### API 分层与挂载

```text
src-tauri/src/http/api/
├── models.rs     /api/models：list / sync / custom / delete / resolve
├── aliases.rs    /api/models/aliases：list / create / delete
└── settings.rs   /api/settings/auto-model-refresh：get / put
```

挂载（`http/router.rs`，注意嵌套顺序）：

```rust
.nest("/api/models", api::models::routes())
.nest("/api/models/aliases", api::aliases::routes())
.nest("/api/settings", api::settings::routes())
```

`resolve` 属于"使用别名"行为，挂在 `models.rs` 的 `/resolve/{alias}`，与"管理别名"的 `aliases.rs` 分离。

### API 契约

```text
GET    /api/models?endpoint_id=&source=&capability=   能力用 LIKE 子串匹配
POST   /api/models/sync                                返回 SyncReport（succeeded/failed/errors）
POST   /api/models/custom                              source='custom'，必须绑定 endpoint_id
DELETE /api/models/{id}                                删除前标记关联 alias 失效
GET    /api/models/resolve/{alias}?tool=&route_id=&endpoint_id=
GET    /api/models/aliases?scope_type=&scope_id=
POST   /api/models/aliases                             单行一个 target
DELETE /api/models/aliases/{id}
GET    /api/settings/auto-model-refresh               返回 {enabled,last_sync_at,last_sync_error}
PUT    /api/settings/auto-model-refresh               body {enabled:bool}，返回 204
```

### 模型刷新覆盖语义（model_sync.rs）

- 只刷新 `enabled=true` 的端点；`source='synced'` 受刷新管理，`source='custom'` 不受影响。
- upsert 用 `ON CONFLICT(endpoint_id, model_name)`，刷新返回的模型 `is_available=1` 并更新 `last_seen_at`。
- 本次未返回的 synced 模型：`mark_unavailable_except` 把 `last_seen_at < sync_time` 的标记 `is_available=0`（下线，不删除）。
- 刷新失败只写 `last_model_sync_error`，不阻塞应用；逐端点失败记入 `SyncReport.failed`/`errors`。
- 设置与时间戳存 `app_metadata` key-value 表，不新增表：`auto_model_refresh_enabled` / `last_model_sync_at` / `last_model_sync_error`。
- `SETTING_*` 常量以 `services/model_sync.rs` 为权威定义；`settings.rs` 只引用 `last_*` 键，开关读写走 `ModelSyncService::is_auto_refresh_enabled` / `set_auto_refresh`，不要重复定义 `auto_model_refresh_enabled` 常量。

### 别名解析引擎（model_alias.rs）

解析优先级（从高到低，命中即返回该作用域候选）：

```text
tool > route > endpoint > global > 原名匹配(name_match) > not_found
```

- 解析只读 alias 表，不修改。返回 `ResolvedAlias { alias_name, matched_scope, candidates }`。
- `query_aliases` 过滤条件含 `r.enabled`：**被标记失效（enabled=0）的别名不会出现在候选中**。
- **取舍/已知限制**：当某别名的所有 target 都已失效时，`resolve` 返回 `matched_scope='not_found'` + 空候选，失效原因不在 resolve 结果中体现（仅能从 `GET /api/models/aliases` 的 `invalid_reason` 看到）。design.md 中 `AliasCandidate.is_valid`/`invalid_reason` 字段的"返回失效候选供故障转移决策"意图当前未在 resolve 路径启用。`routing-failover-core` 子任务若需要遍历含失效项的候选链，需放宽 `query_aliases` 的 `enabled` 过滤并在调用方按 `is_valid` 决策，而不是依赖 resolve 自动跳过。

### 删除模型 → 别名失效

`endpoint_models::mark_alias_invalid_for_model(endpoint_id, model_name)` 按 `target_endpoint_id + target_model_name + enabled=1` 匹配，置 `enabled=0` 并写 `invalid_reason`。

- 匹配键是 endpoint_id + model_name，与模型 source 无关（custom 与 synced 删除都会触发）。
- 别名不自动删除，UI（AliasPanel）显示 `invalid_reason` 提示用户重新选择。

---

## 前端 API 客户端约定

`src/lib/api.ts` 按资源分组：`accountsApi` / `endpointsApi` / `authApi` / `modelsApi` / `aliasesApi` / `settingsApi` / `toolsApi` / `portabilityApi`。

- 所有请求 `Content-Type: application/json`。
- 204 响应返回 `undefined`。
- 错误抛 `Error("${status}: ${body}")`。
- TanStack Query 的 queryKey 按资源命名：`['accounts']` / `['endpoints']` / `['models']` / `['aliases']` / `['auto-refresh']`。
- 带查询参数的 GET 用 `URLSearchParams` 拼接，空参数不下发；path 段（如别名名）用 `encodeURIComponent`。
- 模型页拆分组件：`components/models/CustomModelForm.tsx`、`components/models/AliasPanel.tsx`（内含 AliasForm 与 resolve 测试），主表格在 `pages/ModelsPage.tsx`。

---

## 前端目录结构

```text
src/
├── main.tsx                    TanStack Query Provider + React Router
├── App.tsx                     路由：8 个中文页面
├── components/
│   ├── layout/
│   │   ├── AppShell.tsx        侧边栏 + 主内容区
│   │   └── PagePlaceholder.tsx 空状态占位
│   ├── tools/
│   │   ├── ToolCard.tsx        自动接管工具卡片（Claude Code / Codex）
│   │   └── OpenCodeCard.tsx    手动配置说明卡片
│   └── ui/                     shadcn/ui 组件（后续添加）
├── pages/                      8 个中文页面
├── lib/                        api.ts、utils.ts
└── styles/globals.css          Tailwind CSS
```

约定：

- 所有 UI 文案默认中文。
- 未实现页面使用中文空状态，不写假功能或假数据。
- `QueryClient` 在应用根部创建一次，不要在组件内重复创建。

---

## 工具接管约定

> 来源：`tool-takeover-claude-code-codex` 子任务。接管逻辑集中在 `services/tool_takeover/` 子模块。

### 服务层组织

```
services/tool_takeover/
├── mod.rs          编排入口（enable/disable/reapply/status/list_backups）+ 共享常量
├── claude_code.rs  Claude Code 配置检测与写入
└── codex.rs        Codex 配置检测与写入
```

- `mod.rs` 定义 `Tool` 枚举（ClaudeCode / Codex / OpenCode）、`LiveCategory` 枚举（AgentSwitch / Official / ThirdParty / Unconfigured / Unrecognized）、共享常量（`LOCAL_BASE`、`PLACEHOLDER_TOKEN`、`CODEX_PROVIDER_ID`）。
- 工具配置文件路径通过 `Tool::config_dir()` 解析（内部用 `dirs::home_dir()`），不做硬编码。
- OpenCode 的 `config_dir()` 返回 `None`，`supports_takeover()` 返回 `false`。

### 备份约定（backup_before_write）

- 每次接管写入前在 `<data_dir>/backups/tools/` 保存原始文件备份。
- 备份文件名格式：`<tool>-<原文件名>-<时间戳>.bak`（时间戳中 `:` 替换为 `-` 以兼容 Windows）。
- 原文件不存在的场景：仍写入备份记录，`original_existed=false`。
- 已是 agent-switch 接管态的配置再次写入时，跳过文件复制（R3.4），避免用接管配置覆盖好的原始备份。
- 备份记录持久化在 `tool_takeover_backups` 表，工具页可列出。

### 指向检测（detect，只读）

| 工具 | 读取文件 | 判定逻辑 |
|------|----------|----------|
| Claude Code | `~/.claude/settings.json` → `env.ANTHROPIC_BASE_URL` | 等于 `LOCAL_BASE + /claude-code` → AgentSwitch；含 `anthropic.com` → Official；空/缺失 → Unconfigured；其它 → ThirdParty；解析失败 → Unrecognized |
| Codex | `~/.codex/config.toml` → 顶层 `model_provider` 与 `[model_providers.*]` | `model_provider == "agent-switch"` → AgentSwitch；无 `model_provider` → Unconfigured；provider 表不存在 → Official；其它 → ThirdParty；解析失败 → Unrecognized |

检测异常不 panic：JSON/TOML 解析失败走 `Unrecognized`。

### 配置文件写入契约

- Claude Code `settings.json`：用 `serde_json::Value` 合并，只设 `env.ANTHROPIC_BASE_URL` 和 `env.ANTHROPIC_AUTH_TOKEN`，保留其它键。
- Codex `config.toml`：用 `toml_edit::DocumentMut` 外科式编辑，只改 `model_provider` 顶层值和 `[model_providers.agent-switch]` 表，保留注释、其它表、其它 provider。
- Codex `auth.json`：用 `serde_json::Value` 合并，只覆盖 `OPENAI_API_KEY` 为占位符，保留 `tokens`、`last_refresh` 等字段。
- 所有写入使用原子写模式：先写 `.tmp` 再 `fs::rename` 覆盖，目标目录不存在时 `create_dir_all`。
- 写入绝不包含真实上游 API Key / OAuth token（只写 `PLACEHOLDER_TOKEN`）。

### API 契约

```text
GET    /api/tools
       → [{ tool, supports_takeover, enabled, live_category, last_applied_at, last_target, last_error }]
       opencode 的 supports_takeover=false

GET    /api/tools/{tool}
       → { tool, supports_takeover, enabled, live_category, last_applied_at, ... }

POST   /api/tools/{tool}/takeover    body { enabled: bool }
       enabled=true  → 备份 + 写入 + 置 enabled=1
       enabled=false → 仅置 enabled=0，不改工具文件
       tool=opencode → 400 not_supported

POST   /api/tools/{tool}/reapply
       幂等重写接管配置（要求 enabled=true，否则 409）

GET    /api/tools/{tool}/backups
       → [{ id, original_path, backup_path, original_existed, takeover_target, created_at }]
```

### toolsApi queryKey

- `['tools']` — 所有工具列表
- `['tools', 'backups', <tool>]` — 工具备份列表

### 已知限制

- 关闭接管不自动还原原配置，工具页仅展示备份路径和可复制的恢复说明（无写回按钮）。
- OpenCode 暂不提供自动接管（第一版仅手动配置说明）。
- `live_category` 检测仅做当前指向分类，不验证代理路由是否真正可用。

---

## 路由代理与链路测试约定

> 来源：`routing-failover-core` / `openai-compatible-v1-endpoints` / `chain-testing-debugger` 三个子任务。代理转发逻辑集中在 `http/proxy/` 子模块，协议翻译在 `services/translator/`。

### 代理模块组织

```
http/proxy/
├── mod.rs            RouteProxy 编排入口 + 故障转移主循环
├── constants.rs      协议常量（PROTOCOL_*）、策略常量（FILL_FIRST / ROUND_ROBIN）
├── selector.rs       EndpointSelector：按协议加载候选 + 策略选择 + 能力过滤 + 冷却跳过
├── failover.rs       FailoverState：失败计数 / 冷却回写 / fallback 链记录
├── model_mapper.rs   ModelMapper：别名解析 + 模型改写 + 能力校验
├── auth_injector.rs  inject_auth：API Key / OAuth token 注入
├── oauth_refresh.rs  ensure_valid_token：过期前 60s 刷新
├── capability.rs     path_to_capability / capability_to_protocol（v1 子路径 → 能力 → 协议）
├── logger.rs         RequestLogEntry + write_log（SHA256 body hash，不存正文）
└── stream_guard.rs   SSE 首块缓冲 / 流式守卫
```

### 编排管道（RouteProxy::proxy_request）

签名：`proxy_request(&self, route_id: &str, req: Request<Body>, test_only: bool)`。

管道顺序：`load_route_settings → selector → model_mapper → auth_injector → translator → forward → logger`，外层由 `FailoverState` 驱动循环。

- v1 路由：从子路径 `path_to_capability` 解析 `required_capability`，再 `capability_to_protocol` 推导实际协议类型；selector 用 `filter_by_capability` 预筛，无具备该能力的端点直接返回 502。
- `images` / `audio` 能力走 **媒体透明流转**（`is_passthrough`）：使用原始二进制 body，不经过 translator；响应仅记录 SHA256 哈希和 content-length，不存内容。
- 请求体读取上限 10MB（`to_bytes(body, 1024*1024*10)`）。
- 成功即 `record_success` 并写日志后返回；失败 `record_failure` 后 `continue` 下一个候选；候选耗尽返回 502 + last_error。

### 故障转移语义（FailoverState）

- 受 `route_settings.failover_enabled` / `max_switches` / `same_account_retries` 控制。
- `record_failure` 写 `endpoints.cooldown_until` / `last_failure_at`，`record_success` 写 `last_success_at`，并把每跳记入 `fallback_chain`（JSON，写入日志）。
- **错误分类已接线**（`ProxyError::should_failover()`）：非成功上游响应先判 `should_failover`——可重试错误（网络/超时/408/429/529/5xx/容量类）才 `record_failure` + `continue` 下一候选；不可重试错误（400/405/406/413/414/415/422/501、无效请求、上下文超限、本地配置/DB 错误、已开始输出的流式响应）写日志后直接把上游响应返回客户端，**不切换端点**。401/403、404/model_not_found、余额不足按账号/端点类型谨慎处理。**勿对每个非成功码都 `record_failure`+`continue`**——会让 4xx 误触发端点切换，违反 PRD「默认不切换」规则。

### 测试模式（test_only=true）

链路测试复用同一条 `proxy_request` 管道，仅以 `test_only` 改变副作用：

- **selector**：`set_skip_cooldown(true)`，不跳过冷却中的端点（允许探索全部候选）。
- **failover**：`FailoverState::new(.., test_only)`，`record_failure` / `record_success` **不回写** `cooldown_until` / `last_failure_at` / `last_success_at`。
- **logger**：`is_test=true`，日志 `tool='test'`，LogsPage 可按「全部 / 生产 / 测试」过滤。
- 其余（auth_injector / model_mapper / translator / forward）与生产路径完全一致。

### POST /api/tests 契约

```text
POST /api/tests   body { route, path, model?, prompt, stream }
     route ∈ claude-code | codex | v1（其它 → 400）
     按 route 构建请求体：claude-code=Anthropic messages，codex=responses input，v1=chat messages
     stream=true  → 透传上游 text/event-stream，附 x-test-duration-ms 头
     stream=false → JSON 包装 { status, body, duration_ms, endpoint_id, error }
```

前端：RoutesPage 每条路由卡片底部「测试」折叠面板（path / model 下拉 / prompt / stream 开关 + 结果区 + 统计 + fallback 链）；流式用 EventSource，`AbortController` 取消。媒体结果用 `URL.createObjectURL` 临时展示，不持久化。

### 协议翻译注册表（services/translator/）

- `TranslatorRegistry` 持 `Arc<dyn Translator>`，`resolve(from, to)` 按协议对查找翻译器。
- 同协议（`protocol_from == protocol_to`）走 **Native Passthrough**，不翻译。
- 已注册：native、anthropic↔openai、openai responses。

### 流式翻译 wire-format 契约（Anthropic 方向）

> 来源：`codebase-audit` 任务（2026-07-03）P1-2。`ChatToAnthropicTranslator` 前向流式（入站 openai-chat → 上游 anthropic）不发出 `content_block_stop`，导致符合 Anthropic 协议的客户端无法 finalize `tool_use` 块的 input JSON，工具调用不可用。此 bug 当前被"跨协议未接线"（见下"已知限制"）掩盖，**接线时必须先修**。

**Anthropic 流式 SSE 必须发出的事件序列**（每个 content_block）：

```
event: content_block_start   data: {"type":"content_block_start","index":<i>,"content_block":{"type":"text"|"tool_use","id":...,"name":...}}
event: content_block_delta   data: {"type":"content_block_delta","index":<i>,"delta":{"type":"text_delta","text":"..."} | {"type":"input_json_delta","partial_json":"..."}}
event: content_block_stop    data: {"type":"content_block_stop","index":<i>}      ← 必须发，否则客户端不 finalize 该块
event: message_stop          data: {"type":"message_stop"}
```

**契约**：

- 每个 `content_block_start` 必须有配对的 `content_block_stop`。`text` 块和 `tool_use` 块都如此；`tool_use` 块的 `input_json_delta` 累积体只在 `content_block_stop` 后才被客户端 finalize 为完整 JSON。
- `index` 是所有 content block 的**全局序号**（text + tool_use 混排，从 0 递增），不是 tool 专用序号。AnthropicToChat 反向翻译时须把 Anthropic 全局 `index` 映射到 OpenAI 的 `tool_calls[].index`（只计工具调用，从 0 开始）；`block_to_tool_index` map 用于此映射。
- `partial_json` 字段值必须是 JSON 字符串转义后的形式（`serde_json::to_string(args)`），不能对 args 内部再反斜杠转义（会损坏 JSON）。

**常见错误**（审查已发现）：

- 只发 `content_block_start` + `delta`、在 `finish_reason` / `[DONE]` 路径不发 `content_block_stop` → 工具参数无法闭合。**修复**：检测到块结束（新块开始 / `finish_reason` / `message_stop`）时为每个已打开的 tool_use 块补发 `content_block_stop`。
- `input_json_delta` 的 `acc.arguments` 累积后从不输出（死状态）→ 易误导维护者忽略 stop 缺失。
- `max_tokens` ↔ `max_output_tokens` ↔ `max_completion_tokens` 在 Chat/Responses/Anthropic 三方向不映射（P2-2 / P2-3）→ Anthropic 必填的 `max_tokens` 缺失会 400。

**约定**：

- 新增/修改任一方向的流式翻译后，**必须**用含工具调用的 mock 上游跑端到端流式，断言客户端收到的 SSE 事件序列含完整的 `content_block_start` → `delta*` → `content_block_stop`（每个块）。
- `block_to_tool_index` / `output_index` 映射是双向流的易错点，单测须覆盖"text + 多 tool_use 混排"和"tool_use 块在 text 块之前"两种顺序。

### 已知限制 / 取舍

- **跨协议翻译当前未真正接线**：`mod.rs` 中 `protocol_from != protocol_to` 分支虽 `resolve` 出 translator，但实际以 `let _ = translator;` 丢弃，转发的是未翻译的原始 JSON。真正的请求/响应翻译调用是后续工作；当前生产可用路径是同协议透明转发。
- `body_hash_sync` 为占位函数（模型改写后的 hash 同步未实现）。
- selector 的 `model_lock_check` 回调当前恒返回 `true`（全模型锁表查询未接线）。
- 测试端点非流式响应体上限同样为 10MB。

---

## 配置导入导出（portability）约定

> 来源：`import-export-settings` 子任务（MVP 收尾）。配置导入导出集中在 `services/portability/` 子模块，HTTP 入口扩展在 `http/api/settings.rs`。

### 服务层组织

```
services/portability/
├── mod.rs          编排入口：export(db, mode, password?) / import(db, data_dir, package, password?, conflict_mode?)；full_backup 导入前本地 DB 文件备份
├── package.rs      ExportPackage / KdfParams / Payload 及各 *Export 结构；FORMAT_VERSION=1、PACKAGE_AAD 包级常量、PORTABLE_METADATA_KEYS 白名单
├── crypto_box.rs   双密钥：主密钥模式 seal/open + Argon2id 密码模式 seal/open + gzip 压缩 + 弱密码检测
├── collect.rs      导出：从各表收集 → Payload（portable 剔除凭据列 + ui_settings 非白名单键）
└── apply.rs        导入：replace（full_backup，保留 id） / merge（portable，新 UUID + id 重映射）+ 单事务 + tool_takeover 强制 enabled=0
```

- 新增依赖：`argon2 = "0.5"`（密码派生）、`flate2 = "1"`（gzip）。复用 `aes-gcm` / `base64` / `uuid` / `time`。在 `services/mod.rs` 注册 `pub mod portability;`。

### 双密钥导出策略（核心决策）

两种导出模式加密目标不同，故用不同密钥派生，解决「强制加密」与「可跨机器迁移」的矛盾：

| 模式 | 密钥 | kdf | 可跨机器 | 含凭据 |
|------|------|-----|----------|--------|
| full_backup | 系统主密钥（`keychain::load_master_key`，32B） | none | 否（绑定本机主密钥） | 是 |
| portable | 用户密码 → Argon2id 派生 32B | argon2id | 是（输密码即可解） | 否（脱敏） |

- full_backup **直接装入当前 DB 已加密 BLOB**（base64），不解密重装——减少明文凭据暴露面，且天然实现「绑定主密钥环境」语义。跨主密钥环境无法解密，符合预期。
- portable 密码 Argon2id 参数：salt 随机 16B、m_cost=19456(19MiB)、t_cost=2、p_cost=1，写入包 `kdf_params` 供导入复现。
- 弱密码（< 8 字符或全同字符类）返回 warning，**不阻止**导出。

### ExportPackage 容器格式

```rust
ExportPackage {
  format_version: u32,            // = 1，不匹配拒绝导入
  mode: "full_backup" | "portable",
  algo: "AES-256-GCM",
  kdf: "none" | "argon2id",
  kdf_params: Option<KdfParams>,  // 仅 argon2id：salt(base64 16B)+m_cost+t_cost+p_cost
  nonce: base64(12B),
  created_at, app_version,        // app_version = env!("CARGO_PKG_VERSION")
  ciphertext: base64,            // AES-GCM(gzip(payload_json))
}
```

- **AAD 用包级常量** `agent-switch-export-v1`（包级，非记录级），与凭据加密的「记录 ID 作 AAD」不同——导出包是整包加密，无单条记录 ID。
- **gzip 在加密前**：payload JSON → flate2 gzip → AES-GCM；解密反向（解密 → gunzip → 反序列化）。加密后数据高熵不可再压。
- 文件扩展名：full_backup `.asbak`、portable `.ascfg`，实质都是上述 JSON 文本。

### ui_settings 白名单原则（易踩坑）

> **Warning**：`ui_settings` 只导出**偏好类** app_metadata 键，排除本机运行状态快照。新增 app_metadata 键**默认不导出**，需显式加入白名单。

| key | 导出 | 理由 |
|-----|------|------|
| `auto_model_refresh_enabled` | ✅ | 用户偏好，跨机器有意义 |
| `last_model_sync_at` | ❌ | 本机运行状态，迁移无意义 |
| `last_model_sync_error` | ❌ | 本机运行状态，迁移无意义 |

实现用**白名单常量** `PORTABLE_METADATA_KEYS = &["auto_model_refresh_enabled"]`，而非黑名单。full_backup 同样只导出白名单偏好键（本机状态对同机恢复也无意义）。

#### Wrong vs Correct

```rust
// Wrong：黑名单排除本机状态——后续新增 app_metadata 键默认会被导出，可能误导出新的本机运行状态键
const EXCLUDE_KEYS: &[&str] = &["last_model_sync_at", "last_model_sync_error"];

// Correct：白名单只导出已知偏好键——新增键默认不导出，需显式评估后加入
const PORTABLE_METADATA_KEYS: &[&str] = &["auto_model_refresh_enabled"];
```

### 导入策略

| 数据类型 | full_backup（replace） | portable（merge） |
|----------|------------------------|-------------------|
| 账号/端点元数据 | DELETE + INSERT 保留原 id | 按匹配键 upsert 非敏感字段，未命中新增（新 UUID） |
| API Key / OAuth token | 装入的加密 BLOB 原样写回 | skip（脱敏包本就不含）；命中端点时 UPDATE 不含凭据列，保留本机已有值 |
| custom models / aliases / route_settings | replace | upsert |
| tool_takeover | 整表 DELETE + INSERT (enabled=0) | upsert (enabled=0) |
| request_logs / 测试数据 | skip（collect 不查、apply 不写） | skip |

merge 匹配键：accounts(name+account_type+platform)、endpoints(name+base_url+protocol_type)、endpoint_models(endpoint_id+model_name)、model_aliases(scope_type+scope_id+alias_name)、route_settings(id)。

- **merge id 重映射**：新增账号/端点用新 UUID，建 `old_id→new_id` 映射，改写 `endpoint.account_id` / `alias.target_endpoint_id` / `model.endpoint_id`，保持包内关联完整。
- **merge 不动本机凭据**：命中已有端点时 UPDATE 语句不含 `api_key_encrypted` 列，账号 UPDATE 不含 `credentials_encrypted` 列（COALESCE 保留本机值）。

### 安全底线（导入）

- **导入全程单事务**（`Connection::transaction()`），任一步失败 `rollback()`，不留半成品。
- **full_backup 导入前自动本地 DB 备份**：先 `wal_checkpoint(TRUNCATE)` 再 `std::fs::copy` 复制 sqlite 文件到 `<data_dir>/backups/db/`（参考 `tool_takeover/mod.rs` 的文件复制范式）。返回 `pre_import_backup` 路径供 UI 展示。
- **tool_takeover 导入后强制 enabled=0**，两种模式都绝不写 Claude Code/Codex 配置文件。
- **request_logs / model_locks / tool_takeover_backups / 测试数据永不导出/导入**（collect 不查这些表，apply 不写）。
- 响应不回显任何明文凭据：`ExportResult.package` 内凭据为加密 BLOB；`ImportResult` 只返回计数 + 备份路径 + warnings。

### API 契约

```text
POST /api/settings/export   body { mode: "full_backup"|"portable", password?: string }
     → 200 { package: string, warnings: string[] }
       full_backup 主密钥不可用 → 503
       portable 缺密码 / 未知 mode → 400

POST /api/settings/import   body { package: string, password?: string, conflict_mode?: "auto" }
     → 200 { imported: ImportReport, pre_import_backup?: string, warnings: string[] }
       版本不符 / 解密失败 / 密码错误 → 400
       事务/写入失败 → 500
```

- `conflict_mode` 当前仅占位 `"auto"`（按包 mode 决定 replace/merge），保留扩展。
- `pre_import_backup` 仅 full_backup 返回自动备份路径。
- 错误码映射靠错误串关键字匹配（`map_export_error` / `map_import_error`），新增可读错误文案时注意含已映射关键字（"主密钥"/"系统凭据管理器"→503，"版本"/"解密"/"密码"/"KDF"→400）。

### 前端

- `src/lib/api.ts` 新增 `portabilityApi`（`exportConfig(mode, password?)` / `importConfig({package, password?, conflict_mode?})`）+ 类型 `ExportResult` / `ImportResult` / `ImportReport` / `PortabilityMode`。`ImportReport` 字段名严格对齐后端 struct。
- `SettingsPage.tsx` 新增「配置导入导出」卡片：完整备份导出 / 脱敏导出（密码 + 弱密码 warning）/ 导入（`FileReader` 读文本 + 密码 + 结果计数 + 备份路径）+ 四条中文风险提示（凭据绑定本机、跨机器需重录、覆盖/合并、接管导入后关闭）。
- 导出下载用 `Blob` + `a[download]`，文件名含 mode + 时间戳（`.asbak`/`.ascfg`）；导入成功后 `queryClient.invalidateQueries()`（无参 = 全部失效，因导入改动多张表）。

### 测试要求（已在 `services::portability::tests`）

- `full_backup_roundtrip`：导出 → 同主密钥环境导入，断言凭据 BLOB 恢复、tool_takeover 全部 enabled=0。
- `portable_strip_and_merge_preserves_local_key`：脱敏导出断言凭据列 None；换环境导入 merge，断言本机已有 api_key 保留、新增记录用新 id。
- `weak_password_warning`：弱密码导出返回 warning 但不报错。
- `wrong_key_readable_error`：错误密码/密钥导入返回可读错误不 panic。

### 取舍 / 已知限制

- route_settings 在 replace 模式用 `ON CONFLICT(id) DO UPDATE`（upsert）而非其它表的 DELETE+INSERT——因 route_settings 的 id 固定（claude-code/codex/v1）且总在完整备份中，upsert 等价且对部分包更安全。
- import 主密钥不可用映射 400 而非 503——AC4 的 503 仅 scope export；不可解密的导入包合理归 400。
- 云同步 / WebDAV / 增量导出 / 包签名验证均未纳入（父 PRD 第一版不做）。

---

## 外部数据源导入编排约定

> 来源：`import-from-ccs` 任务。从本地第三方工具（cc-switch 等）一键导入上游渠道到 agent-switch。集中在 `services/importers/` 子模块，HTTP 入口扩展在 `http/api/<domain>.rs`。

### 核心契约：direct provider 不内联明文凭据

agent-switch 的 direct 模式 **刻意偏离 ccs 的明文内联**（`services/tool_takeover/mod.rs:126-128` 注释明说"偏离 ccs 明文做法"）：

- direct provider 的 `settings_config` 是 `{"endpoint_id": <必填>, "model"?, "wire_api"?, "requires_openai_auth"?}`，**不含** `base_url`/`token`。
- 真实 `base_url` 从 `endpoints` 表按 `endpoint_id` 查出，token 从 `endpoints.api_key_encrypted` 经 AES-256-GCM 解密。
- **因此从外部源导入一条"上游渠道"必须拆成两行**：先建 `endpoints`（加密存 token），再建 `providers`（`mode=direct`，`settings_config` 引用 `endpoint_id`）。只写 providers 一行 → 切换时 `resolve_direct_config` 反序列化失败（`endpoint_id` 必填无 default），provider 永远无法激活。

### 批量导入编排模式（逐项独立 + 双表原子 + 幂等追溯）

从外部源批量导入时用以下模式（`services/importers/ccs.rs` 为参考实现）：

1. **detect（只读预览）+ import（写入）分离**：detect 返回 `DetectItem` 列表供 UI 勾选，不含明文凭据（`has_api_key: bool` 而非回传 token）；import 只收 `original_id` + `imported_name`，**后端按 `original_id` 重新读源数据**（不信任前端传 base_url/token，防篡改/漏传）。
2. **逐项独立，非全有全无**：批量导入单个失败记入 `errors`，其余继续。用户看到部分成功 + 失败清单，体验优于整批回滚。
3. **双表原子（单项内）**：建 endpoint 成功但建 provider 失败 → `endpoints::delete` 回滚 endpoint，避免孤儿端点。回滚失败不掩盖原错误（拼进 error message）。
4. **幂等追溯**：导入的 provider 在 `meta` 记 `{"imported_from":"<源>","original_id":"<源id>",...}`；detect/import 按 `meta.original_id` 匹配本地已有项 → `already_imported`，二次导入跳过（`skipped, reason="已导入过"`）。
5. **冲突重命名**：与本地同名 provider 冲突时加后缀保留两者（`原名 (ccs)` → `原名 (ccs 2)` 递增），不覆盖、不跳过。算法：`resolve_unique_name(desired, existing_names)`，`existing_names` 含本地 name ∪ 本次已确定名字（避免批量内部撞名）。
6. **不激活**：导入只创建（`is_current` 恒 0），用户导入后自行切换。

### 加密复用

service 层内联 `crypto.encrypt(json!({"api_key": k}), endpoint_id.as_bytes())`，与 `http/api/mod.rs::encrypt_api_key` 逻辑一致（AAD = endpoint.id）。不复用 HTTP 层 `pub(crate)` 函数以解耦 AppState。crypto 不可用（Keychain 降级）且含 api_key → 该项 `errors` 不建 endpoint；无 api_key 时仍可导入。

### 多数据源屏蔽

外部工具有多种存储格式时，用统一类型 `CcsSourceProvider { id, name, settings_config: Value, website_url, category }` 屏蔽差异，`read_ccs_providers()` 先探新源（SQLite）再探旧源（config.json），后续 extract_env/比对/落库逻辑不感知数据源。SQLite 用 `OpenFlags::SQLITE_OPEN_READ_ONLY` 只读打开，避免锁冲突和误写。

### HTTP 路由注册顺序

导入端点是固定段（如 `/import-ccs/detect`、`/import-ccs`），**必须先于 `/{id}` 参数路由注册**，否则被参数路由吞掉（参考现有 `/reorder` 先例）。

### 测试要求（已在 `services::importers::ccs::tests`）

- detect：文件不存在/正常/空 env/冲突/已导入/多数据源优先级。
- import：正常/冲突重命名/空 env 跳过/二次导入幂等/crypto 不可用含 key 报错/crypto 不可用无 key 仍导入/missing id 跳过/批量内部撞名/不激活/不改源数据。
- resolve_unique_name：无冲突/首后缀/递增。
- SQLite 测试用 tempdir + `Connection::open` 建临时 db + 插 mock 行；config.json 测试注入不存在的 sqlite_path 避免命中本机真实库。

---

## 路径隔离契约

```text
GET  /health          健康检查
GET  /                Web UI 入口（静态资源 + SPA fallback）
ANY  /api/tools/{*path}   工具接管管理 API（已实现）
ANY  /api/{*path}     其余管理 API（accounts / endpoints / models / settings / tests …）
ANY  /claude-code/{*path}  Claude Code 路由（已实现，走 RouteProxy）
ANY  /codex/{*path}        Codex 路由（已实现，走 RouteProxy）
ANY  /v1/{*path}           OpenAI-compatible 路由（已实现，按子路径解析 capability）
```

仍未实现的入口返回统一 JSON（HTTP 501）：

```json
{
  "error": {
    "type": "not_implemented",
    "code": "scope_not_ready",
    "message": "该入口已预留，但当前子任务尚未实现具体功能。",
    "scope": "v1"
  }
}
```

---

## 安全边界

- 第一版本地服务不做 token/session 认证，仅绑定 `127.0.0.1`。
- `127.0.0.1` 不等于本机完全安全；本机其他进程理论上可访问。
- 在 Axum middleware 层预留后续本地认证扩展点。
- CORS 不是安全边界。
- 不记录请求正文、prompt、headers、API Key 或 OAuth token。

---

**相关**：[项目约定](./project-conventions.md)、Trellis 运行时规范 `.trellis/spec/backend/index.md`、agent 平台层规范 `.trellis/spec/frontend/index.md`。
