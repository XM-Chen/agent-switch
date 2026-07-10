# Implement：仅 Windows + 仅简体中文裁剪

> 配套 `prd.md` / `design.md`。本文件是执行清单。按批次顺序执行，每批独立提交、独立验证、独立可回滚。未获 commit 授权停在可审查 diff。所有命令在仓库根 `E:/SynologyDrive/git_files/agent-switch`（分支 `agent-switch-ccs`）执行。

## 执行前门禁

- [ ] 确认 `git status -sb` 干净，当前分支 `agent-switch-ccs`，HEAD 在 bootstrap 归档后的提交（`06e4fa103` 或其后）。
- [ ] 确认 `task.py current --source` 指向 `07-10-ccs-windows-zh-trim`。
- [ ] 确认 `task.py start` 已执行（status=in_progress）后再动代码；本清单的代码步骤在 start 之后。
- [ ] 记录每批前后的验证命令输出摘要到 `research/trim-batch-results.md`（本任务执行时新建）。

## 批次 B1：平台裁剪（打包配置 + 资产）

### B1.1 改 `src-tauri/tauri.conf.json`
- [ ] `"targets": "all"` → `"targets": ["msi"]`。
- [ ] `icon` 数组删 `"icons/icon.icns"`。
- [ ] 删 `"macOS": { "minimumSystemVersion": "12.0" }` 整个键（注意逗号/JSON 合法性）。
- [ ] 不动 `productName`/`version`/`identifier`/`plugins.deep-link`/`plugins.updater`/`bundle.windows.wix`/`createUpdaterArtifacts`。

### B1.2 删非平台资产
- [ ] `git rm -r flatpak/`。
- [ ] `git rm src-tauri/icons/icon.icns src-tauri/icons/dmg-background.png`。
- [ ] `git rm -r src-tauri/icons/android/ src-tauri/icons/ios/`。
- [ ] 保留 `icon.ico`/`icon.png`/`32x32.png`/`64x64.png`/`128x32.png`/`128x128@2x.png`/`Square*.png`/`StoreLogo.png`/`tray/`。

### B1.3 核对
- [ ] `tauri.conf.json` 的 `icon` 数组每项对应文件在 `src-tauri/icons/` 仍存在。
- [ ] `grep -rn "icon.icns\|dmg-background\|flatpak" src-tauri/tauri.conf.json` 无残留引用。

### B1.4 验证（Windows target）
- [ ] `pnpm build:renderer` 通过。
- [ ] `cargo check --locked --manifest-path src-tauri/Cargo.toml` 通过。
- [ ] `cargo clippy --locked --manifest-path src-tauri/Cargo.toml -- -D warnings` 失败数 ≤ 基线（13，不新增）。
- [ ] `pnpm tauri build --no-bundle` 通过（targets 已改 msi，exe 可编译）。
- [ ] 记录结果到 `research/trim-batch-results.md` B1 段。

### B1.5 提交
- [ ] 提交：`chore(platform): trim non-windows bundling and assets`（tauri.conf.json + flatpak/ + 图标删除）。
- [ ] **回滚点 B1**：`git revert <B1-sha>`。

## 批次 B2：语言裁剪（i18n + 切换 UI）

### B2.1 删非 zh locale
- [ ] `git rm src/i18n/locales/en.json src/i18n/locales/ja.json src/i18n/locales/zh-TW.json`。
- [ ] 保留 `src/i18n/locales/zh.json`。

### B2.2 改 `src/i18n/index.ts`
- [ ] 删 `import en/ja/zhTW`，只留 `import zh`。
- [ ] `Language` 类型 → `"zh"`（删其他联合成员）。
- [ ] `getInitialLanguage()` 简化为 `(): Language => "zh"`（删 localStorage/navigator 分支）。
- [ ] `resources` 只留 `zh`。
- [ ] `fallbackLng: "en"` → `fallbackLng: "zh"`。

### B2.3 删语言切换 UI（保留 `Settings.language` 字段）
- [ ] `git rm src/components/settings/LanguageSettings.tsx`。
- [ ] `src/components/settings/SettingsPage.tsx`：删 `import { LanguageSettings }`（line 37 附近）+ 删 `<LanguageSettings ... />` JSX（line 255-258 附近），保留同级 `ThemeSettings` 等。
- [ ] **不动** `useSettings.ts`（line 276/410 写 `localStorage.language`）、`useSettingsForm.ts`（line 84/96 读 + `i18n.changeLanguage`）、后端 Settings 模型。

### B2.4 核对
- [ ] `grep -rn "LanguageSettings" src/` 无残留引用。
- [ ] `grep -rn "zh-TW\|languageOption" src/i18n/index.ts` 无残留（zh.json 里 key 可留）。
- [ ] typecheck 不报 `Language` 类型联合成员缺失。

### B2.5 验证
- [ ] `pnpm typecheck` 通过。
- [ ] `pnpm format:check` 通过。
- [ ] `pnpm test:unit` 失败数 ≤ 基线（4 个 OpenClaw 既有，不新增）。
- [ ] `pnpm build:renderer` 通过。
- [ ] 记录结果到 `research/trim-batch-results.md` B2 段。

### B2.6 提交
- [ ] 提交：`feat(i18n): pin to zh-CN and remove language switcher`。
- [ ] **回滚点 B2**：`git revert <B2-sha>`。

## 批次 B3：文档裁剪

### B3.1 枚举非中文文档
- [ ] `find docs -name '*-en.md' -o -name '*-ja.md'` 列清单，review 无中文文件混入。
- [ ] 确认顶层 README：`README.md`（英）、`README_DE.md`、`README_JA.md` 删；`README_ZH.md` 保留不改名。

### B3.2 删除
- [ ] `git rm README.md README_DE.md README_JA.md`。
- [ ] `git rm docs/**/*-en.md docs/**/*-ja.md`（按枚举清单）。
- [ ] `docs/user-manual/`、`docs/images/` 按后缀处理：`-en`/`-ja` 删，`-zh`/无后缀共用资产保留；无后缀文件单独 review 内容语言。
- [ ] `docs/release-notes/` 无后缀版本说明若有，纯中文保留，其他暂留不删。

### B3.3 核对
- [ ] `find docs -name '*-en.md' -o -name '*-ja.md'` 返回空。
- [ ] `ls README*` 只剩 `README_ZH.md`。
- [ ] `docs/guides/*-zh.md`（含 codex 系列）仍在。

### B3.4 验证
- [ ] 文档删除不影响构建，但跑 `pnpm typecheck` + `pnpm build:renderer` 确认无 README/docs 被代码 import。
- [ ] 记录结果到 `research/trim-batch-results.md` B3 段。

### B3.5 提交
- [ ] 提交：`docs: drop non-chinese readme and docs`。
- [ ] **回滚点 B3**：`git revert <B3-sha>`。

## 全任务最终验证（三批合并后）

- [ ] `git status -sb` 干净（除已提交）。
- [ ] 跑 design §6 全套验证命令，失败数 ≤ bootstrap 基线（前端 4 + clippy 13 + Rust 8，无新增）。
- [ ] AC1-AC9 逐条核对（见 prd.md）。
- [ ] `grep -rn "icon.icns\|dmg-background\|flatpak" src-tauri/` 无残留。
- [ ] `ls src/i18n/locales/` 仅 `zh.json`。
- [ ] `ls README*` 仅 `README_ZH.md`。
- [ ] 产品身份未变：`grep "CC Switch\|com.ccswitch.desktop\|ccswitch" src-tauri/tauri.conf.json` 仍命中（身份任务才改）。
- [ ] 多应用未删：`ls src/components/openclaw/`、`commands/workspace.rs` 等仍在。

## 完成

- [ ] `trellis-check`（Agent 形式）核对 AC1-AC9 + 跨层一致性。
- [ ] 阻塞或偏差写入 `research/trim-batch-results.md`。
- [ ] 汇报结果，后续由用户决定是否进入 `ccs-agent-switch-identity` 子任务或先 finish/archive 本任务。
