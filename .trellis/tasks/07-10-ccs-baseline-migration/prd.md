# 以 ccs 为新基线重建 Agent Switch 定制分支

## Goal

在当前 `agent-switch` Git 仓库中新建本地分支 `agent-switch-ccs`，以 cc-switch（下称 `ccs`）正式版 `v3.16.5` 作为完整产品基线；先原样验证该基线，再在其上有计划地裁剪功能、替换品牌/身份、加入用户自定义需求，最终形成一个仅面向 Windows、仅简体中文、仅 Claude Code、个人自用的独立 Agent Switch。当前 `agent-switch/main` 原样保留为可回退的旧实现。

## Background

- 当前仓库 `XM-Chen/agent-switch`（远程 `origin`），当前分支 `main` = `7e906685e`，比 `origin/main` 超前 1 个提交（Trellis 0.6.6 升级）。工作树含 3 个未跟踪路径：`.trellis-upgrade-audit.json`（188K）、`.trellis/archive/`（3.3M）、`.trellis/tasks/07-10-ccs-baseline-migration/`（本任务）。实际操作前必须保护这些内容，禁止 checkout/reset/clean 丢失。
- 仓库已配置只读参考远程 `ref-cc-switch = https://github.com/farion1231/cc-switch.git`，无需嵌套复制 `.git`。已 fetch 官方 annotated tag `v3.16.5`（tag 对象 `a58917a5…`，peeled commit `8d1b3306d09a27b9d8fc29694791d8421aba5f93`，`chore(release): v3.16.5`，作者时间 2026-07-01）。该提交三处版本（`package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json`）均为 `3.16.5`。ccs 采用 MIT License（`Copyright (c) 2025 Jason Young`）。
- ccs `v3.16.5` 官方身份：显示名 `CC Switch`、identifier `com.ccswitch.desktop`、npm 名 `cc-switch`、Cargo crate `cc-switch`（lib `cc_switch_lib`）、数据根 `~/.cc-switch`（`cc-switch.db` / `backups/` / `skills/` / `logs/cc-switch.log` / `crash.log` / 旧 `config.json`）、Deep Link scheme `ccswitch://`、updater 指向 ccs 官方 GitHub `latest.json` 与内置 ccs 公钥、自定义 per-user WiX 模板 `src-tauri/wix/per-user-main.wxs`。本机已存在真实 `~/.cc-switch`，任何原样运行 smoke 必须隔离（独立 OS 用户 / VM / Windows Sandbox）以免污染。
- ccs 工具链：pnpm（CI 用 10.12.3）、Node 20、Rust 1.95（`rust-toolchain.toml`，MSRV 1.85）、Tauri 2.8.2、React 18、Tailwind 3、Vitest。当前旧 agent-switch 用 npm、React 19、Tailwind 4，并自研代理路由与 AES 加密凭据。两者为独立 Git 根，采用 ccs 基线后旧 agent-switch 产品代码不会自动保留。
- 首期裁剪不是简单“删掉非 Claude 名字”：ccs 的 `ProviderType::CodexOAuth`、`GitHubCopilot`、`OpenRouter` 以及 `streaming_responses.rs` / `transform_responses.rs` 已确认被 Claude Code 的托管账号上游与协议翻译复用，必须保留（**D25**）；数据库首期保留 schema v11 和未用多应用列，仅让代码不再读写非 Claude 数据，降低破坏性迁移风险。`workspace` view/`commands/workspace.rs` 专用于 OpenClaw，随 OpenClaw 裁剪。
- 相邻目录 `../cc-switch` 是脏工作副本（相对其 `origin/main` ahead 4 / behind 202，含大量改动与未跟踪 Trellis 文件，读取时版本 3.15.0）。仅可用于源码研究，不能作为干净基线。
- 历史目标曾是“在 agent-switch 架构上吸收 ccs 缺失功能”；本次请求把方向反转为“先完整继承 ccs，再删增”。该转换必须显式管理，`main` 仅作旧实现参考与回退，不做无差别覆盖合并。
- 研究产物：`research/ccs-v3.16.5-validation-gates.md`（Windows 原样基线验证门与环境阻塞）、`research/branch-trellis-identity-plan.md`（分支/Trellis 迁移/身份映射，含 file:line 锚点）。

## Requirements

### R1. 分支与旧实现隔离

- 新工作发生在本地分支 `agent-switch-ccs`（**D9**），直接从 ccs `v3.16.5` 提交 `8d1b3306…` 创建（**D1/D11**）；`main` 的提交历史和可恢复性不得被覆盖或重写。
- 按用户选择在**当前 `E:/SynologyDrive/git_files/agent-switch` 目录直接切换到新分支**（**D20**），不创建长期相邻 worktree。因此切换前必须先把本任务与未跟踪 Trellis 升级文件安全迁出/提交到可恢复位置，再 checkout；切换后旧 `main` 仍可通过 `git switch main` 回来，但同一时刻当前目录只展示一个产品树。
- 保护主树 3 个未跟踪路径不因分支操作丢失。
- 未经单独授权不推送分支、不改 `origin` 默认分支、不发布安装包（**D8/D9/AC8**）。

### R2. ccs 完整且可追溯的原样基线

- 分支起点固定为官方 tag `v3.16.5` = commit `8d1b3306d09a27b9d8fc29694791d8421aba5f93`（**D11**），不从未发布的最新 `main` 起步；不使用脏的 `../cc-switch`。
- 产品源码基线完整包含该提交的全部受控文件，不嵌套 `.git`，保留 MIT `LICENSE` 与 Jason Young 原版权声明（**D19**）。
- 保留一个纯 ccs 基线提交点（=起点 commit，可选本地 tag `agent-switch-ccs-baseline`）作为回滚锚点；Trellis 迁移与身份改造都在其后的独立提交（**D10**）。

### R3. 原样基线验证门（裁剪/身份改造之前）

- 在干净、精确位于 `8d1b3306…` 的工作树上，按 ccs 官方方式验证：`pnpm install --frozen-lockfile` → `pnpm typecheck` / `pnpm format:check` / `pnpm test:unit` / `pnpm build:renderer` → `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`（`--manifest-path src-tauri/Cargo.toml`）→ Windows x86_64 `pnpm tauri build`。
- 环境阻塞（pnpm 未激活、Node 22 vs 官方 20、Rust 1.95 未装、完整 updater 打包的私钥缺失、VBSCRIPT/MSI、网络等）必须如实记录真实阻塞，不得笼统写“环境问题”，不得把跳过伪装成通过。
- 可先用 `pnpm tauri build --no-bundle` 验证 release 可执行文件而不生成 installer/updater artifact；根据 Tauri v2 当前文档，`--no-bundle` 是官方 CLI 路径。完整 MSI + `.sig` 仍需自有 updater 私钥，并在身份/公钥替换后做端到端签名验证。
- 原样运行 smoke 若执行，必须隔离本机 `~/.cc-switch`。
- 不得为绕过失败而在验证阶段修改 `createUpdaterArtifacts`、updater 公钥/endpoint、bundle target 或 WiX 模板。

### R4. 定制范围先清单化再修改

- 删除功能前建立“保留 / 删除 / 修改 / 新增 / 待定”功能矩阵（**AC5**），每个删除项注明依赖与验收方式，避免按目录直觉粗删导致隐性依赖损坏。
- 矩阵至少覆盖：客户端集合、Provider 管理、代理/故障转移/翻译、OAuth、MCP、Prompts、Skills、Sessions、Deep Link、用量/成本、同步/备份/更新、i18n、平台。裁剪须按依赖顺序清理 UI → 后端服务 → 数据库/迁移 → 配置路径 → Deep Link/导入导出 → 测试，不能只隐藏入口。推荐详细删除顺序与约 130 个候选文件见 `research/ccs-v3.16.5-trim-map.md`；其中明确禁止删除 Claude 路径复用的 CodexOAuth/Copilot/OpenRouter provider types 与 Responses 翻译模块。
- 用户自定义需求形成独立、可验收的需求项；不得在“完整复制 ccs”步骤混入未确认的产品改动（**Out of Scope**）。

### R5. 首期产品范围（裁剪目标）

- **客户端**：仅 Claude Code（**D3**）。删除或降级 Codex、Gemini CLI、OpenCode、Claude Desktop、OpenClaw、Hermes 的集成、投影、开关与适配分支；但不得按名字误删 Claude provider 可复用的 `ProviderType::CodexOAuth` / `GitHubCopilot` / `OpenRouter` 和 Responses 格式翻译代码。OpenClaw 专属 `workspace` view 与 `commands/workspace.rs` 一并删除。
- **数据库策略**：首期保留 ccs schema v11 与多应用列/通用 `app_type` 结构，只允许新代码创建/读取 Claude 数据；不为“列名干净”引入破坏性表迁移。待产品稳定后再评估 schema 收缩。
- **核心模块保留**：Claude Code 的 Provider 配置/预设/切换、本地代理（热切换/格式转换/故障转移等子能力待矩阵细化）、用量/请求日志/Token 与成本统计（**D4**）。
- **Claude Code 配置快照语义**：沿用 ccs 原生模型（**D21**）。`provider.settings_config` 直接保存完整、任意 JSON 形状的用户级 `~/.claude/settings.json` 快照，不新增 `meta.snapshot` 第二份快照。切出当前供应商时从 live 整体回填并剥离 Common Config；切入目标供应商时将其 `settings_config` 与启用的 Common Config 深合并，经 ccs live sanitizer 仅剥离内部字段后整体写入。未知/未来字段必须原样往返；不得把快照收缩成只含 `env.ANTHROPIC_*` 的固定 schema。旧 agent-switch 0.2.2 的 `meta.snapshot` 方案仅是旧端点引用架构的历史实现，不迁入本分支。
- **内置 Provider 预设**：采用精选官方目录（**D22**）。保留 Anthropic 官方与经人工确认的主流模型厂商官方 Claude Code 兼容模板；删除聚合/中转商模板、合作伙伴排名/徽章、促销文案及 `aff`/`ref`/`utm_*`/`ccswitch` 等返利或来源跟踪参数。用户始终可通过完整 JSON 自定义任意供应商，内置目录不构成对第三方可用性或安全性的背书。
- **备份与跨设备同步**：保留 ccs 全部能力（**D23**），包括本地 SQLite 定时/手动备份与恢复、WebDAV、S3、自动同步及 Skills 同步包。沿用 ccs v2 manifest + `db.sql` + `skills.zip` 协议；首期不新增客户端内容加密。由于 `db.sql` 会包含 `providers.settings_config` 中的 API token，启用 WebDAV/S3 前必须明确提示“远端存储管理员或凭据持有者可读取供应商密钥”，并要求用户显式确认；不得仅描述为网络流量风险。
- **首次启动与现有 Claude 配置保护**：新产品数据库从空库开始，但不忽略用户现有 `~/.claude/settings.json`（**D24**）。沿用 ccs 的 import-before-seed：若 Claude provider 表尚无非官方条目且 live 未被代理接管，先将当前完整 live JSON 导入为 `default` 并设为 current，再追加精选官方 seed；不读取 `~/.cc-switch` 或旧 Agent Switch DB。首次切换前必须保证该导入成功或向用户明确提示无法保全，禁止静默覆盖现有 live。
- **Claude Code 托管账号上游**：保留 GitHub Copilot 与 ChatGPT Codex OAuth 两条完整链路（**D25**），包括 OAuth 登录/账号绑定、配额与模型发现、认证刷新、Provider 特殊类型、代理路由及 `openai_chat`/`openai_responses` 转换；它们只作为 Claude Code Provider 的上游，不恢复 Codex 客户端、Codex live 配置或 Codex 会话入口。裁剪时按能力归属而非文件名判断，`commands/copilot.rs`、`commands/codex_oauth.rs`、相关 auth/model/quota 模块列入保护清单。
- **Agents 占位页**：删除 ccs 的 `AgentsPanel` 导航和占位组件（**D26**）。当前实现只有 “Coming Soon”，没有 `~/.claude/agents/*.md` 管理链路；首期不伪装为可用功能。Claude Code 自定义 Agents 管理属于后续独立需求，不影响 Sessions 中读取/显示 subagent 会话日志。
- **本地代理监听与鉴权**：保留代理、热切换、格式转换、故障转移、用量/请求日志、Claude Desktop gateway 等能力（**D27**），默认并推荐绑定 `127.0.0.1`。允许用户在设置页改监听地址；当监听地址不是 `127.0.0.1`/`localhost`/`::1` 时，所有转发端点（`/v1/messages`、`/v1/chat/completions`、`/v1/responses`、`/responses/compact`、`/v1beta/*`、Claude Desktop gateway 等）必须先校验本地 token 后再转发，不得只对 Claude Desktop gateway 鉴权。切换到非 loopback 或保存该配置前必须持久化风险确认。
- **外围模块保留但单应用化**：MCP、Prompts、Skills、Sessions、Deep Link 全部保留，仅服务 Claude Code；裁掉其中面向其他客户端的投影/开关/适配，保持 Claude Code 单应用链路完整（**D5**）。
- **平台**：仅 Windows（**D8**）。裁掉 macOS、Linux/Flatpak/Wayland 专属打包、文档、CI 与代码路径；被 Windows 依赖的跨平台抽象可暂留但不承诺支持。
- **语言**：仅简体中文（**D12**）。删除其他 locale、语言切换 UI 与对应文档/截图；i18n 抽象是否保留由设计按删改成本决定。
- **首期实现以 ccs 行为为准**（**D6**）：凭据存储、备份、路由、HTTP API 等先采用 ccs 现有实现与语义；旧 agent-switch 的 AES 加密、portability、高级路由、本地 HTTP API 不列为首期必须移植项，后续用户提出时再作独立需求。**AC6** 的“显式移植项”在首期为空集，但仍必须显式记录该结论，而非默认旧能力还在。

### R6. 产品身份改造（独立于 ccs、替代旧 Agent Switch）

在原样验证通过后的独立提交中完成，同步修改 `package.json` / `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml` 三处版本与身份：

- **品牌**：显示名 `Agent-Switch`，移除 ccs 名称/图标/截图/赞助商/联盟链接/Funding/社区宣传，替换为 Agent Switch 自有中文资产（**D19**）；保留 MIT `LICENSE`、Jason Young 版权，并在开发文档/关于页注明“基于 CC Switch v3.16.5 修改”。
- **安装身份**：复用旧 Agent Switch 身份使新版成为其后继——identifier `com.agent-switch.app`、Cargo crate `agent-switch`（lib `agent_switch_lib`）、npm `agent-switch`、Windows AUMID 用 Agent Switch 专属值；显式固定 WiX `upgradeCode` 为旧 Agent Switch 安装线的值（若旧版未显式设置，先用 `pnpm tauri inspect wix-upgrade-code` 在旧身份上取默认值并记录），避免未来 `productName`/模板变化意外改变升级关系。`com.agent-switch.app` 与 CC Switch 不同，避免覆盖本机 CC Switch（**D7/D16**，Tauri v2 文档确认 `upgradeCode` 必须跨版本保持不变，默认值由 `productName` 派生而非 identifier）。
- **版本**：`0.3.0`（**D17**），三处同步；ccs 来源版本仅记录于关于页/文档，不作为产品 semver。
- **数据目录**：`~/.agent-switch`（**D15**，数据根），把 ccs 全部 `~/.cc-switch` 派生路径（`config.rs` 根与 Windows legacy fallback、`database/{mod,backup}.rs`、`panic_hook.rs`、`lib.rs` 日志、`skill.rs`、`env_manager.rs`、`hermes_config.rs`、`openclaw_config.rs`、`codex_history_migration.rs`、`CC_SWITCH_TEST_HOME` 等）整体改名到 `~/.agent-switch`，禁止读取 `~/.cc-switch` 或旧 agent-switch DB。首启为全新空库，不自动迁移任何来源数据（**D15**）；未来导入另开显式、可预览的任务。
- **Deep Link**：注册 `agentswitch://`（**D18**），替换 Tauri 配置、backend parser、runtime `starts_with`、前端文案、持久化 `source_protocol` 及测试（旧测试把 `agentswitch://` 断言为非法，需反转）；`ccswitch://` 不注册为系统处理程序，是否作为应用内粘贴兼容输入由 Deep Link 子任务定。
- **自动更新**：保留 Tauri updater，但更新源改为 `XM-Chen/agent-switch` 的 `latest.json`（首期仅 `windows-x86_64` 条目）、内置 Agent Switch 公钥、自有签名私钥（`~/.tauri/agent-switch.key`，绝不入库）、Agent Switch 资产命名；禁止连接或接受 ccs 官方更新（**D14**）。Tauri v2 的静态 manifest 至少要求 `version`、`platforms.windows-x86_64.url`、`signature`；`signature` 必须是生成的 `.sig` 文件内容。updater 签名（minisign `.sig`）与 Windows Authenticode 是两套机制，首期个人自用不把 Authenticode 作为阻塞。

### R7. 分阶段交付与可回滚

- 阶段 A：建立并原样验证 ccs `v3.16.5` 基线（R2 + R3）。
- 阶段 A′：迁入 Trellis 工作流与本任务（**D10**）——独立提交仅含 `.trellis/**`、`.trellis-upgrade-audit.json`、`.trellis/archive/**`、本任务目录；未跟踪文件需物理复制（`git checkout main --` 取不到），`.claude/` 等被忽略的平台目录靠 `trellis init/update` 再生而非提交；不 stage 旧 agent-switch 产品源码。
- 阶段 B：形成并经用户批准功能裁剪矩阵（R4）。
- 阶段 C：按独立可验证批次裁剪（R5）。
- 阶段 D：身份改造（R6），建议在裁剪相对稳定后集中处理，或按 design 拆分。
- 每阶段独立提交 + 验证门 + 回滚点（**AC7**）；大范围裁剪/增补拆为父子 Trellis 任务，本任务持有基线、任务图与跨批次验收，不把所有改动压入单次提交。
- 上游策略：定期手动同步（**D2**），以验证过的 SHA 为基线，按 ccs 版本/修复批次手动 fetch、评估、合并并跑完整回归，不自动追随也不永久冻结。

## Acceptance Criteria

- [ ] AC1：`main` 保持原提交历史和源码不变；`agent-switch-ccs` 可独立 checkout（或在独立 worktree 存在）。
- [ ] AC2：`agent-switch-ccs` 初始产品基线对应官方 ccs tag `v3.16.5` / commit `8d1b3306d09a27b9d8fc29694791d8421aba5f93`，不来自脏的 `../cc-switch`。
- [ ] AC3：新基线含该提交全部受控产品文件、无嵌套 `.git`，保留 MIT `LICENSE` 与 Jason Young 版权声明。
- [ ] AC4：裁剪/身份改造前，ccs 原样基线按 R3 命令通过 typecheck/单测/Rust 检查并完成一次 Windows 构建；无法执行的项记录真实阻塞（如 updater 私钥缺失、工具链版本、隔离要求）。
- [ ] AC5：存在经用户审阅的“保留/删除/修改/新增/待定”功能矩阵，每个删除项注明依赖与验收方式，覆盖 R4 列出的维度与 R5 的首期范围。
- [ ] AC6：明确记录首期“显式移植旧 agent-switch 能力 = 无”（D6），即所有能力先以 ccs 实现为准，而非默认旧能力仍在。
- [ ] AC7：纯 ccs 基线、Trellis 迁入、每个裁剪批次、身份改造、每个新增批次均有独立提交、验证门与回滚点。
- [ ] AC8：未得到单独授权前，没有推送分支、重写远程历史或发布构建产物；`main` 与 `origin/main` 关系不被本任务改变（当前 main ahead 1 的推送与否由用户另行决定）。
- [ ] AC9：身份改造后，产品不再使用 `com.ccswitch.desktop` / `cc-switch` 包名 / `~/.cc-switch` / `ccswitch://` 系统协议 / ccs 官方更新源；显示名、数据目录 `~/.agent-switch`、`agentswitch://`、`XM-Chen/agent-switch` 更新源、`0.3.0` 版本与自有签名密钥全部到位，且不误连原版 CC Switch 或旧 agent-switch 数据。
- [ ] AC10：Claude provider 以 `settings_config` 作为完整 `settings.json` 快照唯一 SSOT；切换往返可保留未被 Agent Switch 识别的顶层/嵌套 JSON 字段，Common Config 不被重复吸收入 provider 快照，且仓库不新增 `meta.snapshot` 第二快照路径。
- [ ] AC11：首期内置 Provider 目录只含 Anthropic 及经人工确认的主流厂商官方兼容模板；无聚合/中转商预设、合作排名/徽章/促销文案及返利或 ccs 来源跟踪参数；自定义完整 JSON 入口仍可用。
- [ ] AC12：本地备份、WebDAV、S3 与自动同步在 Claude-only 裁剪后仍可用，协议 artifact 只含本产品数据；首次启用远端同步前明确披露 `db.sql` 未做客户端内容加密且可能包含 API token，并获得持久化的显式确认。
- [ ] AC13：在全新 `~/.agent-switch` 数据根且已有 `~/.claude/settings.json` 的测试环境中，首次启动先将完整 live 配置导入为 current `default` provider，再添加官方 seed；切换往返后原有未知字段仍可恢复，且 `~/.cc-switch`/旧 Agent Switch DB 未被读取。
- [ ] AC14：产品只暴露 Claude Code 客户端，但 GitHub Copilot 与 ChatGPT Codex OAuth 均可作为 Claude Provider 完成登录、绑定、模型获取、切换、代理请求和配额展示；仓库无独立 Codex 客户端 UI/live/session 链。
- [ ] AC15：代理默认监听 `127.0.0.1`；当监听地址改为非 loopback 时，所有转发路由（含 `/v1/messages`、`/v1/chat/completions`、`/v1/responses`、`/responses/compact`、`/v1beta/*`、Claude Desktop gateway）一律先鉴权本地 token 再转发；保存非 loopback 监听配置前必须取得持久化的风险确认。

## Out of Scope

- 在功能矩阵获批前直接删除 ccs 功能。
- 在需求定义前把旧 agent-switch 的实现整体移植到 ccs 基线。
- 首启自动读取/迁移 `~/.cc-switch` 或旧 agent-switch 数据（改由后续独立、显式、可预览的导入任务）。
- 把 `../cc-switch` 的本地未提交内容或 Trellis 运行文件混入新基线。
- 将 ccs 仓库作为带 `.git` 的嵌套目录复制进来。
- 本规划阶段执行 branch switch、reset、批量删除或源码覆盖。
- 首期承担公开 fork 的社区维护、多平台发布、海外多语言文档、Windows Authenticode 代码签名（**D13**）。

## Decisions（已拍板，2026-07-10）

- **D1 Git 历史策略 = 从 ccs 起分支**：`agent-switch-ccs` 直接以官方 ccs 提交为起点，旧 agent-switch 历史作为独立根保留在 `main`，最利于后续吸收 ccs 上游。
- **D2 上游策略 = 定期手动同步**：不自动追随 `ccs/main`，不永久冻结；按版本/修复批次手动 fetch、评估、合并、回归。
- **D3 首期客户端 = 仅 Claude Code**：删除/降级 Codex、Gemini CLI、OpenCode、Claude Desktop、OpenClaw、Hermes。
- **D4 首期核心模块 = Provider 切换 + 本地代理 + 用量成本**。
- **D5 Claude Code 外围模块全部保留（单应用化）= MCP + Prompts + Skills + Sessions + Deep Link**。
- **D6 第一阶段严格以 ccs 行为为准**：不预设移植旧 agent-switch 的加密/portability/路由/HTTP API；后续用户提出再作独立需求。新分支是以 ccs 为源头的新定制产品，非旧 agent-switch 的升级迁移。
- **D7 产品身份 = 独立 Agent Switch**：独立显示名/包名/数据目录/日志/备份/Deep Link/更新源/发布资产，不与本机 CC Switch 争用。
- **D8 首期平台 = 仅 Windows**。
- **D9 新分支名 = `agent-switch-ccs`**：从锁定的 ccs SHA 创建；未授权不推送，若推送用同名分支并保持 `main` 不变。
- **D10 迁移 Trellis 工作流**：先留纯 ccs 基线点，再以独立提交迁入 `.trellis/`、平台适配与本任务，不混入旧产品源码。
- **D11 ccs 初始基线 = 正式版 `v3.16.5`（commit `8d1b3306…`）**。
- **D12 UI 语言 = 仅简体中文**；新增文档继续用中文。
- **D13 分发范围 = 个人自用**：可裁赞助/Funding/issue·PR 模板/stale 等非运行时内容；保留 MIT LICENSE。
- **D14 自动更新 = 自有更新源**：`XM-Chen/agent-switch` release + 独立公私钥 + Agent Switch 资产命名；私钥不入库；禁止连接 ccs 官方更新。
- **D15 初始用户数据 = 全新空库，数据根 `~/.agent-switch`**：不自动迁移 `~/.cc-switch` 或旧 agent-switch 数据。
- **D16 安装身份 = 替代旧 Agent Switch，与 CC Switch 隔离**：复用 `com.agent-switch.app` 等 Agent Switch 身份成为旧版后继；数据层仍从空库开始。
- **D17 新基线产品版本 = `0.3.0`**：三处版本同步；ccs 来源版本仅记录于文档。
- **D18 Deep Link 注册协议 = `agentswitch://`**：不注册 `ccswitch://` 为系统处理程序。
- **D19 品牌/商业内容 = 全部替换**：保留 MIT LICENSE 与来源归属。
- **D20 工作目录策略 = 当前目录直接切分支**：不使用长期相邻 worktree；切换前必须先保护未跟踪 Trellis 文件，旧 `main` 保留为可切回分支。
- **D21 Claude Code 配置快照 = ccs 原生 `settings_config` 全文快照**：不引入旧 agent-switch 的 `meta.snapshot` 间接层；保持切出 live 回填、Common Config 剥离/叠加、sanitizer 后整文件写入的三层语义，未知 JSON 字段必须往返保留。
- **D22 Claude Provider 预设 = 精选官方模板**：保留 Anthropic 与经人工确认的主流模型厂商官方兼容模板；删除聚合/中转商、合作促销与返利/来源跟踪；保留完整 JSON 自定义供应商入口。
- **D23 备份/同步 = 保留 ccs 全部能力**：本地备份、WebDAV、S3、自动同步与 Skills artifact 均保留；首期沿用未做客户端内容加密的 `db.sql` + `skills.zip` 协议，并在远端同步启用前明确披露 API token 可能被远端读取。
- **D24 首启保护 = 空产品库 + 导入现有 Claude live**：不迁移 ccs/旧 Agent Switch 产品数据，但先把已有 `~/.claude/settings.json` 全文导入为 current `default` provider，再 seed 官方模板；导入失败不得静默覆盖 live。
- **D25 Claude 托管账号上游 = GitHub Copilot + ChatGPT Codex OAuth 都保留**：两者作为 Claude Code Provider 的上游认证/协议来源，保留完整登录、账号、配额、模型和转换链；独立 Codex 客户端仍删除。
- **D26 ccs Agents 页 = 删除占位，不纳入 MVP**：现有页面无真实 Claude Code Agents 管理实现；保留 Sessions 的 subagent 日志能力，未来如需 `~/.claude/agents` CRUD 另开任务。
- **D27 本地代理 = 保留能力 + 非 loopback 强制鉴权**：默认 `127.0.0.1` 无鉴权；监听非 loopback 时所有转发端点必须校验本地 token，并要求保存前持久化风险确认。
