# Quality Guidelines

> Review and validation standards for agent-platform files.

---

## Overview

Platform files should keep Trellis behavior consistent across hosts while respecting host-specific schemas. Most changes are prompt/config/hook changes, so quality means contract consistency, context order correctness, and no stale template language.

Reference files:
- `AGENTS.md` — managed cross-agent instructions.
- `.claude/settings.json` — hook and statusline configuration.
- `.claude/hooks/*.py` — Python hook behavior.
- `.claude/agents/trellis-*.md` — agent prompts.
- `.claude/skills/*/SKILL.md` — skill prompts and references.
- `.opencode/lib/trellis-context.js` and `.pi/extensions/trellis/index.ts` — plugin adapters.
- `.trellis/workflow.md` — source of workflow-state bodies.

---

## Required Patterns

- Keep workflow routing text in `.trellis/workflow.md`; platform adapters should parse or reference it.
- Preserve Trellis sub-agent recursion guards in implement/check agent definitions.
- Preserve `Active task: <path>` dispatch protocol where a platform requires pull-based context loading.
- Keep context load order stable: JSONL manifest entries, `prd.md`, optional `design.md`, optional `implement.md`.
- Keep no-commit/no-push restrictions in implement/check sub-agent definitions.
- Keep host configuration files syntactically valid for their host: JSON for Claude settings, TOML for Codex config/agents, Markdown frontmatter for agents and skills.

---

## Forbidden Patterns

- Generic web frontend guidance about React components, CSS, browser accessibility, or client state unless actual web UI files are added.
- Platform drift: changing a Trellis rule in `.claude/` but leaving equivalent `.cursor/`, `.trae/`, `.pi/`, or `.opencode/` files contradictory without documenting why.
- Hook stdout debug text before/after JSON protocol output.
- Secrets, user-specific absolute paths, or tokens in tracked settings.
- Sub-agent prompts that tell `trellis-implement` or `trellis-check` to dispatch another implement/check agent.

---

## Verification Commands

For Python hook or workflow changes:

```bash
python -m compileall .claude/hooks .codex/hooks .trae/hooks .trellis/scripts
python ./.trellis/scripts/get_context.py --mode phase
python ./.trellis/scripts/get_context.py --mode phase --step 2.1 --platform claude
python ./.trellis/scripts/get_context.py --mode phase --step 2.1 --platform codex
python ./.trellis/scripts/get_context.py --mode phase --step 2.2 --platform claude
```

For manifest/context changes:

```bash
python ./.trellis/scripts/task.py validate 00-bootstrap-guidelines
python ./.trellis/scripts/task.py list-context 00-bootstrap-guidelines
```

For JS/TS adapter changes, run the host package's test/typecheck only if this workspace includes the needed package manifest and dependencies. If no package manager setup exists, record the skipped check explicitly.

---

## Review Checklist

- [ ] Edited Markdown has valid frontmatter and no empty template sections.
- [ ] Equivalent platform copies are consistent or differences are intentional and documented.
- [ ] Hook output remains valid for the target host.
- [ ] Workflow-state edits include both the Phase Index tag block and any detailed step text that must stay in sync.
- [ ] Agent definitions still include context-loading fallback behavior.
- [ ] Settings/config files do not contain local-only secrets or paths.
- [ ] Verification commands were run or explicitly skipped with reasons.

---

## Common Mistakes

- Editing a generated platform file manually but not updating its source/template path when one exists.
- Forgetting that `trellis-check` exists as both a skill and an agent on some hosts; workflow text says which one to prefer.
- Assuming Codex behaves like Claude. This repo documents Codex hook and dispatch-mode constraints in `.codex/config.toml` and `.trellis/config.yaml`.
- Forgetting to preserve the one-shot first-reply notice used by SessionStart context.
