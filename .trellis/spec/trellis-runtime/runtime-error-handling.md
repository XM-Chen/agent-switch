# Error Handling

> How the Trellis Python runtime reports failures and degrades safely.

---

## Overview

The runtime is a set of CLI scripts and hook adapters. Error handling is designed around clear terminal messages, deterministic return codes, and fail-closed hooks that do not break the host agent when optional context is unavailable.

Reference files:
- `.trellis/scripts/task.py` — command return codes and user-facing errors.
- `.trellis/scripts/common/task_context.py` — manifest validation errors.
- `.trellis/scripts/common/active_task.py` — defensive active-task resolution.
- `.claude/hooks/inject-workflow-state.py` — hook fail-closed behavior and platform-specific output.
- `.claude/hooks/inject-subagent-context.py` — context injection warnings and silent exits when not applicable.

---

## CLI Error Pattern

Command handlers return integer exit codes and print actionable messages.

Local examples:
- `task.py cmd_start` prints an error and hint when a task directory cannot be resolved.
- `task_context.cmd_validate` prints per-file validation failures and returns non-zero when any manifest entry is invalid.
- `task_store.cmd_create` validates package names and prints available packages on invalid input.

Rules:
- Return `0` for success, `1` for user-correctable command errors, and `2` only for explicit CLI usage/deprecation errors already handled by the entrypoint.
- Include the bad value and a next step when possible: task name, path, package name, or command syntax.
- Keep command handlers idempotent where practical. For example, adding an existing JSONL context entry prints a warning and returns success.
- Use `common.log.colored`/`Colors` for human terminal output; do not color JSON machine output.

---

## Missing or Invalid Files

File readers generally return `None`/empty data rather than raising through the command stack.

Local examples:
- `common.io.read_json` returns `None` for missing, invalid, or unreadable JSON.
- `common.active_task._read_json` returns `None` for malformed session files.
- `packages_context` handles missing package declarations as single-repo mode.

Rules:
- Check for `None` immediately after reading JSON.
- Preserve existing task files if a related optional file is missing.
- Make absence explicit in validation output when it blocks the user, and silent when the hook is simply not applicable.

---

## Hook Error Pattern

Hooks must not crash the host agent for normal absence cases.

Local examples:
- `.claude/hooks/inject-workflow-state.py` exits `0` with no output when no `.trellis/` directory exists.
- It emits a generic breadcrumb when `.trellis/workflow.md` lacks a workflow-state tag, making broken workflow text visible without masking it in code.
- `.claude/hooks/inject-subagent-context.py` exits `0` when a spawned agent is not a Trellis agent or no active task exists.

Rules:
- Treat malformed hook input as `{}` and continue with safe defaults.
- Emit host-specific output only after all required context has been built.
- Write diagnostic warnings to `stderr`; reserve `stdout` for the hook protocol payload.
- Reconfigure stdin/stdout/stderr to UTF-8 on Windows before reading or writing non-ASCII content.

---

## Active Task Degradation

Session identity may be unavailable in some shells or sub-agent contexts.

Local pattern in `.trellis/scripts/common/active_task.py`:
- Prefer explicit session keys from hook input or `TRELLIS_CONTEXT_ID`.
- Use platform environment variables when available.
- Use a single-session fallback only when exactly one runtime session file exists.
- Refuse to guess when multiple session files exist.

Local pattern in `task.py cmd_start`:
- When no context key exists, print a degraded-mode notice.
- Still update `task.json` from `planning` to `in_progress` so the task lifecycle can proceed.

Rules:
- Do not weaken the multi-window isolation rule to make a command appear convenient.
- If a command cannot persist a session pointer, say so and continue only when the task lifecycle can remain correct.

---

## Platform Protocol Errors

Different hosts require different output shapes.

Examples:
- Gemini expects `BeforeAgent` instead of Claude-style `UserPromptSubmit` for the per-turn hook event.
- Kiro receives plain text from prompt-submit hooks, while Claude/Cursor-style hosts receive JSON envelopes.
- OpenCode and Pi mirror active-task resolution in JavaScript/TypeScript because their plugin APIs run outside the Python hook process.

Rules:
- Keep platform branching isolated in hook/adaptor files.
- Add a platform-specific branch only when the host schema requires it.
- Preserve the silent no-op path for unsupported or irrelevant hook invocations.

---

## Anti-Patterns

- Do not let stack traces escape for normal user mistakes such as unknown task names or missing manifest entries.
- Do not swallow validation failures that should block a task from starting.
- Do not print debug lines to stdout from hooks; that corrupts JSON protocol output.
- Do not collapse all hook failures into success if the hook has already started writing a protocol response.
