# Trellis Workflow 状态契约

> 本文描述当前项目 Trellis 0.6.6 的任务状态与每轮 breadcrumb 契约。实现以 `.trellis/scripts/` 和各平台生成的 `hooks/inject-workflow-state.py` 为准。

## 持久状态

`task.json.status` 的标准值：

| 状态 | 写入时机 | 写入者 |
|---|---|---|
| `planning` | `task.py create` 创建任务 | `.trellis/scripts/task.py` |
| `in_progress` | `task.py start` 激活任务 | `.trellis/scripts/task.py` |
| `completed` | `task.py archive` 归档前 | `.trellis/scripts/task.py` |

`task.py finish` 只清除当前任务指针，不改变 `task.json.status`。因此“finish”不等于任务完成；完成由 archive 表示。

自定义状态必须符合 `[A-Za-z0-9_-]+`，并由显式 lifecycle hook 写回 `task.json`；仅在 `workflow.md` 增加 tag 不会自动产生该状态。

## 当前任务指针

当前任务通过 session-aware active task resolver 解析，来源可能包括：

- 当前 Claude/Codex 等会话的 session pointer；
- 本地 `.trellis/.current-task`；
- 平台提供的上下文。

`.current-task`、`.developer` 和 runtime 状态是本机文件，受 `.trellis/.gitignore` 排除，不进入 Git。

解析到不存在/已移动的任务目录时，hook 使用 `stale_<source_type>` 伪状态；无 active task 使用 `no_task`。伪状态只用于 breadcrumb，不写回 `task.json.status`。

## Breadcrumb SSOT

`.trellis/workflow.md` 中的标签块是每轮提示唯一文案来源：

```text
[workflow-state:STATUS]
...
[/workflow-state:STATUS]
```

生成到各平台的 `hooks/inject-workflow-state.py`：

1. 解析 active task 与 `task.json.status`；
2. 从 `workflow.md` 找同名 tag；
3. 注入短 breadcrumb；
4. tag 缺失时降级为通用提示，不维护隐藏的硬编码文案副本。

Claude Code 的实际生成文件是 `.claude/hooks/inject-workflow-state.py`；Codex 对应 `.codex/hooks/inject-workflow-state.py`。平台目录被根 `.gitignore` 忽略，由 `trellis init/update` 再生。

## 状态与阶段

| 状态/tag | 阶段 |
|---|---|
| `no_task` | Phase 1 之前 |
| `planning` | Phase 1：需求、研究、context、审核 |
| `planning-inline` | Codex inline 的 Phase 1 变体 |
| `in_progress` | Phase 2 + Phase 3.2–3.4 |
| `in_progress-inline` | Codex inline 的执行/收尾变体 |
| `completed` | 当前正常流程中通常不可见：archive 同时移动任务目录，resolver 随即失去路径 |

## 修改规则

修改 workflow state 时必须同步核对：

- `workflow.md` 的 Phase Index、required step 与 tag body；
- `task.py` 所有 status writer；
- `common/active_task.py` 的 resolver 和 stale 语义；
- 平台 hook parser 的 tag regex/fallback；
- `get_context.py --mode phase` 输出；
- create/start/finish/archive 与 session pointer 测试。

运行 `trellis update --dry-run` 确认模板状态；项目自定义文件不得被 `--force` 无审查覆盖。
