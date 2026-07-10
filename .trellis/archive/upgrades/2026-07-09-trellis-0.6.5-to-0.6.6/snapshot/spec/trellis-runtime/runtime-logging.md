# Logging Guidelines

> Terminal and hook output conventions for the Trellis runtime.

---

## Overview

The Python runtime mostly communicates through CLI output and hook protocol payloads. There is no application logger or structured log sink. Use explicit stdout/stderr discipline so humans can read commands and host tools can parse hook output.

Reference files:
- `.trellis/scripts/common/log.py` — ANSI color helpers and simple `log_*` functions.
- `.trellis/scripts/common/session_context.py` — human-readable context output.
- `.trellis/scripts/common/task_store.py` — archive auto-commit warnings and status messages.
- `.claude/hooks/inject-workflow-state.py` — JSON hook response output.
- `.opencode/lib/trellis-context.js` — OpenCode plugin debug log helper.

---

## Output Channels

Use output channels intentionally:

- CLI commands: print human-facing status to stdout or stderr consistently with the existing command.
- Commands that emit a path for scripting, such as `task.py create` and `task.py archive`, print the machine-usable path on stdout and explanatory text on stderr.
- Hooks: print only the host protocol response on stdout; put warnings on stderr or stay silent.
- Session context: `get_context.py` writes a complete Markdown/text block to stdout.

Local examples:
- `task_store.cmd_create` prints setup guidance to stderr and the created task path to stdout.
- `inject-subagent-context.py` logs missing/empty JSONL warnings to stderr but emits hook JSON to stdout only when it can inject context.

---

## Color and Message Style

Use `.trellis/scripts/common/log.py` for terminal coloring:

- Green for success (`✓`, `[SUCCESS]`).
- Yellow for warnings and degraded mode.
- Red for command errors.
- Blue/Cyan for headings or informational labels.

Rules:
- Keep colored output for humans only; do not include ANSI color in JSON output.
- Use concise labels and include the relevant task/path value.
- Follow the existing checkmark and warning style instead of inventing a new status vocabulary.

---

## Hook Diagnostics

Hooks are often called automatically and repeatedly, so diagnostics must be quiet by default.

Rules:
- Silent `exit 0` is correct when the hook is not applicable: no Trellis project, non-Trellis sub-agent, no active task for implement/check injection.
- Print a warning only when the user/operator can act on it, such as an empty manifest after a Trellis sub-agent is being injected.
- Never print debug/progress text to stdout before or after JSON hook output.
- On Windows, configure UTF-8 before emitting Chinese or other non-ASCII text.

Reference examples:
- `.claude/hooks/inject-workflow-state.py` uses `json.dumps` for the Claude/Cursor-style hook envelope and plain text only for Kiro.
- `.claude/hooks/inject-subagent-context.py` uses `ensure_ascii=False` to preserve injected non-ASCII context.

---

## Debug Logs

The only explicit debug-log helper in the inspected platform code is OpenCode's `.opencode/lib/trellis-context.js`, which writes to `/tmp/trellis-plugin-debug.log` and ignores logging failures.

Rules:
- Keep debug logs best-effort and non-fatal.
- Do not add persistent debug files under tracked task/spec directories.
- Do not log secrets, complete hook payloads, or full prompts unless a platform-specific debugging tool explicitly needs them and the file is local-only.

---

## What to Log

Log events that help a user recover:

- Task created, started, archived, or linked/unlinked.
- Context manifest validation errors.
- Session identity degraded mode.
- Git auto-commit skip/failure during archive/session recording.
- Missing setup that prevents a command from working.

---

## What Not to Log

Avoid noisy or sensitive output:

- Do not print full environment variables.
- Do not print full hook input payloads.
- Do not print generated prompts or task context unless that is the explicit command output.
- Do not print Python stack traces for expected user mistakes.
- Do not log ignored runtime files such as `.trellis/.runtime/` or `.trellis/.developer` contents.
