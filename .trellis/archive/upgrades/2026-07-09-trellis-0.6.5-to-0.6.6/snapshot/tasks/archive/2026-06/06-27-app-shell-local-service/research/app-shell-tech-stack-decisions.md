# 应用骨架技术栈与工程约定研究

## 研究问题

为 `应用骨架与本地服务` 子任务确定剩余技术栈与工程默认项，减少低价值逐项确认。

参考来源：

- `ccs`：Tauri/Rust/React 桌面应用形态、SQLite + rusqlite + DAO 分层、Tailwind + shadcn/ui。
- `9router`：React/Next.js + Tailwind 的 Web 管理体验、本地 provider/model 管理思路。
- `cpa`：CLI/API 代理服务形态，可参考健康检查和代理入口，但 UI 不是重点。
- `sub2api`：管理后台、摘要日志、脱敏配置与数据库/配置分层。
- Tauri v2 文档：Vite 前端集成、Tauri setup 生命周期、managed state。
- Axum 文档：Router、State、nest、`axum::serve`、graceful shutdown。
- shadcn/ui 文档：Vite + React + Tailwind CSS 安装与组件复制模式。

## 技术栈决策

| 维度 | 决策 | 参考来源 | 原因 |
|------|------|----------|------|
| 桌面框架 | Tauri v2 | 已由父任务确认，主要参考 `ccs` | 小体积、Rust 后端、适合本地桌面管理工具 |
| 前端框架 | React + TypeScript + Vite | `ccs` + Tauri 社区模板 | 适合复杂管理台、类型安全、组件生态成熟 |
| 样式/组件 | Tailwind CSS + shadcn/ui + Radix UI | `ccs`、`9router` | 适合桌面工具风格、深色模式、长期可定制 |
| HTTP 框架 | Axum + Tokio + tower-http | agent-switch 独立工程选择 | Rust 异步生态成熟，适合流式代理、SSE、中间件和未来本地认证 |
| 数据库访问 | rusqlite + 自维护 SQL 迁移 + DAO/Repository | 主要参考 `ccs` | 本地 SQLite 场景依赖少、行为透明、迁移和加密字段可控 |
| 前端请求状态 | TanStack Query | 参考 `ccs` 前端状态分层 | 管理 API 请求缓存、刷新和错误状态，后续页面复用 |
| 页面路由 | React Router | agent-switch 独立工程选择 | 8 个中文页面导航足够，简单成熟 |
| 包管理器 | npm | Tauri/shadcn 文档示例 + 环境通用性 | 降低新环境依赖；后续如有需要可迁移到 pnpm |
| 日志 | tracing + tracing-subscriber + tower-http TraceLayer | Rust/Axum 常规生态 | 后续可扩展请求摘要日志；本子任务不记录正文 |

## 工程默认项

### 1. 端口冲突

固定使用 `127.0.0.1:42567`，端口被占用时不自动换端口。

原因：父任务明确要求固定服务地址；自动换端口会破坏 Claude Code / Codex / OpenCode 手动配置和自动接管目标。

行为：

- 绑定失败时记录错误；
- UI 显示中文错误说明；
- 不静默回退到其他端口。

### 2. `/` Web UI 与 Tauri WebView

目标语义：`http://127.0.0.1:42567/` 是 Web UI 入口。

工程落地：

- 生产构建中，Axum 在 `/` 服务前端静态资源；
- Tauri WebView 仍可加载打包资源或本地服务入口，具体实现以稳定启动为先；
- 开发模式可使用 Vite dev server，`/` 可代理或跳转到 dev server，但生产语义必须收敛到单端口。

### 3. 占位路由错误格式

未实现的作用域不得返回无意义 404，应返回可解释 JSON 错误，例如：

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

### 4. SQLite 初始边界

本子任务只实现：

- app data dir 定位；
- `agent-switch.db` 创建；
- `schema_migrations` 表；
- 迁移执行框架；
- DAO/Repository 目录结构。

不在本子任务中实现账号、端点、凭据、模型、日志等完整业务表。

### 5. CORS 与安全边界

第一版父任务已选择仅绑定 `127.0.0.1` 且不做本地 token/session 认证。CORS 不能作为安全边界。

本子任务仅允许：

- 同源请求；
- Tauri/WebView 或 Vite dev server 所需的开发来源；
- CLI/本地工具非浏览器请求不依赖 CORS。

后续本地认证扩展点需在 Axum middleware 层预留。

## 结论

应用骨架子任务不再需要继续询问技术栈细节。后续直接把上述决策写入 `prd.md`、`design.md` 和 `implement.md`，交由用户集中 review。
