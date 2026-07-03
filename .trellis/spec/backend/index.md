# agent-switch Rust 后端规范

> 适用范围：`src-tauri/src/` 下的 Tauri/Rust 应用后端、Axum 本地 HTTP 服务、SQLite DAO、代理路由、协议转换和业务服务层。

---

## 概览

agent-switch 后端是桌面应用内嵌的本地服务：Tauri 负责桌面生命周期，Axum 负责 `127.0.0.1:42567` 本地 HTTP 路由，SQLite 保存账号/端点/模型/日志/设置，proxy 层负责协议适配、模型解析、认证注入、故障转移和摘要日志。

参考入口：

- `src-tauri/src/lib.rs` — Tauri setup、数据目录、数据库初始化、本地服务启动
- `src-tauri/src/http/` — Axum API、proxy、health、路由装配
- `src-tauri/src/db/` — migrations 与 DAO
- `src-tauri/src/services/` — 业务服务：crypto/keychain、codex_oauth、model_sync、portability、translator 等

---

## 规范索引

| 规范 | 内容 |
|------|------|
| [目录结构](./directory-structure.md) | Rust 后端模块边界与新增文件位置 |
| [数据库规范](./database-guidelines.md) | SQLite、DAO、迁移、事务与模型同步约定 |
| [HTTP/Proxy 规范](./http-proxy-guidelines.md) | 本地路由、故障转移、SSE、OAuth refresh、日志安全 |
| [Translator 规范](./translator-guidelines.md) | Anthropic/OpenAI Chat/Responses 协议转换与流式事件契约 |
| [Portability 规范](./portability-guidelines.md) | 导入导出、加密包、replace/merge 语义 |
| [质量规范](./quality-guidelines.md) | Rust 检查命令、测试要求、提交前质量门 |

跨层思考指南仍在 `../guides/`；Trellis Python runtime 与平台适配规范已迁移到 `../trellis-runtime/`。

## 提交前质量门

```bash
cd src-tauri
cargo fmt --check
cargo check
cargo clippy --all-targets -- -D warnings
cargo test --lib
```

如果任务只触及后端局部代码，可以先跑 targeted tests；最终收敛提交必须跑全量命令。
