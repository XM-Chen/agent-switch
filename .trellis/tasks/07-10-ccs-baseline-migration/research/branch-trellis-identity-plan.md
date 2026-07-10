# Research: 分支、Trellis 迁移与产品身份计划

- **Query**: 为 ccs v3.16.5 基线创建独立分支、保护现有 main/Trellis 未跟踪文件、保留纯基线点、独立迁入 Trellis，并盘点 Agent Switch 与 CC Switch 隔离的身份改动。
- **Scope**: mixed（仓库/ccs 提交/官方 Tauri 文档）
- **Date**: 2026-07-10
- **边界**: 仅调研；没有创建或切换分支、没有改产品代码。

## Findings

### Git 与基线事实

| 项目 | 发现 |
|---|---|
| 官方 tag | `ref-cc-switch` 的 annotated tag `v3.16.5` 对象为 `a58917a5…`，peeled commit 是 `8d1b3306d09a27b9d8fc29694791d8421aba5f93`。|
| 本地 tag 名 | 本地没有 `refs/tags/v3.16.5`，`git show v3.16.5` 报 unknown revision；正式分支命令应直接使用 commit SHA，或先显式 fetch tag。|
| ccs 内容版本 | `8d1b3306…` 提交消息为 `chore(release): v3.16.5`；`package.json`/`Cargo.toml`/`tauri.conf.json` 均为 `3.16.5`。PRD 背景中的 `3.16.4` 已过期。|
| ccs 与 agent-switch 历史 | 两者为独立根；新分支从 ccs commit 起点不包含旧 agent-switch 历史。|
| 主树 | `E:/SynologyDrive/git_files/agent-switch`：`main`=`7e906685e`，相比 `origin/main`/`8890dbbea` ahead 1（Trellis 0.6.6 升级 commit）。|
| 主树未跟踪文件 | `.trellis-upgrade-audit.json`、`.trellis/archive/`、`.trellis/tasks/07-10-ccs-baseline-migration/`；无已修改或已暂存的跟踪文件。|
| 现有新分支 | `agent-switch-ccs` 当前不存在。|

### 安全的分支/工作树机制

最小破坏的方式是把目标分支放进**独立 worktree**，不在主工作树 checkout：

```bash
git worktree add -b agent-switch-ccs <新工作树目录> \
  8d1b3306d09a27b9d8fc29694791d8421aba5f93
```

- 该操作只创建新的 checkout，不会触碰主树 `main` 的 ahead commit 或 3 个 untracked Trellis 路径。
- 避免在主树做 `checkout`、`reset --hard` 或 `clean`；它们虽不必然删除无冲突 untracked 文件，却会使主树产品文件切换到 ccs，违反 R1 的保留旧实现目标。
- 新分支的首个可回滚纯基线点天然就是 commit `8d1b3306…`。若需要人类可读锚点，可在实施时额外建立只在本地保留的 tag，例如 `agent-switch-ccs-baseline` 指向这个 SHA；所有 Trellis/身份改动都在后续提交。
- D9 禁止 push 仍然适用：本步骤不需要配置 upstream、不改 `origin` 默认分支。

### Trellis 工作流迁入机制（D10）

ccs `8d1b3306…` 中 `.trellis` / `.claude` 文件数为 **0**；agent-switch `main` 跟踪的 `.trellis/**` 有 **344** 文件，二者路径交集为零。因此，迁入不会覆盖 ccs 产品文件，也不会与 ccs 元数据冲突。

| 输入 | 获取方法 | 注意点 |
|---|---|---|
| 跟踪的 `.trellis/**` | 在新 worktree 中从 `main` checkout/restore | 含 workflow、scripts、spec、agents、task archive、workspace；`7e906685e` 的 0.6.6 升级修改了 9 个既有 Trellis 文件。|
| 未跟踪审计/归档/本任务 | 从主工作树物理复制进新 worktree | `git checkout main -- <path>` 无法取到 untracked 文件。|
| `.claude/` 等平台适配 | 重新执行 `trellis init`/`trellis update`，或物理复制 | `.gitignore:2-11` 忽略 `.claude/`、`.codex/`、`.cursor/`、`.opencode/`、`.pi/`、`.agents/`、`AGENTS.md` 等；`git ls-files` 为零，无法靠提交迁入。|

建议的独立提交边界：

1. **纯 ccs 基线**：分支起点 `8d1b3306…`，不额外写产品/Trellis文件。
2. **Trellis 迁入 commit**：仅提交 `.trellis/**`、`.trellis-upgrade-audit.json`、`.trellis/archive/**`、`.trellis/tasks/07-10-ccs-baseline-migration/**`；不 stage `src/`、`src-tauri/`、`package.json` 等旧 agent-switch 产品文件。
3. **平台生成**：在新 worktree 运行 Trellis 初始化/更新产生 ignored 平台目录；它们不成为 git commit 内容。
4. **身份改造 commit**：在上述 commit 之后单独进行，不掺入纯基线或 Trellis 迁入提交。

当前 Trellis 平台适配事实：`.claude/settings.json`、`.claude/agents/trellis-research.md` 都被 `.gitignore` 的 `.claude/` 规则忽略；`.trellis` 自身是跟踪内容，而 `.trellis/.runtime/`/`.developer` 等局部状态由 `.trellis/.gitignore` 排除。

### 产品身份映射

#### Bundle/包/显示身份

| 身份 | ccs v3.16.5 | 目标（复用既有 Agent Switch 身份） | 位置 |
|---|---|---|---|
| 显示名 | `CC Switch` | `Agent-Switch` | ccs `src-tauri/tauri.conf.json:3`; 既有 as `tauri.conf.json:2` |
| Tauri identifier | `com.ccswitch.desktop` | `com.agent-switch.app` | ccs `tauri.conf.json:5`; 既有 as `:4` |
| npm 名 | `cc-switch` | `agent-switch` | ccs `package.json:2`; as `package.json:1` |
| Cargo package | `cc-switch` | `agent-switch` | ccs `src-tauri/Cargo.toml:2` |
| Cargo library | `cc_switch_lib` | `agent_switch_lib` | ccs `Cargo.toml:14`; as `Cargo.toml:8` |
| 作者/仓库 | Jason Young / `farion1231/cc-switch` | XM-Chen / `XM-Chen/agent-switch`，但保留上游 MIT 版权 | ccs `Cargo.toml:5-7` |
| Windows AUMID | ccs 运行时调用 `set_windows_app_user_model_id` | Agent Switch 专属值 | ccs `src-tauri/src/lib.rs` setup 段 |

#### 数据目录、数据库、日志、备份

ccs 的根为 `~/.cc-switch`；核心实现为 `src-tauri/src/config.rs:182-217` 的 `get_app_config_dir()`：

| 资源 | ccs | 证据 |
|---|---|---|
| 数据库 | `~/.cc-switch/cc-switch.db` | `database/mod.rs:95-99`; `database/backup.rs:299` |
| 自动/手动备份 | `~/.cc-switch/backups/` | `database/backup.rs:239, 304-332, 517, 564, 643, 677` |
| Skills SSOT/备份 | `~/.cc-switch/skills/` / `skill-backups/` | `services/skill.rs:483,499` |
| 文件日志 | `~/.cc-switch/logs/cc-switch.log` | `panic_hook.rs:43`; `lib.rs` `TargetKind::Folder { file_name: "cc-switch" }` |
| 崩溃日志 | `~/.cc-switch/crash.log` | `panic_hook.rs:38` |
| 旧 JSON 配置 | `~/.cc-switch/config.json` | `config.rs:220`; 启动时可迁移 |
| 测试 HOME override | `CC_SWITCH_TEST_HOME` | `config.rs:22`，同步/备份测试中多处 |

既有 Agent Switch 根为 OS app-data 下的 `agent-switch`：`src-tauri/src/config/paths.rs:8-11`，数据库 `agent-switch.db`（`:15-16`），运行时优先 `app.path().app_data_dir()`（`src-tauri/src/lib.rs:42-47`）。既有 keychain service 名也为 `agent-switch`（`services/keychain.rs:3`）。

D7/D15 下，ccs 分支中必须**整体**移除 `~/.cc-switch` / `cc-switch.db` 读取、迁移、日志和备份路径，否则会与已安装 CC Switch 争用数据。需要覆盖的 ccs 路径包括：

- `config.rs`（根目录与 Windows `HOME` legacy fallback）；
- `database/mod.rs`、`database/backup.rs`（DB、backup 根、SQL export header `-- CC Switch SQLite 导出` 与导入校验）；
- `panic_hook.rs`、`lib.rs`（logs/`cc-switch.log`/crash.log）；
- `settings.rs`、`services/skill.rs`、`services/env_manager.rs`、`hermes_config.rs`、`openclaw_config.rs`、`codex_history_migration.rs`（所有 `get_app_config_dir()` 派生目录）；
- 全部 `CC_SWITCH_TEST_HOME` 测试变量；
- 启动 JSON→SQLite 迁移逻辑：在 Agent Switch 新根下运行可自然得到空库，但不得增加读取 `~/.cc-switch` 的兼容路径。

D6 要求首期以 ccs 语义为准，故存储根可以保持 ccs 的 home-hidden-dir 结构并重命名为 `~/.agent-switch`；若改为当前 as 的 `dirs::data_dir()/agent-switch`，则是较大架构变更，须在设计中明确。无论选择哪一种，都不能使用 `~/.cc-switch`。

#### Deep Link

| 项目 | ccs 当前值 | 改动 |
|---|---|---|
| Tauri 注册 scheme | `ccswitch` | 换为 Agent Switch 专属 scheme，例如 `agentswitch` |
| URL 契约 | `ccswitch://v1/import?resource=…` | 同时替换为 `agentswitch://v1/import?resource=…` |
| 注册/收件 | `app.deep_link().register_all()` 与 `on_open_url` | 保持机制，替换 scheme 前缀判断 |
| 解析器 | `if scheme != "ccswitch"` | 改为新 scheme |

ccs 文件：`tauri.conf.json`、`src-tauri/src/lib.rs:119-170,861-891,1584-1590`、`src-tauri/src/deeplink/parser.rs:11-48`、`deeplink/tests.rs` 及 resource 子模块。

**关键冲突**：旧 agent-switch main 自己也复用了 `ccswitch`（`tauri.conf.json:52`；`services/deeplink/mod.rs:140-147`；前端 `DeepLinkPage.tsx`/API/对话框）。这与 D7 的独立 scheme 直接矛盾。因此新分支不能简单复用旧 as Deep Link 身份；要替换 Tauri 配置、backend parser、runtime `starts_with`、前端文案/placeholder、持久化 `source_protocol` 字段和测试。旧测试已把 `agentswitch://…` 断言为非法（`services/deeplink/mod.rs:1028`）；身份改造后需要反转该契约。

#### Updater、签名、发布资产

| 项目 | ccs | Agent Switch 目标 |
|---|---|---|
| 更新源 | `https://github.com/farion1231/cc-switch/releases/latest/download/latest.json` | `https://github.com/XM-Chen/agent-switch/releases/latest/download/latest.json` |
| 嵌入公钥 | ccs minisign key（C8028C9A573928E3） | 已有 Agent Switch 公钥（D4D16F6B14B70077） |
| 签名私钥 | ccs Actions Secret `TAURI_SIGNING_PRIVATE_KEY` | `~/.tauri/agent-switch.key` / 自有 GitHub Secret；绝不提交 |
| Windows assets | `CC-Switch-$VERSION-Windows*.msi` | 现有 as 文档中的 `Agent-Switch_<version>_x64_zh-CN.msi` 命名 |
| `latest.json` | ccs CI 自动生成、含多平台 | D8 首期仅保留 `windows-x86_64` |

证据：ccs `tauri.conf.json:62-67`；`.github/workflows/release.yml:132-186`（签名 secret）、`:458-464`（MSI 资产）、`:635-738`（latest.json）；`src-tauri/src/commands/misc.rs:55-63`（手动检查更新 URL）、`:924`（User-Agent `cc-switch`）。既有 as updater identity 在 `tauri.conf.json:42-53`，其安全发版机制在 `docs/release.md:5-145`。

Tauri v2 updater 需要在 `tauri.conf.json` 内嵌 pubkey，构建时 `createUpdaterArtifacts: true` 生成签名资产；`latest.json` 平台条目含具体 URL 与 `.sig` 全文。ccs updater endpoint 与公钥都必须替换：两者一起可防止下载/接受官方 ccs 更新。Tauri 官方文档：

- [Updater plugin](https://v2.tauri.app/plugin/updater/) — endpoints、公钥、签名和 latest.json。
- [Deep Link plugin](https://v2.tauri.app/plugin/deep-link/) — scheme 注册与 URL 事件。
- [Tauri configuration reference](https://v2.tauri.app/reference/config/) — identifier / bundle 配置。

#### Windows 安装器身份

ccs 使用 custom WiX 模板 `src-tauri/wix/per-user-main.wxs`（`tauri.conf.json:47-51`），其中 `Product` 使用 `{{product_name}}`、`{{manufacturer}}`、`{{upgrade_code}}`、`{{version}}`（模板 `:15-20`），并写入 `Software\{{manufacturer}}\{{product_name}}`（`:76-79,122,137,199,218`）。它是 `InstallScope="perUser"`、`InstallPrivileges="limited"`（`:23-30`）。

- Tauri 的 MSI `UpgradeCode` 基于 bundle identifier 派生；identifier 改变就会成为不同 MSI 产品，旧版不会原地升级。
- **D16（替代旧 Agent Switch）**：保持 `identifier = com.agent-switch.app`，使新 ccs 基线版本沿 Agent Switch 的 Windows 安装身份升级。
- **D7（隔离 CC Switch）**：不要用 `com.ccswitch.desktop`，即可使 UpgradeCode 与 CC Switch 不同，避免覆盖/升级本机 CC Switch。
- 实施时需决定保留 ccs custom per-user WiX 模板，还是采用旧 as 的默认 WiX（旧 as 仅配置 `bundle.windows.wix.language="zh-CN"`）。该选择影响安装范围/注册表/安装体验，不影响 identifier 是升级身份根源这一点。

#### 版本策略

- ccs 三处均 `3.16.5`；旧 as 的 `package.json`/`tauri.conf.json` 均 `0.2.2`，但 `Cargo.toml` 是 `0.1.0`，已有三处失配。
- D16 尚未决定版本线。要让已装 Agent Switch 获得 updater 更新，新版本必须严格高于已发布 0.2.x。保留 Agent Switch 版本线（例如 0.3.0）最符合“旧 Agent Switch 后继”；若改用 3.16.5，在 semver 上也会被视为升级，但会模糊 ccs 上游版本与自有版本线。
- 无论选择，`package.json`、`src-tauri/tauri.conf.json`、`src-tauri/Cargo.toml` 三处必须同步；Tauri updater 按 semver 判定新版本。

#### License 与归属

`LICENSE` 原文是 MIT：`Copyright (c) 2025 Jason Young`。R2/D13 要求保留该版权及许可；不得替换或删除。可追加自身版权行，但应保留 Jason Young 原行与完整许可文本。`package.json`/`Cargo.toml` 均可继续声明 `MIT`。

ccs 品牌/归属还出现在 `README*.md`、`package.json`、`Cargo.toml`、`.github/workflows/release.yml`、`.github/FUNDING.yml`、issue templates、ccswitch.io 和 sponsor 链接中。D12/D13（中文、个人自用）允许把非运行时社区/赞助/多语言文档裁掉；不影响保留 MIT LICENSE 的义务。

## Files Found

| 文件 | 作用 |
|---|---|
| `.trellis/tasks/07-10-ccs-baseline-migration/prd.md` | 决策 D1-D16；D7/D10/D14/D15/D16 是本调研主约束。|
| `.gitignore` | 证明 `.claude/` 等平台目录被忽略，`.trellis/` 跟踪。|
| `.trellis/workflow.md` | Trellis 任务、平台适配与研究持久化机制。|
| `.trellis/spec/guides/project-conventions.md` | ccs 远程/文档中文/项目远程约定。|
| `.trellis/spec/trellis-runtime/platform-directory-structure.md` | 平台文件与 `.trellis` 的所有权边界。|
| ccs `package.json` / `src-tauri/Cargo.toml` / `src-tauri/tauri.conf.json` | 包、bundle、版本、updater、scheme 的主要身份来源。|
| ccs `src-tauri/src/config.rs` | `~/.cc-switch` 根目录和 Windows legacy fallback。|
| ccs `src-tauri/src/database/{mod.rs,backup.rs}` | `cc-switch.db`、backup、SQL 导出身份。|
| ccs `src-tauri/src/{lib.rs,panic_hook.rs}` | logs、crash log、AUMID、deep-link 注册。|
| ccs `src-tauri/src/deeplink/**` | `ccswitch://v1/import` 实际 parser、注册后的路由与测试。|
| ccs `.github/workflows/release.yml` | updater 签名资产、latest.json、CC-Switch 资产命名。|
| ccs `src-tauri/wix/per-user-main.wxs` | WiX UpgradeCode/product/manufacturer 绑定与 per-user 安装机制。|
| current `docs/release.md` | 已有 Agent Switch 公钥/私钥、GitHub Release、latest.json、Windows 资产的发版约定。|

## Related Specs

- `.trellis/spec/guides/project-conventions.md` — 迁入后继续约束 `ref-cc-switch` 与 `origin` 的关系。
- `.trellis/spec/trellis-runtime/platform-directory-structure.md` — 平台适配必须复制/再生成，不能靠 git 提交迁移。
- `.trellis/spec/trellis-runtime/runtime-persistence.md` — Trellis 的 task/jsonl/runtime 状态；与产品 SQLite 无关。
- `.trellis/spec/backend/database-guidelines.md`、`backend/portability-guidelines.md` — 当前 as 的旧 DB/导出语义；D6 首期不能无审查地套用。
- `.trellis/spec/backend/http-proxy-guidelines.md` — 现有 as Deep Link 仍写 `ccswitch://`，说明 D7 需要完整替换。

## Caveats / Not Found

- 本调研没有创建分支/worktree/tag，未复制任何文件。
- 当前 research artifact 位于隔离 worktree；主工作树的同名任务目录是 untracked，不能由本 agent 直接写入。
- 新 worktree 的具体目录尚未决策；建议位于仓库外相邻目录，避免把长期工作区塞进产品忽略/Claude 管理目录。
- Tauri 文档的 updater 页已用 urllib 核对可达（HTTP 200）；本机 curl TLS 参数异常，且旧推测的 `/distribute/windows-installer/` URL 返回 404，实施时应通过当前 Tauri 文档站再次确认 Windows Installer 精确链接/字段。
- 数据根方案（改为 `~/.agent-switch` 或切换至 OS app-data）和版本号继承策略均为 D16/D15 下待设计决策；本文只列出兼容/隔离约束。
