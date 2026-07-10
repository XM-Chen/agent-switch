# Research: 决策证据摘要（A 回填保护语义 / B 代理模式外围管理范围）

- **Query**: 基于事实给决策 A 和 B 各列出"如果选 X，需要做什么/影响什么"的证据摘要
- **Scope**: mixed（ccs + agent-switch 对照）
- **Date**: 2026-07-07

> 本文件**不替用户拍板**，只摆事实、影响、工作量。每个决策列出 2-3 个候选路径，标注参考项目（ccs/cpa/独立）。

---

## 前置事实对照（决策基础）

| 维度 | ccs | agent-switch 现状 |
|---|---|---|
| `provider.settings_config` 语义 | 完整 `settings.json` 全文快照 | 端点引用 `{endpoint_id, model, wire_api, ...}` |
| 切换写 live 方式 | **整文件覆盖**（`write_live_snapshot`） | **字段级 merge**（只动 `env.ANTHROPIC_*`） |
| per-provider backfill | 有（live → `providers.settings_config`） | **无**（仅启动迁移 `backfill_from_takeover`，命名误导） |
| Common Config Snippet | 有（`settings` 表 `common_config_<app>`，deep-merge 全局层） | **无** |
| 用户 hooks/permissions/statusLine 命运 | per-provider（随 backfill 存 DB，切回还原） | **全局共享**（takeover 从不触碰，跨 provider 不变） |
| 代理 takeover 写 live | 字段级改 `env`（保留其他键）+ common config 仍生效 | 字段级改 `env`（保留其他键），无 common config |
| takeover 整文件备份 | DB `proxy_live_backup` 表，自动还原 | 文件级 `.bak`，**无自动还原** |
| 外围文件管理（MCP/skills/prompts） | 有独立 service，normal 切换末尾 sync | **完全无** |

---

## 决策 A：回填保护语义

### A1: 全面对齐 ccs（provider.settings_config = full settings.json + backfill + common config）

**参考**：ccs。

**需要做什么**：
1. 把 `providers.settings_config` 从端点引用改为完整 `settings.json` JSON 全文。**冲击现有 direct 模式**——direct provider 现在依赖 `settings_config = {endpoint_id, model, wire_api}`，要么废弃 direct 模式（回归纯 proxy），要么引入双字段（`settings_config` 全文 + `endpoint_ref` 端点引用）。
2. 新增 `settings` 表的 `common_config_<app>` 键 + 前端编辑器（参考 ccs `src/config/`）。
3. 实现 `json_deep_merge`/`json_deep_remove`/`json_is_subset`（ccs `live.rs:51-143` 有完整实现可抄）。
4. 新增 `provider.meta.common_config_enabled` 三态字段 + `provider_uses_common_config` 逻辑（含 legacy 子集检测）。
5. `switch_normal` 流程：切走前 backfill（读 live → strip common config → 存 DB），切回时 `write_live_with_common_config`（整文件覆盖）。
6. 放弃 `tool_takeover::apply`/`apply_direct` 的字段级 merge，改为整文件覆盖（或保留字段级 merge 但补上 per-provider 全文层）。

**影响**：
- ✅ 完整复刻 ccs 语义：per-provider hooks/permissions/statusLine、common config 全局层、切回还原。
- ❌ **破坏现有 direct 模式**——direct provider 的端点引用模型与"settings_config = 全文"冲突，需要重新设计 direct 凭据存取（endpoint 表是否保留？加密链路是否重做？）。
- ❌ 工作量大（DB 迁移 + 切换链路重写 + 前端 + 深度合并算法）。
- ❌ 用户在 live 手改的 hooks 会随切换在 provider 间漂移——与 ccs 一致的"per-provider"语义，但 agent-switch 现有用户习惯是"全局共享"，是行为回归。

**关键证据**：
- ccs `live.rs:713-719` `write_live_snapshot` 整文件覆盖。
- ccs `mod.rs:1684-1702` backfill 存 `providers.settings_config`。
- agent-switch `tool_takeover/mod.rs:129-138` `DirectSettings` = 端点引用，与全文快照不兼容。

---

### A2: 维持现状（字段级 merge，无 backfill，无 common config）

**参考**：agent-switch 现状（独立路径）。

**需要做什么**：不改。

**影响**：
- ✅ 零工作量。切换简单、稳定。
- ✅ 用户 hooks/permissions/statusLine **永不丢失**（takeup 从不碰），用户体验上甚至比 ccs 更"安全"。
- ❌ **无法 per-provider 配置** hooks/permissions/statusLine——所有 provider 共用同一套。这是与 ccs 的功能性差距。
- ❌ 没有 common config 全局层——无法做"所有 provider 都加 `includeCoAuthoredBy: false`"这类需求。
- ❌ 用户在 live 手改的 `env.ANTHROPIC_MODEL` 等 env 字段会在切换时被覆盖（apply/apply_direct 强写），不被捕获。

**关键证据**：
- `claude_code.rs:56-85` apply 只动 2 个 env 键。
- `claude_code.rs:92-124` apply_direct 只动 3 个 env 键。
- grep 确认无 `common_config`/`snippet`。

---

### A3: 混合（保留端点引用 + 新增 per-provider "extra settings JSON" 可选层 + 可选 common config）

**参考**：ccs + 独立改造。

**需要做什么**：
1. `providers` 表加列 `extra_settings TEXT`（或放 `meta` 里），存 per-provider 的可选 `settings.json` 片段（如 `{hooks:{...}, permissions:{...}}`）。
2. 切换时（direct/proxy 两条链）在 `apply`/`apply_direct` 之后，把 `extra_settings` deep-merge 进 live（保留 env.ANTHROPIC_* 已写的值，补上 hooks 等）。
3. 可选：加 `common_config_<app>` 全局层（settings 表），同样 deep-merge。
4. **不做 backfill**——用户若想让某 provider 带特定 hooks，主动在 provider 编辑器里填 `extra_settings`，而非靠切走时自动捕获。

**影响**：
- ✅ 不破坏 direct 模式的端点引用模型。
- ✅ 支持 per-provider hooks/permissions/statusLine（显式声明，不靠 backfill）。
- ✅ 可选 common config 全局层。
- ❌ 与 ccs 语义不完全一致（没有 backfill 自动捕获）——用户在 live 手改的字段不会自动回到 provider 的 `extra_settings`。
- ⚠️ 需要 deep-merge 算法（可抄 ccs `live.rs:99-115`），但不需要 deep-remove/backfill 的复杂度。
- ⚠️ 切换时"切走前是否清理上一个 provider 的 extra_settings 字段"需要设计——若不清理，字段会累积；若清理，需要记录上一次写了哪些键（近似 backfill 的复杂度）。**这是 A3 的关键设计难点**。

**关键证据**：
- ccs `live.rs:99-115` `json_deep_merge` 可直接复用。
- ccs `live.rs:117-143` `json_deep_remove` 是 backfill/strip 用的，A3 若不 backfill 则不需要。
- agent-switch `tool_takeover/mod.rs:200-256` direct 模式链路可在 `apply_direct` 后插入 extra_settings merge 步骤。

---

## 决策 B：代理模式下外围管理范围

### B1: 代理模式只管 settings.json 的 env 部分（维持现状）

**参考**：agent-switch 现状 + ccs takeover 本身也只管 env（外围由独立 service 管）。

**需要做什么**：不改。

**影响**：
- ✅ 零工作量。边界清晰：takeover 只负责把工具指向本地代理/真实端点。
- ✅ 不碰用户 MCP/skills/projects/CLAUDE.md，无破坏风险。
- ❌ 不管理 MCP/skills/prompts——与 ccs 功能性差距（ccs 有 McpService/SkillService/prompt_files）。
- ⚠️ **重要澄清**：ccs 的外围管理**不是 takeover 的职责**，而是独立 service（`McpService::sync_all_enabled`、`SkillService::sync_to_app`）在 `switch_normal` 末尾统一 sync。ccs 的 takeover/hot_switch 模式反而**不 sync MCP**（`mod.rs:1633` 注释明确）。所以"代理模式管外围"是对 ccs 的误解——ccs 是"normal 切换管外围，takeover 只管 env"。

**关键证据**：
- ccs `mod.rs:1774` `McpService::sync_all_enabled(state)` 在 `switch_normal` 末尾，不在 hot_switch。
- ccs `proxy.rs:1150-1214` `takeover_live_configs` 只改 env 字段。
- agent-switch `tool_takeover/` 全模块只读写 settings.json/config.toml/auth.json。

---

### B2: 新增独立的外围管理 service（MCP/skills/prompts），与 takeover 解耦

**参考**：ccs（McpService/SkillService/prompt_files 独立 service，normal 切换末尾 sync）。

**需要做什么**：
1. 新增 `mcp_servers` 表（schema 抄 ccs `database/schema.rs:63-73`：`id, name, server_config, enabled_claude, ...`）。
2. 新增 `McpService::sync_all_enabled`：把 DB 里 `enabled_claude=true` 的 mcpServers 合并写入 `~/.claude.json` 的 `mcpServers` 字段（注意：`~/.claude.json` 不是 `~/.claude/settings.json`，是另一个文件，存 MCP 配置）。
3. 新增 `skills` 表 + `SkillService::sync_to_app`：把启用的 skill 文件同步到 `~/.claude/skills/`。
4. 新增 prompts 管理 + `~/.claude/CLAUDE.md` 写入。
5. 在 `perform_switch` 成功后调用这些 sync（对齐 ccs `switch_normal` 末尾）。
6. **proxy 模式（hot_switch）不调 sync**（对齐 ccs：takeover 下不重写外围，因为 live 不变）。

**影响**：
- ✅ 对齐 ccs 完整功能面（MCP/skills/prompts 管理）。
- ✅ 与 takeover 解耦，职责清晰。
- ❌ 工作量大（3 个新 service + DB 表 + 前端 UI + 文件同步逻辑）。
- ⚠️ `~/.claude.json` 的 mcpServers 合并需要谨慎——用户可能手改，需 deep-merge 而非覆盖（ccs `mcp/claude.rs` 有实现）。
- ⚠️ skills 文件同步涉及 git clone / 文件复制，复杂度高（ccs `services/skill.rs` 1100+ 行）。

**关键证据**：
- ccs `database/schema.rs:63-73` mcp_servers 表、`82-104` skills 表。
- ccs `services/mcp/claude.rs` MCP 同步实现。
- ccs `services/skill.rs:1119+` SkillService。
- ccs `live.rs:908,944` `McpService::sync_all_enabled` / `SkillService::sync_to_app` 调用点。

---

### B3: 只补 MCP 同步（最常用的外围），暂不做 skills/prompts

**参考**：ccs 子集。

**需要做什么**：B2 的 MCP 子集——只新增 `mcp_servers` 表 + `McpService` + `~/.claude.json` 合并写入。skills/prompts 延后。

**影响**：
- ✅ 中等工作量，覆盖最常用的 MCP 管理。
- ✅ 用户能在 agent-switch UI 里管 MCP servers，跨 provider 共享（ccs 的 MCP 是 per-app 启用，不是 per-provider）。
- ❌ 仍有差距（无 skills/prompts）。
- ⚠️ 同 B2 的 `~/.claude.json` deep-merge 风险。

**关键证据**：同 B2。

---

## 交叉影响

- **A1 + B2** = 完整复刻 ccs（工作量最大，但功能对齐）。
- **A2 + B1** = 维持 agent-switch 现状最小可用（零工作量，功能最弱）。
- **A3 + B3** = 增量补能力（per-provider extra settings + MCP 管理），不破坏现有模型，中等工作量。
- **A1 与 direct 模式冲突**：A1 要求 `settings_config` = 全文，direct 模式要求 = 端点引用。若选 A1 必须重新设计 direct 凭据存取（可能放弃 direct 模式，或引入双字段）。
- **A3 的"切走时是否清理上一个 provider 的 extra_settings"** 是设计难点：不清理会字段累积，清理就需要近似 backfill 的"上次写了啥"记录。可考虑：每次写 live 前先从 live 剥离"本 provider 上次写的 extra_settings"（需要存上次的快照≈backfill）或干脆整文件覆盖（回到 A1）。

## Caveats / Not Found

- ccs 前端 `useCommonConfigSnippet.ts`/`claudeProviderPresets.ts` 未逐行读——后端语义已完整，前端是编辑器 UI。
- ccs `sync_live_to_providers`（takeover 进入时同步 token 到 DB）的具体字段级逻辑未逐行读，但语义是 backfill 的 token-only 子集。
- agent-switch `services/importers/ccs.rs` 的 ccs→agent-switch 迁移路径（`extract_env` 抽 base_url/api_key/model 建端点）已读，它是一次性迁移工具，不影响实时切换语义，但若选 A1 需要调整它（因为 A1 下 `settings_config` 变全文，导入器可直接搬 ccs 的 `settings_config`）。
