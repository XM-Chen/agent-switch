# 一键从本地 ccs 导入 Claude 上游渠道

## Goal

让 agent-switch 用户能一键把本地已安装的 cc-switch（ccs）项目里配置好的 Claude 上游渠道批量导入到 agent-switch，免去手动逐条重建。导入后这些渠道即可在切换器页（`/providers`）查看与切换，且切换后能实际生效（`~/.claude/settings.json` 被正确写入）。

## Background

- ccs（`ref-cc-switch/tauri-migration` 分支）是 Claude Code 配置切换器，存储格式（研究文件 `research/ccs-data-format.md`）：
  - 索引：`~/.cc-switch/config.json` → `{ "providers": { id: { id, name, settingsConfig, websiteUrl? } }, "current": id|"" }`（扁平两字段，无 version/category 包裹）
  - `settingsConfig` 即 Claude Code `settings.json` 全文，典型键：`env.ANTHROPIC_BASE_URL`、`env.ANTHROPIC_AUTH_TOKEN`、`env.ANTHROPIC_MODEL`、`env.ANTHROPIC_SMALL_FAST_MODEL`、顶层 `includeCoAuthoredBy`
  - id 由前端 `crypto.randomUUID()` 生成，唯独自动导入现有配置用硬编码 `"default"`
  - 该分支无分组/分类概念，仅 preset 模板有 `isOfficial` 展示标记且不落盘
- agent-switch 的 direct 模式架构（研究文件 `research/agent-switch-direct-provider-path.md`）**刻意偏离 ccs 的明文内联**（代码注释 `mod.rs:126-128` 明说"偏离 ccs 明文做法"）：
  - direct provider 的 `settings_config` 是自定义子集 `{"endpoint_id": "...", "model"?, "wire_api"?, "requires_openai_auth"?}`，**不含** base_url/token
  - 真实 `base_url` 从 `endpoints` 表按 `endpoint_id` 查出，token 从 `endpoints.api_key_encrypted` 经 AES-256-GCM 解密
  - 切换走 `switch` → `enable_direct` → `claude_code::apply_direct`，写 `~/.claude/settings.json`
- 用户决策（本任务）：
  - **沿用 agent-switch 现有架构**（路线 2），不回退到 ccs 明文内联——导入的 ccs 渠道拆成 endpoint（加密存 token）+ direct provider（引用 endpoint_id）
  - 导入冲突时**重命名加后缀**保留两者
- 现有基础设施已就绪：`POST /api/endpoints`（`CreateEndpointRequest` 含 name/base_url/protocol_type/auth_mode/api_key，后端 `encrypt_api_key` 自动加密）、`POST /api/providers`（create）、`POST /api/providers/{id}/switch`（切换并写工具文件）。
- **ccs 新版（main 分支）用 SQLite 存储**：`~/.cc-switch/cc-switch.db`，`providers` 表 schema 含 `id/app_type/name/settings_config/website_url/category/created_at/sort_index/notes/meta/is_current/...`，复合主键 `(id, app_type)`。开发者本机实测 45 个 claude provider、82 个总计，`settings_config` 与 tauri-migration 分支的 `settingsConfig` 结构一致（Claude Code `settings.json` 全文，`env.ANTHROPIC_BASE_URL`/`ANTHROPIC_AUTH_TOKEN` 明文，新版 env 里多了 `ANTHROPIC_DEFAULT_*_MODEL` 等键）。`provider_endpoints` 关联表（`provider_id/app_type/url`）记录多端点，导入用 `settings_config.env.ANTHROPIC_BASE_URL` 作主 base_url，不消费 `provider_endpoints`。本机 45 个 claude provider 全部有 base_url（无官方登录空 env 项），category 多数为空、少量 `aggregator`/`cn_official`。
- **实现范围扩展决策**：detect 同时探 SQLite db（新版）与 config.json（旧版 tauri-migration），两者并存择一/合并；只导入 claude 渠道，codex 不做（agent-switch 已支持 codex direct，但 codex 导入另开任务）。

## Confirmed Facts

- ccs `Provider` 仅 4 字段：`id/name/settingsConfig/websiteUrl?`（`ref-cc-switch:src-tauri/src/provider.rs:10-22`）。
- ccs 索引路径 `~/.cc-switch/config.json`；config.json 不存在时 ccs 回退空管理器（不修复损坏文件）。
- ccs `settingsConfig` 含 `env.ANTHROPIC_BASE_URL` + `env.ANTHROPIC_AUTH_TOKEN`（明文）；官方登录 preset 为 `env: {}`（空 env，走 OAuth 无 token）。
- agent-switch `DirectSettings`（`src-tauri/src/services/tool_takeover/mod.rs:129-138`）：`{"endpoint_id": <必填>, "model"?, "wire_api"?, "requires_openai_auth"?}`，`endpoint_id` 缺失则切换时反序列化失败、provider 无法激活。
- agent-switch `endpoints` 表 `account_id` 是 `ON DELETE SET NULL` 可空外键（`migrations.rs:59`），导入时无需先建 account。
- `encrypt_api_key`（`src-tauri/src/http/api/mod.rs:22`）：`api_key=Some(k)` → `crypto.encrypt(json!({"api_key":k}), id.as_bytes())`；`api_key=None` → `None`；crypto 不可用返回 503。
- `POST /api/endpoints` create（`src-tauri/src/http/api/endpoints.rs:108`）已自动调 `encrypt_api_key`，导入复用此端点即可完成加密。
- `POST /api/providers` create（`providers.rs:136`）：`id` 自动 uuid，`sort_index` 自动 MAX+1，`meta` 恒 `"{}"`，`is_current` 恒 0，`settings_config` 原样存字符串不校验内部结构。
- `ProviderRow.meta` 是 JSON 字符串，可存来源追溯（`{"imported_from":"ccs","original_id":"...","website_url":"..."}`）。
- `providers::create` 当前硬编码 `meta="{}"`（`providers.rs:154` SQL 无 meta 参数占位走默认）——导入若要写 meta，需确认 create 端点是否接受 meta 字段（`CreateProviderRequest` 当前无 meta 字段，见 `providers.rs:59-68`），可能需扩展请求体或走新导入端点。

## Requirements

- **R1 探测与解析**：自动探测本机 ccs 安装，支持两种数据源：
  - **新版（main 分支）**：`~/.cc-switch/cc-switch.db` SQLite，从 `providers` 表查 `app_type='claude'` 的行。
  - **旧版（tauri-migration 分支）**：`~/.cc-switch/config.json` 文件，解析 `providers` map 取 claude 项。
  - detect 同时探两个源，任一存在即返回其 provider 列表；两者都不存在时提示未检测到 ccs 安装。从每个 provider 的 `settings_config`/`settingsConfig` 的 `env` 提取 `ANTHROPIC_BASE_URL` / `ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_MODEL`。SQLite 源额外可读 `category`/`website_url`/`is_current` 字段（config.json 源无 category）。
- **R2 映射与落库（endpoint + provider 双表）**：每个可导入的 ccs provider 拆成两行：
  - endpoint 行：`name`=ccs name，`base_url`=ccs `env.ANTHROPIC_BASE_URL`，`protocol_type`=`"anthropic"`，`auth_mode`=`"api_key"`，`api_key`=ccs `env.ANTHROPIC_AUTH_TOKEN`（经现有 `encrypt_api_key` 加密），`account_id`=null。
  - provider 行：`app_type`=`"claude-code"`，`mode`=`"direct"`，`name`=ccs name（冲突时加后缀），`settings_config`=`{"endpoint_id":"<新建 endpoint id>"}`，`category`=`"custom"`，`meta`=`{"imported_from":"ccs","original_id":"<ccs id>","website_url":"<ccs websiteUrl 或 null>"}`。
- **R3 冲突处理**：导入时若与本地已有 provider 同名（按 `name` 判定），对新导入 provider **重命名加后缀**（`原名 (ccs)`，仍冲突则 `原名 (ccs 2)` 递增）保留两者；不覆盖、不跳过。endpoint 同名不做特殊处理（endpoint 无唯一约束于 name，但可同样加后缀避免混淆）。
- **R4 官方登录/空 env 项处理**：ccs `env.ANTHROPIC_BASE_URL` 缺失或为空（官方登录 preset）的 provider，**在预览界面标注"无 base_url，无法导入"并默认不勾选**；用户强行勾选时按 base_url 为空建 endpoint（切换时可能无法生效，需在预览界面风险提示）。
- **R5 预览与勾选**：UI 提供导入预览——列出待导入的 ccs provider（name / base_url / api_key 是否存在 / 是否冲突 / 导入后名称 / 是否可导入），用户勾选子集，确认后批量导入。
- **R6 不动 ccs 源数据**：导入只读 `~/.cc-switch/cc-switch.db` 与 `~/.cc-switch/config.json`，不修改、不删除 ccs 的任何文件（SQLite 用只读模式打开）。
- **R7 幂等**：重复导入同一 ccs provider 不产生重复行。去重依据 `provider.meta.imported_from="ccs"` + `meta.original_id=<ccs id>`；已导入的在预览界面标注"已导入"并默认不勾选。
- **R8 导入后可切换生效**：导入的 provider 经 `POST /api/providers/{id}/switch` 切换后，`~/.claude/settings.json` 的 `env.ANTHROPIC_BASE_URL` / `env.ANTHROPIC_AUTH_TOKEN` / `env.ANTHROPIC_MODEL`（若有）被正确写入（即复用现有 direct 切换路径，不新增切换逻辑）。

## Acceptance Criteria

- [ ] AC1 本机已装 ccs 且 `~/.cc-switch/config.json` 存在时，切换器页可见"从 ccs 导入"入口，点击后弹出预览列表。
- [ ] AC2 预览列表正确展示每个 ccs provider 的 name、`env.ANTHROPIC_BASE_URL`、api_key 是否存在、冲突状态、导入后名称、是否可导入（空 env 项标注"无 base_url"）。
- [ ] AC3 勾选若干 provider 并确认导入后，`endpoints` 表新增对应行（含加密 `api_key_encrypted`），`providers` 表新增 direct 模式行，`settings_config` 含 `endpoint_id` 指向新建 endpoint，`meta` 含 `imported_from=ccs` 与 `original_id`。
- [ ] AC4 与本地同名 provider 冲突时，导入项被重命名加后缀且不覆盖本地项；本地与导入项均保留。
- [ ] AC5 导入完成后切换到该 provider，`~/.claude/settings.json` 的 `env.ANTHROPIC_BASE_URL` / `env.ANTHROPIC_AUTH_TOKEN` 被正确写入（导入的配置可实际生效）。
- [ ] AC6 重复执行导入，已导入过的 ccs provider（按 `meta.original_id` 匹配）被识别为"已导入"并默认不勾选，不产生重复行。
- [ ] AC7 导入过程不修改 `~/.cc-switch/config.json`（仅读）。
- [ ] AC8 本机未装 ccs（无 `~/.cc-switch/config.json`）时，入口点击后给出明确提示而非崩溃。
- [ ] AC9 crypto 服务不可用（Keychain 降级模式）时，导入含 api_key 的 provider 给出明确错误提示而非静默失败或写入明文。

## Out of Scope

- Codex 渠道导入（ccs 新版 SQLite 有 `app_type='codex'` 的 provider，agent-switch 也支持 codex direct；但本期只做 claude，codex 导入另开任务）。
- 反向导出到 ccs。
- ccs main 分支 `category` 字段的兼容导入（tauri-migration 分支无此字段；serde 默认忽略未知字段，但本次不主动处理 main 分支元数据）。
- 导入时自动激活某个 provider（导入只创建，不切换；用户导入后自行在切换器页点切换）。
- ccs `includeCoAuthoredBy` 等顶层非 env 键的迁移（agent-switch direct 模式当前只写 env 三键，`includeCoAuthoredBy` 等额外键暂不处理，记入 meta 供后续演进）。

## Open Questions

- 无（当前所有阻塞决策已通过证据探查与用户决策解决；若 design 阶段发现新分支再补）。
