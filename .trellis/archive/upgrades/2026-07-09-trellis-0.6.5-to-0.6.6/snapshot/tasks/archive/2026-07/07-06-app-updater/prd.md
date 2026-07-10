# 应用内检查更新与一键增量更新

## Goal

让 agent-switch 已安装的用户能在应用内检查是否为最新版，若有新版可一键增量更新到最新包，免去手动去 GitHub 下载安装包覆盖安装。采用 Tauri 官方 `tauri-plugin-updater` 方案（增量补丁替换，非整包重装）。

## Background

- 现状：agent-switch 已能构建出 MSI（`bundle/msi/Agent-Switch_0.1.0_x64_zh-CN.msi`）与 NSIS 安装包，但应用内无任何更新能力；发版靠手动 push 源码到 GitHub，无 Release、无 CI、无签名。
- 方案选择（用户决策）：**方案 A——Tauri 官方增量更新**（`tauri-plugin-updater`），应用内下载增量补丁校验签名后静默替换。
- 签名密钥（用户决策）：**现在就生成正式密钥对**，私钥妥善保管（密码管理器/离线介质），公钥写进应用。
- 发布源（用户决策）：**公开 GitHub Release**，`latest.json` 与签名后的更新包放 `XM-Chen/agent-switch` 的 Release assets。

## Confirmed Facts

- Tauri v2 updater 配置（官方文档）：`tauri.conf.json` 加 `bundle.createUpdaterArtifacts: true`（生成更新用 MSI/NSIS + `.sig` 签名）+ `plugins.updater.pubkey`（公钥 PEM 内容）+ `plugins.updater.endpoints`（更新清单 URL 数组）。
- 签名密钥生成：`npm run tauri signer generate -- -w ~/.tauri/agent-switch.key`（可 `-p <密码>` 设私钥密码）；生成私钥 `.key` + 公钥 `.key.pub`。构建时设 `TAURI_SIGNING_PRIVATE_KEY` 环境变量（私钥路径或内容）+ 可选 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。
- `latest.json` 格式（官方）：`{ version, notes, pub_date, platforms: { "windows-x86_64": { signature, url } } }`，`signature` 是 `.sig` 文件内容，`url` 指向更新包（`.msi.zip` 或 `-setup.exe`）。
- Windows 更新包：`createUpdaterArtifacts: true` 时 Tauri 生成 `*.msi.zip`（MSI 压缩）和 `*-setup.exe`（NSIS）及其 `.sig`。
- 需新增依赖：后端 `src-tauri/Cargo.toml` 加 `tauri-plugin-updater = "2"`；前端 `package.json` 加 `@tauri-apps/plugin-updater`。
- 现有项目无 CI（`.github/workflows/` 不存在），发版要么手动构建+签名+上传 Release+写 latest.json，要么新建 GitHub Actions workflow 自动化。
- TLS 强制：`endpoints` URL 必须 HTTPS（GitHub Release 满足）。
- agent-switch `tauri.conf.json` 当前 `bundle.targets = "all"`、`windows.wix.language = "zh-CN"`，无任何 updater 配置。
- 应用版本号在 `tauri.conf.json` 的 `version: "0.1.0"` 与 `package.json` 的 `version: "0.1.0"`；updater 比对版本号决定是否有更新。
- `XM-Chen/agent-switch` GitHub 仓库已确认 **PUBLIC**（`gh repo view` 实测），Release 资产可匿名 HTTPS 下载，updater endpoints 指向 `https://github.com/XM-Chen/agent-switch/releases/latest/download/latest.json` 可行。

## Requirements

- **R1 签名密钥**：生成一对正式签名密钥，私钥写入 `~/.tauri/agent-switch.key`（加密码保护），公钥写入 `~/.tauri/agent-switch.key.pub`。私钥由用户妥善保管（密码管理器/离线备份），**不入 git、不入仓库任何目录**。公钥内容写入 `tauri.conf.json` 的 `plugins.updater.pubkey`。
- **R2 构建配置**：`tauri.conf.json` 加 `bundle.createUpdaterArtifacts: true`、`plugins.updater.pubkey`、`plugins.updater.endpoints`（指向 GitHub Release 的 `latest.json`）。`Cargo.toml` 加 `tauri-plugin-updater` 依赖，`lib.rs` 注册插件。
- **R3 前端依赖与 API**：`package.json` 加 `@tauri-apps/plugin-updater`；前端封装一个 updater 模块，提供 `checkForUpdate()`（返回是否有新版+版本号+更新说明）和 `downloadAndInstall()`（下载+校验签名+安装+提示重启）。
- **R4 应用内 UI**：设置页（`/settings`）加"检查更新"入口。点击后显示：当前版本、最新版本（若有）、更新说明（`notes`）、"立即更新"按钮。无更新时显示"已是最新版"。更新下载中显示进度，完成后提示重启生效。
- **R5 更新流程**：`checkForUpdate` → 比对版本 → 有新版则展示 → 用户点"立即更新" → `downloadAndInstall` 下载增量包 → 校验签名（插件自动用 pubkey 校验）→ 安装 → 提示重启。校验失败（签名不匹配/下载损坏）中止并报错，不安装。
- **R6 发版流程（手动 MVP）**：文档化手动发版步骤——本地设 `TAURI_SIGNING_PRIVATE_KEY` → `npm run tauri build` 生成带签名的更新包 → 创建 GitHub Release（tag = 版本号）→ 上传 `.msi.zip`/`.sig`/`-setup.exe`/`.sig` 与 `latest.json` → 发布。`latest.json` 的 `version`/`pub_date`/`signature`/`url` 每次发版更新。
- **R7 错误处理**：网络失败、Release 不存在、`latest.json` 解析失败、签名校验失败、下载中断——均给用户明确中文提示，不崩溃、不半安装。

## Acceptance Criteria

- [ ] AC1 `tauri.conf.json` 含 `createUpdaterArtifacts: true`、`plugins.updater.pubkey`（正式公钥）、`plugins.updater.endpoints`（指向 GitHub Release latest.json 的 HTTPS URL）。
- [ ] AC2 `npm run tauri build`（设 `TAURI_SIGNING_PRIVATE_KEY`）能生成 `*.msi.zip` + `.msi.zip.sig` + `*-setup.exe` + `.sig`，且 `.sig` 文件非空。
- [ ] AC3 设置页有"检查更新"入口，点击后显示当前版本号 `0.1.0` 并发起检查。
- [ ] AC4 模拟一个更高版本的 `latest.json`（指向一个签名正确的更新包），点击"检查更新"后显示"有新版 vx.y.z"+ 更新说明 + "立即更新"按钮。
- [ ] AC5 点击"立即更新"，插件下载更新包、校验签名通过、安装完成、提示重启；重启后应用版本变为新版。
- [ ] AC6 签名被篡改的更新包（用错密钥签名或改包内容）被插件拒绝安装，给出"签名校验失败"提示，不安装。
- [ ] AC7 `latest.json` 指向与当前版本相同或更低版本时，显示"已是最新版"。
- [ ] AC8 网络断开时点"检查更新"，给出"网络不可用，请稍后重试"提示而非崩溃。
- [ ] AC9 私钥文件 `~/.tauri/agent-switch.key` 不出现在 git 仓库任何位置（`.gitignore` 或仓库内无该文件副本）。
- [ ] AC10 发版步骤有文档（`docs/release.md` 或任务 notes），含环境变量设置、构建命令、上传 Release、写 latest.json 的完整流程。

## Out of Scope

- CI/CD 自动发版（本期手动发版 MVP；CI 自动化列为后续演进，design 里给出推荐方向）。
- macOS/Linux 更新（agent-switch 当前主要面向 Windows，`latest.json` 仅填 `windows-x86_64`；其它平台字段留空或后续补）。
- 强制更新/最低版本阻断（本期纯手动检查+用户主动更新，不强制）。
- 自动后台检查更新（本期手动触发；后台定时检查列为后续演进）。
- 更新进度断点续传（插件本身支持流式进度，但断点续传不在本期范围）。
- 增量差异补丁（Tauri updater 是"下载整包替换"式增量，非二进制 diff；这是插件机制决定的，非本期可改）。

## Open Questions

- 无（OQ1 仓库可见性已确认 PUBLIC，所有阻塞决策已解决）。
