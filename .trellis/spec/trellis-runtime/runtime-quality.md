# Quality Guidelines

> Review and verification standards for Trellis Python runtime changes.

---

## Overview

Runtime changes should preserve the Trellis workflow contract: task state is durable in files, active task pointers are session-scoped, workflow breadcrumbs come from `.trellis/workflow.md`, and platform adapters degrade safely. The repository does not currently include a full test suite, so use compile checks plus targeted command exercises.

Reference files:
- `.trellis/workflow.md` — canonical workflow and per-turn breadcrumb contract.
- `.trellis/scripts/common/workflow_phase.py` — parser for workflow step/platform blocks.
- `.trellis/scripts/common/active_task.py` — session isolation and fallback rules.
- `.trellis/scripts/common/task_context.py` — JSONL validation rules.
- `.trellis/scripts/common/session_context.py` — context rendering and Git status handling.

---

## Required Patterns

- Read the relevant spec file before editing runtime code; do not rely on memory.
- Keep entrypoint files thin and shared behavior in `.trellis/scripts/common/`.
- Use public helper functions for paths, JSON, Git, config, and active-task state.
- Preserve Windows compatibility: UTF-8 handling, POSIX-normalized task refs, and no shell-only assumptions in Python code.
- Preserve the single source of truth for workflow-state text: `.trellis/workflow.md` tag blocks.
- Keep sub-agent JSONL manifests limited to spec/research context; do not preload source files.

---

## Forbidden Patterns

- Broad `git add -f .trellis/` or any unsafe staging that bypasses `.trellis/.gitignore` warnings.
- Global active-task state shared across sessions.
- Duplicated platform prose in Python/JS when it can be parsed from `.trellis/workflow.md`.
- Adding framework directories or abstractions for nonexistent web/database layers.
- Hook stdout debug prints that corrupt host protocol JSON.
- Swallowing validation errors that should block a user from proceeding.

---

## Verification Commands

Run commands that match the changed scope. For most runtime edits:

```bash
python -m compileall .trellis/scripts
python ./.trellis/scripts/get_context.py
python ./.trellis/scripts/get_context.py --mode packages
python ./.trellis/scripts/get_context.py --mode phase
python ./.trellis/scripts/get_context.py --mode phase --step 2.1 --platform claude
python ./.trellis/scripts/task.py validate 00-bootstrap-guidelines
```

For task lifecycle changes, add a scratch/manual scenario or inspect the exact command path:

```bash
python ./.trellis/scripts/task.py list --mine
python ./.trellis/scripts/task.py current --source
```

Do not archive or commit from verification unless the current workflow step explicitly calls for it.

---

## Review Checklist

Before reporting a runtime change complete, check:

- [ ] The changed command has a clear success path and a clear user-correctable failure path.
- [ ] File paths are repo-relative where possible and Windows-safe.
- [ ] Missing optional files are handled without stack traces.
- [ ] Hook code is silent when not applicable and emits valid host protocol output when applicable.
- [ ] `workflow.md` parser changes are checked against at least one platform-filtered step.
- [ ] `task.py validate` still accepts seed/comment rows without `file` and reports real missing entries.
- [ ] No generated cache files (`__pycache__`, `*.pyc`) need to be tracked.

---

## Common Mistakes

- Assuming `task.py current` always has a session key. Some sub-agent shells rely on the single-session fallback, and multiple sessions must remain ambiguous.
- Treating `task.py finish` as completion. It clears the active pointer; `task.py archive` writes `completed` and moves the task.
- Editing `.trellis/workflow.md` without checking the hook parser contract described in its comments.
- Adding source files to `implement.jsonl`/`check.jsonl`; manifests are for spec/research context only.
- Forgetting that a repo with no commits cannot create a Git worktree for isolated sub-agents.
