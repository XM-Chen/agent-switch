# Implement: 会话管理（Claude Code JSONL 只读浏览）

## Checklist

1. 新增 `services/sessions/claude.rs`：扫描 root、跳过 `agent-*.jsonl`、解析 metadata、路径校验、消息详情解析。
2. 新增 `http/api/sessions.rs` 与 router 挂载：列表与详情接口，当前仅 `app_type=claude-code`。
3. 新增 `src/lib/api.ts` sessions client 与类型。
4. 新增 `/sessions` 页面、侧栏入口、搜索/分页/详情 UI、敏感内容提示。
5. 补 Rust 单测：不存在 root、跳过 agent、坏行容错、标题优先级、路径越界拒绝、详情解析。
6. 补前端测试：query key/参数、列表空态/错误态、详情 helper/长内容折叠。
7. 回归验证只读边界与现有页面构建。

## Validation Commands

- `cargo test`
- `npm test -- --run`
- `npm run build`

## Review Gates

- 路径安全：详情接口不能读取 `~/.claude/projects` 外文件。
- 只读边界：grep/测试确认没有 write/remove/rename 操作指向 `~/.claude/projects`。
- 性能边界：列表不全量读取每个 JSONL 的所有消息。
- UI 状态：loading/error/empty/warning 不混淆。

## Rollback Points

- 后端 API 与前端页面可独立回滚；无 DB 迁移。
- 若详情虚拟列表复杂度过高，可先使用长内容折叠 + 分段渲染，保留 PRD 验收目标。