# 应用骨架与本地服务 — 技术设计

> 本文件为回溯设计文档：应用骨架在父任务规划期间即与各子任务一起落地实现，此处把已落地的结构、契约、数据流与取舍固化下来，作为本子任务的最终设计依据，并为后续子任务提供承载基础。代码现状与本文一致；若后续发现偏差，以代码为准并回写本文件。

## 1. 范围与边界

本子任务只负责应用骨架与本地服务基础，不实现任何业务功能：

- 初始化 Tauri v2 + Rust + React + TypeScript + Vite 应用骨架。
- 在 Tauri 应用生命周期内启动本地 HTTP 服务，固定绑定 `127.0.0.1:42567`。
- 提供 `/health` 健康检查。
- 建立路径隔离骨架，未实现入口返回可解释占位错误（501），而非无意义 404。
- Web UI 提供覆盖 8 个中文页面的导航骨架。
- 应用关闭（窗口关闭）时本地 HTTP 服务随进程停止。
- 预留 SQLite 初始化入口，本子任务只完成连接与迁移框架的最小闭环；具体账号/端点/凭据表结构由后续子任务细化（实际仓库中这些表已随后续子任务落库，但本子任务的设计边界止于"迁移框架可执行"）。

明确不纳入：账号/端点/凭据完整模型与 UI、Claude Code/Codex 真实路由、`/v1` 真实转发、模型刷新/alias、自动接管、链路测试、导入导出。

## 2. 技术栈与依赖

| 层 | 技术 | 版本/约定 | 来源 |
|----|------|------------|------|
| 桌面壳 | Tauri v2 | `tauri = "2"`，含 shell/dialog/fs 插件 | 参考 `ccs` 桌面形态 |
| 前端 | React 19 + TypeScript 6 + Vite 8 | `react@^19`、`vite@^8` | 参考 `ccs`，Tauri Vite 集成 |
| 样式/组件 | Tailwind CSS v4 + shadcn/ui + Radix UI（后续添加） | `@tailwindcss/vite`，无 config 文件 | 参考 `ccs`、`9router` |
| 前端请求状态 | TanStack Query 5 | `@tanstack/react-query@^5` | 参考 `ccs` |
| 前端路由 | React Router 7 | `react-router-dom@^7` | 轻量独立选择 |
| 本地 HTTP 服务 | Axum 0.8 + Tokio + tower-http | `axum@0.8`、`tower-http@0.6`（trace/cors/fs） | Rust/Tauri 约束下成熟异步 Web 生态 |
| 数据库 | SQLite + rusqlite（bundled） | `rusqlite@0.32`，WAL + foreign_keys | 参考 `ccs` 本地存储 |
| 日志 | tracing + tracing-subscriber | env-filter | Rust/Axum 常规生态 |

依赖选型依据见 `prd.md` 待澄清问题的已确认决策：Axum/Tokio 适合后续异步代理、超时、流式、SSE、日志中间件、本地认证扩展与 `/v1` 多端点转发；代价是需明确区分 Tauri command、HTTP handler、应用服务层，避免生命周期混乱。

## 3. 进程与生命周期模型

### 3.1 单进程单端口

- 单进程：Tauri 主进程同时承载 WebView 与 Rust 后端；本地 HTTP 服务在该进程的 Tokio runtime 内运行，不另起子进程。
- 单端口：本地 HTTP 服务固定绑定 `127.0.0.1:42567`。绑定失败直接启动失败，**不自动寻找下一个可用端口**——服务地址被 Claude Code / Codex / OpenCode 配置和自动接管依赖，端口漂移会破坏工具接入。

### 3.2 启动顺序（`src-tauri/src/lib.rs::run`）

```
tracing 初始化
  → tauri::Builder.setup
      → 解析数据目录（app_data_dir，不存在则 create_dir_all）
      → open_db（WAL + foreign_keys）
      → run_migrations（schema_migrations + 各迁移）
      → 初始化 crypto（Keychain 可用时）
      → 初始化 codex_oauth / model_sync / translator registry / route_proxy
      → oneshot::channel::<()>() 创建 shutdown 通道
      → AppState 组装并 manage()
      → spawn 异步任务：若自动刷新开启则启动时刷新模型（后续子任务能力，骨架已接线）
      → spawn 异步任务：http::start_server(state, shutdown_rx)
      → on_window_event(CloseRequested) → spawn 取出 shutdown_tx 并 send(())
  → invoke_handler(get_app_info)
  → run(generate_context!())
```

`AppState.shutdown_tx` 用 `tokio::sync::Mutex<Option<oneshot::Sender<()>>>` 包裹，确保多次关闭事件只触发一次关闭信号。

### 3.3 关闭策略

- 第一版不做系统托盘和后台常驻：窗口打开时本地服务运行，关闭窗口即停止服务。
- `WindowEvent::CloseRequested` 触发 `shutdown_tx.send(())`；`http::start_server` 的 `with_graceful_shutdown` 等待该信号后退出 `axum::serve`。
- 架构上预留后续托盘/后台常驻扩展：关闭信号集中在 `shutdown_tx`，未来改为"最小化到托盘时不发信号"即可。

## 4. 路径隔离路由（`src-tauri/src/http/router.rs`）

```
GET  /health                        health::health_check
ANY  /api/accounts/*                api::accounts（后续子任务实现，骨架已挂载）
ANY  /api/endpoints/*               api::endpoints
ANY  /api/auth/*                    api::auth
ANY  /api/models/*                  api::models
ANY  /api/models/aliases/*          api::aliases
ANY  /api/settings/*                api::settings
ANY  /api/tools/*                   api::tools
POST /api/tests                     api::tests
ANY  /api/{*path}                   placeholders::not_implemented  ← 其余 /api/* 占位
ANY  /claude-code/{*path}           claude_code_proxy（后续子任务实现，骨架占位）
ANY  /codex/{*path}                 codex_proxy
ANY  /v1/{*path}                    v1_handler
fallback_service                    ServeDir(web_dist_dir).fallback(ServeFile(index.html))  ← Web UI + SPA
```

约定要点：
- Axum 0.8 通配路由用 `/{*path}` 风格，旧式 `/*path` 不适用。
- 管理 API 子路由按顺序 `nest`，**在 `/api/{*path}` catch-all 之前**挂载，否则子路由被吞。
- `/` 返回 `index.html`，`/assets/*` 返回静态资源，`/settings` 等前端路由回退到 `index.html`（SPA）。`tower-http` 启用 `fs` feature。
- `web_dist_dir = env!("CARGO_MANIFEST_DIR")/../dist`；未来改为 Tauri resource 打包时需更新此处。
- 占位错误统一格式（`http/error.rs`，HTTP 501）：
  ```json
  {
    "error": {
      "type": "not_implemented",
      "code": "scope_not_ready",
      "message": "该入口已预留，但当前子任务尚未实现具体功能。",
      "scope": "<scope>"
    }
  }
  ```

> 现状说明：`/claude-code`、`/codex`、`/v1` 与多数 `/api/*` 子路由在后续子任务中已实现为真实功能（走 `RouteProxy` 或具体 handler），不再是占位。本子任务的设计边界止于"路由边界存在 + 未实现入口有可解释占位"，真实功能归属各自子任务。

## 5. 健康检查（`src-tauri/src/http/health.rs`）

```
GET /health
→ 200
{
  "status": "ok",
  "app": "agent-switch",
  "version": "0.1.0",
  "address": "127.0.0.1:42567",
  "database": "ok" | "error: <e>" | "lock_error: <e>"
}
```

`database` 字段通过 `SELECT 1` 探活，既确认服务运行，也确认数据库连接可用。

## 6. 数据库初始化边界（`src-tauri/src/db/`）

### 6.1 连接（`connection.rs`）

- `open_db` 打开/创建 SQLite，执行 `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;`。
- 返回 `Arc<Mutex<Connection>>`：`rusqlite::Connection` 内部含 `RefCell`，**不是 Send**，不能直接放进 Axum/Tauri 共享状态，必须用 `Arc<Mutex<>>` 包裹。

### 6.2 迁移框架（`migrations.rs`）

- `MIGRATIONS` 为有序 `&[Migration]`，新迁移追加到末尾，**已部署后不可重排或删除**，且**版本号必须单调递增**——`run_migrations` 按数组顺序执行 pending 迁移，数组顺序即执行顺序，与版本号数值无关；若一条迁移依赖另一条迁移创建的表，被依赖的迁移必须排在前面。
- 先确保 `schema_migrations` 表存在，读取已应用版本，仅执行 pending 迁移，每条成功后写 `schema_migrations (version, name, applied_at)`。
- 迁移失败 `panic`，应用无法启动（骨架阶段数据库不可用即视为致命）。
- 测试保障：`db::migrations::tests` 含 `fresh_db_runs_all_migrations_in_order`（全新内存库跑完全部迁移，断言依赖表与 ALTER 列存在）与 `migration_versions_are_ascending`（版本号单调递增），防止顺序错乱导致全新机器首次启动崩溃。
- 本子任务的设计边界：迁移框架可执行 + `schema_migrations` 表闭环 + 顺序/单调性测试保障。仓库中后续子任务已追加 v2~v6 迁移（accounts/endpoints、models/aliases、tool_takeover、route_settings/request_logs/model_locks、v1 路由与媒体日志字段），这些表结构归属各自子任务，不在本子任务设计范围内。

> 顺序修复记录：v5（`create_route_settings_request_logs_model_locks`）与 v6（`add_v1_route_and_media_log_fields`）曾被以 v6→v5 的数组顺序提交，导致 v6 对尚未由 v5 创建的 `route_settings`/`request_logs` 操作，全新数据库首次启动会 `execute_batch` 失败并 panic。本任务收尾时按依赖顺序调整为 v5→v6（版本号不变），并补单调性测试以防复发。已部署数据库 pending 为空，不受影响。

### 6.3 路径解析（`config/paths.rs`）

- `app_data_dir()` 优先用 Tauri path resolver（`app.handle().path().app_data_dir()`），resolver 不可用时回退到 `dirs::data_dir()`，再回退到 `$HOME`，最终拼 `APP_DIR_NAME`。
- `db_path(data_dir)` = `data_dir/agent-switch.db`。
- Linux: `~/.local/share`；macOS: `~/Library/Application Support`；Windows: `%APPDATA%`。

## 7. 共享状态（`src-tauri/src/app_state.rs`）

```rust
pub struct AppState {
    pub db: Arc<Mutex<Connection>>,
    pub shutdown_tx: tokio::sync::Mutex<Option<oneshot::Sender<()>>>,
    pub data_dir: PathBuf,
    pub web_dist_dir: PathBuf,
    pub crypto: Option<Arc<CryptoService>>,            // 后续子任务
    pub codex_oauth: Arc<CodexOAuthService>,           // 后续子任务
    pub model_sync: Arc<ModelSyncService>,             // 后续子任务
    pub route_proxy: tokio::sync::RwLock<Option<Arc<RouteProxy>>>,  // 后续子任务
}
```

设计原则：
- Tauri command 不直接拼 SQL、不直接构造 HTTP 响应。
- Axum handler 不直接访问 WebView 或窗口对象。
- 数据库访问集中在 DAO 层。
- `AppState` 作为 `Arc` 同时 `handle.manage()` 给 Tauri 和 `Router::with_state()` 给 Axum，两侧共享同一份状态。

> 骨架阶段实际只需 `db` / `shutdown_tx` / `data_dir` / `web_dist_dir` 四个字段；`crypto` / `codex_oauth` / `model_sync` / `route_proxy` 为后续子任务预留并在仓库中已接线。本子任务的设计边界止于前四个字段与共享机制。

## 8. 前端骨架（`src/`）

### 8.1 目录结构

```
src/
├── main.tsx                QueryClientProvider + BrowserRouter + App
├── App.tsx                 8 个中文页面路由（React Router）
├── components/layout/      AppShell（侧边栏 + 主内容区）、PagePlaceholder
├── pages/                  DashboardPage / AccountsPage / ... / SettingsPage（8 个）
├── lib/                    api.ts、utils.ts
└── styles/globals.css      @import "tailwindcss";
```

### 8.2 路由（`App.tsx`）

```
/             总览（DashboardPage）
/accounts     账号
/endpoints    端点
/models       模型
/tools        工具
/routes       路由
/logs         日志
/settings     设置
*             → Navigate to "/"
```

### 8.3 状态与构建约定

- `QueryClient` 在 `main.tsx` 应用根部创建一次：`retry:1`、`refetchOnWindowFocus:false`、`staleTime:30s`。不在组件内重复创建。
- Vite 8 构建目标 `build.target: 'esnext'`（Tauri WebView 自带现代内核，默认 `safari13` 会破坏现代解构语法）。
- `vite.config.ts` 同时加载 `react()` 与 `tailwindcss()` 插件；`@` 别名指向 `./src`。
- `clearScreen:false`、`strictPort:true`（dev server 5173），`watch.ignored` 排除 `src-tauri/**`。

### 8.4 文案约定

- 所有 UI 文案默认中文。
- 未实现页面使用中文空状态，不写假功能或假数据。

## 9. 安全边界

- 第一版本地服务**不做 token/session 认证**，仅绑定 `127.0.0.1`。
- `127.0.0.1` ≠ 本机完全安全：本机其他进程理论上可访问 `/api/*`、`/claude-code/*`、`/codex/*`、`/v1/*`，可能修改配置或消耗上游额度。
- 在 Axum middleware 层预留后续本地认证扩展点（当前未实现）。
- CORS 不是安全边界。
- 不记录请求正文、prompt、headers、API Key 或 OAuth token（骨架阶段 `/health` 与占位响应本身不含敏感信息）。

## 10. 关键陷阱（已规避）

1. `rusqlite::Connection` 不是 Send → `Arc<Mutex<Connection>>`。
2. Axum 0.8 通配路由 `/{*path}`，非 `/*path`。
3. graceful shutdown 用 `oneshot::channel`，`shutdown_tx` 存 `AppState`（`Mutex<Option<>>` 防重复），`shutdown_rx` 传 `start_server`。
4. 固定端口 `42567`，绑定失败即启动失败，不换端口。
5. 前端静态资源 `ServeDir::new(dist).fallback(ServeFile::new(dist/index.html))` 作 `fallback_service`，保证 SPA 路由回退。
6. Tailwind v4 用 `@tailwindcss/vite` 插件，无 `tailwind.config.js`，`globals.css` 内 `@import "tailwindcss";`。
7. Vite 8 `build.target: 'esnext'`。
8. Windows 构建需 `icons/icon.ico` + MSVC 工具链 + `~/.cargo/bin` 在 PATH。

## 11. 兼容性 / 回滚形态

- 骨架为纯新增，无既有业务代码需兼容。
- 回滚：删除 `src-tauri/` 与 `src/` 骨架文件并回退 `package.json`/`Cargo.toml`/`tauri.conf.json` 即可恢复到规划前状态；数据库文件位于用户数据目录，回滚代码不影响已生成数据库（再次启动会按迁移框架补齐）。

## 12. 后续子任务扩展点

- 本地访问认证：Axum middleware 层插入 token/session 校验。
- 托盘/后台常驻：`shutdown_tx` 信号源改为托盘菜单。
- 真实 `/claude-code`、`/codex`、`/v1` 路由：`RouteProxy`（已实现，归 routing-failover-core / v1-endpoints 子任务）。
- 业务表结构：迁移列表追加（已落地 v2~v6，归各业务子任务）。
- 前端 8 页面真实功能：各页面替换空状态（已陆续实现，归各业务子任务）。
