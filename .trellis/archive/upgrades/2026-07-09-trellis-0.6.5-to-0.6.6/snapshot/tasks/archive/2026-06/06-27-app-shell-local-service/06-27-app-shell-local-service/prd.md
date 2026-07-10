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

## 已确认决策

- 前端技术栈采用 **React + TypeScript + Vite**。
  - 参考来源：主要参考 `ccs` 的 Tauri 桌面应用形态，并结合 Tauri 社区常见 React/Vite 模板。
  - 取舍：React + TypeScript 更适合 agent-switch 后续复杂管理台，包括 8 个页面、表单、模型映射、调试器和日志表格；代价是前端依赖和构建链更重，需要在设计中约束目录结构和组件边界。

## 需求

- 初始化 Tauri + Rust + React + TypeScript + Vite 应用骨架。
- Rust 后端需要在 Tauri 应用生命周期内启动本地 HTTP 服务。
- 本地 HTTP 服务必须绑定 `127.0.0.1:42567`。
- 服务必须提供 `/health`，用于确认本地服务运行状态。
- 服务必须建立路径隔离骨架，即使后续子任务尚未实现，也应有可解释的占位响应或路由边界。
- Web UI 至少提供中文导航骨架，覆盖父任务确认的 8 个页面入口：总览、账号、端点、模型、工具、路由、日志、设置。
- 应用关闭时，本地 HTTP 服务应随进程停止。
- 应用骨架应预留 SQLite 初始化入口，但本子任务只需完成数据库连接/迁移框架的最小闭环；具体账号、端点、凭据表结构由后续子任务细化。

## 验收标准

- [ ] 可以启动 Tauri 桌面应用。
- [ ] Web UI 使用 React + TypeScript + Vite，并显示中文基础导航。
- [ ] 本地 HTTP 服务绑定 `127.0.0.1:42567`。
- [ ] `GET http://127.0.0.1:42567/health` 返回成功状态和基础版本/运行信息。
- [ ] `/`、`/api/*`、`/claude-code/*`、`/codex/*`、`/v1/*` 的路由边界存在，未实现功能返回可解释占位错误，而不是无意义 404。
- [ ] 应用关闭后本地服务停止。
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

## 待澄清问题

- Rust 本地 HTTP 服务框架采用 **Axum + Tokio + tower-http**。
  - 参考来源：agent-switch 独立工程方案；四个参考项目并不都采用 Rust/Tauri 本地 HTTP 服务，因此这里按已确认的 Tauri + Rust 技术栈选择成熟 Rust 异步 Web 生态。
  - 取舍：Axum/Tokio 更适合后续异步代理、超时、流式响应、SSE/chunked body、日志中间件、本地认证扩展和 `/v1` 多端点真实转发；代价是初始结构要明确区分 Tauri command、HTTP handler 和应用服务层，避免生命周期混乱。
- Web UI 组件库/样式方案采用 **Tailwind CSS + shadcn/ui + Radix UI**。
  - 参考来源：主要参考 `ccs` 的 React + Tailwind + shadcn/ui 桌面管理台形态，同时吸收 `9router` 的 Tailwind Web 管理体验。
  - 取舍：该方案适合中文桌面管理台、表单、弹窗、开关、Tabs、Toast、深色模式和长期定制；代价是 shadcn/ui 组件进入项目后需要自行维护，复杂表格后续可能需要 TanStack Table 等补充库。
- SQLite 迁移工具和 Rust 数据访问方式。
