# Design：仅 Windows + 仅简体中文裁剪

> 配套 `prd.md`。本任务只做平台与语言两个维度。复杂任务，本文件负责技术设计：受影响文件、改法、契约、回滚、与身份改造子任务的边界。

## 1. 范围边界（与身份改造子任务 `ccs-agent-switch-identity` 切分）

本任务改 = 仅平台 + 仅语言。**不碰**以下任何一项（留给身份任务或保持原样）：

| 项 | 归属 |
|---|---|
| `productName: "CC Switch"` / `identifier: com.ccswitch.desktop` / `version: 3.16.5` | 身份任务 |
| `identifier` / npm 名 `cc-switch` / Cargo crate `cc-switch` | 身份任务 |
| Deep Link `ccswitch`（`plugins.deep-link`） | 身份任务 |
| updater `pubkey` + `endpoints`（ccs 官方源） | 身份任务（指向改造） |
| `bundle.windows.wix.template`（`wix/per-user-main.wxs`） | 身份任务（upgradeCode 决策） |
| `bundle.macOS.minimumSystemVersion` 键 | **本任务删**（属平台裁剪，非身份） |
| `bundle.targets: "all"` / `icon` 数组含 `icon.icns` | **本任务改**（平台裁剪） |
| `.github/workflows/release.yml`（CC Switch 命名/签名 secret/多平台 matrix） | **本任务不碰**，整体并入身份任务 |
| `.github/workflows/ci.yml`、`claude.yml`、`stale.yml` | **本任务不碰**（CI 保持 ubuntu，已定） |
| `flatpak/`、macOS/Linux 专属图标资产 | **本任务删** |
| `src/i18n/locales/*` 与语言切换 UI | **本任务改** |
| README/docs 非中文 | **本任务删** |

判断准则：**凡是和 `CC Switch` 品牌字符串、签名密钥、identifier、Deep Link、updater endpoint 直接绑定的，一律不动**，避免本任务引入半成品命名/凭据状态。`bundle.macOS`/`icon.icns`/`flatpak` 是纯平台资产裁剪，与身份无关，本任务处理。

## 2. 平台裁剪

### 2.1 `src-tauri/tauri.conf.json`

现状（关键字段）：
```jsonc
"bundle": {
  "targets": "all",
  "icon": ["icons/32x32.png","icons/128x128.png","icons/128x128@2x.png","icons/icon.icns","icons/icon.ico"],
  "windows": { "wix": { "template": "wix/per-user-main.wxs" } },
  "macOS": { "minimumSystemVersion": "12.0" }
}
```

改法：
- `"targets": "all"` → `"targets": ["msi"]`。`msi` 依赖的 `wix/per-user-main.wxs` 模板保留不动（身份任务决定 upgradeCode）。
- `icon` 数组去掉 `"icons/icon.icns"`（macOS 专用，删后只剩 Windows 用的 png/ico）。
- 删整个 `"macOS": { "minimumSystemVersion": "12.0" }` 键。Linux bundle 键（`linux`/`deb`/`rpm`/`appimage`）现状未配置，无需删。
- `createUpdaterArtifacts: true` 保留（updater 指向改造在身份任务，但生成 `.sig` 的开关本身与本任务无关，保留不增删风险最小）。

不改：`productName`、`version`、`identifier`、`build`、`app.*`、`security`、`plugins.deep-link`、`plugins.updater`。

### 2.2 删除文件

- `flatpak/`（整目录：`com.ccswitch.desktop.desktop`、`...metainfo.xml`、`...yml`、`README.md`）。
- `src-tauri/icons/icon.icns`（macOS）。
- `src-tauri/icons/dmg-background.png`（macOS DMG 背景）。
- `src-tauri/icons/android/`、`src-tauri/icons/ios/`（移动端图标，发布目标不含移动端）。
- 保留：`icon.ico`、`icon.png`、`32x32.png`、`64x64.png`、`128x128.png`、`128x128@2x.png`、`Square*.png`、`StoreLogo.png`、`tray/`（Windows/通用）。

资产删除前后核对：`tauri.conf.json` `icon` 数组与 `src-tauri/icons/` 实际文件一致，不引用已删文件。

### 2.3 源码 cfg 分支（D-A 浅）= 不动

`#[cfg(target_os = "macos"|"linux")]` 分支、前端 `navigator.platform`/Tauri `platform()` 判定**全部保留**。后果：仓库在 macOS/Linux 仍能 `cargo check`/`pnpm build`，只是不发布。本任务不引入 target-os 代码删除，避免破坏 bootstrap 阶段已验证的"全平台可编译"基线，也最小化上游同步冲突。

## 3. 语言裁剪（D-B 浅，保留 i18n 抽象）

### 3.1 `src/i18n/locales/`

- 删 `en.json`、`ja.json`、`zh-TW.json`。
- 保留 `zh.json`。
- `zh.json` 中与语言切换 UI 强相关的 key（`settings.language`/`languageHint`/`languageOptionChinese`/`languageOptionTraditionalChinese`/`languageOptionEnglish`/`languageOptionJapanese`）**保留不删**——因为 `LanguageSettings` 组件本次只删调用而非组件内文案，且删 key 是 i18n 内部清理、收益低、风险中（其他位置可能引用）。如删 UI 后这些 key 无任何引用，可在批次内顺手删，但**非硬性 AC**。

### 3.2 `src/i18n/index.ts`

改法：
- 删 `import en/ja/zhTW` 三行，只留 `import zh`。
- `Language` 类型从联合 `"zh"|"zh-TW"|"en"|"ja"` 收为 `"zh"`（或保留联合但运行时只可能 zh——选简化为 `"zh"` 更干净，但为最小改动可保留类型，仅缩 resources）。
- `getInitialLanguage()` 简化为直接 `return "zh"`，或保留函数体但所有分支返回 `"zh"`（删 navigator 分支）。选**直接返回 `"zh"`**，逻辑最简；localStorage/navigator 检测随之失效（不再读 `language` 存储项决定语言）。
- `resources` 只留 `zh`。
- `fallbackLng: "en"` → `fallbackLng: "zh"`（en 已删，fallback 到不存在的 locale 无意义）。

### 3.3 语言切换 UI（删 UI，保留 `Settings.language` 字段）

- 删 `src/components/settings/LanguageSettings.tsx`。
- `src/components/settings/SettingsPage.tsx`：
  - 删 `import { LanguageSettings }`（line 37）。
  - 删 `<LanguageSettings value={settings.language} onChange={(lang) => handleAutoSave({ language: lang })} />`（line 255-258）。保留同级的 `ThemeSettings`/`AppVisibilitySettings`/`SkillStorageLocationSettings`。
- **保留 `Settings.language` 字段与读写链**（`useSettings.ts` line 276/410 写 `localStorage.language`、`useSettingsForm.ts` line 84/96 读+`i18n.changeLanguage`）。理由（已定）：字段固定为 zh，不暴露切换入口；后端 Settings 模型不碰，上游同步友好。`i18n.changeLanguage("zh")` 调用仍安全（resources 含 zh）。
- 风险确认：删 UI 后没有任何代码路径能把 `language` 设为非 zh，`getInitialLanguage()` 也固定返回 zh，故运行时语言恒为 zh，满足"仅简体中文"。残留的 `localStorage.getItem("language")` 在 index.ts 里被删后不再读，无害。

### 3.4 i18n 抽象层（D-B 浅）= 保留

`i18next`/`react-i18next`/`useTranslation`/`t('key')` 全部保留。未来上游 UI 改动合并时 `t('key')` 调用照搬即可。

## 4. 文档裁剪（D-C 浅，全删非中文）

### 4.1 顶层 README

- 删 `README.md`（英文主）、`README_DE.md`、`README_JA.md`。
- 保留 `README_ZH.md`。是否改名回 `README.md`：**本任务不改名**。理由：`README.md`（英文）删掉后 GitHub 仓库首页会回退显示 `README_ZH.md`，无需改名即可生效；改名会动 git 历史 mv，且 README 主标题仍是 `CC Switch`（品牌属身份任务），改名无品牌收益反而增加 diff 噪声。身份任务可一并决定 README 改名+品牌。
- README 内的品牌 `CC Switch` 保留（身份任务处理）。

### 4.2 `docs/`

- `docs/guides/*-en.md`、`*-ja.md` 删；`*-zh.md` 保留（含 codex 系列指南——多应用保留，codex 指南有效）。
- `docs/release-notes/*-en.md`、`*-ja.md` 删；`*-zh.md` 保留。
- `docs/user-manual/`、`docs/images/`：按文件名后缀 `-en`/`-ja` 删，`-zh`/无后缀共用资产保留（实操时枚举目录内容按后缀判断）。
- `docs/release-notes/` 里若存在无后缀的版本说明（待枚举确认），按内容语言判断：纯中文保留，其他暂留不删（避免误删版本记录）。

删除前先 `find docs -name '*-en.md' -o -name '*-ja.md'` 枚举清单，review 后批量删，避免漏删或误删中文文件。

## 5. 受影响文件清单（汇总）

**删除**：
- `flatpak/`（目录）
- `src-tauri/icons/icon.icns`、`dmg-background.png`、`android/`、`ios/`
- `src/i18n/locales/en.json`、`ja.json`、`zh-TW.json`
- `src/components/settings/LanguageSettings.tsx`
- `README.md`、`README_DE.md`、`README_JA.md`
- `docs/**/*-en.md`、`docs/**/*-ja.md`

**修改**：
- `src-tauri/tauri.conf.json`（targets/icon/macOS）
- `src/i18n/index.ts`（resources/类型/getInitialLanguage/fallback）
- `src/components/settings/SettingsPage.tsx`（删 LanguageSettings 引用与调用）

**不动**：`release.yml`、`ci.yml`、`claude.yml`、`stale.yml`、所有 Rust 源码 cfg 分支、前端 platform 判定、`Settings.language` 字段读写、品牌/identifier/Deep Link/updater、`wix/` 模板。

## 6. 验证契约

每批次后必须绿（Windows target）：
```bash
pnpm typecheck
pnpm format:check
pnpm test:unit
pnpm build:renderer
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo check --locked --manifest-path src-tauri/Cargo.toml
pnpm tauri build --no-bundle
```

已知基线既有失败（bootstrap 记录）：前端 4 个 OpenClaw 测试、Rust 13 clippy errors、8 个 Windows 测试。这些是 ccs v3.16.5 上游既有问题，**本任务不得使其恶化**——批次前后失败集应一致或减少，不得新增失败。

`pnpm tauri build --no-bundle` 验证 `targets:["msi"]` 配置下 release exe 可编译。完整 MSI bundle 因 updater 私钥缺失仍阻塞（与 bootstrap 一致，非本任务引入）。

## 7. 批次与回滚

| 批次 | 内容 | 提交 |
|---|---|---|
| B1 | `flatpak/` 删 + tauri.conf.json（targets/icon/macOS）+ 非平台图标资产删 | `chore(platform): trim non-windows bundling and assets` |
| B2 | i18n locale 删 + index.ts 简化 + LanguageSettings 删 + SettingsPage 引用删 | `feat(i18n): pin to zh-CN and remove language switcher` |
| B3 | 非中文 README + docs 删 | `docs: drop non-chinese readme and docs` |

CI/release workflow 本任务不动，故无 CI 批次。每批独立提交、独立 revert。回滚点 = 逐批 `git revert <sha>`，无数据/DB 改动。

## 8. 风险与对策

- **删 `zh-TW.json`/`en.json` 后运行时 fallback**：`fallbackLng` 改 `zh` 后，任何缺失 key 直接 fallback 到 zh 自身（即显示 key 或空），不会崩。删 locale 前确认 `zh.json` key 集合 ⊇ 实际使用的 key（`t('...')` 调用），实操时以 typecheck + 运行时冒烟为准。
- **`Settings.language` 残留字段**：保留字段但无切换入口，未来上游若新增切换 UI 会复活该路径——届时再评估。本任务范围内可接受。
- **docs 误删中文文件**：严格按 `-en`/`-ja` 后缀 + README 精确文件名删除，无后缀文件单独 review。
- **上游同步**：D-A/D-B/D-C 全浅，本任务 diff 集中在 `tauri.conf.json`/`i18n`/`LanguageSettings`/`docs`/`flatpak`，未来 ccs upstream 合并时这些是常见改动点，冲突可控；如 ccs 上游重构 i18n 初始化，B2 需手动 reapply"固定 zh"逻辑。
