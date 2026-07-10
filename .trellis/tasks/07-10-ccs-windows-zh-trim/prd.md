# 仅 Windows + 仅简体中文裁剪

> 父任务：`07-10-ccs-baseline-migration`。前置：`07-10-ccs-baseline-bootstrap` 已归档。范围与父 PRD 中 R5 的「平台」「语言」两维度、R6 更新源/CI 收敛、R7 阶段 C 的独立可回滚批次一致。

## Goal

在 `agent-switch-ccs` 分支上，把 ccs v3.16.5 基线裁剪为「发布目标仅 Windows x86_64」+「界面语言仅简体中文」的定制版本，同时**保留 ccs 现有的全部多应用客户端支持**（Claude Code / Codex / Gemini CLI / OpenClaw / opencode / Hermes / Claude Desktop 等）和多 Provider 能力，为后续可能的身份改造预留干净起点。

## Background

- 父任务 PRD 决策 **D8**（首期平台 = 仅 Windows）与 **D12**（UI 语言 = 仅简体中文）仍然生效。
- 父任务 PRD 决策 **D3**（首期客户端 = 仅 Claude Code）**已被用户于 2026-07-10 撤销**，改为保留 ccs 全部多应用支持；因此本任务不裁 `App.tsx`/`AppSwitcher`/`VALID_APPS`，不删非 Claude 的 commands/services/proxy adapters，不删 OpenClaw 专属 `workspace` view 与 `commands/workspace.rs`。
- 父任务 PRD 决策 **D7/D14/D15/D16/D17/D18/D19**（身份改造：品牌重命名、`com.agent-switch.app`、数据根 `~/.agent-switch`、`agentswitch://`、updater 指向改造、LICENSE 来源说明等）**保留，但放在独立后续子任务 `ccs-agent-switch-identity` 中做**，不在本任务范围内。因此本任务执行期间仓库仍保留 `CC Switch` / `com.ccswitch.desktop` / `~/.cc-switch` / `ccswitch://` / ccs 官方 updater endpoint / `cc-switch` npm 名与 crate 名——这些在下一个子任务才改。本任务只做平台与语言两个维度。
- ccs v3.16.5 现有工具链已在 bootstrap 阶段验证：Node 20 / pnpm 10.12.3 / Rust 1.95 / Tauri 2.8.2；`pnpm tauri build --no-bundle` 通过。
- **ccs 上游同步兼容性**（父 PRD **D2**）：本任务的裁剪范围会影响未来手动同步 ccs upstream 的合并成本。删得越深，合并冲突越多；删得越浅，"仅 Windows/仅中文"越名不副实。
- 已知 ccs 侧多平台代码分布（bootstrap research 已勘察）：
  - `flatpak/` 顶层目录：Linux Flatpak manifest（可整体删）。
  - `.github/workflows/`：多平台 release matrix（含 macOS runner / ubuntu-latest）。
  - `src-tauri/tauri.conf.json` 的 `bundle.targets`：`["msi", "app", "dmg", "deb", "rpm", "appimage"]` 等。
  - Rust 源码内 `#[cfg(target_os = "windows"|"macos"|"linux")]` 分支：路径解析、系统集成、shell 集成、字体、Wayland/X11 逃生舱等。
  - 前端 `navigator.platform` / Tauri `platform()` 判定与 macOS 快捷键差异。
- 已知 ccs 侧 i18n 分布：
  - `src/i18n/locales/`：`zh.json`（简中）、`en.json`、`ja.json`、`zh-TW.json`（繁中）等。
  - `src/i18n/index.ts`：初始化与语言切换。
  - `src/components/` 中的语言切换 UI（Header/Settings 一带的 language selector）。
  - `docs/` 中的多语 README（`README.md` 英文、`README.zh.md` 简中、`README.ja.md` 日文等）+ 各语言 release notes。
- **未被本任务涵盖但仍属"仅 Windows/仅中文"语义的边界**：
  - 品牌资产替换（`CC Switch` → `Agent-Switch`）、LICENSE 来源说明、README 主标题：与身份改造耦合，暂缓。
  - Windows Authenticode 代码签名：父 PRD Out of Scope，本任务不做。
  - ARM64 Windows 支持：父 PRD 已声明"不作为门"，本任务只保留 x86_64。

## Requirements

### R1. 保留边界（**禁止本任务触碰**）

- 不删除任何非 Claude 客户端集成：Codex、Codex OAuth、Gemini CLI、OpenClaw、opencode、Hermes、Claude Desktop 均保留 UI/服务/live 投影/session/MCP 分支。
- 不改动产品身份：`productName`、identifier、npm 名、Cargo crate 名、`~/.cc-switch` 路径、`ccswitch://` Deep Link、updater endpoint、内置公钥、Windows AUMID、WiX `upgradeCode` 均保持 ccs v3.16.5 原值。
- 不改动数据层：schema、迁移、备份/恢复、WebDAV/S3 remote root 命名、manifest 产品身份提示文案。
- 不改动 Provider 预设（`claudeProviderPresets.ts` 等），不删聚合/中转商模板，不去 aff/ref/utm 参数——这些属于身份改造与预设精选，独立子任务处理。
- 不动 sponsor/Funding/community issue-PR templates/stale bot 相关内容——身份改造耦合。

### R2. 平台裁剪（仅 Windows x86_64）

**R2.1 打包与发布层（必做）**

- 删除 `flatpak/` 顶层目录。
- `src-tauri/tauri.conf.json` `bundle.targets` 收缩为 `["msi"]`（保留 ccs 现有 WiX per-user 模板 `src-tauri/wix/per-user-main.wxs`；如 targets 中含 `nsis` 由设计决定）。
- 删除 `src-tauri/tauri.conf.json` 中 macOS/Linux 专属键（`bundle.macOS.*`、`bundle.linux.*`、`bundle.deb`、`bundle.rpm`、`bundle.appimage`、macOS dmg 相关等）。
- GitHub Actions release/build workflow 中的 matrix 收缩到 `windows-latest` + `x86_64-pc-windows-msvc`。删除 `macos-latest` / `ubuntu-latest` runner 与其步骤；保留 lint/typecheck/test 等纯 Node/Rust 任务（若原本跑在 ubuntu-latest 上，可选择改成 windows-latest 或独立平台无关 job，权衡见 design.md）。
- 删除随 macOS/Linux 打包才用到的 CI 步骤（notarize、universal2、Flatpak build 等）。
- ARM64 Windows 不作为发布目标，也不作为 CI 门。

**R2.2 资产层（必做）**

- 删除 macOS 专属图标（`.icns`）、Linux 专属图标（若有独立文件）、Flatpak 图标与 desktop entry。
- Windows 相关图标（`.ico` / png set）与 Tauri `icon` 数组保留并核验。
- 删除专门给 macOS 或 Linux 说明的截图/文档资源；跨平台的通用截图保留。

**R2.3 源码层（D-A = 浅，已定）**

只删打包与 CI；源码内 `#[cfg(target_os = "macos"|"linux")]` 分支与前端 `platform === 'darwin'` 等平台判定**全部保留**，让代码库在 macOS/Linux 仍能编译通过（虽然不发布）。选择理由：ccs 上游同步的合并冲突最小；本任务范围可控；后续若确需收敛再单独处理。

### R3. 语言裁剪（仅简体中文）

**R3.1 语言资源（必做）**

- `src/i18n/locales/` 只保留 `zh.json`（或 ccs 使用的具体 zh-CN 文件名，实操时以文件名为准），删除 `en.json`、`ja.json`、`zh-TW.json` 等其他 locale 文件。
- `src/i18n/index.ts`（或等价配置文件）默认语言与 fallback 语言都固定为 zh；删除或简化语言列表数组，只保留 zh。

**R3.2 UI 语言切换（必做）**

- 删除设置页 / Header 中的语言切换控件（Language Selector）与其状态、localStorage 持久化项。若该控件与其他设置项在同一组件，只删该控件，保留其他设置项。
- 删除随语言切换引入的 date-fns / dayjs locale 动态 import 等分支（如果只是 `zh` 与 `zh-CN`，保留即可）。

**R3.3 i18n 抽象层（D-B = 浅，已定）**

保留 `i18next` 或等价库、`useTranslation`、`t('key')` 调用，只把语言固定为 zh。选择理由：上游同步冲突最小，未来若要加回英文只需补 locale 文件，不用改所有调用点。

**R3.4 文档语言（D-C = 浅，已定）**

删除 `README.md`（英文）、`README.ja.md`、`README.zh-TW.md`，只保留 `README.zh.md`（可按需重命名回 `README.md`）；删除各语言 release notes 与 docs 目录下的非中文文档。文档只维护中文。注意：README 内的品牌命名仍保留 `CC Switch`，因身份改造放在下一个子任务，本任务不改品牌字样。

### R4. 跨维度的安全增强（非本任务范围）

父 PRD 的 **D27**（非 loopback 强制鉴权）和 **D23** 补充（首次远端同步风险确认）**不属于 Windows/中文裁剪范围**，本任务不做，避免范围蔓延；如你希望顺带做，明确告知后我把它加入本 PRD。

### R5. 提交与回滚边界

- 每个批次独立提交。批次（D-A/D-B 浅，无源码平台分支清理与硬编码批次）：
  1. 删除 `flatpak/` + macOS/Linux 图标资产 + tauri.conf.json bundle targets 收缩
  2. GitHub Actions matrix 收敛到 Windows
  3. i18n locale 文件与语言切换 UI 删除
  4. 非中文文档删除
- 每批次执行 typecheck / format / test / renderer build / cargo fmt / cargo clippy `-D warnings` / cargo test / cargo check / `pnpm tauri build --no-bundle` 至少验证 Windows target 编译通过。
- 未获 commit 授权时停在可审查 diff。
- 不 push、不改 `origin` 默认分支。

## Acceptance Criteria

- [ ] AC1：仓库不再包含 `flatpak/` 目录、macOS 专属 `.icns` / Linux 专属独立图标 / Linux desktop entry；`src-tauri/tauri.conf.json` bundle.targets 仅含 Windows target；macOS/Linux 专属 bundle 键已移除。
- [ ] AC2：GitHub Actions release/build 相关 matrix 仅剩 Windows x86_64 runner；`macos-latest` / `ubuntu-latest` 相关 release/artifact 步骤已删除；lint/typecheck/test 保留（可能迁至 windows-latest 或 platform-independent job）。
- [ ] AC3：`src/i18n/locales/` 仅含 zh locale；`i18n` 初始化默认与 fallback 语言均为 zh；语言切换 UI 已删除，切换语言的持久化项不再写入。
- [ ] AC4：所有非中文 README / release notes / docs 已删除或合并为 zh 文档；`README.zh.md` 或等价文件仍然存在并作为项目主 README。
- [ ] AC5：D-A/D-B 均为浅——源码内 `#[cfg(target_os = "macos"|"linux")]` 分支、前端 platform 判定、`i18next` 抽象与 `t('key')` 调用全部保留未被误删；D-C 浅——非中文文档已全删。
- [ ] AC6：仓库仍保留 ccs 现有多应用支持（Claude Code / Codex / Gemini CLI / OpenClaw / opencode / Hermes / Claude Desktop）与所有 Provider 配置/预设/切换/代理/故障转移/翻译/MCP/Prompts/Skills/Sessions/Deep Link/Usage/备份/同步能力；未在保留边界外的模块被误删。
- [ ] AC7：仓库仍保留 ccs 现有产品身份（`CC Switch` / `com.ccswitch.desktop` / `~/.cc-switch` / `ccswitch://` / ccs 官方 updater / 内置公钥 / WiX upgradeCode 默认值）；本任务未做任何身份改造。
- [ ] AC8：每个批次独立提交，每批提交前跑通 typecheck / format / test / renderer build + cargo fmt / clippy `-D warnings` / test / check + `pnpm tauri build --no-bundle`（Windows target）；任一批次可通过 revert 单个提交回滚。
- [ ] AC9：未 push、未改 `origin` 默认分支、未发布任何构建产物。

## Out of Scope

- 应用范围裁剪（Claude-only 已被撤销，本任务不删非 Claude 客户端）。
- 产品身份改造（品牌、identifier、数据根、Deep Link、updater、WiX upgradeCode、README 主标题）。
- 非 loopback 鉴权与远端同步风险确认（父 D27 / D23 补充，需独立子任务）。
- Provider 预设精选、聚合/中转商删除、返利参数清理（父 D22，需独立子任务）。
- schema 迁移、数据库路径改动。
- Windows Authenticode 代码签名。
- ARM64 Windows 发布支持。
- macOS/Linux 用户支持承诺（保守裁剪不代表未来会重启这些平台）。
- 上游 ccs 后续版本合并（本任务只面对 v3.16.5 基线，不主动往前追）。

## 已定决策点（2026-07-10 用户确认）

- **D-A = 浅**：保留 Rust `#[cfg(target_os)]` 与前端 platform 分支，只删打包/CI/资产。
- **D-B = 浅**：保留 i18n 抽象与 `t('key')` 调用，只固定语言为 zh。
- **D-C = 浅**：全删非中文文档，只留中文 README/docs。
- 身份改造（含 updater 指向改造、数据根首启行为）移入独立子任务 `ccs-agent-switch-identity`，不在本任务。
