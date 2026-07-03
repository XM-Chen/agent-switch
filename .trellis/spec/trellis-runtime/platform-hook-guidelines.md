# Hook Guidelines

> How platform hooks and plugin adapters inject Trellis context.

---

## Overview

Hooks are the bridge between host AI tools and Trellis state. They provide SessionStart context, per-turn workflow breadcrumbs, and sub-agent task/spec context. Hook code must be quiet when not applicable, deterministic when active, and aligned with `.trellis/workflow.md`.

Reference files:
- `.claude/hooks/session-start.py` — compact SessionStart context.
- `.claude/hooks/inject-workflow-state.py` — per-turn workflow breadcrumb injection.
- `.claude/hooks/inject-subagent-context.py` — implement/check/research context injection.
- `.claude/settings.json` — Claude Code hook registration.
- `.opencode/lib/trellis-context.js` — OpenCode context helper.
- `.pi/extensions/trellis/index.ts` — Pi extension hooks and sub-agent tool.
- `.codex/hooks/`, `.trae/hooks/` — platform copies of shared Python hooks.

---

## Per-Turn Workflow Breadcrumbs

The source of breadcrumb text is `.trellis/workflow.md`, specifically `[workflow-state:<status>]...[/workflow-state:<status>]` blocks.

Local pattern:
- Python hook: `.claude/hooks/inject-workflow-state.py` parses those blocks and emits `<workflow-state>...</workflow-state>`.
- OpenCode/Pi adapters implement the same tag parsing in JavaScript/TypeScript.
- `common/workflow_phase.py` also parses workflow step sections for `/trellis:continue` and similar commands.

Rules:
- Do not hardcode fallback workflow bodies in hook code. A missing tag should degrade visibly to "Refer to workflow.md for current step."
- Preserve platform-specific event names: Gemini uses `BeforeAgent`, while Claude-style hosts use `UserPromptSubmit`.
- Keep Codex inline/sub-agent mode selection tied to `.trellis/config.yaml` `codex.dispatch_mode`.
- Keep Kiro/plain-text output separate from JSON-envelope platforms.

---

## Sub-Agent Context Injection

Implement/check sub-agents need task artifacts and curated spec/research manifests.

Local pattern in `.claude/hooks/inject-subagent-context.py`:
- Detect Trellis agent names: `trellis-implement`, `trellis-check`, `trellis-research`.
- Resolve the active task through `common.active_task.resolve_active_task`.
- Read `implement.jsonl` or `check.jsonl` entries first, then `prd.md`, optional `design.md`, and optional `implement.md`.
- Build a prompt with `<!-- trellis-hook-injected -->` so the sub-agent knows context was loaded.

Rules:
- Implement/check require an active task; research can run with broader project context.
- JSONL entries can be files or directories of Markdown files.
- Skip seed rows without `file` silently, but warn on stderr when a manifest has no curated entries.
- Keep the context order stable: jsonl entries, PRD, design if present, implement plan if present.

---

## SessionStart Context

SessionStart should orient without dumping every spec.

Local pattern:
- The startup context says Trellis compact context is loaded and instructs the agent to load details on demand.
- The first visible reply notice asks the assistant to mention in Chinese once that the Trellis SessionStart context loaded.
- Full phase details are loaded only when needed via `get_context.py --mode phase --step <X.Y>`.

Rules:
- Keep startup context compact.
- Do not preload all spec files into every session; use task manifests and `trellis-before-dev` for targeted loading.

---

## Hook Registration

Use the host's native settings shape.

Local example in `.claude/settings.json`:
- `SessionStart` runs `.claude/hooks/session-start.py` for startup, clear, and compact.
- `PreToolUse` runs `.claude/hooks/inject-subagent-context.py` for `Task` and `Agent` tools.
- `UserPromptSubmit` runs `.claude/hooks/inject-workflow-state.py`.
- `statusLine` runs `.claude/hooks/statusline.py`.

Rules:
- Keep hook command paths relative to the project root when the host executes them from the project.
- Set reasonable timeouts; hooks should be fast and fail closed.
- If adding a new platform, mirror existing registration concepts but use that platform's schema.

---

## Cross-Platform Adapter Rules

- Keep Python hook logic shared where possible, copied into platform hook directories as generated files.
- When a platform requires JS/TS plugin code, mirror the Python contract: session key resolution, single-session fallback, workflow tag parsing, and manifest loading.
- Keep platform-specific quirks documented near the adapter, as `.codex/config.toml` does for Codex hook and feature limitations.

---

## Anti-Patterns

- Do not print arbitrary debug text to hook stdout.
- Do not guess an active task when multiple session runtime files exist.
- Do not inject all of `.trellis/spec/` into every turn; targeted context keeps agents focused.
- Do not let platform-specific mode switches drift from `.trellis/config.yaml` and `.trellis/workflow.md`.
