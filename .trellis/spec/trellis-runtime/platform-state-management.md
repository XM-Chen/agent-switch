# State Management

> How workflow, task, session, and platform state is coordinated.

---

## Overview

This project has no client-side state library. State is file-backed Trellis workflow state plus host-session context propagated into hooks and sub-agents.

Primary state owners:
- `.trellis/tasks/<task>/task.json` — task lifecycle metadata.
- `.trellis/.runtime/sessions/<context-key>.json` — active task pointer for one AI session/window.
- `.trellis/config.yaml` — project settings and platform mode knobs.
- `.trellis/workflow.md` — workflow-state breadcrumb text and phase instructions.
- Platform settings files such as `.claude/settings.json` and `.codex/config.toml` — host integration state.

---

## State Categories

### Task State

Task state lives in `task.json` and artifacts in the task directory. The status field drives the workflow breadcrumb:

- `planning` → Phase 1 planning routes.
- `in_progress` → Phase 2 implementation/check and Phase 3 spec-update/commit routes.
- `completed` → archive/completion path, normally moved immediately under `tasks/archive/`.

Do not infer task progress only from chat history; task artifacts are the durable source.

### Session State

Active task selection is session-scoped under `.trellis/.runtime/sessions/`.

Local patterns:
- `common.active_task.resolve_context_key` derives a stable key from hook input, platform environment variables, `TRELLIS_CONTEXT_ID`, or a Cursor shell ticket.
- `set_active_task` writes `current_task` under that session key.
- Fallback to a single session file is allowed only when exactly one exists.

### Workflow State

Workflow routing text lives in `.trellis/workflow.md`, not in every hook implementation. Platform hooks parse `[workflow-state:*]` blocks and inject them per turn.

### Platform Configuration State

Host settings define how Trellis hooks and commands are exposed:
- `.claude/settings.json` registers hooks and status line.
- `.codex/config.toml` keeps Codex defaults and documents user-level requirements.
- `.pi/extensions/trellis/index.ts` registers a `trellis_subagent` tool and injects context through Pi events.

---

## Promoting or Sharing State

Use the narrowest durable state owner:

- A requirement or acceptance criterion belongs in `prd.md`.
- A technical decision for one task belongs in `design.md` or task research.
- A reusable convention belongs in `.trellis/spec/`.
- A session pointer belongs in `.trellis/.runtime/sessions/`.
- A platform integration setting belongs in that platform's settings/config file.

Do not store a reusable convention only in a chat message or journal.

---

## Derived State

Several adapters derive state from files:

- `get_context.py` derives current task, Git status, active tasks, journal status, and spec layers.
- `inject-workflow-state.py` derives the breadcrumb key from task status and Codex dispatch mode.
- `packages_context.py` derives available spec layers from `.trellis/spec/` directories.
- Pi/OpenCode adapters derive current task and context snippets from session runtime files and task manifests.

Rules:
- Recompute derived state from source files instead of caching it in additional tracked files.
- If adding derived state for performance, keep the cache local/ignored and make stale/missing cache safe.

---

## Configuration State

`.trellis/config.yaml` is the central project-level runtime config.

Current documented knobs include:
- `session_commit_message`
- `max_journal_lines`
- `session_auto_commit`
- lifecycle `hooks`
- monorepo `packages` and `default_package`
- `channel.worker_guard`
- `codex.dispatch_mode`

Rules:
- Keep defaults sensible when a key is absent.
- Preserve unknown platform sections for forward compatibility.
- Avoid moving user-specific state into tracked config files; use ignored runtime files for local identity and session pointers.

---

## Common Mistakes

- Treating `finish` as complete. `task.py finish` clears the active pointer; `task.py archive` marks completed and moves the task.
- Treating the current shell as the only session. Multiple AI windows may work in the same repo, so active task state must remain keyed.
- Updating platform prompt text without updating `.trellis/workflow.md` when the prompt is workflow-state behavior.
- Putting source code paths into JSONL manifests instead of spec/research context paths.
- Forgetting that this repo's top-level `.gitignore` may ignore `.trellis/`; Trellis runtime files can be local-only by project choice.
