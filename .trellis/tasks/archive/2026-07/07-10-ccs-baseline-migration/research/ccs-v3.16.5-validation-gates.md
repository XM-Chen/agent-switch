# Research: ccs v3.16.5 Windows 原样基线验证门

- **Query**: 研究 ccs v3.16.5 官方构建/测试要求和 Windows 验证门；检查包管理器、工具链、脚本、CI、Tauri 配置、Rust 工程、前端测试、安装器/updater 签名与外部前置条件；定义身份重命名/功能裁剪前验证原样基线的精确命令，并记录本环境可能阻塞项。
- **Scope**: mixed（官方 tag 源码、相邻 ccs 工作副本、当前 Windows 环境、Tauri 官方文档）
- **Date**: 2026-07-10

## 结论摘要

ccs `v3.16.5` 的原样 Windows x86_64 基线应至少通过四层门：

1. **来源/版本门**：工作树必须精确位于官方发布提交 `8d1b3306d09a27b9d8fc29694791d8421aba5f93`，且 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 三处版本均为 `3.16.5`。
2. **官方 CI 等价门**：以 Node 20、pnpm 10.12.3、锁文件冻结安装；运行前端 typecheck、format check、Vitest；运行 Rust fmt、Clippy（warnings 视为错误）和全量 Cargo tests。
3. **Windows 构建门**：在 Windows/MSVC + WebView2 + MSI 所需 VBSCRIPT 环境下执行原样 `pnpm tauri build`。由于 `bundle.targets = "all"` 且配置了自定义 WiX 模板，该命令必须生成 Windows MSI；同时会走前端 `pnpm run build:renderer`。
4. **Updater 发布形态门**：`createUpdaterArtifacts = true` 强制要求 Tauri updater 私钥；成功构建应产生 MSI 旁的 `.sig`。官方发布再将 MSI/`.sig` 改名并写入 `latest.json` 的 `windows-x86_64` 条目。Tauri updater 签名与 Windows Authenticode 代码签名是两套不同机制；ccs v3.16.5 官方 Windows workflow只显式配置前者，没有 Windows 代码签名证书配置或验证步骤。

## Files Found

### 官方 ccs v3.16.5（commit `8d1b3306...`）

| File Path | Description |
|---|---|
| `package.json` | 版本 `3.16.5`；pnpm/Tauri/Vite/Vitest/typecheck/format 脚本；未声明 `packageManager` 或 `engines`。 |
| `pnpm-lock.yaml` | pnpm lockfile v9；官方安装门使用 `--frozen-lockfile`。 |
| `pnpm-workspace.yaml` | 空 workspace 包列表，并允许特定 built dependency。 |
| `rust-toolchain.toml` | 钉住 Rust `1.95`，minimal profile，含 `rustfmt`、`clippy`。 |
| `src-tauri/Cargo.toml` | Rust crate 版本 `3.16.5`，MSRV `1.85.0`；Tauri 2.8.2；Windows 依赖 `winreg`；release profile。 |
| `src-tauri/Cargo.lock` | Rust 依赖锁文件；基线检查应使用 `--locked` 补充验证其不可漂移。 |
| `src-tauri/tauri.conf.json` | 版本、identity、构建钩子、`targets: all`、`createUpdaterArtifacts: true`、自定义 WiX 模板、updater 公钥和官方 endpoint。 |
| `src-tauri/tauri.windows.conf.json` | Windows 窗口覆盖：可见标题栏及标题。 |
| `src-tauri/wix/per-user-main.wxs` | MSI 使用的自定义 WiX 模板；Windows 原样构建必须实际经过该模板。 |
| `vitest.config.ts` | jsdom、两个 setup 文件、全局 API、text/lcov coverage reporter；没有覆盖率阈值。 |
| `tests/**/*.test.{ts,tsx}` | 官方 tag 中共 62 个前端测试文件；未发现 Playwright/E2E 配置。 |
| `src-tauri/tests/*.rs` | 官方 tag 中 11 个 Rust integration test 文件；另有大量内联 Rust tests。 |
| `.github/workflows/ci.yml` | 官方质量门：Node 20、pnpm 10.12.3、冻结安装、typecheck、format、unit tests、fmt、Clippy `-D warnings`、Cargo tests。 |
| `.github/workflows/release.yml` | Windows x64/ARM64 发布构建、Tauri updater 密钥、MSI/portable 资产、签名收集和 `latest.json` 组装。 |
| `CONTRIBUTING.md` | 对外说明 Node 18+/pnpm 8+/Rust 1.85+/Tauri 2 prerequisites；提交前命令与 CI 一致。 |
| `src/lib/updater.ts` | 使用 `@tauri-apps/plugin-updater` 检查、下载、安装和重启。 |
| `src-tauri/capabilities/default.json` | updater/process 等前端 IPC 权限；运行时 updater smoke test 依赖此处授权。 |

### 当前项目相关规范

| File Path | Description |
|---|---|
| `.trellis/tasks/07-10-ccs-baseline-migration/prd.md` | AC4 要求原样基线通过官方依赖安装、typecheck/单测/Rust 检查并至少完成一次构建；D8 首期仅 Windows；D11 固定 v3.16.5；D14 后续更换自有 updater；身份修改必须发生在原样验证之后。 |
| `.trellis/spec/guides/app-stack-conventions.md` | Tauri v2/React/Vite/Rust 栈及 Windows 构建需 MSVC、PATH 中的 rustc/cargo、Windows icon。 |
| `.trellis/spec/backend/quality-guidelines.md` | 当前项目后端通常要求 fmt/check/clippy/test；对本次“官方 ccs 基线”以 ccs 自身 CI 为主。 |
| `.trellis/spec/frontend/quality-guidelines.md` | 当前旧基线的 npm 命令不适用于 ccs；迁移后必须服从 ccs 的 pnpm 脚本。 |

## Code Patterns

### 1. 包管理器和前端工具链

`package.json:6-16` 的官方脚本为：

```json
"dev": "pnpm tauri dev",
"build": "pnpm tauri build",
"build:renderer": "vite build",
"typecheck": "tsc --noEmit",
"format:check": "prettier --check ...",
"test:unit": "vitest run"
```

官方 tag 未在 `package.json` 声明 `packageManager`/`engines`，因此**精确版本以 CI 为准**：`.github/workflows/ci.yml:21-30` 使用 Node 20 和 pnpm 10.12.3；`.github/workflows/ci.yml:44-54` 使用 `pnpm install --frozen-lockfile`、`pnpm typecheck`、`pnpm format:check`、`pnpm test:unit`。

`CONTRIBUTING.md:23-25` 只给宽松下界 Node 18+、pnpm 8+、Rust 1.85+；建立可复现基线时应采用 CI 的更精确版本，而不是只满足下界。

### 2. Rust 工具链与工作区形态

- `rust-toolchain.toml:1-4`：Rust 1.95，minimal，安装 `rustfmt` 与 `clippy`。
- `src-tauri/Cargo.toml:1-9`：单一 crate（根目录没有 Cargo workspace manifest），edition 2021，MSRV 1.85。
- `.github/workflows/ci.yml:89-99`：CI 先创建空 `dist/`，随后运行：

```bash
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

本地 checkout 在仓库根已有 `rust-toolchain.toml` 时 Cargo/rustc 会自动选 1.95。`cargo check` 不是 ccs 官方 CI 的独立门，因为 Clippy 已编译检查所有正常目标；为明确“Rust 检查”状态，可在本地附加 `cargo check --locked --manifest-path ...`，但必须标注为补充门而非官方 CI 原命令。

### 3. 前端测试

- `vitest.config.ts:11-18` 使用 jsdom、`tests/setupGlobals.ts`、`tests/setupTests.ts`、globals。
- 官方 tag 有 62 个前端 test/spec 文件。
- 未发现 Playwright 配置、E2E 脚本或覆盖率阈值；因此 untouched baseline 不应虚构浏览器 E2E 门。

### 4. Tauri Windows 打包

`src-tauri/tauri.conf.json:36-50`：

```json
"targets": "all",
"createUpdaterArtifacts": true,
"windows": { "wix": { "template": "wix/per-user-main.wxs" } }
```

`src-tauri/tauri.conf.json:6-10` 规定 Tauri build 前先运行 `pnpm run build:renderer`。官方 release x86_64 Windows 命令是 `.github/workflows/release.yml:259-270` 的 `pnpm tauri build`；ARM64 是 `pnpm tauri build --target aarch64-pc-windows-msvc --bundles msi`。

Windows 原样基线首要验证 x86_64，与本机架构及官方 `windows-2022` runner 对齐。ARM64 是独立 release matrix，不是当前 x64 主机的最低本地门。

### 5. Updater 签名和发布清单

`src-tauri/tauri.conf.json:39,62-66` 同时启用 updater artifact、内置官方公钥和官方 GitHub `latest.json` endpoint。

`.github/workflows/release.yml:132-186` 在**所有平台构建前**要求 `TAURI_SIGNING_PRIVATE_KEY`，可选 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。因此原样 `pnpm tauri build` 在没有 updater 私钥时很可能在生成 updater artifacts/signatures 阶段失败；这不是产品代码失败，而是发布凭据缺失。

`.github/workflows/release.yml:451-469` 收集 Windows MSI 和 MSI 旁边的 `.sig`；`.github/workflows/release.yml:656-738` 将签名内容（不是路径）写入 `latest.json`，Windows key 为 `windows-x86_64` / `windows-aarch64`。官方 release `v3.16.5/latest.json` 确实含两种 Windows 平台和对应 MSI/签名 URL。

Tauri 官方说明 updater 签名不可禁用；公钥可入库，私钥必须保密，丢失私钥后已安装客户端将无法接受后续更新。构建时私钥必须通过环境变量提供，`.env` 不生效。

### 6. Windows Authenticode 与 updater 签名不是同一门

Tauri 官方 Windows signing 文档说明：Windows 代码签名用于降低 SmartScreen 未信任警告和 Microsoft Store 分发；不是 Windows 执行应用的硬前提。

ccs v3.16.5 中：

- 未发现 `certificateThumbprint`、`digestAlgorithm`、`timestampUrl`、custom sign command 或 Windows certificate secrets。
- release workflow 只显式准备 Tauri updater minisign 私钥。
- workflow 只对 macOS 执行代码签名/公证验证；没有对应 Windows `signtool verify` 门。

因此**“复现官方 ccs v3.16.5 baseline”不应把 Authenticode 证书作为阻塞**；但未来公开发布独立 Agent Switch 时，Windows SmartScreen/代码签名应作为单独发布门处理，不能把 `.msi.sig` 误当 Authenticode 签名。

## 精确 Windows 原样基线验证命令

以下命令应在**干净、精确位于 commit `8d1b3306...` 的工作树**执行。它们不是本次研究中已经运行的命令；本次遵守要求，未安装依赖、未构建、未切分支。

### Gate 0 — 来源、干净度和版本一致性（只读）

PowerShell：

```powershell
$ExpectedCommit = '8d1b3306d09a27b9d8fc29694791d8421aba5f93'
$actual = git rev-parse HEAD
if ($actual -ne $ExpectedCommit) { throw "Wrong ccs baseline: $actual" }
if (git status --porcelain) { throw 'Baseline worktree is not clean' }

git show -s --format='%H %cI %s' HEAD

$pkg = Get-Content package.json -Raw | ConvertFrom-Json
$tauri = Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json
$cargoVersion = (Select-String '^version = "([^"]+)"' src-tauri/Cargo.toml | Select-Object -First 1).Matches.Groups[1].Value
@($pkg.version, $tauri.version, $cargoVersion) | ForEach-Object {
  if ($_ -ne '3.16.5') { throw "Version mismatch: $_" }
}
```

预期：HEAD 精确匹配；状态为空；三处版本都是 3.16.5。

### Gate 1 — 工具链版本和 Windows prerequisites（只读）

```powershell
node --version          # 官方 CI：20.x
corepack --version
pnpm --version          # 官方 CI：10.12.3
rustup show active-toolchain  # 在仓库根应自动解析为 1.95-x86_64-pc-windows-msvc
rustc --version         # 应为 rustc 1.95.x
cargo --version
rustup component list --installed | Select-String 'rustfmt|clippy'

& "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" `
  -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
  -property installationPath

Get-ChildItem "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application" -Directory
Get-WindowsOptionalFeature -Online -FeatureName VBSCRIPT
```

预期：Node 20、pnpm 10.12.3、Rust 1.95 MSVC、rustfmt/clippy、VS C++ Desktop toolchain、WebView2。由于 `targets = all` 包含 MSI，VBSCRIPT 应 Enabled；该检查需要管理员 PowerShell，若无管理员权限可先记录“未确认”，实际 MSI 构建是最终裁决。

### Gate 2 — 锁文件冻结安装

```powershell
pnpm install --frozen-lockfile
```

这是唯一应采用的基线安装命令；不要用 npm/yarn，也不要省略 frozen lockfile。执行后再次确认 `git status --porcelain` 为空，防止 lockfile 漂移。

### Gate 3 — 官方前端 CI 等价门

```powershell
pnpm typecheck
pnpm format:check
pnpm test:unit
```

这三条与 `.github/workflows/ci.yml` 完全一致。`pnpm build:renderer` 会在 Tauri build 中再次执行，但也可在打包前单独执行以更快区分 Vite 与 Rust/installer 故障：

```powershell
pnpm build:renderer
```

### Gate 4 — 官方 Rust CI 等价门

在仓库根执行：

```powershell
New-Item -ItemType Directory -Force dist | Out-Null
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml
```

说明：官方 CI 原命令未写 `--locked`；这里增加 `--locked` 以保证官方 `Cargo.lock` 不漂移。若要求字面复刻 CI，可去掉 `--locked`，但基线验证建议保留。补充检查：

```powershell
cargo check --locked --manifest-path src-tauri/Cargo.toml
```

### Gate 5 — Windows x86_64 release bundle（原样配置）

因为 `createUpdaterArtifacts = true`，先以**安全取得的测试/官方 updater 私钥**设置当前进程变量；不要写入仓库或 `.env`：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = '<private-key-path-or-content>'
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '<password-if-any>'
pnpm tauri build
```

预期产物至少包括：

```text
src-tauri/target/release/cc-switch.exe
src-tauri/target/release/bundle/msi/*.msi
src-tauri/target/release/bundle/msi/*.msi.sig
```

基线只要求证明官方原样构建链可跑通；不要在此阶段改 `createUpdaterArtifacts`、updater 公钥/endpoint、bundle target 或 WiX 模板来绕过失败。若没有可用私钥，应把 Gate 5 记为“由发布凭据阻塞”，不要把无签名 build 伪装成完整原样发布构建。

### Gate 6 — 产物结构/安装/启动 smoke gate

构建成功后：

```powershell
$msi = Get-ChildItem src-tauri/target/release/bundle/msi -Filter *.msi -Recurse | Select-Object -First 1
if (-not $msi) { throw 'MSI not generated' }
if (-not (Test-Path "$($msi.FullName).sig")) { throw 'Updater signature not generated' }

msiexec /i "$($msi.FullName)" /passive /l*v "$env:TEMP\cc-switch-v3.16.5-install.log"
```

人工/运行时 smoke：

1. 能启动 `CC Switch` 主窗口，窗口标题/品牌仍为官方原样值。
2. 首次启动初始化数据目录和数据库成功。
3. 设置页可打开；updater check 能连接官方 endpoint 并正确判断当前 3.16.5（不要真的安装更新）。
4. 退出/卸载行为正常；保留 MSI install log。

**重要隔离条件**：本机已有 `~/.cc-switch`，而原样 ccs 会使用该目录。为了避免污染真实 CC Switch 用户数据，运行 smoke 时必须使用隔离 Windows 用户/VM/Windows Sandbox，或先对真实目录做经用户批准的安全备份后再测试。不要仅设置 Git Bash `HOME` 就假定 Windows GUI/Tauri 一定使用该值；独立 OS 用户或 VM 是更可靠的原样运行隔离。

### Gate 7 — updater manifest 形态（发布验证，非普通本地 build 必需）

对官方 `v3.16.5` 发布，检查：

```powershell
$latest = Invoke-RestMethod 'https://github.com/farion1231/cc-switch/releases/download/v3.16.5/latest.json'
if ($latest.version -ne '3.16.5') { throw 'latest.json version mismatch' }
$win = $latest.platforms.'windows-x86_64'
if (-not $win.url -or -not $win.signature) { throw 'Missing Windows updater entry' }
if ($win.url -notmatch 'CC-Switch-v3\.16\.5-Windows\.msi$') { throw 'Unexpected MSI URL' }
```

未来身份修改后要用自有私钥/公钥和自有仓库重新建立此门；不能复用 ccs 官方私钥，也不能继续指向其 endpoint。

## 推荐的单次 Windows 验证顺序

```powershell
# 0. 只读来源/版本/工具链检查
# 1. 安装（唯一会改变依赖目录的步骤）
pnpm install --frozen-lockfile

# 2. 前端 CI
pnpm typecheck
pnpm format:check
pnpm test:unit
pnpm build:renderer

# 3. Rust CI
New-Item -ItemType Directory -Force dist | Out-Null
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo check --locked --manifest-path src-tauri/Cargo.toml

# 4. 原样签名 Windows build
$env:TAURI_SIGNING_PRIVATE_KEY = '<secure-value>'
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '<secure-value-if-needed>'
pnpm tauri build

# 5. MSI + .sig 存在性、隔离环境安装/启动 smoke
```

通过定义：所有命令 exit code 0，git tracked files 无变化，MSI 与 updater `.sig` 均存在，隔离安装/启动成功；任何跳过项必须记录具体阻塞，不可笼统写“环境问题”。

## 当前环境的已知/高概率阻塞项

| 项目 | 当前观察 | 影响 |
|---|---|---|
| 当前工作树不是 ccs v3.16.5 产品树 | 当前 agent-switch worktree 是旧 npm/Tauri 项目；相邻 `../cc-switch` 为脏副本且显示 3.15.0。官方 commit object `8d1b3306...` 已存在于当前 Git object DB，但未切换。 | 本次不能实际运行 ccs 验证；正式执行必须在新建的干净 `agent-switch-ccs` 工作树/分支进行。 |
| pnpm 未激活 | `pnpm` command not found；Corepack 0.34.0 存在。 | 所有官方前端/构建门立即阻塞。正式执行需按官方 CI 激活 pnpm 10.12.3；本次禁止安装/激活，未执行。 |
| Node 版本不匹配 | 当前 Node `v22.19.0`，官方 CI 是 Node 20。 | 可能可工作，但不是官方可复现基线；应使用 Node 20 复验。 |
| Rust 1.95 未安装 | 当前只有 `stable-x86_64-pc-windows-msvc`，rustc/cargo 1.96；`rust-toolchain.toml` 会要求下载/安装 1.95。 | 在网络受限或禁止安装时，Cargo 命令阻塞；用 1.96 运行也不等于精确官方基线。 |
| MSVC/Windows SDK 已存在 | VS 2022 Community VC tools 14.44 已安装；Windows SDK 中存在 `signtool.exe`；x86_64 MSVC target 已装。 | 原生 x64 编译基础较好。Git Bash 普通 shell找不到 `cl` 属正常；Cargo/MSVC 会通过 VS 工具发现机制使用。 |
| WebView2 已存在 | `Program Files (x86)/Microsoft/EdgeWebView/Application` 下有 149/150 runtime。 | Tauri GUI 前置条件满足。 |
| WiX/NSIS Tauri cache 已存在 | `%LOCALAPPDATA%/tauri/WixTools314` 含 `candle.exe`/`light.exe`；NSIS cache 也存在。 | installer tool 下载大概率不是阻塞；实际 MSI build仍需验证 VBSCRIPT。 |
| VBSCRIPT 状态未完整确认 | `vbscript.dll` 可加载且 System32/SysWOW64 文件存在；DISM 查询因当前进程无管理员权限返回 740。 | 高概率可用，但只有实际 MSI build或管理员 feature 查询能闭环。 |
| updater 私钥不存在 | `TAURI_SIGNING_PRIVATE_KEY` / password 当前均未设置。 | 原样 `pnpm tauri build` 很可能在 updater artifact signing 阶段阻塞；这是当前最明确的发布构建 blocker。不得使用 ccs 官方私钥（不可获得/不可复用）；可在正式基线验证中使用临时自有测试 key，只要不修改内置公钥并明确该 build 仅验证生成链。运行时签名校验若要测试，则私钥必须与配置公钥配对，官方私钥不可用，因此官方 updater 安装链只能验证其已发布资产或在身份提交后用自有 key 完整验证。 |
| Windows Authenticode 未配置 | 当前环境有 signtool，但无 certificate env；ccs 源码/workflow也未配置 Windows证书。 | 不阻塞复现官方 baseline，但会影响未来公开分发的 SmartScreen 体验。 |
| 已有真实 `~/.cc-switch` | 当前用户 home 中该目录已存在。 | 原样应用 smoke test 有读取/修改真实 CC Switch 数据的风险，必须使用隔离用户/VM/Windows Sandbox。 |
| 网络依赖 | frozen pnpm install、首次 Rust 1.95、crates、Tauri tools均可能需要网络。 | 当前 cache 可降低 installer 工具风险，但不能保证 pnpm/Cargo 依赖和 toolchain完全离线。 |
| Windows ARM64 不属于当前最低门 | 官方 release有 `windows-11-arm`、LLVM、aarch64 target和单独 MSI。当前只安装 x86_64 target。 | Windows-only首期若只发布 x64，不应让 ARM64 阻塞 untouched x64 baseline；若承诺复现官方全 Windows matrix，则需真实 ARM runner或交叉环境另验。 |

## External References

- [cc-switch v3.16.5 Release](https://github.com/farion1231/cc-switch/releases/tag/v3.16.5) — 官方正式 release；2026-07-01 发布，含 Windows x64/ARM64 MSI、portable zip、`.sig` 和 `latest.json`。
- [cc-switch v3.16.5 source tree](https://github.com/farion1231/cc-switch/tree/v3.16.5) — 本报告所有官方源码/CI 行号的版本来源。
- [Tauri v2 Prerequisites](https://v2.tauri.app/start/prerequisites/) — Windows 必需 Microsoft C++ Build Tools（Desktop development with C++）、WebView2；MSI/`targets: all` 需要 VBSCRIPT optional feature。
- [Tauri Windows Installer](https://v2.tauri.app/distribute/windows-installer/) — MSI 由 WiX Toolset v3 生成且只能在 Windows 创建；NSIS 生成 setup exe；Windows 上使用 `tauri build`。
- [Tauri Updater Plugin](https://v2.tauri.app/plugin/updater/) — updater 签名不可关闭；私钥环境变量、`createUpdaterArtifacts`、Windows MSI/NSIS `.sig` 和 static JSON schema。
- [Tauri Windows Code Signing](https://v2.tauri.app/distribute/sign/windows/) — Authenticode 的 SmartScreen/Store作用、证书/SignTool 配置；与 Tauri updater minisign 签名不同。

## Related Specs

- `.trellis/tasks/07-10-ccs-baseline-migration/prd.md` — AC4 和 D8/D11/D14/D16 决定了本报告的基线、平台、身份和更新签名边界。
- `.trellis/spec/guides/app-stack-conventions.md` — Tauri/Windows/MSVC 项目级约定。
- `.trellis/spec/backend/quality-guidelines.md` — 当前项目 Rust质量门参考；正式 ccs baseline以官方 ccs CI命令优先。
- `.trellis/spec/frontend/quality-guidelines.md` — 当前旧 npm基线规范；迁移到 ccs后应由pnpm命令替代。

## Caveats / Not Found

- 本次按要求未安装依赖、未构建、未切分支、未修改产品代码，因此命令状态均是“定义/待执行”，不是通过证明。
- 相邻 `E:/SynologyDrive/git_files/cc-switch` 工作副本不是干净 v3.16.5（读取时为 3.15.0），不能用其执行结果代表官方基线；官方结论来自 commit `8d1b3306...` 和 GitHub tag/release。
- ccs 官方 CI 运行在 Linux，不含 Windows PR job；Windows gate来自 release workflow。因此“Windows baseline”应同时跑官方 CI命令和官方 Windows release build，而不是二选一。
- 官方 package scripts列出 `pnpm lint`/CONTRIBUTING列出 lint，但 `package.json` 实际没有 `lint` script；不要把不存在的 `pnpm lint` 加入精确命令，否则会人为制造失败。官方 CI也不运行 lint。
- 官方 Windows workflow没有 Authenticode验证；不能声称官方 MSI 已通过 Windows代码签名，仅能确认其 updater `.sig` 存在。
- 使用临时自有 updater key可以验证 `.sig` 生成，但因原样配置内嵌的是 ccs 官方公钥，不能验证该临时签名的运行时安装。完整自有 updater端到端门应在后续独立身份/更新源提交后执行。
