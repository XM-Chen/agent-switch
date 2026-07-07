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

## 待澄清的跨子任务架构决策（A-E）

以下决策影响多个子任务，在对应子任务规划（brainstorm）阶段逐个澄清，此处登记为待办：

- **A. 回填保护语义**（子任务 1 规划时澄清）：完全复刻 ccs provider 快照 / 保持现状 merge + 只加 Common Config / 混合。→ 参考 ccs
- **B. 代理模式下外围管理范围**（子任务 1 规划时澄清，影响 2/3/6/7）：两种模式都管 / 仅直连模式管。→ 独立决策（agent-switch 代理模式特色）
- **C. env 行为开关承载方式**（子任务 4 规划时澄清）：复刻 ccs 预设机制 / provider settings_config 支持任意 env + 模板 / 结合。→ 参考 ccs，技术栈倾向自行选型后 review
- **D. Skills 的 SSOT 定位与跨工具复用**（子任务 6 规划时澄清）：SSOT 路径 / 是否跨 codex 等工具复用。→ 独立决策
- **E. Deep Link 协议**（子任务 8 规划时澄清）：复刻 `ccswitch://` 兼容 / 自定义 `agentswitch://` / 都支持。→ 参考 ccs

## Notes

- 本父任务不直接实现代码，只持有需求集、任务图、跨子任务验收、最终集成 review。
- 子任务按批次物理创建：当前先创建子任务 1（地基批），P0/P1 批子任务在对应批次推进时创建并 link。
- 各子任务 complex，需各自 `prd.md` + `design.md` + `implement.md` 后再 `task.py start`。
