pub mod codex_oauth;
pub mod crypto;
/// 从外部配置（ccs 等）批量导入 provider。
#[allow(dead_code)]
pub mod importers;
pub mod keychain;
/// Claude Code MCP 服务器全局清单管理（cc-mcp）：写 `~/.claude.json` 的 mcpServers 字段。
pub mod mcp;
pub mod model_alias;
pub mod model_sync;
/// 配置导入导出服务（本机加密完整备份 / 可迁移脱敏配置导出）。
pub mod portability;
/// Claude Code Prompts 管理（cc-prompts）：写 `~/.claude/CLAUDE.md` 单激活清单。
pub mod prompts;
/// Provider 领域层：ccs 式统一切换单元（与 accounts+endpoints 并存）。
/// 供 HTTP API 与工具接管消费；`AppType` 的部分方法预留扩展位，暂未被生产引用。
#[allow(dead_code)]
pub mod provider;
pub mod tool_takeover;
/// 本模块供 proxy 层消费。proxy 层尚未实现时标记该模块为 unused 是错误的。
/// 将在 proxy 模块接入后移除该属性。
#[allow(dead_code)]
pub mod translator;
