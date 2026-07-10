# 应用骨架与本地服务实现计划

## 前置条件

- 当前任务：`.trellis/tasks/06-27-app-shell-local-service`
- 父任务：`.trellis/tasks/06-26-agent-switch-web-router-mvp`
- 不开始实现，直到用户 review 当前 `prd.md` / `design.md` / `implement.md` 并明确同意进入实现。

## 实现顺序

### 1. 初始化 Tauri + React + TypeScript + Vite

目标：创建可启动的桌面应用骨架。

建议命令：

```bash
npm create tauri-app@latest . -- --template react-ts
```

如果模板命令不适合非空仓库，则采用临时目录生成后复制必要文件的方式，避免覆盖 `.trellis/`、`.gitignore` 和现有配置。

关键文件：

- `package.json`
- `vite.config.ts`
- `tsconfig.json`
- `src-tauri/Cargo.toml`
- `src-tauri/tauri.conf.json`
- `src-tauri/src/main.rs`
- `src-tauri/src/lib.rs`
- `src/main.tsx`
- `src/App.tsx`

### 2. 配置 Tailwind CSS + shadcn/ui + Radix UI

目标：建立中文管理台的样式和组件基础。

建议命令：

```bash
npm install tailwindcss @tailwindcss/vite
npx shadcn@latest init
npx shadcn@latest add button card dialog dropdown-menu tabs toast sheet separator scroll-area badge input label switch
```

按 shadcn/ui Vite 文档配置：

- `vite.config.ts` 增加 Tailwind 插件和 `@` alias；
- `src/index.css` 或 `src/styles/globals.css` 引入 Tailwind；
- 建立 `src/components/ui/`；
- 建立 `src/lib/utils.ts`。

### 3. 建立前端路由和中文布局

目标：创建 8 个中文页面入口。

关键文件：

- `src/App.tsx`
- `src/routes/index.tsx`
- `src/routes/pages/*.tsx`
- `src/components/layout/AppShell.tsx`
- `src/components/layout/Sidebar.tsx`
- `src/components/layout/Topbar.tsx`

页面：

- 总览
- 账号
- 端点
- 模型
- 工具
- 路由
- 日志
- 设置

每个尚未实现页面使用中文空状态，不写假功能。

### 4. 增加 TanStack Query 基础 Provider

目标：为后续管理 API 请求提供统一缓存/错误状态基础。

建议依赖：

```bash
npm install @tanstack/react-query react-router-dom
```

关键文件：

- `src/lib/query-client.ts`
- `src/lib/api.ts`
- `src/main.tsx`

本子任务不需要实现复杂 API 客户端，只需预留基础结构。

### 5. 增加 Rust 后端模块结构

目标：区分 Tauri、HTTP、数据库、配置路径和命令层。

关键目录：

```text
src-tauri/src/
├── app_state.rs
├── commands/
├── config/
├── db/
└── http/
```

### 6. 实现 SQLite 初始化与迁移框架

目标：启动时创建数据库并初始化 `schema_migrations`。

建议依赖：

```toml
rusqlite = { version = "0.32", features = ["bundled"] }
time = { version = "0.3", features = ["formatting"] }
```

实现点：

- 从 Tauri app handle 获取 app data dir；
- 创建数据目录；
- 打开 `agent-switch.db`；
- 创建 `schema_migrations`；
- 实现迁移列表和事务执行框架；
- 迁移失败则启动失败。

### 7. 实现 Axum 本地 HTTP 服务

目标：启动 `127.0.0.1:42567` 服务。

建议依赖：

```toml
axum = "0.8"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "signal"] }
tower-http = { version = "0.6", features = ["trace", "fs", "cors"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

实现点：

- `GET /health`；
- `/` 静态资源入口；
- `/api/*`、`/claude-code/*`、`/codex/*`、`/v1/*` 占位 handler；
- 统一 JSON 错误；
- 端口冲突时返回启动错误，不自动换端口；
- graceful shutdown。

### 8. 接入 Tauri 生命周期

目标：应用启动时启动服务，关闭时停止服务。

实现点：

- 在 `tauri::Builder::setup` 中初始化 AppState；
- 启动 Axum server task；
- 保存 shutdown sender / join handle；
- 在退出事件中触发 shutdown；
- 错误显示给 UI 或启动失败日志。

### 9. 验证

建议命令：

```bash
npm install
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

运行期验证：

```bash
curl http://127.0.0.1:42567/health
curl http://127.0.0.1:42567/api/placeholder
curl http://127.0.0.1:42567/claude-code/placeholder
curl http://127.0.0.1:42567/codex/placeholder
curl http://127.0.0.1:42567/v1/placeholder
```

预期：

- `/health` 返回 `status: ok`；
- 占位入口返回统一 `not_implemented` JSON；
- 关闭应用后 `42567` 不再监听。

### 10. 文档和上下文

实现后需要：

- 更新 `README` 或开发说明（如本轮范围内创建）；
- 如发现新的工程约定，更新 `.trellis/spec/`；
- 根据实际实现补充 `implement.jsonl` / `check.jsonl` 的上下文条目。

## 风险与回滚点

### 非空仓库初始化风险

风险：Tauri 模板可能覆盖已有文件。

处理：

- 初始化前检查目标文件；
- 必要时在临时目录生成模板后复制；
- 不覆盖 `.trellis/`、`.claude/`、`.gitignore`、`AGENTS.md`。

### 端口占用

风险：`42567` 被其他进程占用导致应用无法启动服务。

处理：

- 不自动换端口；
- 返回中文可解释错误；
- 保持父任务固定服务地址约束。

### Tauri WebView 与 Axum `/` 静态资源重复

风险：开发模式和生产模式路径不一致。

处理：

- 开发模式优先保证 Tauri + Vite HMR；
- 生产模式保证浏览器访问 `http://127.0.0.1:42567/` 可打开 UI 或等价入口；
- 在设计/代码中明确静态资源来源。

### SQLite 迁移失败

风险：数据库处于半初始化状态。

处理：

- 迁移在事务内执行；
- 失败则启动失败；
- 本子任务只做最小 schema，降低风险。

## 完成标准

- `prd.md`、`design.md`、`implement.md` 已完成并经用户 review。
- 用户明确同意进入实现后，才能运行 `task.py start`。
- 实现完成后必须运行构建/检查/运行期验证，并报告真实结果。
