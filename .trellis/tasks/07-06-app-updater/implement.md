# Implement Plan: 应用内检查更新与一键增量更新

> 配套：`prd.md`、`design.md`。
> 质量门（每步结束跑）：`cd src-tauri && cargo fmt --check && cargo check && cargo clippy --all-targets -- -D warnings && cargo test --lib`，前端 `npm run build`。

## 执行检查表（有序）

### 步骤 1：生成签名密钥对（一次性，开发者本机）
- [ ] 跑 `npm run tauri signer generate -- -w ~/.tauri/agent-switch.key -p <强密码>`，生成 `.key`（私钥）+ `.key.pub`（公钥）。
- [ ] 把私钥密码记录到密码管理器；私钥文件留在 `~/.tauri/`（不拷进仓库）。
- [ ] 仓库 `.gitignore` 加 `*.key` 和 `*.key.pub` 兜底防误拷。
- [ ] **验证**：`ls ~/.tauri/agent-switch.key*` 两文件都在；`git check-ignore ~/.tauri/agent-switch.key` 不适用（在 home 不在仓库），但仓库内 `*.key` 应被忽略（`git check-ignore -v test.key` 测试）。
- **风险**：密码丢失=无法签名新版（私钥作废，需重新生成密钥对+发一版带新公钥的强制更新）。务必妥善保管密码。

### 步骤 2：后端依赖与插件注册
- [ ] `src-tauri/Cargo.toml` `[dependencies]` 加：
  - `tauri-plugin-updater = "2"`
  - `tauri-plugin-process = "2"`（relaunch 用）
- [ ] `src-tauri/src/lib.rs` 的 `tauri::Builder::default()` 链上加：
  - `.plugin(tauri_plugin_updater::init())`
  - `.plugin(tauri_plugin_process::init())`
- [ ] **验证**：`cargo check` 通过（插件能编译）。
- **风险**：插件版本要和 `tauri = "2"` 主版本对齐，用 `= "2"` 指定。`cargo build` 首次会拉新 crate，网络慢属正常。

### 步骤 3：tauri.conf.json updater 配置
- [ ] `bundle` 加 `"createUpdaterArtifacts": true`。
- [ ] 顶层加 `plugins.updater`：
  ```json
  "plugins": {
    "updater": {
      "pubkey": "<粘入 ~/.tauri/agent-switch.key.pub 完整内容>",
      "endpoints": [
        "https://github.com/XM-Chen/agent-switch/releases/latest/download/latest.json"
      ]
    }
  }
  ```
- [ ] **验证**：`npm run tauri build -- --debug`（或正式 build）能解析配置不报错；产出含 `.sig` 文件（见步骤 5）。
- **风险**：pubkey 多行 PEM 粘进 JSON 要转义换行为 `\n`，或用 `tauri` 接受的格式——参考官方示例确认 PEM 怎么嵌入 JSON（可能需要把换行转义）。`endpoints` URL 拼写错误会导致检查更新 404。

### 步骤 4：前端依赖与 updater 模块
- [ ] `npm install @tauri-apps/plugin-updater @tauri-apps/plugin-process`。
- [ ] 新建 `src/lib/updater.ts`，实现 `checkForUpdate()` 与 `downloadAndInstall()` 封装（design 第 3 节契约）。
  - `check()` 返回 `Update | null`；封装为 `UpdateInfo`。
  - `downloadAndInstall` 调 `update.downloadAndInstall()` 后 `relaunch()`。
  - 错误统一 try/catch 转 Error message 给 UI。
- [ ] **验证**：`npm run build`（tsc 通过）。
- **风险**：`@tauri-apps/plugin-process` 的 `relaunch` 需要后端 `tauri-plugin-process` 已注册（步骤 2 已做）。类型定义要和安装的插件版本匹配。

### 步骤 5：带签名构建验证
- [ ] 设环境变量：
  - `export TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/agent-switch.key)`（Windows bash）
  - `export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=<密码>`
- [ ] 跑 `npm run tauri build`。
- [ ] **验证**：`ls src-tauri/target/release/bundle/msi/*.msi.zip *.sig` 与 `bundle/nsis/*-setup.exe*.sig` 都存在且 `.sig` 非空。
- **风险**：env 未设会导致 `.sig` 不生成或为空——这是发版硬阻塞。PowerShell 设 env 语法不同（`$env:TAURI_SIGNING_PRIVATE_KEY=...`），文档里两种都写。

### 步骤 6：设置页 UI
- [ ] `src/pages/SettingsPage.tsx` 加"关于与更新"区块：
  - 当前版本（从现有 `app_info` API 取，或 `import { getVersion } from '@tauri-apps/api/app'`）。
  - "检查更新"按钮 → `checkForUpdate()` → loading/有新版/已最新三态。
  - 有新版时显示 `version` + `notes` + "立即更新"按钮。
  - "立即更新" → `downloadAndInstall()` + 进度（插件 `onEvent` 给 `Progress` 事件，显示百分比）→ 完成 relaunch。
  - 错误态红色提示，不崩溃。
- [ ] 对齐现有 SettingsPage 样式（深色模式、卡片布局）。
- [ ] **验证**：`npm run build` + `npm run tauri dev` 手动点检查更新（此时 GitHub 还没 latest.json，应显示网络/404 错误，确认错误处理正常）。
- **风险**：UI 复杂度中等，进度事件订阅要正确清理（useEffect cleanup）。没有真实 latest.json 前只能测错误态，正常态留到步骤 8 验证。

### 步骤 7：发版文档
- [ ] 新建 `docs/release.md`（或任务 notes），写完整发版步骤：
  1. 改 `tauri.conf.json.version` + `package.json.version`（同步）
  2. 设 `TAURI_SIGNING_PRIVATE_KEY` + `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  3. `npm run tauri build`
  4. 创建 GitHub Release：`gh release create vX.Y.Z --title "..." --notes "..."`
  5. 上传资产：`gh release upload vX.Y.Z <bundle/msi/*.msi.zip> <*.msi.zip.sig> <bundle/nsis/*-setup.exe> <*.sig> latest.json`
  6. `latest.json` 模板 + 每次填 `version`/`pub_date`/`signature`（粘 `.sig` 全文）/`url`
- [ ] **验证**：文档能独立复现，另一开发者照着能发版。
- **风险**：`.sig` 内容粘进 JSON 要保留原始格式（base64 文本），不要被编辑器改换行。

### 步骤 8：端到端验证（真实发一次测试版）
- [ ] 改版本号为 `0.2.0`（`tauri.conf.json` + `package.json`）。
- [ ] 带签名 build → 发 GitHub Release `v0.2.0` → 上传资产 + `latest.json`。
- [ ] 本机装一个旧版（`0.1.0`，无 updater——这一步验证的是"从 0.2.0 起后续更新"，0.1.0 用户需手动装 0.2.0 一次）。所以实际验证：装 `0.2.0`，再发一个 `0.2.1` Release，从 `0.2.0` 应用内检查更新到 `0.2.1`。
- [ ] 在 `0.2.0` 应用内点"检查更新" → 显示有 `0.2.1` → 点"立即更新" → 下载+安装+重启 → 版本变 `0.2.1`。
- [ ] 签名篡改验证：手动改 `latest.json` 的 `signature` 为乱串 → 检查更新能检测但安装时报"签名校验失败"。
- [ ] **验证**：AC4-AC8 全过。

### 步骤 9：质量门收敛
- [ ] `cd src-tauri && cargo fmt && cargo fmt --check && cargo check && cargo clippy --all-targets -- -D warnings && cargo test --lib`。
- [ ] `npm run build`。
- [ ] `git status` 确认 `~/.tauri/*.key` 未被追踪、仓库内无私钥副本。
- [ ] 全绿后进入 finish 阶段（spec 更新 + commit）。

## 验证命令汇总

```bash
# 签名构建（发版）
export TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/agent-switch.key)
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="<密码>"
cd src-tauri && cargo fmt --check && cargo check && cargo clippy --all-targets -- -D warnings && cargo test --lib
cd .. && npm run build
npm run tauri build

# 验证产出
ls src-tauri/target/release/bundle/msi/*.msi.zip *.sig
ls src-tauri/target/release/bundle/nsis/*-setup.exe *.sig

# 发版
gh release create vX.Y.Z --title "vX.Y.Z" --notes "..."
gh release upload vX.Y.Z <assets...> latest.json
```

## 风险文件 / 回滚点

- **新文件**：`src/lib/updater.ts`、`docs/release.md`——失败可整文件删回滚。
- **改动现有文件**：
  - `src-tauri/Cargo.toml`（加 2 依赖）：删依赖行回滚。
  - `src-tauri/src/lib.rs`（加 2 个 `.plugin()`）：删两行回滚。
  - `src-tauri/tauri.conf.json`（加 `createUpdaterArtifacts` + `plugins.updater`）：删配置块回滚。
  - `package.json`（加 2 依赖）：`npm uninstall` 回滚。
  - `src/pages/SettingsPage.tsx`（加"关于与更新"区块）：删区块回滚。
  - `.gitignore`（加 `*.key`）：可保留（无害）或删。
- **不动**：providers/endpoints/proxy/tool_takeover 等业务逻辑全不动。
- **不可回滚项**：已发布到 GitHub Release 的版本——可删除 Release/draft 撤回，但已升级用户不受影响（这是 updater 固有特性）。

## Review Gate（task.py start 前）

- [ ] PRD 已收敛（无 TBD、Open Questions 已解决、仓库 PUBLIC 已确认）。
- [ ] design.md 覆盖配置/契约/签名管理/trade-off/回滚/演进。
- [ ] implement.md 步骤可执行、验证命令明确。
- [ ] 用户确认私钥已生成且密码已妥善保管（步骤 1 是本机操作，不在代码里，需用户自证）。
- [ ] 用户 review 通过后再 `task.py start`。
