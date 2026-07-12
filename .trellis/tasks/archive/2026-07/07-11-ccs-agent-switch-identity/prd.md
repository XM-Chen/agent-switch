# 身份改造为独立 Agent Switch

## Goal

在当前 `agent-switch-ccs` 分支（ccs v3.16.5 基线 + 已完成 Windows/中文裁剪）上，把产品身份从 **CC Switch** 改造为与本机原版 ccs **完全区分、不冲突** 的独立产品 **Agent Switch 0.3.0**。功能能力保持 ccs 多应用完整支持；本任务只做身份/品牌/数据根/更新源/Deep Link/图标隔离，不裁应用、不改切换语义、不做正式发版。

## Background

- 父任务：`.trellis/tasks/07-10-ccs-baseline-migration`；兄弟任务 bootstrap / windows-zh-trim 均已归档。
- 父 PRD 2026-07-10 范围修订：取消 Claude-only；**身份改造保留**；真实 updater 发布 / 预设精选 / 安全增强暂缓。
- 权威记忆：[[project-goal-approach-ccs]]、[[agent-switch-ccs-baseline-decisions]]。
- 研究锚点：父任务 `research/branch-trellis-identity-plan.md`。
- 旧 `main` 已核实：`productName=Agent-Switch`、`identifier=com.agent-switch.app`、updater 公钥 D4D16F6B…、endpoints 已指向 `XM-Chen/agent-switch`；图标仅有 32/128/128@2x/icon.ico（无 Store/Square 全套）；Deep Link 旧版仍误用 `ccswitch`（本任务必须纠正）。
- 本机已有 `~/.tauri/agent-switch.key` + `.key.pub`（公钥 D4D16F6B…）。
- 首启语义代码已存在（`lib.rs`：空 providers → 各非 additive 应用 import live 为 `default` current → seed 官方预设）；数据根改空后自然生效，**禁止**读 `~/.cc-switch`。
- 粗扫：全仓 ccs 字面量约 1650+ 处；本任务只改运行时身份 + 关键 UI，不扫历史 release-notes / 伙伴促销全文。

### 身份映射（目标）

| 维度 | 当前（ccs） | 目标 |
|------|-------------|------|
| 显示名 | `CC Switch` | `Agent-Switch` |
| identifier / AUMID | `com.ccswitch.desktop` | `com.agent-switch.app` |
| npm / 版本 | `cc-switch` @ `3.16.5` | `agent-switch` @ `0.3.0` |
| Cargo package / lib | `cc-switch` / `cc_switch_lib` | `agent-switch` / `agent_switch_lib` |
| 数据根 / DB / 日志 | `~/.cc-switch` / `cc-switch.db` / `cc-switch.log` | `~/.agent-switch` / `agent-switch.db` / `agent-switch.log` |
| 测试 HOME | `CC_SWITCH_TEST_HOME` | `AGENT_SWITCH_TEST_HOME` |
| Deep Link 系统注册 | `ccswitch` | 仅 `agentswitch` |
| Deep Link 解析 | 仅 `ccswitch` | 主 `agentswitch` + 粘贴兼容 `ccswitch` |
| Live 归属文件 | `cc-switch-model-catalog.json` | `agent-switch-model-catalog.json`（只认新名） |
| Updater endpoint / 公钥 | farion1231/cc-switch + C802… | XM-Chen/agent-switch + D4D1… |
| WiX upgradeCode | ccs 派生 | **显式固定**为旧 0.2.x `com.agent-switch.app` 线 |
| 图标 | ccs 品牌全套 | 自有 Agent Switch 图标（从 `main` 迁入并补齐缺失尺寸） |
| 仓库元数据 | farion1231/cc-switch | XM-Chen/agent-switch；`LICENSE` 保留 Jason Young MIT |

## Requirements

### R1. 安装与进程身份

- 显示名 `Agent-Switch`；identifier `com.agent-switch.app`；npm `agent-switch`；Cargo `agent-switch` / lib `agent_switch_lib`。
- 版本 `0.3.0` 在 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json` 三处同步。
- Windows AUMID 随 identifier（当前 `lib.rs` 直接用 `app.config().identifier`）。
- WiX：在自定义模板或配置中**显式固定** upgradeCode 为旧 agent-switch 0.2.x 安装线（实施前用 `pnpm tauri inspect wix-upgrade-code` 或等价方式在旧身份上取值并记录）；不得与 `com.ccswitch.desktop` 升级关系混淆。
- 安装/运行不得覆盖、卸载或升级本机已安装的 CC Switch。

### R2. 产品数据根与内部文件名（全量改名）

- 默认数据根 `~/.agent-switch`（保持 ccs home-hidden-dir 结构）。
- `cc-switch.db` → `agent-switch.db`；日志 `cc-switch.log` → `agent-switch.log`；crash 等派生名同步。
- `CC_SWITCH_TEST_HOME` → `AGENT_SWITCH_TEST_HOME`（含全部测试引用）。
- Windows legacy `HOME` fallback 目录名同步。
- **禁止**读取、迁移、探测 `~/.cc-switch` 与旧 0.2.x DB。

### R3. Live 侧归属文件隔离

- `CC_SWITCH_CODEX_MODEL_CATALOG_FILENAME` 及写入 `~/.codex` 的文件名改为 `agent-switch-model-catalog.json`，只认新文件名。
- 已知共存副作用（接受）：`~/.codex/config.toml` 的 `model_catalog_json` 指针是单一 live 字段，两产品切换会互相改写该指针；改名只避免 catalog **文件本身**互覆。
- 若代码中另有同类「产品写入用户工具目录」的硬编码 ccs 文件名，按同原则改名并只认新名（实施时盘点，不得漏 `codex` 这一已确认项）。

### R4. Deep Link

- 系统只注册 `agentswitch`（`tauri.conf.json` schemes）。
- 解析器接受 `agentswitch`（主）与 `ccswitch`（应用内粘贴兼容）；路径契约 `v1/import?...` 不变。
- `lib.rs` 的 `starts_with("ccswitch://")` / 处理入口改为优先 `agentswitch://`，并允许粘贴路径传入 `ccswitch://`。
- 前端 i18n / placeholder 示例改为 `agentswitch://`；可注明粘贴仍兼容 `ccswitch://`。
- 测试：主路径用 `agentswitch`；单独覆盖 `ccswitch` 粘贴兼容仍可解析；其它 scheme 非法。

### R5. Updater 指向（不含正式发版）

- endpoints → `https://github.com/XM-Chen/agent-switch/releases/latest/download/latest.json`。
- 内置公钥 → 本机 `agent-switch.key.pub`（D4D16F6B…）；私钥不入库。
- `commands/misc.rs` 等硬编码 ccs releases URL / User-Agent 产品名同步。
- **不做**：真实 push release、线上 `latest.json`、Authenticode、要求 CI 发版 workflow 跑通。

### R6. 图标与品牌 UI

- 必须替换应用/托盘图标为 Agent Switch 自有资产：优先从 `main` 迁入现有 32/128/128@2x/icon.ico；缺失的 Windows Store/Square 等资源用同套主图生成或扩展，**不**把 ccs 品牌图标作为最终产物。
- 运行时 UI（`zh.json`、窗口标题、关于/欢迎/设置提示）产品自称改为 Agent Switch / Agent-Switch。
- 关于页或等价位置注明「基于 CC Switch v3.16.5 修改」；`LICENSE` 保留 Jason Young MIT 全文。
- **品牌替换深度 = 运行时身份 + 关键 UI**：不全文替换历史 `release-notes`、`CHANGELOG`、深度 user-manual、合作伙伴促销/aff 文案（后置或另任务）。

### R7. 首启

- 新数据根空库时沿用现有 import-before-seed（全非 additive 应用）；不新增/删除该语义。
- 不实现从 `~/.cc-switch` 一键迁移。
- 欢迎首启 notice 去 ccs 品牌化。

### R8. 交付纪律

- 按可验证批次提交，不与无关重构混提。
- 每批后跑既有质量门（typecheck / unit / cargo fmt·clippy·test / build:renderer；可 `tauri build --no-bundle`）。
- 未经授权不 push、不改默认分支、不发布安装包。

## Acceptance Criteria

- [ ] AC1：三处版本均为 `0.3.0`；`productName` / `identifier` / npm / Cargo 名均为目标值。
- [ ] AC2：默认数据根 `~/.agent-switch`；DB `agent-switch.db`；日志 `agent-switch.log`；测试环境变量 `AGENT_SWITCH_TEST_HOME`；在测试 HOME 下启动不创建、不读取 `~/.cc-switch`。
- [ ] AC3：系统 schemes 仅 `agentswitch`；解析主路径 `agentswitch://` 成功；粘贴 `ccswitch://` 仍可解析；其它 scheme 失败；相关单测更新且通过。
- [ ] AC4：updater endpoint 与公钥均为 Agent Switch；仓库运行时配置中无 ccs 官方 latest.json URL / ccs 公钥。
- [ ] AC5：空数据根 + 存在 live 配置时仍执行 import-before-seed；providers 非空不重复 import。
- [ ] AC6：运行时用户可见产品自称不再是「CC Switch」；关于/文档注明基于 ccs v3.16.5；`LICENSE` 保留 Jason Young。
- [ ] AC7：`~/.codex` 侧 catalog 文件名为 `agent-switch-model-catalog.json` 且只认新名；相关单测通过。
- [ ] AC8：应用图标/托盘资源为 Agent Switch 自有资产，非 ccs 品牌图。
- [ ] AC9：WiX upgradeCode 已显式固定为 0.2.x 线并写入配置/模板（值记录在 design 或 implement 笔记）；与 ccs 不同。
- [ ] AC10：质量门相对裁剪后基线不恶化；身份相关单测全绿。
- [ ] AC11：未授权情况下无 push / 无正式 release 产物上传。

## Out of Scope

- Claude-only 或其它应用裁剪。
- 预设精选 / 返利参数 / sponsor 链接全文清理（父任务暂缓 D22）。
- 非 loopback 强制鉴权、远端同步风险确认。
- 真实发版流程（release workflow 跑通、latest.json 上传、Authenticode）。
- 从 `~/.cc-switch` 或旧 0.2.x DB 迁移导入。
- 切换语义 / 代理 / MCP 等功能行为变更。
- 非中文文档恢复；历史 release-notes / CHANGELOG 全文重写。
- 解决两产品共用 live 指针（如 `model_catalog_json`）的互写问题（仅文件名隔离）。

## Notes

- 任务目录：`.trellis/tasks/07-11-ccs-agent-switch-identity`；父 children 已链接。
- 复杂任务：需 `design.md` + `implement.md`，经用户审核后再 `task.py start`。
- 规划阶段不修改产品代码。
