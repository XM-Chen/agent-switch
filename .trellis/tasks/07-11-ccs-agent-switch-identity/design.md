# Design：身份改造为独立 Agent Switch

> 依据 `prd.md`。目标：在 ccs v3.16.5 基线上做**身份隔离**，不改功能语义。所有改动按批次可回滚。

## 1. 架构边界

身份改造分层，从「机器身份」到「用户可见品牌」逐层隔离，各层可独立提交、独立验证：

```
L0 版本/包名/identifier  ── package.json / Cargo.toml / tauri.conf.json
L1 数据根/DB/日志/测试env ── config.rs / lib.rs / database/* / 各测试
L2 Live 归属文件名        ── codex_config.rs (CC_SWITCH_CODEX_MODEL_CATALOG_FILENAME)
L3 Deep Link scheme       ── tauri.conf.json / deeplink/parser.rs / lib.rs / 前端
L4 Updater 指向           ── tauri.conf.json / commands/misc.rs
L5 图标资产               ── src-tauri/icons/**
L6 用户可见品牌文案       ── zh.json / tauri.windows.conf.json / 关于·欢迎
```

依赖关系：L0 是根（改 identifier 会派生新 AUMID / WiX upgradeCode，需同批固定 upgradeCode）；L1–L6 相互独立，可任意顺序，但建议 L0→L1→L2→L3→L4→L5→L6 以便每批验证聚焦。

## 2. 各层设计

### L0 包/版本/identifier + WiX upgradeCode

- 三处版本 `3.16.5` → `0.3.0`：`package.json:3`、`src-tauri/Cargo.toml:3`、`src-tauri/tauri.conf.json:4`。
- 名称：`package.json:2` name、`Cargo.toml:2` package name、`Cargo.toml:14` lib name `cc_switch_lib`→`agent_switch_lib`、`Cargo.toml:7` repository。
- `tauri.conf.json:3` productName、`:5` identifier。
- **lib rename 波及面**：`cc_switch_lib` 被 `src-tauri/tests/*.rs`（support/skill_sync/proxy_commands/provider_service/provider_commands/mcp_commands/import_export_sync/hermes_roundtrip/deeplink_import/app_type_parse/app_config_load）大量 `use cc_switch_lib::...`。renaming crate lib 后这些 `use` 全部要改。纯机械替换，但必须一次改齐否则 tests 不编译。
- **WiX upgradeCode（AC9）**：`wix/per-user-main.wxs:18` 是 `UpgradeCode="{{upgrade_code}}"` 占位，值由 Tauri 从 productName 派生。改 productName（`CC Switch`→`Agent-Switch`）会改变派生 upgradeCode。为「承接 0.2.x 升级线」：
  1. 先在旧身份（productName `Agent-Switch`）上取默认 upgradeCode：`pnpm tauri inspect wix-upgrade-code`（或从旧 0.2.x 构建产物读取），记录 GUID 到 implement 笔记。
  2. 在 `tauri.conf.json` 的 `bundle.windows.wix.upgradeCode` 显式写死该 GUID（Tauri v2 支持显式配置，优先于派生），避免未来 productName 变动再次漂移。
  3. 由于新 productName 恰为 `Agent-Switch`（与旧 0.2.x 相同），派生值理论上一致；显式固定是为防御未来变动，且满足 AC9「已显式固定」。
- **注意**：`Cargo.toml` lib name 与 `tauri.conf.json` 前端 invoke 无关（invoke 走命令名，不走 crate 名），但 `main.rs`/`lib.rs` 内 `cc_switch_lib` 自引用需核对（通常 bin 通过 crate name 引用，改名后 `src-tauri/src/main.rs` 里的 `cc_switch_lib::run()` 之类要改）。

### L1 数据根 / DB / 日志 / 测试 env

- `config.rs:188` `get_home_dir().join(".cc-switch")` → `.agent-switch`；`:196,201-202` Windows legacy fallback 中的 `.cc-switch` / `cc-switch.db` 同步（legacy 探测本是为兼容 ccs 旧路径，改名后此 fallback 语义变为探测 agent-switch 自身旧路径——**保留结构、只改名**，不新增读 `~/.cc-switch`）。
- DB 文件名：`database/mod.rs:97`、`lib.rs:375`、`database/backup.rs:299` 等 `cc-switch.db` → `agent-switch.db`。
- 日志：`lib.rs:343` `cc-switch.log`、`:354` `file_name: "cc-switch"` → `agent-switch`；`panic_hook.rs` crash.log 目录随数据根。
- 测试 env：`CC_SWITCH_TEST_HOME` → `AGENT_SWITCH_TEST_HOME`（`config.rs:23`、`app_config.rs`、`hermes_config.rs`、`tests/support.rs:16-17` 等全部引用）。
- 测试内硬编码路径：`tests/provider_commands.rs:525,528`、`tests/mcp_commands.rs:64,147` 的 `.cc-switch` / `cc-switch.db` 断言同步。
- **不迁移**：删除任何「读 `~/.cc-switch` 后拷入新根」的逻辑（当前 ccs 无此跨产品迁移，只有自身 legacy fallback；确认不引入）。

### L2 Live 归属文件名

- `codex_config.rs:15` `CC_SWITCH_CODEX_MODEL_CATALOG_FILENAME = "cc-switch-model-catalog.json"` → `"agent-switch-model-catalog.json"`。
- 该常量在 `:157,891,899,1028` 及大量测试使用；改常量值即可，只认新名。
- 归属判定：ccs 用「文件名 == 常量」判断 catalog 是否本产品生成（`:899,1028`）。改名后，本机 ccs 写的旧 `cc-switch-model-catalog.json` 会被 Agent Switch 视为「用户自管理外部文件」而不接管——符合隔离预期。
- 已知副作用（PRD 记录，不解决）：`~/.codex/config.toml` `model_catalog_json` 指针单一，谁后切谁占。

### L3 Deep Link（主 agentswitch + 粘贴兼容 ccswitch）

- 系统注册：`tauri.conf.json:55` schemes `["ccswitch"]` → `["agentswitch"]`（仅此一个）。
- 解析器 `deeplink/parser.rs:22`：`if scheme != "ccswitch"` → 接受 `agentswitch` 或 `ccswitch`（`scheme != "agentswitch" && scheme != "ccswitch"` 才报错）。
- 运行时 `lib.rs:130` `starts_with("ccswitch://")`、`:885` 注释、`:1590` `starts_with("ccswitch://")`：主判定改 `agentswitch://`，并保留对 `ccswitch://` 的接受（粘贴路径与系统 open_url 路径都经 `handle_deeplink_url`，统一放宽）。
- `source_protocol` 持久化：按实际 scheme 记录（若字段存在），不硬写。
- 前端：`lib/api/deeplink.ts:75` 注释、i18n `zh.json` 中协议示例、`DeepLinkPage`/placeholder → `agentswitch://`；可加一行「也支持粘贴 ccswitch:// 链接」。
- 测试 `deeplink/tests.rs`：新增/改 `agentswitch://` 主用例；保留一条 `ccswitch://` 断言为**合法**（粘贴兼容）；`test_parse_invalid_scheme` 改用真正非法 scheme（如 `httpx://`）。
- `deplink.html`（根目录调试页，27 处）：低优先，属调试资产，可在 L6 顺带或跳过（不影响运行时）。

### L4 Updater 指向

- `tauri.conf.json:59` pubkey → 本机 `agent-switch.key.pub` 内容（base64，D4D16F6B…；即旧 main 已用的公钥）。
- `tauri.conf.json:61` endpoint → `https://github.com/XM-Chen/agent-switch/releases/latest/download/latest.json`。
- `commands/misc.rs:59` 手动检查更新跳转 URL、`:924` User-Agent `cc-switch` → agent-switch。
- 私钥不入库；本任务不生成 latest.json、不 push release。
- `createUpdaterArtifacts`：保持现状（构建时行为，不因指向改造而改）。

### L5 图标

- 从 `main` 迁入：`src-tauri/icons/{32x32.png,128x128.png,128x128@2x.png,icon.ico}`（已确认存在）。
- ccs 额外资源：`Square*Logo.png`、`StoreLogo.png`、`icon.png`、`64x64.png`、`tray/`。旧 main 无这些尺寸。
  - 方案：用旧 main 的 `icon.png`/`.ico` 作主图，`pnpm tauri icon <主图>` 重新生成全套（Tauri 官方生成器一次产出所有尺寸，含 Square/Store/tray）。这是最干净路径，避免手工拼图。
  - 若主图分辨率不足，退化为：替换核心 32/128/128@2x/ico，其余 Square/Store 用现有主图缩放生成。
- `tauri.conf.json` bundle.icon 列表按裁剪后（windows-only）现状保留引用，确保引用的文件都存在。

### L6 品牌文案

- `tauri.windows.conf.json` window title `CC Switch` → `Agent-Switch`（这是 Windows 覆盖配置，实际生效）。
- `zh.json`：`nav.title`、welcome `title`/`bodyDefault`/`bodyOfficial`、settings 中 `CC Switch 配置目录`/`launchOnStartupDescription`/skills `ccSwitch`/`ccSwitchHint`/`appConfigDir*`/`restartRequiredMessage`、about 等产品自称 → Agent Switch。
- **不改**：`partners`/促销 `code0`/`packycode`/`aff=ccswitch` 等第三方文案（Out of Scope，父任务暂缓 D22）；但其中把「CC Switch 用户专属」这类自称是否改，取决于是否算「产品自称」——判定标准：**描述本产品能力/归属的自称改；纯第三方营销话术留**。促销文案里的「CC Switch 用户」属营销话术边界，本任务**留**（记为已知残留，AC6 只约束产品自身页面）。
- 关于页：加「基于 CC Switch v3.16.5 修改，遵循 MIT，© Jason Young」。`LICENSE` 不动。
- `App.tsx:1179` `href="https://ccswitch.io"` 官网链接：改为项目仓库或移除（属产品自身外链，改）。
- `UpdateContext.tsx:31` localStorage key `ccswitch:update:dismissedVersion`：改 `agentswitch:...`（避免与本机 ccs 若共用 webview 存储冲突；低风险，顺带改）。

## 3. 兼容性 / 迁移

- 无 DB schema 变更；数据根改名后为全新空库，首启 import-before-seed 自然生效。
- 无跨产品数据迁移（禁止读 `~/.cc-switch`）。
- Deep Link 向后：系统层弃用 `ccswitch` 注册，但粘贴兼容保证用户手上的 ccs 分享链接仍可用。
- lib rename 是编译期变更，无运行期兼容问题。

## 4. 关键权衡

- **lib name 改不改**：AC1 要求 Cargo 名改。代价是全测试 `use` 改。选择改（满足既定身份决策），机械替换 + 一次编译验证。
- **upgradeCode 显式固定 vs 依赖派生**：派生值随 productName，脆弱。选择显式固定，防御未来漂移，满足 AC9。
- **促销/第三方文案边界**：全改会与 D22（暂缓）冲突且扩大范围。选择只改产品自称，第三方营销留，记为已知残留。
- **图标生成 vs 手工**：优先 `tauri icon` 重生成全套，避免 Square/Store 缺尺寸。

## 5. 运行/回滚

- 每层一个 commit；层内如过大（如 L1 波及测试多）可再拆。
- 回滚点：L0 前为纯裁剪基线；任一层出问题 `git revert` 该层 commit 即可，层间低耦合。
- 验证门（每层后）：`pnpm typecheck` / `pnpm test:unit` / `pnpm build:renderer` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`（`--manifest-path src-tauri/Cargo.toml`）。Windows 最终 `pnpm tauri build --no-bundle`。
- 已知基线预存失败（windows-zh-trim 记录：vitest 3 fail / clippy 13 error 属基线恒等）——身份改造不得使其恶化，新增测试须全绿。
