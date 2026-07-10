# Design: 会话管理（Claude Code JSONL 只读浏览）

## 范围与边界

- 仅 Claude Code：扫描 `~/.claude/projects/**/*.jsonl`。
- 严格只读：不删除、不写回、不移动、不启动终端。
- 不持久化会话内容到 DB；每次请求按需扫描/读取。

## 架构

```
/sessions 页面
  ↓ src/lib/api.ts
GET /api/sessions
GET /api/sessions/messages
  ↓
http/api/sessions.rs
  ↓
services/sessions/claude.rs
  ├─ scan_sessions(root, query)
  ├─ parse_session_meta(path)
  ├─ read_session_messages(path)
  └─ validate_source_path(root, path)
  ↓
~/.claude/projects/**/*.jsonl（只读）
```

## 后端设计

- `services/sessions/claude.rs`：纯业务逻辑与解析器，便于单测。
- `http/api/sessions.rs`：负责 query 解析、错误码映射、JSON response。
- `SessionListResponse`：`items/total/limit/offset/scan_root`。
- `SessionMeta`：`app_type/session_id/title/summary/project_dir/created_at_ms/last_active_at_ms/source_path/resume_command/warnings`。
- `SessionMessage`：`role/content/timestamp_ms/raw_kind`。

## 解析策略

- 列表只读 head/tail：head 找第一条真实用户消息、project/session 信息；tail 找 `custom-title`、summary、last_active_at。
- 文件名 `agent-*.jsonl` 跳过。
- JSON 行解析失败只产生 warning 或跳过，不中断整个扫描。
- 消息详情逐行解析，跳过 metadata-only 行；content 统一转为前端可展示字符串/结构摘要。

## 路径安全

- 根目录固定 `home/.claude/projects`。
- 详情请求的 `source_path` canonicalize 后必须位于根目录内。
- 只接受 `.jsonl` 文件。
- 不接受任意路径读取；不返回任意本地文件内容。

## 前端设计

- 新增 `/sessions` 路由与侧栏项“会话”。
- 列表支持搜索、分页、最近活跃排序；详情 panel 展示消息。
- 长消息折叠；单条消息可复制；`resume_command` 仅可复制。
- 空目录、扫描错误、详情坏行 warning 分开展示。
- 页面提示：会话可能含密钥、路径、命令输出，复制/截图需谨慎。

## 风险与回滚

- 大目录扫描可能慢：先采用分页 response 但 total 仍需扫描；后续可加轻量缓存，不在本任务首版强制。
- 大消息详情可能卡顿：前端使用虚拟列表或分批渲染。
- 回滚只需移除 API route 与前端入口；不涉及 DB 或文件迁移。