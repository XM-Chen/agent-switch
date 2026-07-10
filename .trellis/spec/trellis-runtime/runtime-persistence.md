# Persistence and Configuration Guidelines

> File-backed state patterns for the Trellis runtime.

---

## Overview

This project has no database, ORM, migrations, or transaction manager. Runtime state is stored in small text files under `.trellis/` and read through focused helpers. Treat `database-guidelines.md` as the guide for Trellis persistence and configuration.

Primary state files:
- `.trellis/tasks/<task>/task.json` — task metadata and lifecycle state.
- `.trellis/tasks/<task>/prd.md`, `design.md`, `implement.md` — task artifacts.
- `.trellis/tasks/<task>/implement.jsonl` and `check.jsonl` — curated spec/research manifests.
- `.trellis/.runtime/sessions/<context-key>.json` — session-scoped active-task pointers.
- `.trellis/workspace/<developer>/journal-N.md` and `index.md` — per-developer session history.
- `.trellis/config.yaml` — project-level runtime configuration.

---

## Task Metadata

Use `common.io.read_json` and `common.io.write_json` for task JSON files. They are the local pattern for UTF-8 JSON reads and pretty JSON writes.

Reference files:
- `.trellis/scripts/common/task_store.py` creates `task.json`, sets status fields, writes parent/child links, and updates archive metadata.
- `.trellis/scripts/task.py` flips `planning` to `in_progress` during `task.py start` and leaves archive completion to `task_store.cmd_archive`.

Rules:
- Preserve the existing `task.json` shape: identity fields, status, package/scope metadata, Git metadata, parent/children arrays, `relatedFiles`, `notes`, and `meta`.
- Use status values already consumed by workflow hooks: `planning`, `in_progress`, and `completed` unless a workflow change deliberately adds more.
- Keep task paths repo-relative and POSIX-formatted when writing references.
- Keep PRD/design/implementation artifacts as Markdown files next to `task.json`; do not embed their bodies in JSON.

---

## JSONL Manifests

`implement.jsonl` and `check.jsonl` list spec and research files that sub-agents should load. They are not source file inventories.

Reference files:
- `.trellis/scripts/common/task_context.py` validates entries and supports both file and directory entries.
- `.claude/hooks/inject-subagent-context.py` reads manifest entries and skips seed rows without a `file` field.

Entry shapes:

```json
{"file": ".trellis/spec/backend/index.md", "reason": "Runtime guidelines for task lifecycle changes"}
{"file": ".trellis/spec/frontend/", "type": "directory", "reason": "Platform prompt guidance"}
```

Rules:
- Use `file` for both file and directory paths; set `type: "directory"` for directory entries.
- Store spec/research paths only. Let agents search code files during the task.
- Treat rows without `file` as comments/seed rows; consumers already skip them.
- Validate manifests with `python ./.trellis/scripts/task.py validate <task-dir>` before dispatching sub-agents.

---

## Session Runtime State

Active tasks are session-scoped, not global.

Reference file: `.trellis/scripts/common/active_task.py`.

Local pattern:
- Resolve a stable context key from `TRELLIS_CONTEXT_ID`, platform hook payloads, platform-specific environment variables, or the Cursor shell ticket bridge.
- Store the current task in `.trellis/.runtime/sessions/<context-key>.json`.
- If a class-2 sub-agent cannot inherit a session key, fall back only when exactly one session file exists; never guess across multiple windows.
- `.trellis/.gitignore` excludes `.runtime/`, `.developer`, Python caches, and other local runtime files.

Do not reintroduce a single global `.current-task` pointer for new code. It is listed only as legacy/local state in `.trellis/.gitignore`.

---

## Configuration

Read `.trellis/config.yaml` through `common.config` or `common.trellis_config` helpers. The project uses a small dependency-free parser, so runtime scripts can run in a fresh Python environment.

Reference files:
- `.trellis/scripts/common/config.py` parses session settings, package declarations, and Codex dispatch configuration.
- `.trellis/config.yaml` documents `session_auto_commit`, package configuration, channel worker guards, and Codex dispatch mode.

Rules:
- Keep defaults in code and make config overrides optional.
- Preserve inline-comment handling and quote handling when extending the parser.
- Validate configured package names in CLI paths that accept `--package`.
- Keep platform-specific knobs scoped, such as `codex.dispatch_mode`; other platforms should ignore unknown sections.

---

## Workspace Journals

Workspace journals are append-only session records, not task requirements. Use `.trellis/workspace/<developer>/journal-N.md` and rotate at `max_journal_lines` from `config.yaml`.

Reference files:
- `.trellis/scripts/add_session.py` writes journal records.
- `.trellis/scripts/common/session_context.py` reports the active journal file and line count.

Rules:
- Do not store active task state in journals.
- Do not rely on journal content for deterministic task behavior; task artifacts and specs are the durable inputs for agents.

---

## Anti-Patterns

- Do not add SQL, ORM, or migration abstractions for Trellis state.
- Do not stage ignored `.trellis/` data with broad `git add -f .trellis/`; the archive code already warns against unsafe staging.
- Do not store platform hook payloads wholesale; extract only the stable fields needed for a session key.
- Do not write absolute paths into task metadata unless the path cannot be represented relative to the repository root.
