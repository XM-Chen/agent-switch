# Directory Structure

> How the Python Trellis runtime is organized in this repository.

---

## Scope

This guide covers `.trellis/scripts/` and shared runtime data under `.trellis/`. Platform-facing prompts, hooks, settings, and generated agent definitions live in the frontend/platform spec layer.

---

## Runtime Layout

```text
.trellis/
├── workflow.md                 # Human-readable workflow and workflow-state tag source
├── config.yaml                 # Project-level Trellis runtime configuration
├── .gitignore                  # Local Trellis runtime exclusions
├── agents/                     # Channel-runtime agent definitions
├── scripts/
│   ├── task.py                 # Task lifecycle CLI entrypoint
│   ├── get_context.py          # Context CLI entry shim
│   ├── init_developer.py       # Developer identity bootstrap
│   ├── add_session.py          # Workspace journal/session recorder
│   └── common/
│       ├── paths.py            # Path constants and repo/task/workspace resolution
│       ├── active_task.py      # Session-scoped active-task resolver
│       ├── config.py           # Dependency-free config.yaml reader
│       ├── task_store.py       # Create/archive/metadata/subtask command handlers
│       ├── task_context.py     # implement.jsonl/check.jsonl command handlers
│       ├── session_context.py  # SessionStart/default context rendering
│       ├── packages_context.py # Package/spec layer discovery
│       ├── workflow_phase.py   # Workflow phase extraction and platform filtering
│       ├── io.py               # JSON file I/O helpers
│       └── log.py              # ANSI color and log helpers
├── spec/                       # Project guidance consumed by agents
├── tasks/                      # Active task directories
└── workspace/                  # Per-developer journals and indexes
```

---

## Module Boundaries

Use the existing split between thin entrypoints and shared modules:

- Keep CLI parsing in entrypoint files such as `.trellis/scripts/task.py` and `.trellis/scripts/get_context.py`.
- Put reusable behavior in `.trellis/scripts/common/`. For example, `task.py` delegates create/archive operations to `common/task_store.py` and JSONL manifest operations to `common/task_context.py`.
- Put path constants in `common/paths.py`; do not scatter literal `.trellis`, `tasks`, `workspace`, or `task.json` strings through new runtime code.
- Put active-task session resolution in `common/active_task.py`. Other modules should call its public helpers instead of reading `.trellis/.runtime/sessions/` directly.
- Put Git execution behind `common/git.py` and safe staging logic behind `common/safe_commit.py`.

Reference pattern:
- `.trellis/scripts/get_context.py` is intentionally a minimal shim that imports `common.git_context.main`.
- `.trellis/scripts/common/git_context.py` routes `--mode default`, `--mode packages`, and `--mode phase` to focused modules rather than owning every detail itself.

---

## Naming Conventions

- Python modules use lowercase snake_case: `active_task.py`, `workflow_phase.py`, `task_context.py`.
- Public command handlers use `cmd_<subcommand>` names: `cmd_start`, `cmd_archive`, `cmd_add_context`.
- Constants use uppercase names in the module that owns them: `DIR_WORKFLOW`, `FILE_TASK_JSON`, `AGENTS_REQUIRE_TASK`.
- Runtime task references are repo-relative POSIX paths where possible, such as `.trellis/tasks/00-bootstrap-guidelines`, even on Windows.
- Platform names are normalized to stable lowercase tokens such as `claude`, `codex`, `cursor`, `opencode`, `pi`, and `trae`.

---

## Where New Code Goes

- Add a new `task.py` subcommand by creating a focused `cmd_*` handler in the relevant `common/` module, then registering it in `task.py`'s argparse table and dispatch map.
- Add a new context output mode by extending `common/git_context.py` and keeping rendering logic in a dedicated module if it grows beyond routing.
- Add workflow parsing behavior in `common/workflow_phase.py`; platform prompt text remains in `.trellis/workflow.md` tag blocks.
- Add file-state helpers next to existing state helpers: JSON helpers in `common/io.py`, config helpers in `common/config.py`, path helpers in `common/paths.py`.
- Do not add web-style directories such as `routes/`, `controllers/`, `models/`, or `services/`; this repository has no server/API layer.

---

## Examples to Follow

- `.trellis/scripts/common/active_task.py` demonstrates defensive parsing of environment variables, hook payloads, and runtime session files while preserving multi-window isolation.
- `.trellis/scripts/common/task_context.py` demonstrates JSONL manifest validation while skipping seed rows that have no `file` field.
- `.trellis/scripts/common/workflow_phase.py` demonstrates extracting behavior from `.trellis/workflow.md` instead of duplicating prompt text in Python.
- `.trellis/scripts/common/session_context.py` demonstrates bounded Git/status context rendering without treating a non-Git root as clean.

---

## Anti-Patterns

- Do not bypass `common/paths.py` or `common/active_task.py` with local path/session parsing.
- Do not duplicate workflow-state prose in Python; `.trellis/workflow.md` is the source for per-turn breadcrumbs.
- Do not add dependencies for simple YAML/JSON parsing unless the project explicitly adopts dependency management.
- Do not mix platform-specific prompt files into `.trellis/scripts/`; platform adapters belong under `.claude/`, `.cursor/`, `.codex/`, `.opencode/`, `.pi/`, `.trae/`, `.zcode/`, and similar directories.
