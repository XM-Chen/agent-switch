# 双模式接管改造与 apply_direct

## Goal

迁移 v7 给 tool_takeover 加 mode/active_provider_id 列；tool_takeover/mod.rs 的 enable 加 mode+provider（proxy 走现有 apply、direct 走 apply_direct）；disable 语义明确（direct 关闭回退 proxy）；detect 增加 direct 识别；claude_code.rs/codex.rs 新增 apply_direct 写真实 base_url+凭据；单测覆盖 proxy/direct 产物、direct→disable 回退、占位符不泄露真实 key。

## Requirements

- TBD

## Acceptance Criteria

- [ ] TBD

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
