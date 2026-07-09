# Deep Link 一键导入（v1/import 四类资源）

## Goal

P1 子任务：为 agent-switch 补齐 ccs 兼容的 Deep Link 一键导入能力。按用户拍板，本批**仅支持 `ccswitch://v1/import` 协议**，不注册 `agentswitch://`。导入资源覆盖 `provider`、`prompt`、`mcp`、`skill` 四类，并在导入前展示确认预览，避免用户误导入含 API Key 或外部 repo 的链接。

## Background

### 已确认的 agent-switch 现状

- `src-tauri/Cargo.toml` 当前没有 `tauri-plugin-deep-link` 依赖，`src-tauri/tauri.conf.json` 的 `plugins` 只有 updater，没有 deep-link scheme 注册。
- `src-tauri/src/lib.rs` 当前注册 shell/dialog/fs/updater/process 插件，没有 deep-link 插件或 URL 事件处理。
- 当前已有本地 ccs 导入器与前端对话框，但它只探测本地 cc-switch 配置并批量导入 provider，不处理 URL deep link。
- 当前已有 provider、MCP、Prompts 后端 API/service；Skills 子任务尚未实现，是 DeepLink `resource=skill` 的前置依赖。
- agent-switch 的 provider 凭证边界不同于 ccs：API Key 应创建/复用 endpoint 并走 AES-256-GCM 加密，不应把 deep link 里的 token 明文写入 `providers.settings_config`。

### ccs 参考事实

- ccs parser 只接受 `ccswitch://v1/import`：scheme 必须是 `ccswitch`，host 必须是 `v1`，path 必须是 `/import`。
- ccs `DeepLinkImportRequest` 支持四类资源：`provider`、`prompt`、`mcp`、`skill`。
- `provider` 参数包括：`app`、`name`、`endpoint`、`apiKey`、`homepage`、`model`、`haikuModel`、`sonnetModel`、`opusModel`、`notes`、`icon`、`config`、`configFormat`、`configUrl`、`enabled`、usage script 系列字段。
- `prompt` 参数包括：`app`、`name`、`content`、`description`、`enabled`；content 在 ccs importer 中按 Base64 解码。
- `mcp` 参数包括：`apps`、`config`、`enabled`；config 为 Base64 JSON，内容必须含 `mcpServers` object。
- `skill` 参数包括：`repo=owner/name`、`directory`、`branch`；ccs 保存 skill repo 配置。
- ccs 使用 Tauri deep-link 插件注册 `ccswitch` scheme，并在前端调用 parse/merge/import command 展示确认对话框。

## Decisions

- **协议 = 仅 `ccswitch://`**（2026-07-08 用户拍板）：本批不注册 `agentswitch://`。目标是最大化兼容现有 ccs 链接与生成器。
- **导入必须二次确认**：DeepLink 可能包含 API Key、Prompt 内容、MCP 命令或外部 skill repo，自动打开应用后只展示预览，不自动写入。
- **仅 Claude Code 优先落地，兼容参数保留**：agent-switch 当前成熟支持 Claude Code、Codex provider 与 Claude MCP/Prompts/Skills；不支持的 app 参数应明确拒绝或降级提示，不静默误导入。

## Requirements

### R1. 协议注册与入口

- R1.1 引入 `tauri-plugin-deep-link` 并在 Tauri 配置中注册 `ccswitch` scheme。
- R1.2 应用启动时监听 deep-link URL 事件；收到 URL 后传给前端展示导入确认对话框。
- R1.3 冷启动和已运行实例收到链接都应可处理；如需 single-instance 插件配合，在 design/实现中补齐。
- R1.4 仅接受 `ccswitch://v1/import`；其它 scheme/version/path 返回明确错误。

### R2. parser 与预览模型

- R2.1 新增 DeepLink parser：解析 query 参数为 `DeepLinkImportRequest`，字段命名与 ccs 兼容（camelCase URL 参数映射到 Rust/TS 类型）。
- R2.2 支持资源类型：`provider`、`prompt`、`mcp`、`skill`；其它类型拒绝。
- R2.3 提供 `parse` / `merge-config` / `import` 三段式能力：先解析、再合并 inline config、最后用户确认导入。
- R2.4 `config` / `content` / `usageScript` 等 Base64 参数必须校验 UTF-8 与格式；解析失败不落库。
- R2.5 `configUrl` 涉及外部网络获取；本批默认不自动拉取，除非用户在确认对话框中显式同意获取。

### R3. Provider 导入

- R3.1 `resource=provider` 支持 `app=claude` / `app=codex` 的 agent-switch 现有 app 类型映射：`claude` → `claude-code`，`codex` → `codex`。
- R3.2 Claude provider 导入必须创建/复用 encrypted endpoint，再创建 direct provider，其 `settings_config` 保存 endpoint 引用和模型信息；不得把 `apiKey` 明文写入 `providers.settings_config`。
- R3.3 Claude `model`、`haikuModel`、`sonnetModel`、`opusModel` 等非连接 env 键应写入 `provider.meta.snapshot.env`，对齐 `cc-env-switches` 决策。
- R3.4 `endpoint` 支持 ccs 的逗号分隔多个 URL：第一个作为 primary endpoint，其余作为同账号/同 provider 的附加 endpoint 或返回明确 unsupported 提示（实现时按现有 endpoint 模型确定）。
- R3.5 `enabled=true` 只在用户确认后执行切换；若切换失败，provider/endpoint 创建结果与错误必须如实返回。
- R3.6 不支持的 app（gemini/opencode/openclaw/hermes）本批返回“暂不支持该 app”的预览错误，不创建半成品。

### R4. Prompt 导入

- R4.1 `resource=prompt` 当前仅支持 `app=claude` → Claude Code Prompts。
- R4.2 `content` 按 ccs 兼容 Base64 解码为 UTF-8 Markdown，写入 `prompts` 表。
- R4.3 `enabled=true` 时，走 Prompts service 的 enable 流程，触发单激活与 live `CLAUDE.md` 回填保护；否则只创建未启用 prompt。
- R4.4 导入前预览 prompt 名称、描述、内容摘要与是否启用。

### R5. MCP 导入

- R5.1 `resource=mcp` 的 `config` 按 Base64 JSON 解码，必须包含 `mcpServers` object。
- R5.2 本批只实际支持 `apps` 中的 `claude`，写入 `mcp_servers.enabled_claude`；其它 app 值在预览中标注暂不支持并忽略或拒绝（实现需一致）。
- R5.3 已存在同名/同 id server 时，不覆盖既有 `server_config`；只合并启用状态或提示冲突，避免误覆盖用户配置。
- R5.4 导入后按 MCP service 即时投影到 `~/.claude.json`（如果 Claude 安装路径存在），对齐 `cc-mcp` 语义。

### R6. Skill 导入

- R6.1 `resource=skill` 依赖 `cc-skills` 子任务完成后端安装能力。
- R6.2 `repo` 必须是 `owner/name`，`directory` 和 `branch` 可选。
- R6.3 导入前展示 GitHub repo/subdir/branch 预览；安装为外部网络动作，必须用户确认。
- R6.4 导入后调用 Skills service 安装 repo/subdir，并按用户选择或默认策略启用到目标 app。

### R7. 前端确认对话框

- R7.1 新增 DeepLink import dialog：展示资源类型、关键字段、敏感字段遮蔽、将执行的写入动作。
- R7.2 API Key、usage token 等敏感字段默认遮蔽，只显示前后少量字符或“已提供”。
- R7.3 用户可取消；取消时不落库、不发起网络请求。
- R7.4 导入结果显示 created/skipped/failed，并刷新对应页面缓存（providers/prompts/mcp/skills）。

### R8. 安全与边界

- R8.1 DeepLink 不得自动导入；必须确认。
- R8.2 不在日志打印完整 URL 中的 API Key、usage token、prompt 全文等敏感内容。
- R8.3 不支持的 app/resource/configUrl 需明确错误，不静默成功。
- R8.4 所有导入复用现有 service/DAO，不绕过凭证加密、MCP 校验、Prompts 回填、Skills 路径安全。

## Acceptance Criteria

- [ ] AC1：安装包注册 `ccswitch://` scheme；点击 `ccswitch://v1/import?...` 可唤起/聚焦 Agent-Switch 并显示导入确认对话框。（R1）
- [ ] AC2：parser 接受合法 `ccswitch://v1/import`，拒绝其它 scheme/version/path/resource，并给出明确错误。（R1/R2）
- [ ] AC3：provider deep link 导入 Claude provider 时，endpoint 凭证加密存储，provider `settings_config` 不含明文 API Key；模型/env 参数写入正确层。（R3）
- [ ] AC4：`enabled=true` 的 provider 只有在用户确认后才切换；切换失败时结果如实显示。（R3/R7）
- [ ] AC5：prompt deep link Base64 content 正确解码；`enabled=true` 走 Prompts 单激活与回填保护；取消导入不落库。（R4/R7）
- [ ] AC6：mcp deep link Base64 JSON 正确解码，批量导入 `mcpServers`，已存在 server 不被静默覆盖，Claude enabled 后即时投影。（R5）
- [ ] AC7：skill deep link 调用 Skills service 安装 repo/subdir/branch；安装前确认，失败可见，取消不联网。（R6/R7）
- [ ] AC8：确认对话框遮蔽 API Key/token，日志不输出完整敏感 URL 或 prompt 全文。（R7/R8）
- [ ] AC9：不支持 app（gemini/opencode/openclaw/hermes）在本批给出明确 unsupported，不创建半成品。（R3/R5/R8）
- [ ] AC10：`cargo test`、`npm test`、`npm run build` 通过；新增测试覆盖 parser、Base64 解码、各资源导入、敏感日志脱敏、前端确认/取消流程。（R1-R8）

## Constraints

- 仅注册 `ccswitch://`，不注册 `agentswitch://`。
- 参考 ccs 协议和参数，但按 agent-switch 架构适配，不照抄 ccs Tauri command / plaintext provider 存储。
- DeepLink 是对外入口，所有导入动作必须确认，敏感信息必须遮蔽。
- 所有文档与 UI 文案使用中文。

## Out of Scope

- `agentswitch://` 自有协议。
- DeepLink 生成器网页。
- 自动拉取 `configUrl`（除非实现时明确加入用户确认后的 fetch）。
- Gemini/OpenCode/OpenClaw/Hermes provider/MCP/Prompt 的完整导入支持。
- 绕过 Skills 子任务直接实现一套简化 skill 安装器。

## Open Questions

- 无阻塞问题。协议已按用户选择锁定为仅 `ccswitch://`；不支持的 app 在本批明确拒绝。