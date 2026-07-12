# 清除继承自 ccs 的预设广告与返利内容（A1 / D22 精选）

## Goal

清除 ags 内置 Provider 预设中继承自 cc-switch 的**广告、返利/来源跟踪参数、促销文案**，落实 D22「精选官方目录」：保留一手模型厂商官方模板 + 少数知名聚合入口，删除聚合/中转商预设，并把保留项 URL 洗成裸官方链接。用户始终可通过完整 JSON 自定义任意供应商，内置目录不构成对第三方的背书。

## 范围来源与已决策

- 源自父任务 `07-10-ccs-baseline-migration` 的 D22 暂缓项，2026-07-12 复核后确认执行。
- **保留边界（用户 2026-07-12 拍板）= 中等档**：一手模型厂商官方 + `OpenRouter` / `SiliconFlow` / `ModelScope` 三个知名聚合入口；其余聚合/中转商删除。
- **返利参数处理（用户拍板）= 洗成裸官方链接**：保留项保留 preset 与官方域名 URL，仅删除 `aff/ref/utm_*ccswitch/from=CH_/邀请码/优惠码` 等返利或来源跟踪参数。
- 影响面：ccs 多应用支持已保留（Claude-only 裁剪撤销），故 **7 个应用预设文件全部涉及**：`claude / codex / gemini / claudeDesktop / openclaw / opencode / hermes`。`universalProviderPresets.ts` 无广告，不动。
- 约束：最小化与 ccs 的差异——只做「删条目 / 洗参数 / 删文案 key」，不重构预设数据结构、不改类型定义、不动无关字段。

## 保留 / 删除白名单（规范集 claudeProviderPresets.ts，其余 6 文件按名镜像同步）

### KEEP（保留，不删）
一手模型厂商官方：
- Claude Official、Gemini Native、DeepSeek、Zhipu GLM / GLM en、MiniMax / MiniMax en、Kimi / Kimi For Coding、StepFun / StepFun en、Baidu Qianfan Coding Plan、Bailian / Bailian For Coding、Xiaomi MiMo / MiMo Token Plan、火山Agentplan、BytePlus、DouBaoSeed、AWS Bedrock (AKSK) / (API Key)、Nvidia、KAT-Coder、Longcat、BaiLing
托管上游：GitHub Copilot、Codex
知名聚合入口（中等档特批）：OpenRouter、SiliconFlow / SiliconFlow en、ModelScope

### KEEP-BUT-WASH（保留 preset，删 URL 里的返利/邀请参数）
- 火山Agentplan：删 `utm_content=ccswitch&utm_source=OWO` 及邀请码 `rc=6J6FV5N2`
- BytePlus、DouBaoSeed：删 `utm_content=ccswitch&utm_source=OWO`
- Zhipu GLM / GLM en：删邀请码 `ic=...`
- Kimi / Kimi For Coding：删 `aff=cc-switch`
- SiliconFlow / en：删邀请码 `i/YflgU2Ve`
- MiniMax / en：清理其 partnerPromotion 文案（见下），URL 若含参一并洗

### DELETE（删除整个 preset + 其 partnerPromotionKey 引用 + zh.json 文案 key + 悬空图标 import + 对应测试）
- 全部 `category: "aggregator"` 且不在 KEEP 名单：Shengsuanyun、CCSub、SubRouter、Unity2.ai、Qiniu、FennoAI、ZetaAPI、TeamoRouter、Amux、AiHubMix、CherryIN、DMXAPI、AtlasCloud、ClaudeAPI、Code0、NekoCode、RunAPI、Compshare / Compshare Coding Plan、TheRouter、Novita AI、PIPELLM
- 伪装成 third_party 的中转商：PatewayAI、PackyCode、APIKEY.FUN、APINebula、ClaudeCN、Cubence、AIGoCode、RightCode、AICodeMirror、CrazyRouter、SSSAiCode、Micu、OpenCode Go
- 小众非一手 third_party（无返利参数但非模型厂商官方，按精选目录删）：SudoCode、RelaxyCode、E-FlowCode、ETok.ai

> 注：KEEP 名单里 Nvidia 在规范文件被标 aggregator，但 Nvidia NIM 属一手官方入口，归入 KEEP。若某应用文件缺失/多出个别镜像条目，以「按 name 匹配」为准，不臆造条目。

## Requirements

- R1 删除 DELETE 名单中所有预设在 7 个应用文件里的**全部镜像条目**（按 name 匹配，行号仅参考、执行时重新定位防漂移）。
- R2 保留 KEEP 名单预设；对 KEEP-BUT-WASH 项，只删 URL 中的返利/邀请/来源参数，保留官方域名与路径主体，确保链接仍指向官方注册/文档页。
- R3 回收被删预设独占的 `zh.json > providerForm.partnerPromotion.<key>` 文案；`providerForm.presets.<key>` 里随之失去引用的中文名映射一并删除。
- R4 清理 `src/icons/extracted/index.ts` 中因删预设而悬空的 `import` 与 map 项（仅当该图标不再被任何保留预设引用）。
- R5 同步删除/修改仅针对被删预设的测试文件（如 `subrouterProviderPresets.test.ts`、`therouterProviderPresets.test.ts`、`therouterOpenCodeOpenClawPresets.test.ts` 等）；KEEP 项测试若断言了被洗掉的 URL 参数，更新断言。
- R6 不改预设类型定义、不动 `universalProviderPresets.ts`、不重构数据结构；diff 仅限删除与参数清洗。
- R7 全量门禁不恶化（相对当前 windows-zh-trim/identity 基线的已知失败数）。

## Acceptance Criteria

- [ ] AC1 DELETE 名单预设在 7 个应用文件中按 name 搜索均为 0 命中。
- [ ] AC2 全仓预设与 zh.json 中 `aff=`、`ref=`（返利语义）、`utm_content=ccswitch`、`utm_source=cc_switch|OWO`、`from=CH_`、`ic=`、`invite/CC-SWITCH`、`invitecode=`、`ytag=...cc-switch` 及优惠码 `cc-switch`/`CCSWITCH`/`ccswitch` 残留 = 0（KEEP-BUT-WASH 项已洗净；deeplink 粘贴兼容的 `ccswitch://` 与源码归属注释不在此列）。
- [ ] AC3 KEEP 名单预设全部保留，URL 指向官方域名且可用；`OpenRouter/SiliconFlow/ModelScope` 三个特批聚合入口保留。
- [ ] AC4 `zh.json > partnerPromotion` 中被删预设的文案 key 全部移除；保留项（如 minimax）若保留文案则不含 "CC Switch/cc-switch/ccswitch" 字样与优惠码。
- [ ] AC5 `src/icons/extracted/index.ts` 无悬空 import（typecheck 通过即证）。
- [ ] AC6 完整 JSON 自定义供应商入口仍可用（不受本次删除影响）。
- [ ] AC7 门禁：`pnpm typecheck`、`pnpm build:renderer` 通过；`pnpm vitest run --dir tests` 失败数 ≤ 基线（3 flaky）；被删预设的测试已同步移除，无新增红。
- [ ] AC8 未 push、未改默认分支；改动在 `agent-switch-ccs` 分支分批提交。

## Out of Scope

- 新增任何供应商预设或调整保留项的模型目录。
- 重构预设类型 / 数据结构 / 表单逻辑。
- 处理 A2（非 loopback 鉴权）、A3（同步披露门）——经 ccs 行为核对后已撤销。
- 其它语言 locale（仓库仅 zh.json）。
