# Trim 批次验证结果

> 本任务 `07-10-ccs-windows-zh-trim` 执行期间的每批次验证输出摘要。基线既有失败（bootstrap 记录）：前端 4 个 OpenClaw 测试、Rust 13 clippy errors、8 个 Windows 测试、10 个 cargo warning。批次不得使其恶化。

## B1：平台裁剪（打包配置 + 资产）

**改动**
- `src-tauri/tauri.conf.json`：`"targets": "all"` → `["msi"]`；`icon` 数组删 `icons/icon.icns`；删整个 `macOS.minimumSystemVersion` 键。`productName`/`version`/`identifier`/`deep-link`/`updater`/`wix`/`createUpdaterArtifacts` 均未动。
- `git rm`：`flatpak/`（4 文件）、`src-tauri/icons/android/`（15 文件）、`src-tauri/icons/ios/`（18 文件）、`src-tauri/icons/icon.icns`、`src-tauri/icons/dmg-background.png`。
- 保留：`icon.ico`/`icon.png`/png set/`Square*`/`StoreLogo.png`/`tray/`。

**核对**
- `grep icon.icns|dmg-background|flatpak src-tauri/` → 仅 `.gitignore` 构建产物忽略规则命中（保留），`tauri.conf.json` 无残留引用。
- `tray/` 仍在（design 要求保留，属源码级平台代码不删范围）。

**验证（Windows target）**
- `pnpm build:renderer` → 通过（built in 18.55s）。
- `cargo check --locked --manifest-path src-tauri/Cargo.toml` → 通过（10 warning，无 error，与基线一致）。
- `pnpm tauri build --no-bundle` → 通过（exit 0，release exe 构建成功，12m12s）。
- clippy 未单独重跑：B1 未改任何 Rust 源码，clippy error 集与基线（13）恒等。

**结论**：B1 无新增失败，验证门绿。待提交。
