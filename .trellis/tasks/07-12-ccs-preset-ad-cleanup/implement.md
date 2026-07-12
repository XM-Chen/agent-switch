# Implement：清除 ccs 预设广告与返利内容

> 执行前须用户审核 prd/design/本计划并批准 `task.py start`。规划阶段不改产品代码。

## 0. 执行前门禁

- [ ] 用户审核 PRD / design / 本计划，确认 KEEP/DELETE 白名单与 KEEP-BUT-WASH 洗参规则。
- [ ] `task.py current` 指向本任务且 status=planning，再 `task.py start`。
- [ ] 工作树干净（除本任务文档）；分支 `agent-switch-ccs`。
- [ ] 记录基线门禁数：vitest 3 flaky、clippy/cargo 基线（前端改动为主，Rust 不涉及）。

## 1. 批次 B1：规范集 claudeProviderPresets.ts

- [ ] 按 DELETE 名单删除该文件中所有条目（PatewayAI/PackyCode/…/所有 aggregator 及小众 third_party）。
- [ ] KEEP-BUT-WASH 项按 design §3 洗 URL。
- [ ] 记录本文件实际删除的 name 集合 + 各自 `partnerPromotionKey`/`icon` 名，作为后续回收依据。
- [ ] 验证：该文件内 DELETE name 0 命中；KEEP name 全在。

**回滚点 B1**。

## 2. 批次 B2：其余 6 个应用预设文件镜像同步

- [ ] 对 `codex/gemini/claudeDesktop/openclaw/opencode/hermes` 各文件，按 B1 的 DELETE name 集合删镜像条目（按 name grep 定位，逐文件）。
- [ ] 同文件内 KEEP-BUT-WASH 项洗 URL。
- [ ] 验证：6 文件内 DELETE name 各 0 命中。

**回滚点 B2**。

## 3. 批次 B3：zh.json 文案回收

- [ ] 删除 `providerForm.partnerPromotion` 下所有被删预设的 key（对照 B1/B2 删除集）。
- [ ] 删除 `providerForm.presets` 下因删预设而失引用的中文名 key。
- [ ] 保留项若仍留 promotion 文案，去除 "CC Switch/cc-switch/ccswitch" 字样与优惠码（AC4）。
- [ ] 验证：zh.json 中被删 key 0 命中；残留 "CC Switch" 促销文案 0（partnerPromotion 节）。

**回滚点 B3**。

## 4. 批次 B4：图标注册表清理

- [ ] 跨 7 文件 grep 每个「候选悬空」icon 名，确认不再被任何保留预设引用。
- [ ] 删 `src/icons/extracted/index.ts` 对应 `import _x` 与 map 项；如无独占 svg 资源引用可留 svg 文件（不强制删资源）。
- [ ] 验证：`pnpm typecheck` 通过（兜底 noUnusedLocals）。

**回滚点 B4**。

## 5. 批次 B5：测试同步

- [x] 删除 3 个专属测试文件：`subrouterProviderPresets.test.ts`、`therouterProviderPresets.test.ts`、`therouterOpenCodeOpenClawPresets.test.ts`。
- [x] 共享测试：`codexChatProviderPresets.test.ts` 移除 Novita AI 期望项；其余共享测试无被删 name/URL/计数断言（rg 核验 0 残留）。
- [x] KEEP-BUT-WASH 项：无测试断言旧带参 URL（rg 核验 0）。

**回滚点 B5**。

## 6. 全量门禁与收尾

- [x] `pnpm typecheck` ✅ pass
- [x] `pnpm format` + `format:check` ✅ pass
- [x] `pnpm build:renderer` ✅ built（chunk 警告为基线）
- [x] 确定性 config 测试 12 files / 69 tests 全绿；全量 `vitest` 失败为 JSDOM `useContext` 环境噪声（stash 基线 216 fail/1100 噪声 vs 本次 229/402，非删预设导致，逐条核验无新增功能性红）
- [x] 静态扫描：AC1 DELETE name=0、AC2 返利参数=0、AC4 zh.json 促销文案=0
- [x] 对照 AC1–AC8 逐条核验通过（AC5 typecheck 兜底：23 个孤立图标 import 由自动生成 registry 消费，非 unused-local，编译 0 error；作为已知良性残留保留）
- [ ] 更新父任务 notes（A1 完成）；经用户同意后分批 commit；不 push。

## 7. 验证命令速查

| 命令 | 用途 |
|------|------|
| `rg -n "PackyCode\|SubRouter\|NekoCode\|<各 DELETE name>" src/config` | AC1 删除核验 |
| `rg -n "aff=\|ref=\|utm_content=ccswitch\|utm_source=cc_switch\|from=CH_\|invite/CC-SWITCH\|invitecode=\|ic=[A-Z0-9]\|ytag=.*cc-switch" src/config src/i18n/locales/zh.json` | AC2 返利参数残留 |
| `rg -n "CC Switch\|cc-switch\|ccswitch" src/i18n/locales/zh.json` | AC4 促销文案残留（排除必要保留） |
| `pnpm typecheck` | AC5 图标悬空兜底 |
| `pnpm vitest run --dir tests` | AC7 测试门禁 |
