# Type Safety

> Contract safety for Trellis platform documents and adapters.

---

## Overview

This layer uses several contract formats rather than one type system:

- Markdown with YAML frontmatter for agent definitions and skills.
- JSON settings for Claude and some platform configs.
- TOML for Codex agent/config files.
- Python hook scripts with dictionary-based host payloads.
- JavaScript/TypeScript plugin adapters for OpenCode and Pi.

The goal is to keep these contracts explicit and validated at their boundaries.

---

## Markdown and Frontmatter Contracts

Agent files usually begin with frontmatter.

Local examples:
- `.claude/agents/trellis-implement.md` has `name`, `description`, and `tools` frontmatter.
- `.trellis/agents/implement.md` has `name`, `description`, `provider`, and `labels` frontmatter.

Rules:
- Preserve frontmatter delimiters exactly: opening `---`, YAML-like fields, closing `---`.
- Keep tool names in the format expected by the host. Claude platform files use names such as `Read, Write, Edit, Bash, Glob, Grep`.
- Keep `name` aligned with file name unless the platform has an established exception.
- Do not put host-specific unsupported fields into a platform file just because another platform supports them.

---

## JSON Contracts

JSON files must remain parseable and match host schemas.

Local examples:
- `.claude/settings.json` defines `env`, `hooks`, `enabledPlugins`, and `statusLine`.
- `.trellis/tasks/<task>/task.json` defines task metadata and status.
- `.trellis/.runtime/sessions/<key>.json` stores session metadata and `current_task`.

Rules:
- Use double-quoted JSON, no comments.
- For task JSON writes, use `common.io.write_json` or the existing active-task writer to preserve UTF-8 and pretty formatting.
- For hook settings, preserve arrays and matcher names exactly as required by the host.
- Validate edited JSON by reading it through the relevant tool or command before reporting success.

---

## TOML Contracts

Codex files use TOML.

Local example: `.codex/config.toml` documents project defaults and intentionally avoids `[features.multi_agent_v2]` because some Codex versions reject that table shape.

Rules:
- Keep compatibility comments near version-sensitive settings.
- Do not add feature blocks that are known to break older host versions unless the project explicitly drops support.
- Keep agent TOML files aligned with the platform's expected keys.

---

## Python Hook Payload Safety

Hook scripts receive untyped JSON payloads from host tools. Parse defensively.

Local examples:
- `.claude/hooks/inject-workflow-state.py` treats malformed or absent stdin as `{}`.
- `.claude/hooks/inject-subagent-context.py` extracts agent names from multiple platform encodings.
- `.trellis/scripts/common/active_task.py` recursively looks up session/conversation/transcript keys in nested hook payloads.

Rules:
- Check `isinstance` before using nested dictionary/list values.
- Normalize string values with `.strip()` and reject empty strings.
- Keep platform aliases explicit and narrow.
- When emitting hook JSON, use the host's required field names and `ensure_ascii=False` where non-ASCII context may appear.

---

## TypeScript/JavaScript Adapter Safety

Pi and OpenCode adapters mirror Python contracts in JS/TS.

Local examples:
- `.opencode/lib/trellis-context.js` uses `stringValue`, `sanitizeKey`, and `lookupString` before reading runtime session files.
- `.pi/extensions/trellis/index.ts` defines interfaces such as `SubagentInput`, `AgentConfig`, `PiRunConfig`, and runtime type guards such as `isObj` and `str`.

Rules:
- Keep runtime type guards close to host-event parsing.
- Limit tool arguments and output buffers as the Pi adapter does with constants such as `MAX_TOOL_ARG_CHARS`, `MAX_STDOUT`, and `MAX_TAIL`.
- Keep session key normalization compatible with Python `active_task.py`.
- Prefer explicit interfaces for extension/tool payloads in TypeScript files.

---

## Validation Patterns

- Run `python -m compileall` after Python hook changes.
- Use `python ./.trellis/scripts/get_context.py --mode phase --step <X.Y> --platform <platform>` to validate workflow platform block filtering.
- Use `python ./.trellis/scripts/task.py validate <task>` to validate JSONL manifest paths.
- Read or parse edited JSON/TOML files after changes when no dedicated test exists.

---

## Forbidden Patterns

- Do not use unchecked raw dictionary fields from hook payloads as paths or session keys.
- Do not assume every platform uses Claude Code's hook envelope.
- Do not write malformed Markdown frontmatter or mix YAML and TOML syntax.
- Do not use broad `any`-style assumptions in TypeScript adapter changes when a small interface/type guard would document the contract.
