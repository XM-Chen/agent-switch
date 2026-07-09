# Common Config 裸 JSON 编辑器（permissions/hooks/statusLine/outputStyle）

## Goal

P1 子任务：补齐 Claude Code Common Config 裸 JSON 编辑器与 provider 级三态开关，让用户能用全局 snippet 管理 `~/.claude/settings.json` 中跨 provider 共享的任意顶层键（如 `permissions`、`hooks`、`statusLine`、`outputStyle`、`env`），并在切换 provider 时经已完成的地基快照层安全落 live。

## Background

### 已确认的 agent-switch 现状

- 后端 Common Config API 已存在并挂载：`GET/PUT /api/common-config/{tool}`，当前只支持 `claude-code`，PUT 要求 JSON object。见 `src-tauri/src/http/api/common_config.rs`、`src-tauri/src/http/router.rs`。
- Common Config 存在 `app_metadata`，key 为 `common_config_<tool>`；未设置时返回默认 `{"includeCoAuthoredBy": false}`，provider 未显式设置时默认启用。见 `src-tauri/src/services/tool_takeover/mod.rs`。
- provider API 已支持 `meta.common_config_enabled` 三态和完整 `meta` 透传；二者同时提交时以 `meta` 为基底再叠加开关，避免覆盖 `meta.snapshot`。见 `src-tauri/src/http/api/providers.rs`。
- Claude Code 切换已走地基三层：切走前 backfill live，剥离连接 env 和 common config，写入 `provider.meta.snapshot`；切入时 `snapshot + common` deep-merge 后整文件写 `settings.json`，再注入连接层。见 `src-tauri/src/services/tool_takeover/mod.rs`、`src-tauri/src/services/tool_takeover/claude_snapshot.rs`。
- 前端 `src/lib/api.ts` 已暴露 `commonConfigApi` 和 `UpdateProviderBody.common_config_enabled/meta`，但 UI 尚未消费 `commonConfigApi`；`ProviderForm.tsx` 主要写 `meta.snapshot.env`，没有 Common Config editor 或 per-provider 三态开关入口。

### ccs 参考事实

- ccs 的 Common Config UI 是 provider 表单内的 `CommonConfigEditor`：有“写入通用配置” checkbox、“编辑通用配置”入口、全屏 JSON 编辑器和“从编辑内容提取”。参考 `E:/SynologyDrive/git_files/cc-switch/src/components/providers/forms/CommonConfigEditor.tsx`。
- ccs 后端语义与 agent-switch 地基一致：snippet 存 `common_config_<app_type>`，provider meta 控制启用，写 live 时 common deep-merge 覆盖 provider 配置，backfill 时 deep-remove common。参考 `E:/SynologyDrive/git_files/cc-switch/src-tauri/src/services/provider/live.rs`。

## Requirements

### R1. 全局 Common Config 裸 JSON 编辑器

- R1.1 新增前端入口读取/保存 `/api/common-config/claude-code`，编辑对象为裸 JSON object。
- R1.2 支持任意 `settings.json` 顶层键，包括但不限于 `permissions`、`hooks`、`statusLine`、`outputStyle`、`env`、`includeCoAuthoredBy`。
- R1.3 非 object JSON（数组、字符串、数字、null）与非法 JSON 必须在 UI 层阻止保存，并显示明确错误。
- R1.4 默认值显示为后端默认 `{"includeCoAuthoredBy": false}`，保存后重新打开内容一致。

### R2. provider 级 Common Config 三态开关

- R2.1 在 Claude Code provider 表单中显示 Common Config 控制；非 Claude Code provider 不显示。
- R2.2 三态：跟随默认（写 `common_config_enabled: null`）、强制启用（`true`）、强制禁用（`false`）。
- R2.3 保存 provider 时必须保留 `meta.snapshot`、`meta.snapshot.env`、`settings_config`、`notes`、`category` 等既有字段，不因三态开关覆盖其它 meta。

### R3. 生效语义

- R3.1 保存 Common Config 或 provider 三态开关只更新 DB，不隐式改写 live `~/.claude/settings.json`。
- R3.2 已激活 provider 若要立即生效，应沿用现有显式“应用到 live/重切”模式；不新增隐式切换副作用。
- R3.3 下次切换 provider 时，地基 `snapshot + common` 语义必须按既有规则生效：common 覆盖 provider snapshot，切走 backfill 时 common 贡献键不会被吸收到 provider snapshot。

### R4. 边界约束

- R4.1 本任务主要补前端 UI，不新增后端核心语义或 DB 迁移。
- R4.2 不改连接层：`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` 仍由既有 direct/proxy 链路注入，token 不落 DB。
- R4.3 不触碰 `~/.claude.json`、`CLAUDE.md`、`skills/`、`projects/`；仅通过已有 Common Config API 和 provider API 写 DB。
- R4.4 不在本任务实现 ccs 的“从编辑内容提取 common config”后端能力，除非后续明确扩 scope。
- R4.5 不在本任务扩展 ccs 导入器导入非连接 settings 快照；该问题独立于编辑器。

## Acceptance Criteria

- [ ] AC1：打开 Common Config 编辑器能读取默认/已保存 snippet；保存合法 JSON object 后重新打开一致；非法 JSON 与非 object JSON 被拒绝。（R1）
- [ ] AC2：全局 snippet 写入 `permissions`、`hooks`、`statusLine`、`outputStyle` 后，切换到默认启用 common 的 Claude provider，live `~/.claude/settings.json` 含这些键。（R1/R3）
- [ ] AC3：某 provider 显式 `common_config_enabled=false` 后，切换该 provider 时 live 不含 common snippet；清除为默认后重新跟随默认启用。（R2/R3）
- [ ] AC4：common 与 provider snapshot 冲突时 common 覆盖；切走 backfill 后 common 贡献键不会被吸收到 `provider.meta.snapshot`。（R3）
- [ ] AC5：保存 common 控制时不破坏 `meta.snapshot.env`、`settings_config`、`notes`、`category`；env 行为开关 UI 与 Common Config 控制可同时保存。（R2）
- [ ] AC6：当前激活 provider 修改 common config 后，不点击显式应用/重切时 live 不立即改变；下次切换或显式应用后才生效。（R3）
- [ ] AC7：回归 direct/proxy 连接层、token 不落 DB、Bedrock/env 开关、Codex 路径无回归。（R4）

## Out of Scope

- “从当前 provider snapshot 提取 Common Config”的自动化提取。
- ccs 导入时导入 `hooks/permissions/statusLine/outputStyle` 等非连接 settings 快照。
- 针对 `permissions` / `hooks` / `statusLine` / `outputStyle` 的结构化表单；本任务先做裸 JSON。
- MCP / Prompts / Skills / Sessions / DeepLink。

## Open Questions

- 无阻塞问题。已按最小 P1 范围收敛为“裸 JSON 编辑器 + provider 三态开关”，快捷 toggle、自动提取、导入保真均作为后续增强。