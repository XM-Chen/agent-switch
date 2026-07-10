# 旧 Agent Switch 0.2.2 规范归档

> **警告：这些文件描述的是 `main` 分支上的旧 Agent Switch 0.2.2 架构，不是当前 `agent-switch-ccs` 分支的实施规范。**

本目录在 2026-07-10 的 ccs 基线迁移中，从旧 `main` 的 `.trellis/spec/{frontend,backend,guides}` 原样归档，用途仅限：

- 理解旧产品能力和历史设计取舍；
- 对照后续明确要求移植的能力；
- 回溯旧任务、提交与测试证据。

禁止把这里的目录结构、API、数据库 schema、AES/portability/HTTP API 或路由语义直接套用到 ccs v3.16.5 基线。当前分支的权威规范位于同级 active 目录：

- `../frontend/`
- `../backend/`
- `../guides/`

产品方向、裁剪边界和身份决策的权威来源为：

- `.trellis/tasks/07-10-ccs-baseline-migration/prd.md`
- `.trellis/tasks/07-10-ccs-baseline-migration/design.md`
- `.trellis/tasks/07-10-ccs-baseline-migration/implement.md`

首期明确以 ccs 行为为准；“显式移植旧 Agent Switch 能力”为空集。未来若要恢复某项旧能力，必须另建任务、重新验证其适配性，不能从本目录直接复制实现。
