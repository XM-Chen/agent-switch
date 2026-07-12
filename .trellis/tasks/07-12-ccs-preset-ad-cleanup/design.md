# Design：清除 ccs 预设广告与返利内容

## 1. 边界与契约

- 分类判定（谁保留、谁删）在 PRD 白名单已固化，本设计不再重判；执行时**按 name 匹配**定位条目，行号仅辅助、每次操作前重新 grep 防漂移。
- 单一事实源：以 `claudeProviderPresets.ts` 的 DELETE/KEEP name 集合为规范，其余 6 个应用文件（codex/gemini/claudeDesktop/openclaw/opencode/hermes）按同名镜像同步。某文件不含某 name 属正常（应用支持面不同），不臆造。
- 洗 URL 规则（KEEP-BUT-WASH）：删查询串里的返利/邀请/来源参数键值对，保留 scheme+host+path 主体与功能性参数；删空后不留悬挂的 `?`/`&`。

## 2. 数据流与依赖链

```
preset 条目 (7 files)
  ├── partnerPromotionKey: "x"  ──► zh.json providerForm.partnerPromotion.x   (删)
  ├── nameKey: "presets.x"      ──► zh.json providerForm.presets.x            (失引用则删)
  └── icon: "x"                 ──► src/icons/extracted/index.ts import _x + map.x (悬空则删)
测试:
  tests/config/<name>ProviderPresets.test.ts 等  ──► 删预设后同步删/改
```

删除顺序：先删 7 文件的 preset 条目 → 回收 zh.json 文案/名映射 → 清悬空图标 import → 删/改测试 → 跑门禁。

## 3. 保留项 URL 洗净对照（KEEP-BUT-WASH）

| 预设 | 删除的参数 | 洗后目标 |
|------|-----------|---------|
| 火山Agentplan | `utm_content=ccswitch`、`utm_source=OWO`、`rc=6J6FV5N2` | 火山方舟官方活动/控制台裸链 |
| BytePlus | `utm_content=ccswitch`、`utm_source=OWO` | BytePlus 官方裸链 |
| DouBaoSeed | `utm_content=ccswitch`、`utm_source=OWO` | 豆包官方裸链 |
| Zhipu GLM / en | `ic=RRVJPB5SII` / `ic=8JVLJQFSKB` | 智谱开放平台裸链 |
| Kimi / For Coding | `aff=cc-switch` | Moonshot/Kimi 官方裸链 |
| SiliconFlow / en | 邀请路径 `i/YflgU2Ve` | 硅基流动注册裸链 |
| MiniMax / en | URL 若含参则洗；促销文案按 R3 处理 | MiniMax 官方裸链 |

洗净判据：AC2 的残留扫描对上述文件命中 = 0。

## 4. 风险与兼容

- **图标悬空 import**：若 `noUnusedLocals` 开启，遗漏清理会 typecheck 失败——以 typecheck 作为兜底门（AC5）。删图标 import 前须确认该 icon 名不再被任何保留预设引用（跨 7 文件 grep）。
- **测试断言硬编码**：部分测试断言被删预设的 `partnerPromotionKey`/URL/数量。策略：被删预设的专属测试文件整体删除；共享测试（如 `ProviderPresetSelector.test.tsx`、`EditProviderDialog.test.tsx`）若断言了具体计数或被删名，改断言而非删文件。
- **多应用镜像不一致**：同一中转商在不同应用文件的 name 拼写/大小写可能有别；按盘点报告的 file:line 复核，执行时以实际 grep 为准。
- **最小 diff**：只删不构造。保留项除洗参数外一字不动，降低与 ccs 上游未来同步的冲突面。

## 5. 回滚

每批（按文件或按 KEEP-BUT-WASH/DELETE 分批）独立提交；出问题 revert 对应批 commit。不改结构，回滚无副作用。

## 6. 验证策略

- 静态：AC1（DELETE name 0 命中）、AC2（返利参数 0 残留）、AC4（文案 key 清理）用 grep/rg 扫描。
- 编译：`pnpm typecheck`（兜底图标悬空）、`pnpm build:renderer`。
- 测试：`pnpm vitest run --dir tests`，失败数 ≤ 基线 3 flaky；确认无因删预设新增的红。
