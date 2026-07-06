# Design: 应用内检查更新与一键增量更新

> 配套：`prd.md`、`implement.md`。官方文档依据见 `https://v2.tauri.app/plugin/updater`。

## 设计总览

接入 Tauri 官方 `tauri-plugin-updater` 插件，实现"应用内检查→下载→校验签名→安装→重启"全链路。更新包用 Tauri signer 签名，发布到 `XM-Chen/agent-switch` 的 GitHub Release，`latest.json` 作为更新清单。本期手动发版 MVP，CI 自动化列为后续演进。

## 架构边界

```
构建期（开发者本机）
  npm run tauri signer generate → 生成私钥 .key + 公钥 .key.pub（一次性）
  TAURI_SIGNING_PRIVATE_KEY=<...> npm run tauri build
    → 生成 *.msi + .msi.sig + *-setup.exe + .exe.sig
  手动创建 GitHub Release（tag=vX.Y.Z）
    → 上传更新包 + .sig + latest.json

运行期（已安装应用）
  前端 src/lib/updater.ts ── checkForUpdate() / downloadAndInstall()
        │  @tauri-apps/plugin-updater (JS API)
        ▼
  后端 src-tauri/src/lib.rs ── .plugin(tauri_plugin_updater::init())
  tauri.conf.json ── plugins.updater.{pubkey, endpoints}
        │
        │  HTTPS GET latest.json
        ▼
  GitHub Release: latest.json + 更新包 + .sig
        │
        │  下载 → 用 pubkey 校验 .sig → 安装 → 提示 relaunch
```

**不改动**现有应用业务逻辑（providers/endpoints/proxy 全不动）。updater 是横切能力，只接 `lib.rs` 插件注册 + 设置页 UI。

## 数据流与契约

### 1. `tauri.conf.json` 新增配置

```json
{
  "bundle": {
    "createUpdaterArtifacts": true,
    "...": "现有 targets/icon/windows.wix.language 不变"
  },
  "plugins": {
    "updater": {
      "pubkey": "<~/.tauri/agent-switch.key.pub 的完整内容>",
      "endpoints": [
        "https://github.com/XM-Chen/agent-switch/releases/latest/download/latest.json"
      ]
    }
  }
}
```

- `createUpdaterArtifacts: true` 让构建产出可更新的 `.msi`/`-setup.exe` + 各自 `.sig`（Tauri 2 直接签 MSI 本身，不是 `.msi.zip`）。
- `endpoints` 用 `releases/latest/download/latest.json`——GitHub 自动把 `latest` 解析为最新 Release tag。
- `pubkey` 是公钥 PEM 文本，写死在配置里（公钥可公开，私钥不可）。

### 2. `latest.json` 契约（每次发版手写）

```json
{
  "version": "0.2.0",
  "notes": "本次更新内容摘要（中文）",
  "pub_date": "2026-07-06T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<.msi.sig 文件全文内容>",
      "url": "https://github.com/XM-Chen/agent-switch/releases/download/v0.2.0/Agent-Switch_0.2.0_x64_zh-CN.msi"
    }
  }
}
```

- `version` 必须高于当前 `tauri.conf.json.version` 才触发更新。
- `signature` 是 `.sig` 文件的**完整文本内容**（不是 URL）。
- `url` 用具体 tag 的下载链接（`/releases/download/vX.Y.Z/<file>`），不用 `latest`（`latest` 在文件名维度不稳定）。
- 选 `.msi` 作为更新包（Tauri 2 直接签 MSI 本身产出 `.msi` + `.msi.sig`）；NSIS `-setup.exe` 作为备选/手动安装用。

### 3. 前端 updater 模块（`src/lib/updater.ts`）

```ts
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

export interface UpdateInfo {
  available: boolean;
  version?: string;
  notes?: string;
  downloadAndInstall?: () => Promise<void>;
}

export async function checkForUpdate(): Promise<UpdateInfo> {
  const update = await check();
  if (update) {
    return {
      available: true,
      version: update.version,
      notes: update.body,
      downloadAndInstall: async () => {
        await update.downloadAndInstall();   // 插件内部校验签名+安装
        await relaunch();                     // 需 tauri-plugin-process
      },
    };
  }
  return { available: false };
}
```

- `check()` 内部：GET `endpoints[0]` → 拉 `latest.json` → 比对版本 → 返回 `Update` 对象或 null。
- `update.downloadAndInstall()` 内部：下载 `url` → 用配置里的 `pubkey` 校验 `signature` → 安装 → 安装完成。
- `relaunch()` 需要 `tauri-plugin-process`（额外加一个依赖）。

### 4. 设置页 UI（`src/pages/SettingsPage.tsx`）

新增"关于与更新"区块：
- 显示当前版本（从 `app_info` API 或 `package.json` version 取）。
- "检查更新"按钮 → 调 `checkForUpdate()` → 三态展示：
  - 检查中（loading）
  - 有新版：显示 `version` + `notes`（markdown 渲染或纯文本）+ "立即更新"按钮
  - 已是最新：显示"当前已是最新版本"
- "立即更新"按钮 → 调 `downloadAndInstall()` → 显示下载进度（插件 `onEvent` 给 progress 事件）→ 完成后自动 relaunch。
- 错误态：网络失败/签名失败/下载失败 → 红色提示，不崩溃。

### 5. 后端插件注册（`src-tauri/src/lib.rs`）

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_updater::init())   // 新增
    .plugin(tauri_plugin_process::init())   // 新增（relaunch 用）
    // ...
```

### 6. 签名密钥管理

- **生成**（一次性）：`npm run tauri signer generate -- -w ~/.tauri/agent-switch.key -p <强密码>`，产出 `.key`（私钥，加密）+ `.key.pub`（公钥）。
- **保管**：私钥 `.key` 与密码存用户密码管理器/离线介质。**不进 git**。`~/.tauri/` 不在仓库目录内，天然不入库；额外在仓库 `.gitignore` 加 `*.key` 兜底防误拷。
- **使用**：构建前 `export TAURI_SIGNING_PRIVATE_KEY=$(cat ~/.tauri/agent-switch.key)` + `export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=<密码>`（或直接传路径）。
- **公钥**：`~/.tauri/agent-switch.key.pub` 内容粘进 `tauri.conf.json` 的 `plugins.updater.pubkey`。公钥进 git（配置文件在仓库里）是安全的。

## 兼容性与迁移

- **对已安装旧版的影响**：当前已构建的 `0.1.0` MSI **没有 updater 插件**，无法收到本次更新能力。本次改动从 `0.2.0`（或下一个版本号）起生效——已装 `0.1.0` 的用户需手动下载一次新版安装包，之后的更新才能走应用内。这是 updater 方案的固有冷启动限制，文档里说明。
- **版本号**：发新版要同步改 `tauri.conf.json.version` + `package.json.version` + `latest.json.version`。`latest.json.version` 必须大于当前才触发更新。
- **bundle.targets = "all"**：保持不变，仍同时产出 MSI + NSIS。updater 用其中 `.msi` + `.msi.sig` 那份。
- **`windows.wix.language = "zh-CN"`**：保持不变。

## 关键 trade-off

1. **手动发版 vs CI 自动化**：本期手动。代价是每次发版开发者本地跑 4 步（设 env、build、创 Release、传资产+latest.json）；收益是不引入 CI 复杂度，先把功能跑通。design 末尾给 CI 演进方向。
2. **`.msi` vs `-setup.exe` 作更新包**：选 `.msi`。Tauri 2 直接签 MSI 本身产出 `.msi` + `.msi.sig`；NSIS 也可但 MSI 是 Windows 标准。两个 `.sig` 都生成，`latest.json` 只填 MSI 那份。
3. **`endpoints` 单 URL vs 多 URL 镜像**：单 URL（GitHub Release）。后续若考虑 CDN 镜像可加备用 URL，本期不需要。
4. **私钥加密码 vs 不加**：加密码（`-p`）。代价是构建时要额外传 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`；收益是私钥文件泄露也不能直接签名。密码同样不入 git。
5. **检查更新手动触发 vs 启动自动检查**：本期手动（设置页按钮）。自动后台检查涉及启动时网络请求+频率控制+不打扰策略，列为后续演进。
6. **`tauri-plugin-process` 依赖**：必须加，`relaunch()` 靠它。代价是多一个插件；无替代（自己实现重启进程在 Tauri 沙箱里不安全）。

## 回滚考虑

- **配置回滚**：`tauri.conf.json` 的 updater 配置新增项可整段删除回滚；`createUpdaterArtifacts` 删了就回到不产出 `.sig` 的旧构建。
- **依赖回滚**：`tauri-plugin-updater`/`tauri-plugin-process` 从 `Cargo.toml`/`package.json` 删即可；`lib.rs` 的 `.plugin(...)` 两行删即可。
- **前端回滚**：`src/lib/updater.ts` 整文件删；`SettingsPage.tsx` 的"关于与更新"区块删。
- **已发布更新包**：GitHub Release 可删除/标记 draft 撤回；`latest.json` 可改回旧版本（但已升级的用户不受影响）。
- **私钥泄露应急**：若私钥泄露，生成新密钥对 → 公钥更新进 `tauri.conf.json` → 发一个用新私钥签名的强制更新版（用户装该版后，之后的校验用新公钥）。本期不实现强制更新，但 design 记下这个应急路径。

## 操作约束

- 私钥与密码**永不入 git**：`~/.tauri/` 在 home 目录不在仓库内；`.gitignore` 加 `*.key`、`*.key.pub`（防开发者误把公钥/私钥拷进仓库——公钥虽可公开，但约定只把内容写进 tauri.conf.json，不留仓库内副本文件）。
- 构建带签名必须设 `TAURI_SIGNING_PRIVATE_KEY` 环境变量；未设则 `createUpdaterArtifacts` 产出无 `.sig`（或构建报错，取决于 Tauri 版本）——发版前必须确认 env 已设。
- `latest.json` 的 `version` 必须**严格大于**当前已发布版本（semver 比较），否则 updater 判定无更新。
- `pub_date` 用 ISO 8601 UTC（`2026-07-06T12:00:00Z`）。

## 后续演进（不在本期范围）

- **CI 自动发版**：GitHub Actions workflow——push tag `vX.Y.Z` 触发 → `tauri build`（ secrets 存 `TAURI_SIGNING_PRIVATE_KEY`/密码）→ 用 `gh release create` 上传资产 → 生成 `latest.json` 并上传。这是手动 MVP 跑通后的自然下一步。
- **自动后台检查**：应用启动后异步 `check()`，有新版时顶部 banner 提示（不打断用户）。
- **macOS/Linux 更新**：`latest.json` 补 `darwin-x86_64`/`darwin-aarch64`/`linux-x86_64` 字段，需在对应平台构建签名。
- **强制更新/最低版本阻断**：`latest.json` 加 `v2_field` 或在 `notes` 里约定，应用侧逻辑判断。
