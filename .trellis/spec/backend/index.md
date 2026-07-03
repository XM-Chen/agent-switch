# Trellis Runtime Guidelines

> Project-specific guidance for the Python Trellis runtime in this repository.

---

## Overview

This repository is a Trellis workspace and agent-platform bundle. In this spec layer, "backend" means the local Python runtime under `.trellis/scripts/`: task lifecycle commands, session context discovery, spec/package discovery, workflow phase extraction, safe Git helpers, and file persistence. It is not a web server and has no ORM, HTTP route layer, or long-running service process.

Reference files:
- `.trellis/scripts/task.py` — command entrypoint for task lifecycle operations.
- `.trellis/scripts/get_context.py` — entry shim for session, package, and workflow context.
- `.trellis/scripts/common/active_task.py` — session-scoped active task resolution.
- `.trellis/scripts/common/task_store.py` — task creation, archive, hierarchy, and metadata updates.
- `.trellis/scripts/common/session_context.py` — session overview and Git/package status output.
- `.trellis/scripts/common/workflow_phase.py` — workflow step extraction and platform block filtering.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Python runtime module boundaries and naming | Project-specific |
| [Persistence and Configuration](./database-guidelines.md) | File-backed task, session, config, and manifest state | Project-specific |
| [Error Handling](./error-handling.md) | CLI return codes, warnings, hook fail-closed behavior | Project-specific |
| [Logging Guidelines](./logging-guidelines.md) | Terminal output, hook stdout/stderr, and quiet utility rules | Project-specific |
| [Quality Guidelines](./quality-guidelines.md) | Verification commands and runtime review checklist | Project-specific |

---

## Pre-Development Checklist

Before modifying Trellis runtime code:

1. Identify the runtime boundary being changed:
   - CLI command surface: `task.py`, `get_context.py`, `init_developer.py`, `add_session.py`.
   - Shared behavior: `.trellis/scripts/common/*.py`.
   - Workflow contract: `.trellis/workflow.md` plus `common/workflow_phase.py`.
   - Hook/platform adapters: read the frontend platform spec as well.
2. Read [Directory Structure](./directory-structure.md) before adding or moving Python modules.
3. Read [Persistence and Configuration](./database-guidelines.md) before touching `task.json`, JSONL manifests, `.runtime/`, `.workspace/`, or `config.yaml` handling.
4. Read [Error Handling](./error-handling.md) before changing command failure modes or hook behavior.
5. Read [Logging Guidelines](./logging-guidelines.md) before changing stdout/stderr or hook output.
6. Read [Quality Guidelines](./quality-guidelines.md) for the verification commands expected after runtime changes.

---

## Quality Check

For runtime-only changes, run the smallest relevant subset and record any skipped checks with a reason:

```bash
python -m compileall .trellis/scripts
python ./.trellis/scripts/get_context.py
python ./.trellis/scripts/get_context.py --mode packages
python ./.trellis/scripts/get_context.py --mode phase --step 2.1 --platform claude
python ./.trellis/scripts/task.py validate 00-bootstrap-guidelines
```

If the change affects platform prompt injection or generated agent files, also follow `.trellis/spec/frontend/index.md`.

---

**语言**：除非用户明确要求其他语言，或某个工具/模板强制要求，否则本规范树中的所有文档都应使用中文。
