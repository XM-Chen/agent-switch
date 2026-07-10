# 后端模块分层与 IPC

## 目录职责

ccs 后端是单个 Tauri crate（`src-tauri/Cargo.toml:1-15`），`src-tauri/src/lib.rs` 组装插件、state、启动迁移和 command 注册。

| 层 | 路径 | 责任 |
|---|---|---|
| IPC | `commands/<domain>.rs` | 解析/验证参数、提取 State、调用 Service、把 `AppError` 映射给前端 |
| 业务 | `services/<domain>/` | 跨 DAO/live/proxy 的事务与编排 |
| 数据 | `database/dao/` | SQLite 查询和持久化；schema/migration/backup 分离 |
| live adapter | `services/provider/live.rs` 及各 app config module | 读写 Claude/Codex/Gemini 等真实配置文件 |
| proxy | `proxy/` | Axum server、路由、provider adapter、协议翻译、streaming/failover |
| 横切 | `mcp/`、`deeplink/`、`session_manager/` | 独立领域能力；裁剪后单应用化 |
| 配置/错误 | `config.rs`、`settings.rs`、`error.rs` | 集中路径、设备级设置、结构化错误 |

`commands/provider.rs` 展示标准模式：`AppType::from_str` 后调用 `ProviderService`（`src-tauri/src/commands/provider.rs:23-90`）。command 不得直接执行 SQL 或重建 `settings.json`。

## Tauri command 契约

每个 command 变更同步检查：

1. `commands/mod.rs` 导出；
2. `src-tauri/src/lib.rs` 的 `generate_handler!` 注册（起点 `src-tauri/src/lib.rs:1184`）；
3. 前端 `src/lib/api/` wrapper；
4. 参数命名（前端 camelCase / Rust 参数）与 serde shape；
5. 返回错误的可读性和敏感信息脱敏；
6. command 单元测试及至少一个 TS wrapper/组件测试。

只删除前端调用而保留 handler/module 属于未完成裁剪；反之会导致运行时 unknown command。

## 启动顺序是不变量

ccs 启动涉及 DB 初始化与 schema 迁移、旧 JSON 迁移、Skills SSOT、live import-before-seed、MCP/Prompt 导入、proxy/state 注册等（`src-tauri/src/lib.rs:446-599,672-823,949-988`）。身份/单应用裁剪不得随意重排：

- **Claude 首启必须 import-before-seed**：空产品库且 live 可读时，先导入完整 `~/.claude/settings.json` 为 current `default`，再 seed 精选官方模板；
- 导入失败不能静默 seed 并覆盖 live；
- 不读取 `~/.cc-switch` 或旧 Agent Switch DB；目标数据根为 `~/.agent-switch`；
- Copilot/Codex OAuth manager 仍需在启动时注册，因为它们是 Claude 托管上游。

## 错误处理

`AppError` 用 `thiserror` 建模并实现序列化/到 String 的边界转换（`src-tauri/src/error.rs:4-117`）。

- IO 错误带安全路径上下文，不带文件内容或 token；
- DB/HTTP/解析错误在 Service 层补业务语义；
- 可恢复失败返回 `Result`，日志分 warn/error；
- 不用 `unwrap/expect` 处理用户文件、网络、锁、数据库；测试中可用；
- 代理对外错误保持稳定 HTTP 状态，内部原因写脱敏日志。

## Windows-only 目标

首期只保证 Windows。删除 Unix/macOS/Linux 代码前先确认没有 Windows 共用抽象；保留成本低、由 cfg 保护且仍被共享代码引用的部分可以延后。每轮必须在 Windows 跑 cargo check/clippy/test 和 Tauri no-bundle build。
