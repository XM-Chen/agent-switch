# 应用骨架实现验证记录

验证时间：2026-06-27

## 构建验证

| 检查 | 命令 | 结果 |
|------|------|------|
| 前端类型检查 + 构建 | `npm run build` | ✅ 通过（269 KB JS + 9 KB CSS） |
| Rust 格式检查 | `cargo fmt --check` | ✅ 通过 |
| Rust 编译 | `cargo check` | ✅ 通过 |
| Rust lint | `cargo clippy --all-targets -- -D warnings` | ✅ 通过 |
| Rust 二进制构建 | `cargo build` | ✅ 通过 |
| Tauri CLI | `npx tauri --version` | ✅ 2.11.3 |

## 运行期验证

启动 `src-tauri/target/debug/agent-switch.exe`，日志显示：

```text
INFO agent_switch_lib: Agent-Switch 正在启动...
INFO agent_switch_lib: 数据目录：C:\Users\chen_\AppData\Roaming\com.agent-switch.app
INFO agent_switch_lib::db::migrations: 执行迁移 v1: create_schema_migrations
INFO agent_switch_lib::db::migrations: 数据库迁移：1 项迁移已执行
INFO agent_switch_lib::http: 本地服务已启动：http://127.0.0.1:42567/
```

端点探测结果：

| 端点 | HTTP 状态 | 响应 |
|------|-----------|------|
| `GET /health` | 200 | `{"status":"ok","app":"agent-switch","version":"0.1.0","address":"127.0.0.1:42567","database":"ok"}` |
| `GET /` | 200 | 返回 `dist/index.html`（中文 Web UI 入口） |
| `GET /api/test` | 501 | `{"error":{"type":"not_implemented","code":"scope_not_ready","message":"该入口已预留，但当前子任务尚未实现具体功能。","scope":"api"}}` |
| `GET /claude-code/test` | 501 | 同结构，scope=`claude-code` |
| `GET /codex/test` | 501 | 同结构，scope=`codex` |
| `GET /v1/test` | 501 | 同结构，scope=`v1` |

关闭验证：

- 进程退出后端口 `42567` 释放，`/health` 不再响应。
- 窗口关闭事件触发 `shutdown_tx`，Axum graceful shutdown 已接入。

## 与设计文档的偏差

- `design.md` 中 `/` 描述为"静态资源入口 / Web UI 入口"，实现使用 `tower_http::services::ServeDir` + `ServeFile` fallback 服务前端 `dist/`，并支持 SPA 路径回退到 `index.html`。
- `web_dist_dir` 路径定位使用 `env!("CARGO_MANIFEST_DIR")/../dist`，开发和 `cargo build` 后均可用；后续若改为 Tauri resource 打包，需更新此处。

## 未验证项

- 端口 `42567` 被占用时的启动失败中文提示：未单独模拟占用场景，但代码路径为 `TcpListener::bind` 失败 → 返回错误 → 日志记录，不自动换端口。
- `npm run tauri dev` 完整开发体验：未单独运行，因为需要桌面会话；`cargo build` + 手动运行已验证功能。
- 生产打包 `npm run tauri build`：未运行，因为耗时较长且本子任务只要求骨架可运行。
