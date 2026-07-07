//! Claude Code MCP 服务器管理（cc-mcp，仅 Claude Code）。
//!
//! MCP 是独立于 provider 切换的全局清单：DB `mcp_servers` 表存全部 server，
//! `enabled_claude=1` 的项全量投影写入 `~/.claude.json` 的 `mcpServers` 字段。
//! CRUD 后即时同步，与 `tool_takeover` 完全解耦（对齐 B1 决策）。

pub mod claude;
pub mod validation;
