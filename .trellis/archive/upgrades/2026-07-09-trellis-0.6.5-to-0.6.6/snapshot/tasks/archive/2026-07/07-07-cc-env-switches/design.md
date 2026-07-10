# Design: env 行为开关注入（cc-env-switches，仅 Claude Code）

## 范围与边界

- **仅 Claude Code**：只编辑 `~/.claude/settings.json` 的非连接 `env` 键（经地基快照层落 live）。Codex 零改动。
- **不新建独立 service**：env 开关是 `settings.json` 的一部分，天然属地基 `meta.snapshot` 快照层（区别于 cc-mcp/cc-prompts 的外围文件）。本任务主体是**前端结构化编辑器 + 前端预设模板**，后端仅保证编辑入口把 env 键写入 `provider.meta.snapshot.env` 并被既有 `switch_claude` 如实落 live。
- **承载层 = `provider.meta.snapshot.env`**（决策 C 已拍板，复刻 ccs「env 开关即 provider env 键」）：不新增 DB 字段/迁移。持久化与落 live 完全复用地基 `write_claude_snapshot_layer` + `backfill_claude_snapshot`。
- **不碰连接层**：`ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` 由 `apply`/`apply_direct` 注入，编辑器不暴露这两个键；`strip_connection_env`（`claude_snapshot.rs:26`）无论用户怎么写都会剥离它们出 snapshot。

## ccs 参考映射（已读 `ClaudeFormFields.tsx` + `useModelState.ts` + `claudeProviderPresets.ts`）

ccs 的 env 开关**没有独立机制**：
- 行为开关 = provider `settingsConfig.env` 的普通键：`ANTHROPIC_DEFAULT_{HAIKU,SONNET,OPUS}_MODEL`(+`_NAME`)、`ANTHROPIC_MODEL`、`API_TIMEOUT_MS`、`CLAUDE_CODE_USE_BEDROCK`、`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`、`CLAUDE_CODE_MAX_OUTPUT_TOKENS`、`CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS` 等。
- 结构化表单 `ClaudeFormFields` + `useModelState`：把已知 env 键解析为 UI 字段（3 模型角色 + 显示名 + `[1M]` 勾选 + 兜底模型 + 认证字段选择 + API 格式），用户编辑后写回 `settingsConfig.env`。`[1M]` 标记是 ccs 对「声明支持 1M」的约定后缀（`CLAUDE_ONE_M_MARKER = "[1M]"`）。
- 预设模板 `providerPresets`（61 项）：胜算云/Kimi/GLM/MiniMax/DeepSeek/Bedrock 等，选中预填整套 env；Bedrock 预设含 `CLAUDE_CODE_USE_BEDROCK: "1"` + AWS 区域/凭证（用 `${...}` 模板变量）。
- 切换 = 整文件覆盖 live，`settingsConfig.env` 原样落 `settings.json`。

**agent-switch 映射差异**：ccs 的 `settingsConfig` 就是 settings.json 全文；agent-switch 的 `settings_config` 是端点引用（direct）或代理占位（proxy），**非连接 env 键落在 `meta.snapshot`**。因此编辑器读写 `meta.snapshot.env` 而非 `settings_config`。连接层仍由端点体系 + `apply`/`apply_direct` 注入，与 ccs 的「env 内含 base_url/token」不同——这是 agent-switch 加密优势的体现，不退化。

## 数据流

```
前端编辑器（结构化字段 + 裸 JSON 逃生舱）
    ↓ 读写 provider.meta.snapshot.env（经 PUT /api/providers/{id}，meta 透传）
provider.meta.snapshot（DB，明文 env 键，不含连接层/token）
    ↓ 切换该 provider 时
switch_claude_at:
    backfill（切走前 provider）→ write_claude_snapshot_layer（target）
        → deep_merge(snapshot, common?) → write_live_snapshot（整文件覆盖 settings.json）
        → apply / apply_direct（注入连接层 base_url/token）
    ↓
live ~/.claude/settings.json.env（含行为开关 + 连接层）
```

关键：编辑器写入 `meta.snapshot.env` 后，**下次切换该 provider** 即把 env 开关落 live。若该 provider 是当前激活态，编辑后需触发一次 reapply/重切才能让 live 生效——design 决策见下「即时生效语义」。

## 即时生效语义（当前激活 provider 编辑 env 后）

两种选项：
- **A（推荐，最简）**：编辑保存只更新 `meta.snapshot`，不立即改 live；提示用户「下次切换该 provider 生效」或提供「应用」按钮触发 `reapply`/重切。对齐地基「snapshot 是切换时投影」语义，零新增写链路。
- **B**：编辑保存后若该 provider 是当前激活态，自动调 `switch_claude`（prev=target）重切一次，即时落 live。更顺滑但引入「编辑即重切」副作用（备份/状态机联动）。

→ **选 A**：最小 blast radius，复用地基链路。UI 提供「应用到 live」按钮（仅当前激活 provider 显示）显式触发 reapply，避免隐式重切。reapply 已是既有能力（`tool_takeover::reapply`，mode-aware）。

## 结构化编辑器字段（首批，对齐 ccs 已知键）

- 模型三档：`ANTHROPIC_DEFAULT_HAIKU_MODEL` / `ANTHROPIC_DEFAULT_SONNET_MODEL` / `ANTHROPIC_DEFAULT_OPUS_MODEL`（+ 可选 `_NAME` 显示名，1M 标记复刻 ccs `[1M]` 约定）。
- 兜底模型：`ANTHROPIC_MODEL`。
- 超时：`API_TIMEOUT_MS`（数字校验）。
- Bedrock 开关：`CLAUDE_CODE_USE_BEDROCK`（布尔→`"1"`/缺省）+ `AWS_REGION` + AWS 凭证 `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`（明文承载，2026-07-07 用户拍板，对齐 ccs；secret 字段 UI 用密码框遮显，但存储/落 live 为明文）。
- 其它常见 `CLAUDE_CODE_*` 行为键：`CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC`、`CLAUDE_CODE_MAX_OUTPUT_TOKENS`、`CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS` 等，首批取高频项，其余走逃生舱。
- **裸 JSON 逃生舱**：对未结构化的任意 env 键，提供 `meta.snapshot.env` 的裸 JSON 编辑（类似既有 `ProviderForm` 的 `settings_config` textarea 风格），保留表达力。
- **禁止字段**：编辑器不暴露 `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN`（连接层，由端点/代理注入）。

校验：模型 id 非空即接受（不硬编码白名单，聚合商模型名各异）；`API_TIMEOUT_MS` 数字；Bedrock 布尔。空值 = 删除该键（对齐 ccs `useModelState` 的 `delete env[field]`）。

## 预设模板（纯前端，对齐 ccs `claudeProviderPresets.ts`）

- `src/config/claudeProviderPresets.ts`（或类似）：首批取 agent-switch 后端 `commands/provider.rs` 已内置示例集（GLM/Kimi/MiniMax 等）作为最小代表集，扩充为后续增量。每个预设 = `{ name, env: { ANTHROPIC_DEFAULT_*_MODEL, API_TIMEOUT_MS, ... } }`，**不含连接层**（base_url/token 走端点体系）。
- Bedrock 预设：`CLAUDE_CODE_USE_BEDROCK: "1"` + `AWS_REGION` + AWS 凭证字段（`AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY`）。**AWS 凭证明文纳入结构化编辑器**（2026-07-07 用户拍板，对齐 ccs）：整套随 env 落 `meta.snapshot.env`，明文存 DB + 明文落 live `settings.json`。secret 字段 UI 用密码框遮显但不加密存储。**已知权衡见 PRD R4.2**：偏离 agent-switch 对连接 token 的加密边界，为对齐 ccs Bedrock 体验的显式降级。

## 前端落点

- `src/components/providers/ProviderForm.tsx`：新增「Claude Code 行为开关」分区（仅 `app_type=claude-code` 显示），含结构化字段 + 裸 JSON 逃生舱 + 预设选择。读写 `meta.snapshot.env`（经既有 `meta` 透传字段）。
- `src/lib/api.ts`：provider update 已支持 `meta` 透传（`UpdateProviderBody.meta`），无需新 API；新增前端预设常量模块 + `meta.snapshot.env` 的解析/序列化 helper（参考 ccs `useModelState.parseModelsFromConfig` / `handleModelChange`）。
- 状态同步：结构化字段 ↔ 裸 JSON 逃生舱双向同步（参考 ccs `useModelState` 的 `isUserEditingRef` 防回填覆盖）。

## 后端落点

- **无新 API**：编辑器经既有 `PUT /api/providers/{id}`（`meta` 透传）写入 `meta.snapshot.env`。`providers` DAO 已按原样存 `meta` JSON（`database-guidelines.md`）。
- **无新迁移**：`meta` 已是 JSON TEXT，`meta.snapshot` 由地基任务确立。
- 即时生效的「应用到 live」按钮 → 调既有 `tool_takeover::reapply`（mode-aware，direct/proxy 各自重写连接层；但 reapply 不重写 snapshot 层——需确认是否要改 reapply 让它也走 `write_claude_snapshot_layer`，或用 `switch_claude(prev=target)` 显式重切。**design 决策**：用 `switch_claude(db, data_dir, Some(target), target, crypto)` 重切，复用地基三层链路；reapply 保持现状不动）。

## 兼容 / 回归

- **无 DB schema 迁移**（复用 `meta` JSON）。
- 不改 `tool_takeover` 切换链路、`claude_snapshot`、common config、ccs 导入器、portability、代理网关。
- Codex 路径零改动（编辑器仅 claude-code 分支）。
- proxy 既有消费（`is_bedrock_provider` 读 `env.CLAUDE_CODE_USE_BEDROCK`；别名读 `ANTHROPIC_DEFAULT_*_MODEL`）不受影响——env 键落 live 后 proxy 自然读到。

## 风险 / 回滚点

- **最高风险 = 编辑器写 `meta.snapshot` 与地基 backfill 的交互**：用户编辑写入 snapshot 后，切换往返不得丢失/串改。地基已保证非连接 env 键往返（`strip_connection_env` 只剥两键），本任务需回归测试覆盖「编辑 → 切走 → 切回 → env 开关如实恢复」。
- **即时生效语义**：选 A（编辑不立即落 live），避免隐式重切副作用；UI 显式「应用到 live」按钮触发 `switch_claude(prev=target)`。
- **首切兼容**：地基「快照缺失=老 merge 行为」下，编辑器首次写入 snapshot 即进入 per-provider 快照态；测试覆盖「编辑器首次写入 snapshot」路径。
- **AWS 敏感凭证明文落库（用户拍板接受）**：`AWS_SECRET_ACCESS_KEY`/`AWS_ACCESS_KEY_ID` 明文落 `meta.snapshot`（DB）+ 明文落 live `settings.json`——这是「明文承载」决策的预期暴露面。**已核对 portability**：`collect.rs` 只导出 accounts/endpoints/models/aliases/route_settings/tool_takeover/ui_settings，**不导出 `providers` 表**，因此 `meta.snapshot` 不经导出泄漏，无需新增脱敏钩子（AC6 = 负向断言：确认 providers 不入导出物）。若未来给 portability 加 providers 导出，须同步把 AWS 凭证纳入脱敏字段集。
- **1M 标记约定**：复刻 ccs `[1M]` 后缀，前端解析/序列化须一致，单测覆盖。
- 回滚：撤前端编辑器分区 + 预设模块即可，无后端/迁移副作用。
