//! Claude Code Prompts 管理（cc-prompts，仅 Claude Code）。
//!
//! Prompts 是独立于 provider 切换的全局单激活清单：DB `prompts` 表存多份提示词，
//! 任一时刻至多一份 `enabled_claude=1` 并投影写入 `~/.claude/CLAUDE.md`。启用前做
//! 回填保护捕获 live 手改，CRUD 后即时同步，与 `tool_takeover` 完全解耦（对齐 B1）。

pub mod claude;
