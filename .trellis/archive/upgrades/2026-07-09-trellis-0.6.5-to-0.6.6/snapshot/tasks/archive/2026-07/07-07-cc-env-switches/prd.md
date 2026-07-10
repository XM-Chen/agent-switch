# env 行为开关注入（CLAUDE_CODE_* / DEFAULT model / API_TIMEOUT_MS / Bedrock）

## Goal

P0批子任务#4：为 agent-switch 补齐 ccs 式 Claude Code **env 行为开关**的可视化管理——让用户能为每个 Claude Code provider 便捷设置 `ANTHROPIC_DEFAULT_{HAIKU,SONNET,OPUS}_MODEL`、`API_TIMEOUT_MS`、`CLAUDE_CODE_*`（含 Bedrock `CLAUDE_CODE_USE_BEDROCK` 等）等行为开关，并在切换该 provider 时如实写入 `~/.claude/settings.json` 的 `env`。父任务 `07-07-claude-code-ccs-align` 的 P0 批，弱依赖已完成的 `cc-switch-semantics`（地基）。参考 ccs 实现，按 agent-switch 既有架构适配，不照抄代码。

## Background

### ccs 现状（参考蓝本，已读源码 `claudeProviderPresets.ts` + provider live 写入）
- ccs **没有独立的「env 开关」机制**：这些行为开关就是 provider `settingsConfig.env` 里的**任意键**（`ANTHROPIC_DEFAULT_HAIKU_MODEL` / `ANTHROPIC_DEFAULT_SONNET_MODEL` / `ANTHROPIC_DEFAULT_OPUS_MODEL` / `API_TIMEOUT_MS` / `CLAUDE_CODE_USE_BEDROCK` / ...）。
- ccs 通过**预设模板**（`providerPresets`：胜算云、Kimi、GLM、MiniMax 等）预填这些 env 键，用户新建 provider 时选预设即带上整套 env；也可在 provider 表单里裸编辑 `settingsConfig` JSON 手改任意 env 键。
- 切换 provider = `write_live_snapshot` 整文件覆盖 `settings.json`，`settingsConfig.env` 原样落 live → 这些 env 开关随之生效。

### agent-switch 现状（已读源码，关键结论）
- **env 键已被地基任务的 `meta.snapshot` 层原样持久化 + 往返**：`cc-switch-semantics` 完成后，切走 provider 时 `backfill_claude_snapshot` 读 live `settings.json` → `strip_connection_env`（只删 `ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN` 两个连接键）→ 存回 `provider.meta.snapshot`；切回时 `write_claude_snapshot_layer` 整文件覆盖写回。**即 `env.ANTHROPIC_DEFAULT_*_MODEL` / `API_TIMEOUT_MS` / `CLAUDE_CODE_USE_BEDROCK` 等非连接 env 键已经 per-provider 持久化并在切换时如实写 live**（见 `claude_snapshot.rs:26` `CONNECTION_ENV_KEYS` 仅两键；测试 `strip_connection_env_removes_only_connection_keys` 显式保留 `ANTHROPIC_MODEL`/`CLAUDE_CODE_USE_BEDROCK`）。
- proxy 网关侧已消费部分 env 开关：`is_bedrock_provider` 读 `env.CLAUDE_CODE_USE_BEDROCK == "1"`（`forwarder.rs:1931`）；模型别名等读 `ANTHROPIC_DEFAULT_*_MODEL`。
- **真正的差距 = UX / 数据入口**：agent-switch 前端 `ProviderForm.tsx` 只把 `settings_config` 当**裸 JSON textarea** 编辑；而 direct provider 的 `settings_config` 是端点引用 `{endpoint_id, model, wire_api}`，**不是** settings.json 全文——行为 env 开关实际落在 `meta.snapshot`，当前**前端无任何入口编辑 `meta.snapshot` 里的 env 键**。用户想设 `ANTHROPIC_DEFAULT_SONNET_MODEL` 只能手改 live `settings.json`（会被 backfill 捕获），无结构化 UI、无预设模板。
- 后端已有大量 `ANTHROPIC_DEFAULT_*_MODEL` 常量散落在 proxy/别名/import 路径，可复用为「已知开关」清单。

### 关键差距（本任务要补的）
1. **无结构化 env 开关编辑 UI**：模型三档（haiku/sonnet/opus）、超时、Bedrock 开关等应有表单/校验，而非让用户裸手写 JSON 或手改 live。
2. **无预设模板**：ccs 有胜算云/Kimi/GLM/MiniMax 等一键预填整套 env；agent-switch 无。
3. **写入落点需明确**：env 开关应写进 provider 的哪一层（`meta.snapshot` vs 其它），以在切换时经地基快照层如实落 live，且不与连接层/加密冲突——这是父任务**决策 C** 的核心。

## Decision C（已拍板 2026-07-07）：承载方式 = 复刻 ccs（env 开关即 provider env 键，落 `meta.snapshot.env`）

用户拍板「学习 ccs 做法」。ccs 的 env 行为开关**没有独立机制**，就是 provider `settingsConfig.env` 里的普通键，由结构化表单（`ClaudeFormFields` + `useModelState`：解析已知 env 键为 UI 字段、写回 env）编辑，预设模板预填。

映射到 agent-switch：这些非连接 env 键的对应承载层 = 地基任务的 **`provider.meta.snapshot.env`**（切换/backfill 已如实往返，见 Background）。因此**本任务不新增承载字段、不新增写链路**——只补 ccs 缺的两块 UX：结构化 env 编辑器（写入 `meta.snapshot.env`）+ 预设模板预填。持久化与落 live 完全复用地基快照层。

## Requirements

### R1. env 行为开关的承载与写入落点（决策 C：复用 `meta.snapshot.env`）
- R1.1 行为 env 开关（非连接键）承载在 `provider.meta.snapshot.env`，**不新增 DB 字段/迁移**。编辑即读-改-写 `meta.snapshot`（对齐地基 `claude_snapshot::snapshot_from_meta`/`snapshot_into_meta` 语义）。
- R1.2 写入经地基快照层如实落 `settings.json.env`（`write_claude_snapshot_layer` 已有），且**不干扰**连接层（`ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN` 仍由 `apply`/`apply_direct` 注入）与凭证加密。编辑器**禁止**写这两个连接键（`strip_connection_env` 无论如何会剥离，但 UI 层也不暴露）。
- R1.3 backfill 往返不得丢失或串改 env 开关（地基已保证非连接 env 键往返，本任务需回归验证）。

### R2. 结构化 env 开关编辑 UI
- R2.1 provider 表单提供结构化编辑：模型三档（`ANTHROPIC_DEFAULT_HAIKU_MODEL` / `SONNET` / `OPUS`）、`ANTHROPIC_MODEL`、`API_TIMEOUT_MS`、Bedrock 开关（`CLAUDE_CODE_USE_BEDROCK` + 相关凭证/区域模板键）等常见开关；保留裸 JSON 逃生舱编辑其余任意 env 键。
- R2.2 校验：模型 id 非空即接受（不硬编码白名单，因聚合商模型名各异）；`API_TIMEOUT_MS` 数字；Bedrock 开关布尔/`"1"`。
- R2.3 编辑仅改承载层（R1），不触碰连接层字段。

### R3. 预设模板（env 开关一键预填，对齐 ccs `providerPresets`）
- R3.1 提供若干 Claude Code 聚合商预设，选中预设把整套 env 开关预填进结构化表单。首批清单取 agent-switch 后端 `commands/provider.rs` 已内置的示例集（GLM/Kimi/MiniMax 等）对齐，作为最小代表集；扩充为后续增量。
- R3.2 预设为**纯前端模板**（`src/config/*` 常量，不落库为独立实体，对齐 ccs `claudeProviderPresets.ts`）。仅预填 env 开关字段，不含连接层（base_url/token 走端点体系）。

### R4. Bedrock 预设（含 AWS 凭证，明文承载，2026-07-07 用户拍板）
- R4.1 Bedrock 作为一个预设项：结构化开关 `CLAUDE_CODE_USE_BEDROCK`（布尔/`"1"`）+ AWS 区域 + AWS 凭证（`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`）整套随 env 落 `meta.snapshot.env`；proxy `is_bedrock_provider`（`forwarder.rs:1931`）已消费 `CLAUDE_CODE_USE_BEDROCK`，本任务保证一致。
- R4.2 **AWS 凭证明文承载（用户拍板，对齐 ccs）**：`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` 作为普通 env 键明文存进 `meta.snapshot.env`、随切换明文落 `settings.json`。**已知权衡**：这偏离 agent-switch 对连接 token 的加密边界（token 走 endpoints 表 AES-256-GCM），是用户为对齐 ccs Bedrock 体验显式接受的降级。缓解：AWS 凭证仅在用户主动填 Bedrock 预设时出现；不与连接 token 混用同一存储路径（token 仍加密）。
- R4.3 **明文暴露面（已核对代码，2026-07-07）**：portability `collect`（`services/portability/collect.rs`）**不导出 `providers` 表**（仅 accounts/endpoints/models/aliases/route_settings/tool_takeover/ui_settings），故 `meta.snapshot.env` 里的 AWS 凭证**不随 portability 导出泄漏**——无需新增脱敏钩子。明文的实际暴露面仅为：① 本机 DB 文件（`providers.meta` 明文）；② live `settings.json`（切换时明文落盘）。二者在「明文承载」决策下本就是预期行为，与 ccs 一致。**约束**：若未来给 portability 增加 providers 导出，必须同步把 `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` 纳入脱敏字段集（在 design「后续扩展注意」登记）。

### R5. 解耦与回归边界（约束）
- R5.1 复用地基切换链路（`switch_claude` / `meta.snapshot` / common config），**不**新建独立 sync service（区别于 cc-mcp/cc-prompts——env 开关是 settings.json 的一部分，天然属地基快照层，不是外围文件）。
- R5.2 不破坏：连接层注入、凭证加密、proxy 网关对 env 开关的既有消费（Bedrock 检测/别名）、Codex 路径零改动。

## Acceptance Criteria

- [ ] AC1（结构化写入落 live）：在 provider 表单设 `ANTHROPIC_DEFAULT_SONNET_MODEL=X` → 切换该 provider → live `settings.json.env` 含 `ANTHROPIC_DEFAULT_SONNET_MODEL=X`。（R1–R2）
- [ ] AC2（切换往返不丢）：设多个 env 开关 → 切走再切回 → 全部 env 开关如实恢复，且连接层仍正确、DB 无明文 token。（R1.3）
- [ ] AC3（预设预填）：选某聚合商预设 → 表单 env 开关被整套预填 → 保存并切换后 live 一致。（R3）
- [ ] AC4（逃生舱）：裸 JSON 编辑一个未在结构化字段中的 env 键 → 保存并切换后 live 保留该键。（R2.1）
- [ ] AC5（Bedrock 预设含凭证）：启用 Bedrock 预设并填 AWS 区域/凭证 → 切换后 live `env` 含 `CLAUDE_CODE_USE_BEDROCK="1"` + 区域 + `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`（明文，对齐 ccs）；proxy `is_bedrock_provider` 识别一致。（R4.1–R4.2）
- [ ] AC6（明文暴露面无扩散）：含 Bedrock AWS 凭证的 provider 经 portability 导出后，导出物**不含 provider `meta`**（因 `collect` 不导出 providers 表）；即 AWS 凭证不随导出泄漏。（R4.3）
- [ ] AC7（不干扰连接/加密）：direct provider 设 env 开关后切换，`ANTHROPIC_BASE_URL`/`AUTH_TOKEN` 仍由端点解密注入，token 明文不落 DB。（R1.2、R5.2）
- [ ] AC8（回归）：`cargo test` + `npm test` 全绿；Codex 路径与既有 proxy Bedrock/别名消费无回归。

## Constraints

- 参考 ccs「env 开关即 provider env 键 + 预设模板」思路，按 agent-switch 架构适配。**不新建独立 service**——env 属 settings.json，复用地基 `meta.snapshot` 快照层。
- 连接凭证（`ANTHROPIC_AUTH_TOKEN`）仍走端点加密链路，不降级。**例外（2026-07-07 用户拍板）**：AWS Bedrock 凭证按 ccs 做法明文随 env 落 `meta.snapshot`（DB）；这是父任务「不降级加密」约束的显式已知例外，代价是 DB 明文 AWS 凭证。已核对：portability 不导出 providers 表，故明文不随导出泄漏（R4.3），暴露面仅本机 DB + live 文件，与 ccs 一致，不做静态加密。
- 不硬编码模型白名单（聚合商模型名各异），仅结构化常见开关 + 裸 JSON 逃生舱。

## Out of Scope

- Prompts / MCP / Skills / 会话 / DeepLink（其它子任务）。
- 连接层（base_url/token）与端点管理、模型别名锁、故障转移（既有能力）。
- Codex 的 env 行为开关（本任务仅 Claude Code）。

## Open Questions

（无——决策 C 承载方式与 AWS Bedrock 凭证明文承载均已拍板；明文暴露面已核对代码确认不经 portability 导出。「应用到 live」即时生效语义属技术设计，留 design.md 定。）
