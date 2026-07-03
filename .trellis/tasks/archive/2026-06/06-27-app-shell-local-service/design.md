# 应用骨架与本地服务设计

## 设计目标

本子任务只建立 agent-switch 的可运行应用底座，不实现后续业务能力。完成后应具备：

- 一个可启动的 Tauri v2 桌面应用；
- 一个随应用生命周期启动/停止的 Rust Axum 本地 HTTP 服务；
- 固定监听 `127.0.0.1:42567`；
- 一个中文 React 管理台骨架；
- 路径隔离与统一占位错误；
- SQLite 初始化与迁移框架。

## 总体架构

```text
agent-switch 进程
├── Tauri Runtime
│   ├── WebView：加载前端 UI
│   ├── Tauri commands：面向桌面能力的 IPC 扩展点
│   └── AppState：持有数据库、服务句柄、配置路径等共享状态
├── Axum HTTP Server（127.0.0.1:42567）
│   ├── /                         Web UI 静态资源入口
│   ├── /health                   健康检查
│   ├── /api/*                    管理 API 预留
│   ├── /claude-code/*            Claude Code 路由预留
│   ├── /codex/*                  Codex 路由预留
│   └── /v1/*                     OpenAI-compatible 预留
├── SQLite
│   ├── agent-switch.db
│   └── schema_migrations
└── 前端资源
    ├── React + TypeScript + Vite
    ├── Tailwind CSS
    └── shadcn/ui + Radix UI 组件目录
```

## 技术栈与来源

| 领域 | 决策 | 来源 |
|------|------|------|
| 桌面壳 | Tauri v2 | 父任务确认，参考 `ccs` 桌面应用形态 |
| 前端 | React + TypeScript + Vite | 参考 `ccs`，结合 Tauri 官方 Vite 集成 |
| 样式组件 | Tailwind CSS + shadcn/ui + Radix UI | 参考 `ccs` 与 `9router` 的 Tailwind 管理台经验 |
| 本地服务 | Axum + Tokio + tower-http | agent-switch Rust/Tauri 约束下的独立最优解 |
| 数据库 | SQLite + rusqlite | 参考 `ccs` 的 Tauri/Rust 本地存储方案 |
| 前端请求状态 | TanStack Query | 参考 `ccs` 前端缓存/同步分层 |
| 页面路由 | React Router | agent-switch 独立轻量方案 |
| 日志基础 | tracing + tracing-subscriber + tower-http TraceLayer | Rust/Axum 常规方案，为后续摘要日志预留 |

## Rust 后端边界

建议 Rust 后端分层如下：

```text
src-tauri/src/
├── lib.rs                      Tauri Builder、setup、managed state
├── main.rs                     桌面入口
├── app_state.rs                AppState 定义
├── http/
│   ├── mod.rs                  HTTP server 启停入口
│   ├── router.rs               Axum Router 组装
│   ├── error.rs                统一错误响应
│   ├── health.rs               /health
│   ├── static_ui.rs            / 静态资源入口
│   └── placeholders.rs         /api、/claude-code、/codex、/v1 占位
├── db/
│   ├── mod.rs                  数据库初始化入口
│   ├── connection.rs           SQLite 连接和路径定位
│   ├── migrations.rs           schema_migrations 与迁移执行
│   └── dao/
│       ├── mod.rs              DAO 模块聚合
│       └── settings.rs         可选：最小设置 DAO 占位
├── commands/
│   ├── mod.rs                  Tauri command 聚合
│   └── app.rs                  基础应用信息 command
└── config/
    ├── mod.rs
    └── paths.rs                应用数据目录、数据库路径、日志路径
```

设计原则：

- Tauri command 不直接拼 SQL，不直接构造 HTTP 响应。
- Axum handler 不直接访问 WebView 或窗口对象。
- 数据库访问集中在 DAO/Repository 层。
- 后续账号、端点、模型、路由、日志等子任务在现有分层内增加模块，不重写骨架。

## Tauri 生命周期

应用启动流程：

1. Tauri `setup` 阶段初始化日志。
2. 定位应用数据目录。
3. 初始化 SQLite：创建数据库、执行迁移框架。
4. 构造共享 `AppState`。
5. 启动 Axum 本地服务任务。
6. 打开主窗口并加载前端。

应用关闭流程：

1. Tauri 收到退出/窗口关闭事件。
2. 触发 HTTP server graceful shutdown 信号。
3. 等待 Axum 任务退出或超时。
4. 释放数据库连接。
5. 进程退出。

第一版不做托盘和后台常驻；关闭窗口即停止本地服务。

## 本地 HTTP 服务设计

### 监听地址

固定绑定：

```text
127.0.0.1:42567
```

端口冲突策略：

- 不自动换端口；
- 返回启动错误；
- UI 显示中文提示：端口被占用，请关闭占用程序或稍后重试；
- 日志记录绑定失败原因。

原因：服务地址会被 Claude Code、Codex、OpenCode 手动配置和自动接管依赖，自动换端口会破坏已确认产品约束。

### 路由结构

```text
GET  /health          健康检查
GET  /                Web UI 入口
ANY  /api/*           管理 API 预留
ANY  /claude-code/*   Claude Code 路由预留
ANY  /codex/*         Codex 路由预留
ANY  /v1/*            OpenAI-compatible 预留
```

未实现作用域返回统一 JSON：

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

HTTP 状态建议使用 `501 Not Implemented`。

### `/health` 响应契约

```json
{
  "status": "ok",
  "app": "agent-switch",
  "version": "0.1.0",
  "address": "127.0.0.1:42567",
  "database": "ok"
}
```

如数据库初始化失败，服务不应进入“假健康”。本子任务中可选择启动失败或 `/health` 返回 `database: "error"`；推荐启动失败，避免后续子任务在坏状态上运行。

## 前端设计

### 页面结构

```text
src/
├── main.tsx
├── App.tsx
├── routes/
│   ├── index.tsx
│   └── pages/
│       ├── DashboardPage.tsx       总览
│       ├── AccountsPage.tsx        账号
│       ├── EndpointsPage.tsx       端点
│       ├── ModelsPage.tsx          模型
│       ├── ToolsPage.tsx           工具
│       ├── RoutesPage.tsx          路由
│       ├── LogsPage.tsx            日志
│       └── SettingsPage.tsx        设置
├── components/
│   ├── layout/
│   │   ├── AppShell.tsx
│   │   ├── Sidebar.tsx
│   │   └── Topbar.tsx
│   └── ui/                         shadcn/ui 组件
├── lib/
│   ├── api.ts                      管理 API 客户端占位
│   └── utils.ts                    shadcn cn 工具
└── styles/
    └── globals.css 或 index.css
```

### 中文 UI

第一版所有导航、标题、空状态和错误提示默认中文。页面内容可先使用空状态，例如：

```text
账号管理将在后续子任务中实现。
```

### 状态管理

- TanStack Query Provider 放在应用根部。
- 本子任务只建立 QueryClient，不实现复杂请求缓存。
- 后续管理 API 子任务复用同一请求层。

## SQLite 初始化与迁移

### 存储位置

数据库路径通过 Tauri app data dir 决定，例如：

```text
<app_data_dir>/agent-switch.db
```

不要把数据库放在仓库目录内。

### 最小 schema

```sql
CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  applied_at TEXT NOT NULL
);
```

本子任务可添加 `app_metadata` 或 `settings` 占位表，但不应抢先定义账号、端点、模型等业务表。

### 迁移策略

- 使用 Rust 代码维护迁移列表。
- 启动时在事务内按版本顺序执行未应用迁移。
- 迁移失败则启动失败。
- 后续涉及真实业务表和加密字段时，再补充迁移前备份策略。

## 安全边界

父任务已选择第一版本地服务不做 token/session 认证，仅绑定 `127.0.0.1`。

本设计必须明确：

- `127.0.0.1` 不等于本机完全安全；
- 本机其他进程理论上可访问管理 API 和代理入口；
- 本子任务在 Axum middleware 层预留本地认证扩展点；
- CORS 不是安全边界；
- 本子任务不记录请求正文、prompt、headers、API Key 或 OAuth token。

## 开发模式与生产模式

### 开发模式

- Vite dev server 负责前端热更新。
- Tauri 使用 Vite dev URL。
- Axum 仍启动 `127.0.0.1:42567`，用于 `/health` 和路径隔离验证。

### 生产模式

- 前端构建为静态资源。
- Tauri 打包 WebView 资源。
- Axum 在 `/` 提供 Web UI 入口，满足浏览器访问本地管理台语义。

如果 Tauri WebView 打包资源与 Axum 静态资源存在重复，优先保证：

1. Tauri 桌面窗口能打开 UI；
2. 浏览器访问 `http://127.0.0.1:42567/` 能打开同一 UI 或等价入口。

## 后续子任务扩展点

- `accounts-endpoints-credential-security`：在 `db/dao/`、`commands/`、`http/api` 中增加账号端点与凭据接口。
- `model-management-refresh-alias`：增加模型、alias、能力类型表和 API。
- `tool-takeover-claude-code-codex`：增加工具配置文件写入服务，不改变 HTTP 骨架。
- `routing-failover-core`：替换 `/claude-code/*`、`/codex/*` 占位 handler。
- `openai-compatible-v1-endpoints`：替换 `/v1/*` 占位 handler。
- `chain-testing-debugger`：复用 Axum streaming 与前端调试页面结构。
- `import-export-settings`：复用 SQLite 路径、配置路径和设置页面。

## 重要取舍

- 选择固定端口而不是自动换端口，牺牲启动容错，换取工具配置稳定性。
- 选择 rusqlite 而不是 ORM，牺牲部分类型自动化，换取本地 SQLite 控制力。
- 选择 Axum 而不是极简 HTTP 库，增加初始依赖，换取后续流式代理和中间件扩展能力。
- 选择 Tailwind + shadcn/ui，增加组件维护责任，换取桌面管理台长期可定制性。
