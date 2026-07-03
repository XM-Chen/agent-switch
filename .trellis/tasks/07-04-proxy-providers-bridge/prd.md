# 代理模式与 providers 桥接及升级回填

## Goal

P1 不重写 selector 管道；proxy 模式 provider 语义=工具指向本地代理，上游仍由现有 endpoints 管道路由；迁移 v7 数据回填：为现有已启用接管的 claude-code/codex 各生成 mode=proxy 默认 provider（is_current=1）保证升级无缝；集成测试验证回填后转发行为与改造前一致；明确 providers→endpoints 深度绑定留待 P1 后独立阶段。

## Requirements

- TBD

## Acceptance Criteria

- [ ] TBD

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
