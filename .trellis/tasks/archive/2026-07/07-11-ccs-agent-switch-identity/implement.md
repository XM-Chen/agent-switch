# Implement：身份改造为独立 Agent Switch

> 执行前须用户审核 `prd.md` / `design.md` / 本计划，并批准 `task.py start`。规划阶段不改产品代码。

## 0. 执行前门禁

- [x] 用户审核 PRD / design / 本计划。
- [x] `task.py current` 指向 `07-11-ccs-agent-switch-identity` 且 status 仍为 `planning`，再 `task.py start`。
- [x] 工作树干净或仅有本任务文档改动；分支 `agent-switch-ccs`。
- [x] 本机存在 `~/.tauri/agent-switch.key.pub`（公钥 D4D16F6B…）；无私钥入库。
- [x] 未获授权：不 push、不改默认分支、不发布。

## 1. 批次 B0：预备（不改产品逻辑）

- [x] 从旧身份取 upgradeCode 并记录到本文件「笔记」节：
  ```bash
  # 方式 A：若 Tauri CLI 支持
  pnpm tauri inspect wix-upgrade-code
  # 方式 B：临时在干净 worktree 用 productName=Agent-Switch 的 conf 取派生值
  ```
- [x] 记录公钥 base64 全文（与 main 分支 `tauri.conf.json` plugins.updater.pubkey 对照一致）。
- [x] 盘点 `rg` 确认 L1/L2/L3 硬编码清单与 design 一致（执行时再跑一次防漂移）。

**回滚点 B0**：无产品改动。

## 2. 批次 B1：L0 包/版本/identifier + WiX upgradeCode

- [x] `package.json`：name `agent-switch`，version `0.3.0`。
- [x] `src-tauri/Cargo.toml`：package `agent-switch`，version `0.3.0`，lib `agent_switch_lib`，repository `XM-Chen/agent-switch`。
- [x] `src-tauri/tauri.conf.json`：productName `Agent-Switch`，version `0.3.0`，identifier `com.agent-switch.app`；`bundle.windows.wix.upgradeCode` 写入 B0 记录的 GUID。
- [x] `src-tauri/src/main.rs`（或等价入口）`cc_switch_lib::` → `agent_switch_lib::`。
- [x] 全量替换 `src-tauri/tests/**` 中 `use cc_switch_lib` / `cc_switch_lib::` → `agent_switch_lib`。
- [x] 验证：`cargo check --manifest-path src-tauri/Cargo.toml` 编译通过（exit 0，10 条 Unix dead_code 警告为基线）。

**回滚点 B1**：revert 本批 commit。

## 3. 批次 B2：L1 数据根 / DB / 日志 / 测试 env

- [x] `config.rs`：`.cc-switch` → `.agent-switch`；`CC_SWITCH_TEST_HOME` → `AGENT_SWITCH_TEST_HOME`；legacy fallback 中目录/DB 名同步。
- [x] `lib.rs`：`cc-switch.db` / `cc-switch.log` / log `file_name` / 相关注释与错误文案。
- [x] `database/mod.rs`、`database/backup.rs`、`panic_hook.rs` 等派生路径。
- [x] 全部测试与源码中 `CC_SWITCH_TEST_HOME`、`.cc-switch`、`cc-switch.db` 字面量（运行时路径，不含 docs/release-notes）；前端 mock 路径同步。
- [x] 验证：`cargo check` 通过；运行时路径残留扫描 `CC_SWITCH_TEST_HOME|/.cc-switch|cc-switch.db|cc-switch.log` = 0（排除 docs/changelog/catalog）。

**回滚点 B2**。

## 4. 批次 B3：L2 Live 归属文件名

- [x] `codex_config.rs` 常量 `CC_SWITCH_CODEX_MODEL_CATALOG_FILENAME` 值改为 `agent-switch-model-catalog.json`；常量名可改为 `AGENT_SWITCH_CODEX_MODEL_CATALOG_FILENAME`（或保留旧常量名仅改值——**推荐改名**以免误导）。
- [x] 相关测试字面量同步。
- [x] 验证：codex catalog 相关单测通过。

**回滚点 B3**。

## 5. 批次 B4：L3 Deep Link

- [x] `tauri.conf.json` schemes：`["agentswitch"]`。
- [x] `deeplink/parser.rs`：接受 `agentswitch` 或 `ccswitch`。
- [x] `lib.rs` handle / starts_with / 注释 / Linux desktop 文件名提示（`cc-switch-handler.desktop` → `agent-switch-handler.desktop` 若硬编码）。
- [x] 前端 deeplink 注释与 i18n 示例。
- [x] `deeplink/tests.rs` + 集成测试更新。
- [x] 验证：主 scheme 绿、粘贴兼容绿、非法 scheme 红（`cargo test --lib deeplink` 33 passed）。

**回滚点 B4**。

## 6. 批次 B5：L4 Updater 指向

- [x] `tauri.conf.json` pubkey + endpoints。
- [x] `commands/misc.rs` 硬编码 URL / User-Agent。
- [x] 全仓运行时配置中无 `farion1231/cc-switch` latest.json / ccs 公钥。
- [x] 验证：配置静态检查 + 编译通过（不要求线上有 latest.json）。

**回滚点 B5**。

## 7. 批次 B6：L5 图标

- [x] 从 `main` 取出主图（优先 `icon.png` 若无则用 `128x128.png` / `icon.ico`）。
- [x] `pnpm tauri icon <主图>` 重新生成 `src-tauri/icons/**` 全套。
- [x] 确认 `tauri.conf.json` icon 列表文件均存在；删除残留 ccs 品牌图若生成器未覆盖。
- [x] 验证：资源存在；可选 dev 启动看托盘/窗口图标。

**回滚点 B6**。

## 8. 批次 B7：L6 品牌文案

- [x] `tauri.windows.conf.json` title。
- [x] `zh.json` 产品自称（nav/welcome/settings/about/skills 路径提示等）；**不改** partners 促销长文案。
- [x] 关于页「基于 CC Switch v3.16.5」归属说明。
- [x] `App.tsx` 外链、`UpdateContext` localStorage key。
- [x] 验证：`pnpm typecheck` / `pnpm test:unit` / `pnpm build:renderer`。

**回滚点 B7**。

## 9. 全量门禁与收尾

- [x] 全量门禁结果（2026-07-11）：
  | 门禁 | 结果 | 说明 |
  |------|------|------|
  | `pnpm typecheck` | ✅ pass | |
  | `pnpm format`+`format:check` | ✅ pass | 已 prettier 写入 8 个改动文件 |
  | `pnpm build:renderer` | ✅ built | chunk>500kB 警告为基线 |
  | `pnpm vitest run --dir tests` | 375 pass / 2–3 fail | `App.test.tsx` OpenClaw MSW timeout，= 裁剪基线 3 fail（flaky） |
  | `cargo fmt --check` | ✅ pass | 已 `cargo fmt` 写入 |
  | `cargo clippy -D warnings` | 13 error | 全在 `commands/misc.rs`（Unix dead_code + needless_return），= 基线 13，零新增 |
  | `cargo test --lib -j2 --test-threads=1` | 1706 pass / 8 fail | 8 = 6 Codex Win upgrade + 1 codex_history + 1 claude_desktop 端口串扰，= 基线 8，零新增 |
  | 集成测试二进制 | 环境受限 | os error 1455/页面文件不足致链接期 mmap 失败，非代码问题 |
  | `pnpm tauri build --no-bundle` | ✅ | release `agent-switch.exe` 已产出（约 29MB，10 条 Unix dead_code 警告为基线） |
- [x] 对照 AC1–AC11：AC1/2/3/4/7/8/9 静态核验通过；AC5（首启 import-before-seed 未改动，语义保留）；AC6 运行时自称已改、关于页加归属；AC10 门禁不恶化；AC11 未 push/未发版。
- [x] 记录已知残留：`zh.json > providerForm.partnerPromotion` 促销「CC Switch 用户/合作伙伴」；`transform_codex_chat.rs:3065` issue 链接注释；源码内部注释/格式说明中的 "CC Switch"（非用户可见）；`main` 源图近空壳致重生成图标偏小。
- [x] 更新父任务 notes / 本任务 check 结果（父任务 notes 已于 commit 7217f0486 记录身份改造完成状态；**不**在未授权下归档父任务）。
- [x] 经用户同意后再 commit（L0–L6 已分批提交 9998b7a6f…a61b530d0，trellis/spec 收尾 7217f0486、29b2d1d20）。

## 10. 验证命令速查

| 命令 | 用途 |
|------|------|
| `rg -n "com\\.ccswitch|~/.cc-switch|cc-switch\\.db|CC_SWITCH_TEST_HOME|farion1231/cc-switch" --glob '!docs/**' --glob '!.trellis/**' --glob '!node_modules/**'` | 运行时身份残留扫尾 |
| `rg -n "scheme != |schemes|agentswitch|ccswitch" src-tauri/src/deeplink src-tauri/tauri.conf.json` | Deep Link 契约 |
| `cargo test --manifest-path src-tauri/Cargo.toml deeplink` | Deep Link 单测 |
| `pnpm tauri build --no-bundle` | Windows release 可执行文件 |

## 11. 笔记（执行时填写）

- upgradeCode GUID：`370c2d3e-a326-5100-8146-efa72c816642`（`pnpm tauri inspect wix-upgrade-code`，productName=Agent-Switch 派生；2026-07-11 B0 记录）
- 公钥确认：与 main 一致  
  `dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQ0RDE2RjZCMTRCNzAwNzcKUldSM0FMY1VhMi9SMUg4WHllSlpzZEQ2YkU0UUpGR0U1ZzMyK1A3eGF2cVcwZGRtM0dFTktkRXIK`  
  （`~/.tauri/agent-switch.key.pub` 内容一致）
- `tauri icon` 主图路径：`tools/_tmp_as_icons/128x128@2x.png`（main 的 256px，已 `pnpm tauri icon` 重生成；main 源图极小≈空壳，已知局限）
- 基线预存失败数（windows-zh-trim）：vitest 3 fail / clippy 13 error 等——身份改造后不得恶化。
- B0 硬编码盘点（2026-07-11）：`CC_SWITCH_TEST_HOME` 多处；`.cc-switch`/`cc-switch.db`/`cc-switch.log` 运行时+测试；`cc_switch_lib` Cargo+tests；catalog 常量 `CC_SWITCH_CODEX_MODEL_CATALOG_FILENAME`；deeplink scheme `ccswitch`。与 design 一致。

