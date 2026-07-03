# Provider CRUD 与切换 HTTP API

## Goal

新建 http/api/providers.rs 挂 /api/providers：list（按 sort_index）/create/get/put/delete/switch（设 is_current+调 tool_takeover.enable 按 mode）/reorder（批量 sort_index）；router.rs 注册 nest；切换正确性 per-app 锁+失败回滚 is_current+返回 warnings；API 层测试。

## Requirements

- TBD

## Acceptance Criteria

- [ ] TBD

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
