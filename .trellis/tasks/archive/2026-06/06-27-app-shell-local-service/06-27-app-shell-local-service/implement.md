# 应用骨架与本地服务 — 实现计划

> 本文件为回溯实现记录：应用骨架已落地实现并随仓库演进。此处记录已完成的执行清单、关键文件、验证命令与回滚点，用于本子任务的收尾验证与归档依据。带 ✅ 的步骤为已落地状态。

## 执行清单

### 阶段 A：Rust 后端骨架

- [x] ✅ 初始化 `src-tauri` Tauri v2 工程：`Cargo.toml`、`tauri.conf.json`、`build.rs`、`main.rs`、`lib.rs`。
- [x] ✅ 配置依赖：`tauri 2` + shell/dialog/fs 插件、`axum 0.8`、`tokio`、`tower-http`（trace/cors/fs）、`rusqlite 0.32`（bundled）、`tracing`、`serde`、`time`。
- [x] ✅ `src-tauri/src/lib.rs::run`：tracing 初始化 + `tauri::Builder::setup` 全流程。
- [x] ✅ `src-tauri/src/app_state.rs`：`AppState` 共享状态（db / shutdown_tx / data_dir / web_dist_dir + 后续子任务字段）。
- [x] ✅ `src-tauri/src/config/paths.rs`：`app_data_dir()` + `db_path()`。
- [x] ✅ `src-tauri/src/db/connection.rs`：`open_db`（WAL + foreign_keys，`Arc<Mutex<Connection>>`）。
- [x] ✅ `src-tauri/src/db/migrations.rs`：`MIGRATIONS` 有序列表 + `run_migrations`（schema_migrations 闭环）。
- [x] ✅ 迁移顺序修复：v5（`create_route_settings_request_logs_model_locks`）与 v6（`add_v1_route_and_media_log_fields`）按依赖顺序排列为 v5→v6（版本号不变）。v6 依赖 v5 创建的 `route_settings`/`request_logs` 表，原 v6→v5 顺序会导致全新数据库首次启动崩溃。
- [x] ✅ 迁移测试：`db::migrations::tests` 新增 `fresh_db_runs_all_migrations_in_order`（全新内存库跑完全部迁移）与 `migration_versions_are_ascending`（版本号单调递增断言），防止顺序再次错乱。
- [x] ✅ `src-tauri/src/http/mod.rs`：`start_server` 绑定 `127.0.0.1:42567` + graceful shutdown。
- [x] ✅ `src-tauri/src/http/router.rs`：路径隔离路由 + `fallback_service`（ServeDir + SPA fallback）。
- [x] ✅ `src-tauri/src/http/health.rs`：`GET /health`。
- [x] ✅ `src-tauri/src/http/error.rs` + `placeholders.rs`：统一 501 占位错误。
- [x] ✅ `src-tauri/src/commands/app.rs`：`get_app_info` Tauri command。
- [x] ✅ `on_window_event(CloseRequested)` → `shutdown_tx.send(())` 停止本地服务。

### 阶段 B：前端骨架

- [x] ✅ 初始化 `package.json`：React 19 + TS 6 + Vite 8 + TanStack Query 5 + React Router 7 + Tailwind v4。
- [x] ✅ `vite.config.ts`：`react()` + `tailwindcss()` 插件、`@` 别名、`build.target:'esnext'`、dev 5173 strictPort。
- [x] ✅ `src/main.tsx`：`QueryClientProvider` + `BrowserRouter` + `App`，根部单例 `QueryClient`。
- [x] ✅ `src/App.tsx`：8 个中文页面路由 + `*` 回退到 `/`。
- [x] ✅ `src/components/layout/`：`AppShell`（侧边栏 + 主内容区）、`PagePlaceholder`。
- [x] ✅ `src/pages/`：Dashboard / Accounts / Endpoints / Models / Tools / Routes / Logs / Settings 八个页面。
- [x] ✅ `src/lib/`：`api.ts`、`utils.ts`。
- [x] ✅ `src/styles/globals.css`：`@import "tailwindcss";`。

### 阶段 C：资源与构建配置

- [x] ✅ `src-tauri/icons/`：32x32 / 128x128 / 128x128@2x / icon.ico（Windows 构建必需）。
- [x] ✅ `tauri.conf.json`：`frontendDist: ../dist`、`devUrl: localhost:5173`、窗口 1200x800、`beforeDevCommand/beforeBuildCommand`。
- [x] ✅ `index.html`：前端入口。

### 阶段 D：验证（收尾阶段执行）

- [ ] 前端类型检查与构建：`npm run build`（`tsc --noEmit && vite build`）。
- [ ] Rust 编译检查：`cargo check --manifest-path src-tauri/Cargo.toml`（或 `cargo build`）。
- [ ] 骨架验收项确认（见下方验收核对）。

## 关键文件

| 文件 | 作用 |
|------|------|
| `src-tauri/src/lib.rs` | Tauri Builder、setup、生命周期、服务启停 |
| `src-tauri/src/app_state.rs` | AppState 共享状态 |
| `src-tauri/src/config/paths.rs` | 数据目录 / 数据库路径解析 |
| `src-tauri/src/db/connection.rs` | SQLite 连接（WAL + Arc Mutex） |
| `src-tauri/src/db/migrations.rs` | 迁移框架（schema_migrations 闭环） |
| `src-tauri/src/http/mod.rs` | Axum 服务启动 + graceful shutdown |
| `src-tauri/src/http/router.rs` | 路径隔离路由 + 静态资源 fallback |
| `src-tauri/src/http/health.rs` | `/health` |
| `src-tauri/src/http/error.rs` | 统一占位错误格式 |
| `src-tauri/src/http/placeholders.rs` | 未实现入口 501 占位 |
| `src-tauri/src/commands/app.rs` | Tauri command |
| `src/main.tsx` | 前端入口 + QueryClient + Router |
| `src/App.tsx` | 8 页面路由 |
| `src/components/layout/AppShell.tsx` | 侧边栏 + 主内容区骨架 |
| `vite.config.ts` | Vite + React + Tailwind 配置 |
| `src-tauri/tauri.conf.json` | Tauri 应用配置 |

## 验证命令

```bash
# 前端：类型检查 + 构建（产物到 dist/）
npm run build

# Rust：编译检查
cargo check --manifest-path src-tauri/Cargo.toml

# Rust：迁移测试（验证全新数据库迁移顺序 + 版本号单调递增）
cargo test --manifest-path src-tauri/Cargo.toml db::migrations

# Rust：全部测试
cargo test --manifest-path src-tauri/Cargo.toml

# （可选，需桌面环境）启动 Tauri 开发模式，确认窗口可打开、/health 可访问
#   npm run tauri dev
#   curl http://127.0.0.1:42567/health
#   curl -i http://127.0.0.1:42567/api/unknown/path   # 应返回 501 占位 JSON
```

> 注：`cargo build` / `npm run tauri dev` 需要完整桌面与工具链环境（MSVC / WebView）。在当前 WSL 无 GUI 环境下，以 `cargo check` + `npm run build` 作为可执行验证下限；启动与 `/health` 实测由用户在有桌面环境时执行。

## 验收核对（对照 prd.md 验收标准）

- [ ] 可以启动 Tauri 桌面应用 → `lib.rs` setup 完整，需桌面环境实测。
- [ ] Web UI 使用 React+TS+Vite，显示中文基础导航 → `App.tsx` + `AppShell` + 8 页面，`npm run build` 通过即佐证。
- [ ] 本地 HTTP 服务绑定 `127.0.0.1:42567` → `http/mod.rs:23` 固定绑定。
- [ ] `GET /health` 返回成功状态 + 版本/运行信息 → `health.rs` 返回 status/app/version/address/database。
- [ ] `/`、`/api/*`、`/claude-code/*`、`/codex/*`、`/v1/*` 路由边界存在，未实现入口返回可解释占位 → `router.rs` + `placeholders.rs`（501 JSON）。
- [ ] 应用关闭后本地服务停止 → `on_window_event(CloseRequested)` → `shutdown_tx` → graceful shutdown。
- [ ] 设计文档明确单进程单端口 / Tauri 生命周期 / 启停策略 / SQLite 边界 / 扩展点 → `design.md`。
- [ ] 实现计划列出初始化命令 / 关键文件 / 验证命令 / 回滚点 → 本文件。

## 回滚点

1. **前端构建失败**：回退 `src/` 与 `vite.config.ts`/`package.json`，重新 `npm install && npm run build`。不影响 Rust。
2. **Rust 编译失败**：回退 `src-tauri/src/` 对应文件，`cargo check` 复核。数据库文件位于用户数据目录，回滚代码不删除已生成数据库。
3. **启动/绑定失败（端口 42567 被占）**：按设计**不自动换端口**；排查占用进程后重启，不修改端口常量。
4. **迁移失败**：按设计迁移失败即 `panic` 阻止启动；修复迁移 SQL 后重启，迁移框架会跳过已应用版本只补 pending。

## 后续子任务衔接

- 本子任务只交付骨架；账号/端点/模型/工具/路由/v1/测试/导入导出归各自子任务，已在仓库中陆续实现。
- 收尾阶段（3.3）需确认 `spec/guides/app-stack-conventions.md` 已覆盖骨架约定（当前已覆盖，无需新增 spec）。
