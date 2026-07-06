# 发版流程（应用内更新 MVP）

> 配套任务：`.trellis/tasks/07-06-app-updater`。本文档描述**手动发版**的完整步骤：构建带签名的更新包 → 创建 GitHub Release → 上传资产与 `latest.json`。
> CI 自动发版列为后续演进，本期手动执行。

## 前置条件（一次性）

- 已生成 Tauri 签名密钥对：
  - 私钥 `~/.tauri/agent-switch.key`（加密码保护），**永不入 git**，存密码管理器 / 离线备份。
  - 公钥 `~/.tauri/agent-switch.key.pub` 内容已写入 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`。
- GitHub 仓库 `XM-Chen/agent-switch` 为 **PUBLIC**（Release 资产可匿名 HTTPS 下载）。
- 本机装有 `gh` CLI 并已登录（`gh auth status`）。
- Windows 构建环境：MSVC 生成工具、`cargo`/`rustc` 在 PATH、`icons/icon.ico` 存在。

> 若密钥尚未生成（首次），执行：
> ```bash
> npm run tauri signer generate -- -w ~/.tauri/agent-switch.key -p <强密码>
> ```
> 把 `.key.pub` 全文粘进 `tauri.conf.json` 的 `plugins.updater.pubkey`，密码记入密码管理器。

## 冷启动限制（务必知晓）

已安装的 `0.1.0`（无 updater 插件）用户**收不到**应用内更新，需手动下载一次带 updater 的新版安装包。从该版本起，之后的更新才能走应用内。这是 updater 方案的固有冷启动特性。

## 发版步骤

### 1. 同步版本号

发新版前把版本号从当前值提升（必须**严格大于**线上已发布版本，否则 updater 判定无更新）：

- `src-tauri/tauri.conf.json` 的 `version`
- `package.json` 的 `version`

两处保持一致（例如 `0.1.0` → `0.2.0`）。

### 2. 设置签名环境变量

构建前必须设 `TAURI_SIGNING_PRIVATE_KEY`（私钥文件内容）+ 密码，否则 `createUpdaterArtifacts` 产出的更新包无 `.sig` 签名，无法用于更新。

Git Bash：

```bash
export TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/agent-switch.key)
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="<私钥密码>"
```

PowerShell：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content -Raw "$HOME\.tauri\agent-switch.key"
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "<私钥密码>"
```

### 3. 质量门 + 构建

```bash
cd src-tauri && cargo fmt --check && cargo check && cargo clippy --all-targets -- -D warnings && cargo test --lib
cd .. && npm run build
npm run tauri build
```

### 4. 确认产出（`.sig` 必须非空）

```bash
ls src-tauri/target/release/bundle/msi/*.msi src-tauri/target/release/bundle/msi/*.msi.sig
ls src-tauri/target/release/bundle/nsis/*-setup.exe src-tauri/target/release/bundle/nsis/*-setup.exe.sig
```

- Tauri 2 对 Windows 的 updater 产物是 `.msi` + `.msi.sig`（直接签 MSI 本身，不是 `.msi.zip`）。`latest.json` 只填 MSI 那份。
- `.sig` 为空 = 环境变量未生效，回到步骤 2 重设后重新 build。

### 5. 编写 `latest.json`

新建一个 `latest.json`（不入仓库，随每次发版临时生成），按下面模板填充。`signature` 是 `.msi.sig` 文件的**完整文本内容**（base64，原样粘贴，不要被编辑器改换行）。`url` 用具体 tag 的下载链接。

```json
{
  "version": "0.2.0",
  "notes": "本次更新内容摘要（中文）",
  "pub_date": "2026-07-06T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<粘贴 *.msi.sig 文件全文>",
      "url": "https://github.com/XM-Chen/agent-switch/releases/download/v0.2.0/Agent-Switch_0.2.0_x64_zh-CN.msi"
    }
  }
}
```

字段约束：

- `version`：必须严格大于线上已发布版本（semver）。与步骤 1 的版本号一致。
- `signature`：`.msi.sig` 全文，非 URL。
- `url`：`/releases/download/vX.Y.Z/<文件名>` 具体 tag 链接，不用 `latest`（文件名维度不稳定）。
- `pub_date`：ISO 8601 UTC，如 `2026-07-06T12:00:00Z`。

取签名内容可用（示例文件名按实际替换）：

```bash
cat src-tauri/target/release/bundle/msi/Agent-Switch_0.2.0_x64_zh-CN.msi.sig
```

### 6. 创建 GitHub Release

tag 用 `vX.Y.Z`：

```bash
gh release create v0.2.0 --repo XM-Chen/agent-switch --title "v0.2.0" --notes "本次更新内容摘要"
```

### 7. 上传资产 + `latest.json`

```bash
gh release upload v0.2.0 --repo XM-Chen/agent-switch \
  src-tauri/target/release/bundle/msi/Agent-Switch_0.2.0_x64_zh-CN.msi \
  src-tauri/target/release/bundle/msi/Agent-Switch_0.2.0_x64_zh-CN.msi.sig \
  src-tauri/target/release/bundle/nsis/Agent-Switch_0.2.0_x64-setup.exe \
  src-tauri/target/release/bundle/nsis/Agent-Switch_0.2.0_x64-setup.exe.sig \
  latest.json
```

`endpoints` 配的是 `releases/latest/download/latest.json`——GitHub 自动把 `latest` 解析为最新 Release，无需每次改 endpoint。

### 8. 验证

- 装一个上一版应用，在设置页「关于与更新」点「检查更新」→ 应显示新版本号 + notes + 「立即更新」。
- 点「立即更新」→ 下载 + 校验签名 + 安装 + 自动重启 → 版本变为新版。
- 篡改验证：把 `latest.json` 的 `signature` 改成乱串重新上传 → 检查能检测到新版，但安装时报签名校验失败，不安装。

## 回滚 / 撤回

- 发错版本：`gh release delete v0.2.0 --repo XM-Chen/agent-switch` 或在 Release 页面标记 draft。已升级的用户不受影响（updater 无降级回滚，只能再发一个更高版本修复）。
- 改回 `latest.json` 指向旧版本不会让已升级用户降级。

## 私钥泄露应急

若私钥泄露：生成新密钥对 → 公钥更新进 `tauri.conf.json` → 用新私钥签名发一个更高版本。用户装该版后，之后的校验用新公钥。本期不实现强制更新，泄露期间需提示用户手动升级。

## 常见坑

- `.sig` 为空：`TAURI_SIGNING_PRIVATE_KEY` / 密码未设或未生效。发版硬阻塞，必须先确认 env。
- `signature` 粘错：`latest.json` 里必须是 `.sig` 文件全文（base64），不是文件路径，也不要被编辑器改动换行。
- `version` 未提升：updater 判定无更新，用户看到「已是最新版」。
- `url` 用了 `latest`：文件名随 tag 变化，`latest` 链接不稳定，必须用具体 `vX.Y.Z`。
- endpoint 拼写错 / Release 不存在：应用内检查更新 404，设置页显示错误提示。
