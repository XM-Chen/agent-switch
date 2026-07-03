# 前端切换器页面

## Goal

新增 /providers 页面（ProvidersPage.tsx）插入 AppShell NAV_ITEMS（不动现有 8 页）；组件 AppTypeSection 分组 + ProviderCard（名称/category badge/mode 标签/激活态/切换/mode 切换）+ 上下移排序（@dnd-kit 留 P2）；lib/api.ts 加 providersApi；presentation.ts 扩展 APP_TYPE_LABELS+category/mode 文案；切换排序逻辑抽纯函数 providersUtils.ts 配 Vitest。

## Requirements

- TBD

## Acceptance Criteria

- [ ] TBD

## Notes

- Keep `prd.md` focused on requirements, constraints, and acceptance criteria.
- Lightweight tasks can remain PRD-only.
- For complex tasks, add `design.md` for technical design and `implement.md` for execution planning before `task.py start`.
