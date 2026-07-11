# Trim 批次验证结果

> 本任务 `07-10-ccs-windows-zh-trim` 执行期间的每批次验证输出摘要。基线既有失败（bootstrap 记录）：前端 4 个 OpenClaw 测试、Rust 13 clippy errors、8 个 Windows 测试、10 个 cargo warning。批次不得使其恶化。
>
> **实测基线（B1 提交点、排除 `.claude/worktrees/**`）**：`1 failed file / 3 failed tests / 385 passed`（`tests/integration/App.test.tsx` 3 个 OpenClaw/MSW 相关）。vitest 默认不读 `.gitignore`，会把 `.claude/worktrees/` 里的重复测试副本也收集进来导致假性膨胀（224 fail）；本任务验证统一用 `--exclude='.claude/**' --exclude='**/.claude/**'`。

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

**提交**
- `dbce5ccc1 chore(trellis): plan ccs-windows-zh-trim task (batches + B1 result)`
- `db9a8958e chore(platform): trim non-windows bundling and assets`

**结论**：B1 无新增失败，验证门绿。已提交。

## B2：语言裁剪（i18n + 切换 UI）

**改动**
- `git rm`：`src/i18n/locales/en.json`、`ja.json`、`zh-TW.json`；`src/components/settings/LanguageSettings.tsx`。
- `src/i18n/index.ts`：只留 `import zh`；`Language = "zh"`；`getInitialLanguage() => "zh"`；`resources` 仅 zh；`fallbackLng: "zh"`。
- `src/components/settings/SettingsPage.tsx`：删 `LanguageSettings` import 与 JSX 调用；保留 `ThemeSettings` 等同级设置项。
- **不动** `Settings.language` 字段、`useSettings.ts`/`useSettingsForm.ts` 读写链、后端 Settings 模型（design §3.3）。
- 连带测试（本批次必改，非基线既有）：
  - `tests/components/SettingsDialog.test.tsx`：删 LanguageSettings mock；就绪信号改用 `theme-settings`；删 `change-language` 交互断言。
  - `tests/integration/SettingsDialog.test.tsx`：同上，就绪信号改 `getByTestId("theme-settings")`。

**核对**
- `grep LanguageSettings src/` → 无残留引用（仅 tests 文件被同步清理）。
- typecheck 不报 `Language` 联合成员缺失。
- 其他 `zh-TW`/`en`/`ja` 引用（`useSettingsForm.ts`/`types.ts`/`schemas/settings.ts`/`format.ts`/`useDragSort.ts` 等）按 design 保留：`Settings.language` 字段类型仍可写 zh，运行时入口固定 zh。

**验证**
- `pnpm typecheck` → 通过。
- `pnpm format:check` → 通过。
- `pnpm vitest run --exclude='.claude/**' --exclude='**/.claude/**'` → `1 failed file / 3 failed tests / 385 passed`，与 B1 提交点实测基线完全一致，无新增失败。
- `pnpm build:renderer` → 通过（B2 改动后已在 SettingsDialog 专项验证前跑过，typecheck 绿即足以保证渲染构建）。

**结论**：B2 无新增失败，验证门绿。已提交（`ebf11f331 feat(i18n): pin to zh-CN and remove language switcher`）。

## B3：文档裁剪

**改动**
- `git rm` 顶层非中文 README：`README.md`（英）、`README_DE.md`、`README_JA.md`。保留 `README_ZH.md` 不改名。
- `git rm` `docs/guides/*-en.md` + `*-ja.md`（8 文件）；`docs/guides/*-zh.md` 保留（含 codex 系列指南）。
- `git rm` `docs/release-notes/*-en.md` + `*-ja.md`（51 文件）；`*-zh.md` 保留（23 文件）。
- `git rm -r docs/user-manual/en docs/user-manual/ja`（各 26 文件 + README）；`docs/user-manual/zh` 保留。
- `git rm` `docs/user-manual/assets/*-en.png` + `*-ja.png`（12 文件）；无后缀共用 png + `image-2026...png` 保留。
- `docs/user-manual/README.md`：删多语索引表，改为只指向中文手册（en/ja 链接已失效）。品牌 `CC Switch` 保留（身份任务处理）。

**核对**
- `find docs -type f \( -name '*-en.md' -o -name '*-ja.md' -o -name '*-en.png' -o -name '*-ja.png' \)` → 0 残留。
- `ls README*` → 仅 `README_ZH.md`。`ls src/i18n/locales/` → 仅 `zh.json`。
- 身份未变：`grep CC Switch|com.ccswitch.desktop|ccswitch src-tauri/tauri.conf.json` 仍命中（身份任务才改）。
- 多应用未删：`src/components/openclaw/`、`src-tauri/src/commands/workspace.rs` 仍在。

**验证**
- `pnpm typecheck` → 通过。
- `pnpm build:renderer` → 通过（built in 16.07s）。文档不被代码 import，删除不影响构建。
- 文档批无单测覆盖；B2 已确认全测 3 fail / 385 pass 与基线一致，B3 不触源码故维持。

**结论**：B3 无新增失败，验证门绿。待提交。
