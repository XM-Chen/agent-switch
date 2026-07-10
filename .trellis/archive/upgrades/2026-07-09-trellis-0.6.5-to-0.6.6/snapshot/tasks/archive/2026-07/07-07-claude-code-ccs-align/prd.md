# Claude Code 功能对齐 ccs（吸收全部缺失项）

## Goal

父任务：把 cc-switch（ccs）中 Claude Code 部分的全部缺失功能吸收到 agent-switch，使 agent-switch 在 Claude Code 维度上的功能集与 ccs 对齐。吸收方式为「参考 ccs 实现但按 agent-switch 既有架构适配」，不照抄代码。

## 背景

经对比调研（2026-07-07），agent-switch 在 Claude Code 上只吸收了 ccs 的底层切换骨架（`~/.claude/settings.json` 接管 + 本地反代网关 + direct/proxy 双模式 + 备份），ccs 在 Claude Code 上做的外围管理功能几乎全部缺失。本父任务负责把 9 个缺失维度全部补齐。

agent-switch 优于 ccs 的部分（凭证 AES-256-GCM 加密 + 明文不落盘、本地反代网关 + 跨协议翻译 + 端点选择/故障转移/模型别名锁）不在本任务范围，作为既有优势保留，不被 ccs 实现覆盖。

## Requirements

### 子任务图（8 个子任务，按依赖排序）

| # | 子任务 | slug | 批次 | 依赖 | 参考来源 |
|---|---|---|---|---|---|
| 1 | 切换语义增强：回填保护 + Common Config Snippet（地基） | `cc-switch-semantics` | 地基 | 无 | ccs |
| 2 | Prompts 管理（`CLAUDE.md` 单激活 + 回填保护应用） | `cc-prompts` | P0 | 1 | ccs |
| 3 | MCP 统一管理（`~/.claude.json` mcpServers 字段级增量 + Windows cmd /c 包装 + WSL 检测） | `cc-mcp` | P0 | 无 | ccs |
| 4 | env 行为开关注入（`CLAUDE_CODE_*` / `DEFAULT_{HAIKU,SONNET,OPUS}_MODEL` / `API_TIMEOUT_MS` / Bedrock 凭证模板） | `cc-env-switches` | P0 | 弱依赖 1 | ccs |
| 5 | permissions/hooks/statusLine/outputStyle 裸 JSON 编辑器（Common Config 应用） | `cc-common-config-editor` | P1 | 1 | ccs |
| 6 | Skills 管理（`~/.claude/skills` symlink 到 SSOT，跨应用复用） | `cc-skills` | P1 | 无 | ccs |
| 7 | 会话管理（扫描 `~/.claude/projects/**/*.jsonl` 只读） | `cc-sessions` | P1 | 无 | ccs |
| 8 | Deep Link 一键导入（`v1/import` 4 种 resource） | `cc-deeplink` | P1 | 无 | ccs |

### 分批推进

- **地基批**：子任务 1（切换语义增强）。先规划+实现+验证，因为它被 2/5 依赖，且其设计影响后续所有外围子任务的定位。
- **P0 批**：子任务 2/3/4。地基验证通过后推进，三者中 2 依赖 1，3/4 相对独立可并行。
- **P1 批**：子任务 5/6/7/8。5 依赖 1，6/7/8 独立可并行。

每批按 Phase 1（规划）→ Phase 2（实现+检查）→ Phase 3（归档）推进，批次间回归验证 Claude Code 切换 + 代理网关无回归。

### 跨子任务需求

- 每个 ccs 的 Claude Code 功能点在 agent-switch 有对应实现，且行为可被验证（手动或测试）。
- 切换 Claude Code 供应商时不丢失用户自加的 hooks/permissions/statusLine 等配置（回填保护 + Common Config 协同）。
- 代理模式与直连模式下，外围管理（MCP/Prompts/Skills/会话）的行为边界明确且一致（具体边界由架构决策 B 定）。
- 不破坏现有：`settings.json` 合并写入语义、本地反代网关 `/claude-code` 路由、direct/proxy 双模式状态机、凭证加密存储、ccs 导入器、portability 备份/恢复。

## Constraints

- 参考 ccs 实现思路，但按 agent-switch 既有架构（Rust 后端 + React 前端 + SQLite + Tauri）适配，不直接拷贝 ccs 代码。
- 保留 agent-switch 既有优势：凭证加密、反代网关、跨协议翻译。不为对齐 ccs 而降级这些能力。
- ccs 已知缺陷不照搬：ccs 凭证明文存储 + 导出/WebDAV 不脱敏——agent-switch 沿用加密 + 脱敏导出，不退化。
- 各子任务的配置文件操作沿用 agent-switch 既有 `atomic_write` + `backup_before_write` 模式，保证原子性与可回滚。
- Windows / macOS / Linux 三平台路径与行为差异按 ccs 的处理方式对齐（如 `~/.claude.json` 的 Windows cmd /c 包装、WSL 检测）。

## Acceptance Criteria（跨子任务集成验收）

- [ ] 8 个子任务全部完成并归档。
- [ ] ccs Claude Code 功能清单中的每一项在 agent-switch 有对应实现（对照表见各子任务 prd）。
- [ ] 切换供应商 N 次，用户自加的 hooks/permissions/statusLine/MCP/prompts 配置不丢失（回填保护 + Common Config 验证）。
- [ ] 代理模式与直连模式下，MCP/Prompts/Skills/会话管理功能均按架构决策 B 的定义可用。
- [ ] 现有 Claude Code 切换 + 反代网关 + ccs 导入 + portability 回归测试通过。
- [ ] 父任务集成 review：8 子任务联调无冲突，配置文件读写顺序无死锁/覆盖。

## 跨子任务架构决策（A-E）

以下决策已在对应子任务规划（brainstorm）阶段澄清，并写入各子任务 PRD / design：

- **A. 回填保护语义**：采用 agent-switch 适配版 A1-hybrid。`settings_config` 保持端点引用语义不变，全文快照存 `provider.meta.snapshot`；切走前 backfill live 的非连接层，切入时 `snapshot + common` 后再注入连接层，token 不落 DB。
- **B. 代理模式下外围管理范围**：外围管理与 takeover 解耦。MCP / Prompts / Skills / Sessions / DeepLink 均为独立 service 或独立入口，不绑定 provider switch；CRUD / enable / import 后按各自语义即时投影或只读展示。
- **C. env 行为开关承载方式**：复刻 ccs「env 开关即 provider env 键」思路，但落点为 `provider.meta.snapshot.env`；前端提供结构化编辑器 + 预设模板。Bedrock AWS 凭证明文承载为用户已接受的显式例外。
- **D. Skills 的 SSOT 定位与跨工具复用**：用户选择“完整 ccs”范围。规划覆盖 agent-switch SSOT、可选 `~/.agents/skills`、多应用投影、GitHub / skills.sh 发现、备份恢复与批量更新。
- **E. Deep Link 协议**：用户选择“仅 ccswitch”。本批只注册/解析 `ccswitch://v1/import`，不注册 `agentswitch://`；四类资源为 provider / prompt / mcp / skill。

## Notes

- 本父任务不直接实现代码，只持有需求集、任务图、跨子任务验收、最终集成 review。
- 子任务按批次物理创建：地基批与 P0 批已完成并归档；P1 批四个子任务已创建并 link：`07-08-cc-common-config-editor`、`07-08-cc-skills`、`07-08-cc-sessions`、`07-08-cc-deeplink`。
- 各子任务 complex，需各自 `prd.md` + `design.md` + `implement.md` 后再 `task.py start`。
