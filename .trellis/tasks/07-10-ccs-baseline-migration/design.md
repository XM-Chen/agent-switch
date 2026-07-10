# Design：以 ccs v3.16.5 为基线重建 Agent Switch

## 1. 设计目标与边界

本设计解决四个相互耦合的问题：

1. 在**当前工作目录**把 Git checkout 从旧 `agent-switch/main` 安全切到以 ccs `v3.16.5` 为根的 `agent-switch-ccs`，而不重写/污染 `main`。
2. 在独立 Git 根之间迁移 Trellis 工作流与本任务，但不把旧 agent-switch 产品源码或失效的产品规范误当成 ccs 规范。
3. 从完整 ccs 建立“Claude Code + Windows + 简体中文”裁剪架构，同时保住 Claude provider 复用的协议适配能力。
4. 把 ccs 身份系统性改为 Agent Switch 0.3.0，替代旧 Agent Switch 安装线但与原版 CC Switch 完全隔离。

不在本设计中预设移植旧 agent-switch 的 AES 凭据加密、portability、高级路由或本地 HTTP API；这些后续按独立需求评估。

## 2. Git 拓扑与工作目录切换

### 2.1 分支拓扑

```text
旧 agent-switch 根
  main ── ... ── 7e906685e                （保留，不重写）

ccs 根
  ... ── 8d1b3306 (v3.16.5)               （纯基线/回滚锚点）
             \
              agent-switch-ccs
                 ├─ Trellis 迁入提交
                 ├─ 裁剪批次提交...
                 ├─ 身份改造提交...
                 └─ Agent Switch 0.3.0
```

`agent-switch-ccs` 直接从 peeled commit `8d1b3306d09a27b9d8fc29694791d8421aba5f93` 创建，不通过 merge 把两棵历史强行接起来。`main` 仍能随时 `git switch main` 恢复旧产品树。

### 2.2 当前目录切换前的保护

当前工作树有三类未跟踪内容，不能依赖 Git checkout “通常会保留”这一偶然行为：

- `.trellis-upgrade-audit.json`
- `.trellis/archive/`
- `.trellis/tasks/07-10-ccs-baseline-migration/`

执行时先复制到仓库外、带时间/任务标识的 bootstrap 备份目录，并生成清单/哈希。备份成功后才允许 `git switch -c agent-switch-ccs 8d1b3306...`。禁止在主树执行 `reset --hard` 或 `clean`。

切换后先检查 HEAD/版本/工作树，再迁回 Trellis；如果切换失败，保持 `main` 不动并从备份恢复。

## 3. Trellis 迁移设计

### 3.1 迁移内容分层

| 层 | 处理方式 | 原因 |
|---|---|---|
| `.trellis/scripts/`、`workflow.md`、agents、config | 从 `main` 恢复 | 平台中立的任务工作流 |
| 当前任务与 task archive/journal | 从外部 bootstrap 备份 + `main` 恢复 | 保留决策、研究与审计链 |
| `.trellis/spec/` | **不可直接作为新产品规范使用**；迁入后标记/归档旧规范并基于 ccs 重建 | 现规范描述旧 agent-switch 架构，会误导 ccs 实现 |
| `.claude/`、`.codex/` 等平台目录 | 运行 Trellis init/update 重新生成 | 被 `.gitignore` 忽略，不能靠 Git 提交迁移 |
| 根 `.gitignore` | 以 ccs `.gitignore` 为基础合并 Trellis 平台忽略规则 | 不能用旧 `.gitignore` 覆盖 ccs 规则 |

### 3.2 提交边界

1. **纯基线点**：`8d1b3306…`，不新增提交。
2. **Trellis bootstrap 提交**：仅 Trellis/任务/合并后的 ignore；不包含任何旧 `src/`、`src-tauri/`、package manifest。
3. **Spec refresh 提交**：把旧产品规范归档为 legacy reference 或替换为基于 ccs `v3.16.5` 的新规范索引。该提交完成前禁止启动产品裁剪子任务。
4. 后续每个产品批次独立提交。

若用户尚未授权 commit，执行可以先停在 staged/working-tree 状态并请求确认；但逻辑边界仍按上述提交划分。

## 4. 父子任务架构

当前任务作为**父任务**，持有源需求、决策、任务图、跨批次验收与最终集成 review；不直接承载全部产品改动。

子任务（1 已创建 `07-10-ccs-baseline-bootstrap` 并 link；2–6 待地基完成后创建）：

| 顺序 | 子任务 | 交付物 | 依赖 |
|---|---|---|---|
| 1 | `ccs-baseline-bootstrap`（已建） | 分支切换、纯基线验证、Trellis 迁入、spec refresh | 无 |
| 2 | `ccs-claude-only-trim` | 删除 6 个非 Claude 应用链路，保留 Claude 复用的 OAuth/Copilot/OpenRouter/Responses 适配；schema v11 暂不收缩 | 1 |
| 3 | `ccs-windows-zh-trim` | 仅 Windows、仅简体中文、删除 OpenClaw workspace、社区/赞助/多平台资产 | 1；可与 2 分批但最终需联调 |
| 4 | `ccs-agent-switch-identity` | Agent Switch 品牌、0.3.0、`~/.agent-switch`、`agentswitch://`、安装身份、LICENSE/来源说明 | 2、3 后更稳妥 |
| 5 | `ccs-updater-release` | 自有公钥/endpoint/Windows x64 release、MSI + `.sig` + `latest.json` | 4 |
| 6 | `ccs-retained-features-regression` | Provider/proxy/usage/MCP/Prompts/Skills/Sessions/Deep Link 全链路回归与残留扫描 | 2–5 |

删除规模预计约 130 个文件、25k–30k 行；不能压成一个“删掉其他客户端”的提交。

## 5. 功能裁剪架构

### 5.1 AppType 与 UI 收缩

最终产品级 app 集合只暴露 Claude Code：

```text
Frontend AppId / VALID_APPS: ["claude"]
Backend AppType runtime paths: Claude only
DB app_type columns/schema: retained for compatibility, only Claude rows created/read
```

删除 AppSwitcher 或降为无切换单应用壳；Provider forms、presets、settings、commands、session providers、MCP projections、usage adapters、Deep Link app branches均按 `research/ccs-v3.16.5-trim-map.md` 的顺序处理。

### 5.2 不可按名称删除的 Claude 依赖

以下名称看似属于 Codex/OpenAI，但被 Claude provider 的不同上游 API 复用，必须保留并建立回归测试：

- `ProviderType::CodexOAuth`
- `ProviderType::GitHubCopilot`
- `ProviderType::OpenRouter`
- `commands/codex_oauth.rs`、`commands/copilot.rs`
- `proxy/providers/codex_oauth_auth.rs`
- `proxy/providers/copilot_auth.rs`、`copilot_model_map.rs`
- `services/codex_oauth_models.rs` 及两类账号/配额服务
- 前端 `CodexOAuthSection`、`CopilotAuthSection`、认证中心与 Claude Provider 表单接线
- `streaming_responses.rs`、`transform_responses.rs`
- Claude 上游实际引用的 Codex Chat/Copilot streaming/transform 模块（实施时按引用闭包确认，不能按文件名前缀批删）

原则：删除的是**Codex 客户端集成**，不是 Claude provider 能使用的**上游协议/托管账号认证类型**。最终 UI 将托管账号上游放在 Claude Provider 创建/认证中心内，不出现独立 Codex 客户端导航。

### 5.3 数据库策略

首期保持 schema version 11：

- `providers/prompts/provider_health/proxy_request_logs/stream_check_logs` 的 `app_type` 结构保留。
- `mcp_servers` / `skills` 的非 Claude enable 列先停止读写但不 drop。
- `proxy_config` 的多 app 结构先保持，业务只生成 Claude 行；不为表面整洁做破坏性迁移。
- 稳定后如确有收益，再另开 schema compact 任务。

此策略让 ccs 原始 DB 初始化/迁移测试继续发挥作用，降低一次同时改 UI、服务和数据库的风险。

### 5.4 保留模块的单应用化

- **Provider**：保留预设、普通切换、Common Config、回填、健康检测。
- **Proxy**：保留 Claude adapter、热切换、故障转移、格式转换、请求日志和用量采集。
- **MCP**：只保留 Claude live 投影和导入。
- **Prompts**：只同步 CLAUDE.md。
- **Skills**：只投影 Claude Code skills 目录；保留安装/更新/SSOT 行为。
- **Sessions**：只扫描/恢复 Claude Code 会话。
- **Deep Link**：资源类型保留 provider/prompt/mcp/skill，app 固定 Claude；scheme 后续改为 `agentswitch://`。
- **Usage**：只保留 Claude 请求与会话来源，保留定价/图表/日志能力。

### 5.5 Claude Code 全文快照与三层 live 语义

当前 ccs 基线已经具备目标模型，不从旧 agent-switch 迁入 `meta.snapshot`：

```text
DB provider.settings_config（供应商完整 settings.json 快照）
  + 可选 Common Config（全局共享层，深合并覆盖）
  → sanitize ccs 内部字段
  → 用户级 ~/.claude/settings.json（live，整文件替换）
```

切换时先读取当前 live，将用户在 Claude Code 或外部编辑器中产生的变更回填到切出方 `settings_config`，并剥离属于 Common Config 的共享字段；随后构建目标方有效快照并写入 live。`settings_config` 保持 `serde_json::Value` / 任意 JSON 对象，不建立固定 Claude settings schema，因此 hooks、permissions、statusLine、sandbox、未来新增字段等都可随快照往返。结构化表单只编辑自己拥有的路径，不能以重新构造对象的方式删除未知键。

`meta` 只存 ccs 内部供应商元数据（例如 Common Config 启用状态、provider type），不存第二份 settings 快照。这样保持一个配置 SSOT，复用 ccs 的 backfill/Common Config/代理接管锁与既有测试。

### 5.6 精选官方 Provider 目录

首期不直接继承 ccs 的商业合作目录。`claudeProviderPresets.ts` 收缩为两类：Anthropic Official，以及经人工确认具备官方站点、官方文档和官方 Claude Code 兼容接入方式的主流模型厂商模板。聚合/中转服务、邀请注册链接、合作徽章、促销排序和来源跟踪字段全部删除。

模板只是创建 `settings_config` 的便捷初值；用户仍能通过裸 JSON 编辑器创建任意完整快照。官方模板清单在实施子任务中逐项列出证据并测试，不因原 ccs 的 `isPartner`/`category` 标记自动判定是否保留。

### 5.7 本地代理监听与鉴权

保留 ccs 代理：`127.0.0.1` 默认监听、热切换、`openai_chat`/`openai_responses`/`gemini_native` 格式转换、熔断与故障转移、用量与请求日志、Claude Desktop gateway。默认 loopback 无需鉴权。

当用户把监听地址改为非 loopback（`0.0.0.0`、局域网 IP、非 `::1` 的 IPv6）时，鉴权必须覆盖全部转发路由，而非当前只校验 Claude Desktop gateway。实现思路：在 axum 路由最外层加一个基于本地 token 的中间件，监听地址为 loopback 时放行，非 loopback 时校验 `Authorization: Bearer <local-token>`，缺失或不匹配返回 401。本地 token 复用或扩展 `claude_desktop_config::get_or_create_gateway_token` 的生成与存储，不要求用户手填。前端保存非 loopback 地址前弹出风险确认并持久化标志，确保只有显式同意才进入 LAN 模式。

## 6. Windows 与简体中文裁剪

### 6.1 Windows-only

删除 Linux/macOS/移动端专属资产和 release matrix，但保留被编译条件保护且删除收益低的共享抽象，直到引用归零：

- 删除 `flatpak/`、`Info.plist`、iOS/Android/macOS tray icon、Linux-only `linux_fix.rs`。
- release CI 只保留 Windows x86_64；ARM64 不作为首期门。
- `src/lib/platform.ts` 收缩为 Windows 语义。
- Windows 安装器继续评估 ccs per-user WiX 模板；若保留，需显式固定 `upgradeCode`。

### 6.2 zh-CN only

- 保留 `src/i18n/locales/zh.json`。
- 删除 en/ja/zh-TW locale 和语言切换 UI。
- i18n runtime 可暂留并固定 `zh`，避免一轮内把约 2920 条 key 全部改成硬编码。
- README/用户文档以中文为唯一维护版本。

## 7. 备份与同步保留策略

保留 ccs 的两套恢复面：

- 本地 `~/.agent-switch/backups/*.db` SQLite 一致性快照，支持周期、保留数量、手动恢复及恢复前安全备份；
- WebDAV/S3 v2 同步，继续使用 `manifest.json` + `db.sql` + `skills.zip`，并保留自动同步。

Claude-only 裁剪只清理同步数据中的非 Claude 业务分支，不删除协议/transport/冲突校验。首期不改变 ccs 的明文 artifact 契约，因此远端对象内容不具备端到端保密性：TLS 只保护传输，S3/WebDAV 鉴权只控制访问，远端管理员仍可能读取 SQL 中的 API token。设置页在首次启用任一远端同步方式前必须展示具体风险并持久化显式确认；同步目标应改用 Agent Switch 独立 remote root，不能与 CC Switch 共用路径。

## 8. Agent Switch 身份与数据隔离

### 8.1 身份映射

| 维度 | ccs v3.16.5 | 目标 |
|---|---|---|
| productName | `CC Switch` | `Agent-Switch` |
| identifier | `com.ccswitch.desktop` | `com.agent-switch.app` |
| npm/Cargo | `cc-switch` / `cc_switch_lib` | `agent-switch` / `agent_switch_lib` |
| version | `3.16.5` | `0.3.0` |
| data root | `~/.cc-switch` | `~/.agent-switch` |
| DB/log | `cc-switch.db` / `cc-switch.log` | `agent-switch.db` / `agent-switch.log` |
| scheme | `ccswitch://` | `agentswitch://` |
| updater | `farion1231/cc-switch` + ccs pubkey | `XM-Chen/agent-switch` + Agent Switch pubkey |

### 8.2 数据根

保持 ccs 的 home-hidden-dir 结构，只做根名称与派生文件名改造，避免在第一阶段同时改成旧 agent-switch 的 OS AppData 架构。所有路径通过单一 `get_app_config_dir()` / 常量派生；禁止散落硬编码。

首次启动始终在 `~/.agent-switch` 创建空产品库，不读取：

- `~/.cc-switch`
- 旧 Agent Switch OS AppData DB

但产品库为空不代表覆盖 Claude Code 当前状态。启动顺序沿用 ccs 的 import-before-seed：先读取用户级 `~/.claude/settings.json` 全文，导入为 current `default` provider，再添加精选官方模板。若 live 缺失则仅 seed；若 live 解析失败或处于代理接管占位状态，则阻止任何会覆盖它的自动切换并向用户报错。未来从 ccs/旧 Agent Switch 导入产品数据必须显式、可预览、可回滚，另开任务。

### 8.3 Windows 安装升级身份

Tauri v2 文档确认 MSI `upgradeCode` 必须跨版本保持不变，默认由 `productName` 派生。为确保 0.3.0 能替代已安装旧 Agent Switch：

1. 在旧 Agent Switch 身份配置上运行 `pnpm tauri inspect wix-upgrade-code`（或使用对应 Tauri CLI）记录默认值。
2. 在新 ccs 基线的 `bundle.windows.wix.upgradeCode` 显式固定该 GUID。
3. identifier 保持 `com.agent-switch.app`，产品名使用 Agent-Switch；不使用 CC Switch 的 installer identity。

不要假定仅改 identifier 就能保持旧 MSI 升级线。

## 9. Deep Link 与 Updater

### 9.1 Deep Link

系统只注册 `agentswitch`：

```text
agentswitch://v1/import?resource=provider|prompt|mcp|skill&...
```

需要同步修改 Tauri scheme、注册/事件处理、parser、前端文案/placeholder、持久化 `source_protocol`、单元与集成测试。`ccswitch://` 不注册；是否允许在应用内粘贴解析作为兼容层由 Deep Link 子任务决定。

### 9.2 Updater

- `createUpdaterArtifacts: true`。
- `pubkey` 为 Agent Switch 公钥内容，不是路径。
- endpoint：`https://github.com/XM-Chen/agent-switch/releases/latest/download/latest.json`。
- 私钥仅通过 `TAURI_SIGNING_PRIVATE_KEY` / password 环境变量注入，不写 `.env`、仓库或日志。
- Windows x64 bundle生成 MSI 与 `.sig`；`latest.json` 的 `signature` 是 `.sig` 文件内容。
- static manifest 最少包含 `version`、`platforms.windows-x86_64.url`、`signature`，整份 manifest 会在版本比较前校验。
- `pnpm tauri build --no-bundle` 可先验证 release executable，不需要 installer/updater artifact；完整发布门仍需匹配公私钥、MSI + `.sig` + `latest.json` 端到端验证。
- Authenticode 与 updater minisign 无关；个人自用首期不要求 Authenticode。

## 10. 原样基线验证与定制后验证

### 10.1 Untouched ccs gate

精确命令见 `research/ccs-v3.16.5-validation-gates.md`。核心门：

```text
source/version/cleanliness
pnpm install --frozen-lockfile
pnpm typecheck
pnpm format:check
pnpm test:unit
pnpm build:renderer
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo test --locked
cargo check --locked
pnpm tauri build --no-bundle
完整 bundle（凭据可用时）
隔离环境安装/启动 smoke
```

### 10.2 每个裁剪批次

- TypeScript typecheck + format + Vitest。
- Rust fmt + Clippy `-D warnings` + tests。
- 前端引用/路由残留扫描。
- Rust `mod`/command invoke handler 残留扫描。
- 非 Claude app ID / path / locale / platform asset 专项扫描。
- 保留模块的针对性行为验证。

### 10.3 最终端到端

在 Windows 隔离测试环境验证：

1. 安装/升级身份不会覆盖 CC Switch。
2. 首启仅创建 `~/.agent-switch`，不读写 `~/.cc-switch` 或旧 DB。
3. Provider 直连/代理切换、格式翻译、故障转移与日志。
4. Usage/成本数据链。
5. MCP/Prompts/Skills 投影。
6. Sessions 浏览/恢复。
7. `agentswitch://` 四类资源导入。
8. 自有 updater 检测、下载、签名验证（发布时）。

## 11. 风险与回滚

| 风险 | 缓解 | 回滚点 |
|---|---|---|
| 当前目录切分支导致 Trellis 未跟踪文件丢失 | 外部 bootstrap 备份 + 清单/哈希后才切换 | `git switch main` + 恢复备份 |
| 旧 spec 误导新 ccs 实现 | Trellis 迁入后先归档/刷新 spec，未完成前禁止产品开发 | 回退 Trellis/spec 提交 |
| 误删 Claude 复用的 Codex/OpenAI 适配 | 基于引用与 Claude translation tests，保留指定 ProviderType/Responses 模块 | 回退对应裁剪批次 |
| 同时收缩 DB 导致迁移爆炸 | 首期保持 schema v11 和未用列 | 无 DB schema 回滚需求 |
| 改品牌后 MSI 不升级旧 Agent Switch | 显式固定旧 Agent Switch UpgradeCode | 回退 installer identity 提交 |
| 新版污染 CC Switch 数据 | 单一 `~/.agent-switch` 根 + 全局残留扫描 + 隔离 smoke | 回退 identity/data-root 提交 |
| updater 接受 ccs 发布或签名失配 | endpoint + pubkey 一起替换；端到端签名测试 | 回退 updater 提交，停发 release |
| 后续同步 ccs 冲突巨大 | 按 release 手动同步；每次先建同步分支、看功能矩阵、跑完整回归 | 放弃该次 merge，保留上次验证 SHA |

## 12. 技术选择总结

- 以 ccs release commit 为 Git 根，不做快照覆盖提交。
- 当前目录直接切分支，但先外部备份 Trellis 未跟踪内容。
- Trellis 工作流迁入，旧产品 spec 不直接沿用，先 refresh。
- 先完整 ccs、后裁剪；裁客户端而不裁 Claude 可复用上游协议。
- schema v11 暂不物理收缩。
- ccs home-dir 语义改名为 `~/.agent-switch`，不迁旧数据。
- Agent Switch 0.3.0 复用旧安装身份，显式固定 WiX UpgradeCode。
- Windows x64 + zh-CN + Claude Code；所有保留外围模块单应用化。
