# 应用骨架与本地服务

## 目标

建立 agent-switch 第一版的应用基础：Tauri 桌面壳、Rust 本地服务、Web 前端骨架、单进程单端口服务、路径隔离、SQLite 初始化与健康检查，为后续账号端点、模型管理、工具接管、路由、测试与导入导出等子任务提供承载基础。

## 父任务约束

父任务：`.trellis/tasks/06-26-agent-switch-web-router-mvp`

本子任务必须遵守父任务已确认的跨模块约束：

- 第一版采用 Tauri + Rust + Web 前端。
- 服务地址固定为 `http://127.0.0.1:42567`。
- 第一版采用单体应用、单进程、单端口、路径隔离。
- 路径隔离目标：
  - `/`：Web UI；
  - `/api/*`：管理 API；
  - `/claude-code/*`：Claude Code 路由；
  - `/codex/*`：Codex 路由；
  - `/v1/*`：OpenAI-compatible 入口；
  - `/health`：健康检查。
- 第一版本地服务默认不做本地 token/session 认证，仅绑定 `127.0.0.1`；设计文档必须明确该选择的本机进程访问风险，并预留后续本地认证扩展点。
- 第一版不做系统托盘和后台常驻；窗口打开时本地服务运行，关闭窗口即停止服务，架构上预留后续托盘/后台常驻扩展。
- 所有项目文档和 UI 默认使用中文。

## 已确认技术决策

- 前端技术栈采用 **React + TypeScript + Vite**。
  - 参考来源：主要参考 `ccs` 的 Tauri 桌面应用形态，并结合 Tauri 社区常见 React/Vite 模板。
  - 取舍：React + TypeScript 更适合 agent-switch 后续复杂管理台，包括 8 个页面、表单、模型映射、调试器和日志表格；代价是前端依赖和构建链更重，需要在设计中约束组件边界。
- Rust 本地 HTTP 服务框架采用 **Axum + Tokio + tower-http**。
  - 参考来源：agent-switch 独立工程方案；四个参考项目并不都采用 Rust/Tauri 本地 HTTP 服务，因此这里按已确认的 Tauri + Rust 技术栈选择成熟 Rust 异步 Web 生态。
  - 取舍：Axum/Tokio 更适合后续异步代理、超时、流式响应、SSE/chunked body、日志中间件、本地认证扩展和 `/v1` 多端点真实转发；代价是初始结构要明确区分 Tauri command、HTTP handler 和应用服务层。
- Web UI 组件库/样式方案采用 **Tailwind CSS + shadcn/ui + Radix UI**。
  - 参考来源：主要参考 `ccs` 的 React + Tailwind + shadcn/ui 桌面管理台形态，同时吸收 `9router` 的 Tailwind Web 管理体验。
  - 取舍：该方案适合中文桌面管理台、表单、弹窗、开关、Tabs、Toast、深色模式和长期定制；代价是 shadcn/ui 组件进入项目后需要自行维护，复杂表格后续可能需要 TanStack Table 等补充库。
- SQLite 访问与迁移方案采用 **rusqlite + 自维护 SQL 迁移 + DAO/Repository 层**。
  - 参考来源：主要参考 `ccs` 的 Tauri/Rust + SQLite + rusqlite + DAO 分层做法。
  - 取舍：该方案适合本地 SQLite 桌面应用，依赖少、行为透明，便于后续加密字段、备份和导入导出；代价是需要自行维护 SQL、迁移版本和 DAO 代码。
- 前端请求状态管理采用 **TanStack Query**。
  - 参考来源：参考 `ccs` 的前端状态/缓存分层做法。
  - 取舍：适合管理 API 请求、缓存、刷新、错误状态和后续多页面复用；本子任务只建立基础 Provider，不展开复杂缓存策略。
- 前端页面路由采用 **React Router**。
  - 参考来源：agent-switch 独立工程选择。
  - 取舍：足以承载第一版 8 个中文页面；比引入全栈框架更轻。
- 包管理器默认采用 **npm**。
  - 参考来源：Tauri/shadcn 文档示例与通用环境兼容性。
  - 取舍：降低初始环境要求；后续若需要 monorepo 性能优化，可再评估 pnpm。
- 日志基础采用 **tracing + tracing-subscriber + tower-http TraceLayer**。
  - 参考来源：Rust/Axum 常规生态，并为后续 `sub2api` 风格摘要日志预留基础。
  - 取舍：本子任务只记录启动、停止、端口绑定和占位路由级别日志，不记录请求正文。

## 需求

- 初始化 Tauri v2 + Rust + React + TypeScript + Vite 应用骨架。
- Rust 后端在 Tauri 应用生命周期内启动本地 Axum HTTP 服务。
- 本地 HTTP 服务必须绑定 `127.0.0.1:42567`。
- 端口被占用时不得自动切换端口；应启动失败并给出中文可解释错误，因为自动换端口会破坏固定服务地址和后续工具接管配置。
- 服务必须提供 `/health`，用于确认本地服务运行状态。
- 服务必须建立路径隔离骨架，即使后续子任务尚未实现，也应有可解释的占位响应或路由边界。
- Tauri WebView 可加载打包前端资源；Axum 也需要在 `/` 提供 Web UI 静态资源入口，以满足浏览器访问 `http://127.0.0.1:42567/` 的产品语义。开发模式可使用 Vite dev server，但生产语义必须收敛到单端口。
- Web UI 至少提供中文导航骨架，覆盖父任务确认的 8 个页面入口：总览、账号、端点、模型、工具、路由、日志、设置。
- 应用关闭时，本地 HTTP 服务应随进程停止。
- 应用骨架应完成 SQLite 最小闭环：应用数据目录定位、数据库文件创建、`schema_migrations` 表、迁移执行框架和 DAO/Repository 目录结构。
- 本子任务不实现完整业务表；账号、端点、凭据、模型、日志等表结构由后续子任务细化。
- 未实现的 `/api/*`、`/claude-code/*`、`/codex/*`、`/v1/*` 入口应返回统一 JSON 占位错误，不返回无意义 404。

## 验收标准

- [ ] 可以启动 Tauri 桌面应用。
- [ ] Web UI 使用 React + TypeScript + Vite，并显示中文基础导航。
- [ ] UI 样式基础使用 Tailwind CSS，基础组件目录兼容 shadcn/ui / Radix UI。
- [ ] 本地 HTTP 服务绑定 `127.0.0.1:42567`。
- [ ] 端口 `42567` 被占用时，应用给出可解释错误且不自动切换端口。
- [ ] `GET http://127.0.0.1:42567/health` 返回成功状态和基础版本/运行信息。
- [ ] `GET http://127.0.0.1:42567/` 在生产语义下能提供 Web UI 入口。
- [ ] `/api/*`、`/claude-code/*`、`/codex/*`、`/v1/*` 的路由边界存在，未实现功能返回统一可解释 JSON 占位错误。
- [ ] 应用关闭后本地服务停止。
- [ ] SQLite 数据库文件可创建，`schema_migrations` 初始化成功，迁移框架可运行。
- [ ] 设计文档明确单进程单端口结构、Tauri 生命周期、本地服务启动/停止策略、SQLite 初始化边界和后续子任务扩展点。
- [ ] 实现计划列出初始化命令、关键文件、验证命令和回滚点。

## 暂不纳入本子任务

- 账号/端点/凭据安全的完整数据模型与 UI。
- Claude Code / Codex 的真实路由实现。
- OpenAI-compatible `/v1` 多端点真实转发。
- 模型刷新、alias、能力类型。
- 自动接管本地工具配置。
- 真实链路测试与调试器。
- 导入/导出功能。

## 开放问题

当前子任务不再保留阻塞性开放问题。技术栈和工程默认项已按用户要求，基于既有约束与参考项目自行确定；后续由用户集中 review 并指出不合理点。
