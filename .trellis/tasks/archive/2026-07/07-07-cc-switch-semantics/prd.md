# 切换语义增强：回填保护 + Common Config Snippet（地基）

## Goal

为 agent-switch 的 Claude Code 切换补齐两个 ccs 式机制：**回填保护**（切换供应商时不丢失用户自加的 hooks/permissions/statusLine 等配置）+ **Common Config Snippet**（跨 provider 共享字段 deep-merge）。本子任务是父任务 `07-07-claude-code-ccs-align` 的地基批，被 `cc-prompts` / `cc-common-config-editor` 依赖。

## Background

### agent-switch 现状
- 切换时对 `~/.claude/settings.json` 做字段级 merge：只覆盖 `env.ANTHROPIC_BASE_URL` / `env.ANTHROPIC_AUTH_TOKEN`（+可选 `env.ANTHROPIC_MODEL`），保留文件其它顶层键和 env 内其它键（合并写入，非整文件替换）。见 `src-tauri/src/services/tool_takeover/claude_code.rs:56-85`（apply）、`92-124`（apply_direct）。
- 原子写 + 写前备份：`mod.rs:552-563`（atomic_write）、`mod.rs:480-541`（backup_before_write）。
- 双模式状态机：proxy（占位符 `agent-switch-managed` + 本地代理 URL）/ direct（真实 base_url + 解密 key）。disable direct→回退 proxy；reapply 幂等 mode-aware。见 `mod.rs:145-415`。
- **无** provider 级字段快照、**无** Common Config Snippet、**无**跨 provider 共享字段机制。

### ccs 现状（对比调研结论，待 research 精确化）
- 切换时 `write_live_snapshot` 用 `write_json_file` **整文件覆盖** settings.json（非字段级 merge）+ 排序键 + 原子写。
- **回填保护**：切走前抓取"当前 provider 的字段"，下次切回该 provider 才恢复；跨 provider 共享只能靠 Common Config Snippet。
- **Common Config Snippet**：deep-merge 跨 provider 共享字段，默认 `{"includeCoAuthoredBy": false}`，是裸 JSON 编辑器（可手写 hooks/permissions/statusLine 等任意键）。
- **代理接管模式**：用 `PROXY_MANAGED` 占位符 + 本地代理 URL，不写 settings.json 走热切换，退出时 `cleanup_claude_takeover_placeholders_in_live` 清理占位符。

### 关键差距
agent-switch 的字段级 merge 已部分避免"覆盖用户 env 内其它键"，但与 ccs 语义不同：ccs 是整文件覆盖 + 回填保护 + Common Config 三层协同。用户自加的 hooks/permissions/statusLine 在 ccs 切换时若不在快照/common snippet 会丢失；agent-switch 因 merge 保留顶层键反而不会丢——但这也意味着 agent-switch 没有"per-provider 配置隔离"能力。

## Requirements

按 A1-hybrid + B1 拍板，本子任务范围锁定为 **settings.json 三层语义 + token 加密调和**，不含任何外围文件（B1 划走）。

### R1. `settings_config` 语义升级为「全文快照（token 脱敏）」
- R1.1 `providers.settings_config` 从端点引用 `{endpoint_id, model, wire_api}` 升级为**完整 `settings.json` 全文 JSON**，唯独 `env.ANTHROPIC_AUTH_TOKEN` 存 endpoint 引用占位（脱敏形态），不落明文。
- R1.2 提供 DB 迁移：把存量 provider 的端点引用重建为全文快照（`env.ANTHROPIC_BASE_URL` / `ANTHROPIC_MODEL` 由端点信息回填，token 保持引用占位）。迁移幂等、可回退（保留原列或版本标记）。
- R1.3 direct 与 proxy 两种模式都基于新的全文快照模型工作，且**都不降级凭证加密**：明文 token 只在写入 live 的瞬间由 `endpoints` 表解密得到，绝不写回 DB、不入日志。

### R2. 切换 = 整文件覆盖 + token 解密填充
- R2.1 切换目标 provider 时，用 `write_live_snapshot` 对 `~/.claude/settings.json` **整文件覆盖**（对齐 ccs，取代现字段级 merge），保持排序键 + 原子写（`.tmp`→rename）。
- R2.2 覆盖前把 `settings_config` 全文中的 token 占位替换为 `endpoints` 表解密出的明文 token，再写 live。
- R2.3 覆盖前保留现有写前备份（`backup_before_write` → `data_dir/backups/tools/`）语义。

### R3. per-provider backfill（切走前自动捕获 live 手改）
- R3.1 切走当前 provider 前，读取 live `settings.json` 全文 → strip 掉 common config 贡献的键 → 把 token 明文**脱敏回 endpoint 引用占位** → 存回**当前（切走前）provider** 的 `settings_config`。
- R3.2 backfill 后再执行 R2 的整文件覆盖切到新 provider。切回旧 provider 时其 live 手改（hooks/permissions/statusLine 等）如实恢复。
- R3.3 backfill 绝不把明文 token 写入 DB（脱敏是 R3.1 的强制步骤，需测试覆盖）。

### R4. Common Config Snippet（跨 provider 全局层）
- R4.1 新增全局 common config 存储（`settings` 表键 `common_config_claude-code` 或等价），默认 `{"includeCoAuthoredBy": false}`，裸 JSON 编辑器语义（可手写任意键）。
- R4.2 写 live 时 common config **deep-merge 覆盖在 provider settings 之上**（source/common 赢，对齐 ccs `live.rs:354-369`）。
- R4.3 per-provider 三态开关（`provider.meta.common_config_enabled`：启用/禁用/未设→默认），控制该 provider 是否叠加 common config。
- R4.4 backfill strip common config 键时用 `json_deep_remove`（对齐 ccs `live.rs:117-143`），避免把 common 贡献的键误存进 provider 全文。
- R4.5 提供 `json_deep_merge` / `json_deep_remove` / `json_is_subset`（可移植 ccs `live.rs:51-143`）。

### R5. B1 解耦边界（约束，非交付物）
- R5.1 本子任务**不**读写 `~/.claude.json`（mcpServers）、`~/.claude/CLAUDE.md`、`~/.claude/skills/`、`~/.claude/projects/`——这些由 cc-mcp/cc-prompts/cc-skills/cc-sessions 独立子任务实现。
- R5.2 为后续子任务预留切换成功后的 sync 钩子点（不实现 sync，仅确认钩子位置在 design 标注）。

## Acceptance Criteria

- [ ] AC1（全文快照 + 加密）：新建/编辑 provider 后，DB `settings_config` 为完整 settings.json 全文，且 `env.ANTHROPIC_AUTH_TOKEN` 为 endpoint 引用占位，grep DB 文件无明文 token。（R1）
- [ ] AC2（迁移）：对含存量 provider 的旧 DB 执行迁移后，所有 provider 可正常切换，base_url/model 正确，token 走加密链路；迁移可重复执行不损坏数据。（R1.2）
- [ ] AC3（整文件覆盖切换）：切换 provider 后 live `settings.json` 内容 == 该 provider 全文快照叠加 common config（token 为明文），键有序、文件原子写、切换前有 `.bak` 备份。（R2）
- [ ] AC4（backfill 往返）：provider A live 手加 `hooks` 字段 → 切到 B → 切回 A，A 的 `hooks` 如实恢复；且 A 的 `settings_config` 中 token 仍为引用占位（无明文）。（R3）
- [ ] AC5（common config 全局层）：设 common config `{"includeCoAuthoredBy": false}` → 任意启用 common 的 provider 切换后 live 含该键；关闭某 provider 三态开关后其 live 不含该键。（R4.1–R4.3）
- [ ] AC6（strip 正确）：common config 含键 X，provider 全文不含 X → 切走 backfill 后该 provider 的 `settings_config` 不含 X（未被误吸收）。（R4.4）
- [ ] AC7（proxy 模式仍工作）：proxy 模式 provider 切换后 live 指向本地代理 URL + 占位 token，direct 模式 live 为真实 base_url + 解密 token，两模式均不降级加密。（R1.3）
- [ ] AC8（B1 边界）：本子任务改动不触碰 `~/.claude.json`/`CLAUDE.md`/`skills/`/`projects/`；grep 确认无新增对这些路径的写入。（R5.1）
- [ ] AC9（回归）：`cargo test` + 现有 tool_takeover/proxy 相关测试全绿；新增单测覆盖 deep_merge/deep_remove/backfill 脱敏/迁移。

## Decisions（已拍板）

- **A. 回填保护语义 = A1 全对齐 ccs**（2026-07-07 用户拍板）：`providers.settings_config` 改为完整 `settings.json` 全文快照；切换 = 整文件覆盖 live（`write_live_snapshot`）+ per-provider backfill（切走前读 live → strip common config → 存回当前 provider 的 `settings_config`）+ common config 全局层（deep-merge 覆盖在 provider settings 之上，source 赢）。参考 ccs `live.rs:713-719`（write_live_snapshot 整文件覆盖）、`mod.rs:1684-1702`（backfill 存 providers.settings_config）、`live.rs:354-369`（common_config 三态开关）。
  - **已调和冲突 1+2（A1-hybrid，token 加密）**（2026-07-07 用户拍板）：保留 A1 全语义（`settings_config` = 完整 settings.json 全文 + 整文件覆盖 `write_live_snapshot` + per-provider backfill 自动捕获 live 手改 + common config 全局层 deep-merge），但 `env.ANTHROPIC_AUTH_TOKEN` 字段在 DB 里存 **endpoint_id 引用而非明文**：切换时从 `endpoints` 表解密填入 live 明文 token；backfill 时 live 的明文 token **脱敏回 endpoint_id 引用**再存 `settings_config`。这样 backfill 自动捕获语义不变（其它字段照常存全文），唯独 token 走加密链路。direct/proxy/凭证加密三者全保留，不违背父任务「不降级加密」约束。参考 agent-switch `crypto.rs` + ccs `live.rs:713-719`。
  - 未选 A1-strict 原因：违背父任务 PRD constraint「不为对齐 ccs 而降级凭证加密」+ ccs 明文是已知缺陷（导出/WebDAV 不脱敏），agent-switch 凭证加密是核心安全改进。

- **B. 外围管理与 takeover 解耦 = B1 独立解耦**（2026-07-07 用户拍板）：MCP/skills/prompts 管理**不并入 takeover/本子任务**，各自作为独立 service（对齐 ccs：`McpService::sync_all_enabled`、`SkillService::sync_to_app` 是独立 service，在 `switch_normal` 末尾统一 sync，takeover/hot_switch 模式**不** sync 外围）。本子任务只负责 settings.json 的整文件覆盖 + backfill + common config 三层；外围文件（`~/.claude.json` mcpServers、`~/.claude/CLAUDE.md`、`~/.claude/skills/`）由 cc-mcp/cc-prompts/cc-skills 子任务各自实现。参考 ccs `mod.rs:1774`（McpService sync 在 switch_normal 末尾）、`mod.rs:1633`（hot_switch 不 sync MCP 注释）。
  - 影响后续子任务定位：cc-prompts（CLAUDE.md 单激活）、cc-mcp、cc-skills、cc-sessions 均为独立 service，在切换成功后的钩子点统一调用 sync（钩子点待各子任务 design 确认，候选：`perform_switch` 成功后 / `providers.rs` switch API 末尾）。

## Open Questions

（无——A/B 已拍板；direct 模式与「settings_config=全文」的结构调和方案属技术设计，留 design.md 定）

## Out of Scope

- Prompts 管理、MCP 管理、env 行为开关、permissions·hooks 编辑器 UI、Skills、会话、DeepLink（其它子任务）。
- 代理网关本身、跨协议翻译、端点选择/故障转移/模型别名锁（agent-switch 既有优势，不在对齐范围）。
- 凭证加密、portability 备份/恢复（既有能力，不退化）。
