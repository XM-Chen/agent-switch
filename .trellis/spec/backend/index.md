# 后端规范索引

> 适用基线：cc-switch v3.16.5（`8d1b3306d`）。目标：保留成熟 Rust/Tauri 架构，将产品收敛为 Windows + 简体中文 + Claude Code。

## 必读

1. [模块分层与 IPC](architecture-and-ipc.md)
2. [Provider 全文快照与切换](provider-snapshot-and-switching.md)
3. [数据库、备份与同步](database-backup-and-sync.md)
4. [代理、安全与托管上游](proxy-security-and-managed-upstreams.md)
5. [Rust 质量与测试](quality-and-testing.md)

## 核心不变量

- `provider.settings_config` 是 Claude 用户级 `settings.json` 的完整、任意 JSON 快照唯一 SSOT；不新增 `meta.snapshot`。
- 切出：读 live → 同步/剥离 Common Config → 回填当前 Provider；切入：Provider 快照 + Common Config → sanitizer → 原子写 live。
- 首期保持 SQLite schema v11 和多应用列，不做破坏性物理收缩；业务只创建/读取 Claude 数据。
- GitHub Copilot、ChatGPT Codex OAuth、OpenRouter、Responses 翻译是 Claude Provider 的上游能力，不是独立客户端，必须保护。
- 默认代理监听 `127.0.0.1`；非 loopback 时目标要求所有转发路由统一 Bearer token 鉴权。ccs 当前只对 Claude Desktop gateway 鉴权，此项明确属于待实现安全增强。
- live/config/DB 写入必须经集中 adapter/service；command 不直接拼路径、SQL 或协议转换。
- 新错误使用 `AppError`；不在业务路径 `unwrap/expect`，不在日志/前端 payload 输出 token。

## 当前与目标边界

ccs 当前 `AppType` 有七类应用，并区分 switch mode 与 additive mode（`src-tauri/src/app_config.rs:338-393`）。目标只暴露 `AppType::Claude`，但 schema v11 的 `app_type` 列暂留。裁剪必须按 `../guides/single-app-trimming.md` 做引用闭包，而不是按文件名前缀批删。

旧 Agent Switch 0.2.2 后端规范已归档至 `../legacy-agent-switch-0.2.2/backend/`，只作历史参考。
