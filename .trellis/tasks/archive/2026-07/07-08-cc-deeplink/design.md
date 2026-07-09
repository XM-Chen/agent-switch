# Design: Deep Link 一键导入（ccswitch://v1/import）

## 范围与边界

- 协议仅支持 `ccswitch://v1/import`，不注册 `agentswitch://`。
- 导入覆盖 `provider`、`prompt`、`mcp`、`skill` 四类资源。
- DeepLink 是 outward-facing 入口：只解析与预览，不自动写入；用户确认后才导入。
- `skill` 导入依赖 `cc-skills` 子任务提供安装 repo/subdir 的 service。

## 架构

```
OS deep link: ccswitch://v1/import?resource=...
  ↓ tauri-plugin-deep-link
Tauri URL event handler
  ↓ emit to frontend
DeepLinkImportDialog
  ↓ parse / merge / preview
http/api/deeplink.rs 或 Tauri command wrapper
  ↓ user confirm
services/deeplink
  ├─ parser.rs
  ├─ provider.rs → endpoints + providers + optional switch
  ├─ prompt.rs   → prompts service enable/import
  ├─ mcp.rs      → mcp service import/sync
  └─ skill.rs    → skills service install repo
```

## 协议解析

- 使用 `url` crate 解析。
- scheme 必须为 `ccswitch`。
- host 必须为 `v1`。
- path 必须为 `/import`。
- `resource` 必须为 `provider|prompt|mcp|skill`。
- URL 参数保持 ccs 兼容：`apiKey`、`haikuModel`、`sonnetModel`、`opusModel`、`configFormat`、`configUrl` 等 camelCase。

## 导入映射

### Provider

- `app=claude` 映射为 `app_type=claude-code`。
- `app=codex` 映射为 `app_type=codex`。
- `apiKey` 创建/复用 endpoint，凭证走现有加密链路。
- Claude 的 `model` 与默认模型字段进入 `provider.meta.snapshot.env`，不塞回连接层 env。
- `enabled=true` 导入后执行 switch，但仅在用户确认后发生。
- `endpoint` 逗号分隔：第一个为主 endpoint；其余按现有 endpoint 模型作为附加 endpoint 或明确 unsupported。

### Prompt

- 仅支持 `app=claude`。
- `content` 为 Base64 UTF-8 Markdown。
- `enabled=true` 复用 Prompts service enable，保留单激活与回填保护。

### MCP

- `config` 为 Base64 JSON，必须含 `mcpServers` object。
- 仅实际写入 Claude enabled；其它 app 在预览/结果中提示暂不支持。
- 已存在 server 默认不覆盖 `server_config`，只合并启用状态或提示冲突。

### Skill

- `repo` 必须为 `owner/name`。
- `directory`、`branch` 可选。
- 安装是网络动作，确认后调用 Skills service；取消时不联网。

## 前端设计

- 启动/运行中收到 URL 后打开确认对话框。
- 对话框展示资源类型、名称、目标 app、配置摘要、将执行的写入动作。
- API Key、usage token 等敏感字段默认遮蔽。
- 提供“取消”和“确认导入”；取消不调用 import。
- 导入成功后刷新对应缓存：providers/prompts/mcp/skills。

## 敏感信息与日志

- 不打印完整 URL。
- 日志中的 query 参数需要脱敏：`apiKey`、`usageApiKey`、`usageAccessToken`、`content`、`config`、`usageScript`。
- 前端错误可显示资源类型与字段错误，但不显示完整敏感 payload。

## 兼容与依赖

- 需要新增 `tauri-plugin-deep-link` 依赖与 `tauri.conf.json` scheme 配置。
- 冷启动/单实例 URL 传递可能需要 `tauri-plugin-single-instance`；实现时按 Tauri 2 插件实际要求确认。
- `provider/prompt/mcp` 可先实现；`skill` 需等 `cc-skills` service 就绪后接入，或先返回明确 pending/unsupported。

## 风险与回滚

- DeepLink 链接可能含明文 API Key：必须确认和脱敏。
- 兼容 ccs provider 导入时不能照搬 ccs 明文 settings_config，必须走 agent-switch endpoint 加密。
- 回滚时移除 scheme 注册和 URL handler；已导入资源按各自页面删除。