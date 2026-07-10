# 会话管理（Claude Code JSONL 只读浏览）

## Goal

P1 子任务：补齐 ccs 式 Claude Code 会话管理的只读浏览能力。扫描 `~/.claude/projects/**/*.jsonl`，展示会话列表与消息详情，帮助用户在 agent-switch 中快速查找和阅读本地 Claude Code 历史会话。本任务严格只读，不删除、不修改、不移动 JSONL，不启动终端恢复会话。

## Background

### 已确认的 agent-switch 现状

- 当前前端路由和侧栏没有会话页。
- 当前 HTTP API 没有 `/api/sessions` 或类似接口。
- 当前 Claude Code 能力主要是代理与配置接管；不涉及 `~/.claude/projects/**/*.jsonl`。
- 当前已有日志 API 使用 `limit` / `offset` / `{ items, total }` 的分页契约，可作为会话列表 API 风格参考。
- 父任务明确 `cc-sessions` 是 P1 子任务，目标是“会话管理（扫描 `~/.claude/projects/**/*.jsonl` 只读）”。

### ccs 参考事实

- ccs 的会话模型包含 `SessionMeta` 和 `SessionMessage`，字段包括 providerId/sessionId/title/summary/projectDir/createdAt/lastActiveAt/sourcePath/resumeCommand，以及 message role/content/ts。
- ccs Claude Code 会话扫描根目录是 `~/.claude/projects`，递归收集所有 `.jsonl`。
- ccs 会跳过 `agent-*.jsonl` 子代理会话，避免主列表混入 subagent 文件。
- ccs 列表解析只读 head/tail，避免为列表全量读取大 JSONL。
- ccs 标题优先级：`custom-title` > 第一条真实用户消息 > project basename。
- ccs 对消息详情逐行解析 JSONL，跳过 meta，抽取 `message.role/content/timestamp`；纯 tool_result 的 user 消息改为 `tool` 角色。
- ccs 还支持删除 session、删除 sidecar、启动终端恢复会话；这些超出本子任务“只读扫描”边界，不纳入本期。

## Requirements

### R1. 只读扫描 Claude Code 会话

- R1.1 扫描根目录固定为 `home_dir().join(".claude").join("projects")`。
- R1.2 递归收集 `*.jsonl` 文件，跳过文件名匹配 `agent-*.jsonl` 的子代理会话。
- R1.3 根目录不存在时返回空列表与明确 `scan_root`，不报错、不创建目录。
- R1.4 扫描过程只读，不写入、不删除、不移动 `~/.claude/projects` 下任何文件。

### R2. 会话列表 metadata

- R2.1 列表返回字段：`app_type`、`session_id`、`title`、`summary`、`project_dir`、`created_at_ms`、`last_active_at_ms`、`source_path`、`resume_command`。
- R2.2 列表只读每个 JSONL 的必要 head/tail 信息，避免为列表全量读取大文件。
- R2.3 标题提取优先级：`custom-title` > 第一条真实用户消息 > project basename > session id 前缀。
- R2.4 按 `last_active_at_ms || created_at_ms` 倒序排序。
- R2.5 单个损坏 JSONL 或坏 JSON 行不导致整个列表失败；记录 warning 或跳过坏行。

### R3. 分页与搜索

- R3.1 `GET /api/sessions?app_type=claude-code&limit=50&offset=0&search=...` 返回 `{ items, total, limit, offset, scan_root }`。
- R3.2 当前只支持 `app_type=claude-code`；其它值返回 400，未来再扩展 Codex/Gemini。
- R3.3 `search` 覆盖 `title`、`summary`、`project_dir`、`session_id`、`source_path`。
- R3.4 分页在搜索过滤后执行，`total` 与过滤条件一致。

### R4. 消息详情

- R4.1 `GET /api/sessions/messages?app_type=claude-code&source_path=<urlencoded>` 读取单个 JSONL 的消息详情。
- R4.2 `source_path` 必须 canonicalize 后位于 `~/.claude/projects` 下，且扩展名为 `.jsonl`；越界、非 jsonl、不存在均拒绝。
- R4.3 逐行解析 JSON，坏行跳过并返回 warning，不让单行损坏导致详情失败。
- R4.4 抽取 `role`、`content`、`timestamp_ms`、`raw_kind` 等展示字段。
- R4.5 支持 string / array / object content 的可读渲染；`tool_use` 显示为工具调用摘要，纯 `tool_result` 的 user message 显示为 `tool` 角色。

### R5. 前端会话页

- R5.1 新增 `/sessions` 页面与侧栏“会话”入口。
- R5.2 页面包含列表、搜索、分页、详情阅读；loading/error/empty 状态区分清楚。
- R5.3 详情使用虚拟列表或等价方案避免大对话卡顿；长内容可折叠，消息可复制。
- R5.4 `resume_command` 仅显示/复制，不执行命令、不启动终端。
- R5.5 会话内容默认原样展示本地 JSONL 内容；UI 提醒“会话可能包含密钥、路径、命令输出等敏感信息，请谨慎分享截图/复制”。

### R6. 边界约束

- R6.1 不提供删除 session、删除 sidecar、写入标题/摘要、移动/归档 JSONL。
- R6.2 不把会话内容导入 agent-switch DB，不做持久索引库。
- R6.3 不挂 provider switch，不改 Claude Code 配置文件。

## Acceptance Criteria

- [ ] AC1：`/sessions` 页面可访问，侧栏出现“会话”，现有页面不受影响。（R5）
- [ ] AC2：本机没有 `~/.claude/projects` 时，API 返回空列表，页面显示明确空态，不创建目录、不报错。（R1/R5）
- [ ] AC3：列表 API 只读扫描 `~/.claude/projects/**/*.jsonl`，跳过 `agent-*.jsonl`，返回 `{ items, total, limit, offset, scan_root }`。（R1/R3）
- [ ] AC4：列表按最近活跃时间倒序；分页与搜索正确，`total` 与过滤条件一致。（R2/R3）
- [ ] AC5：标题提取符合优先级：`custom-title` > 第一条真实用户消息 > project basename > session id 前缀。（R2）
- [ ] AC6：单个损坏 JSONL 或坏 JSON 行不会导致整个列表/详情崩溃。（R2/R4）
- [ ] AC7：详情接口只接受位于 `~/.claude/projects` 下的 `.jsonl`，越界路径被拒绝。（R4）
- [ ] AC8：详情页能展示 user/assistant/tool/system 消息、时间戳、长内容折叠、复制单条消息，并对敏感内容风险给出提示。（R4/R5）
- [ ] AC9：`resume_command` 只显示/复制，不执行、不启动终端。（R5/R6）
- [ ] AC10：前端 API 只通过 `src/lib/api.ts`，TanStack Query key 包含分页/search 参数。（R5）
- [ ] AC11：Rust parser/path validation 有单元测试；前端列表/详情 helper 有 Vitest 覆盖；`cargo test`、`npm test`、`npm run build` 通过。（R1-R6）
- [ ] AC12：grep 或测试确认本功能不写入、不删除、不移动 `~/.claude/projects` 下任何文件。（R6）

## Constraints

- 本任务只覆盖 Claude Code；Codex/Gemini/OpenCode 等会话管理是未来扩展。
- 参考 ccs JSONL 解析与 UI 思路，但按 agent-switch REST API + React 架构适配。
- 所有文档与 UI 文案使用中文。

## Out of Scope

- 删除会话、删除 sidecar、启动终端恢复会话。
- 会话内容数据库索引、全文检索引擎、跨设备同步。
- 多 provider / 多工具会话统一管理。
- 编辑会话标题/摘要或写回 JSONL。

## Open Questions

- 无阻塞问题。按父任务“只读”约束，本任务默认原样展示本地会话内容但不执行 resume、不做删除；敏感内容以 UI 提示而非自动脱敏处理。