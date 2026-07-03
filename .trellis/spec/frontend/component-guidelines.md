# Component Guidelines

> How agent, skill, and command documents are structured in this repository.

---

## Overview

There are no UI components here. The reusable "components" are agent definitions, skill entrypoints, command prompts, and managed instruction blocks that host AI tools read as structured Markdown/TOML.

Reference files:
- `.claude/agents/trellis-implement.md` and `.claude/agents/trellis-check.md` — sub-agent definitions with YAML frontmatter.
- `.trellis/agents/implement.md` and `.trellis/agents/check.md` — channel-runtime agent definitions.
- `.claude/skills/trellis-before-dev/SKILL.md` — skill entrypoint structure.
- `.claude/commands/trellis/continue.md` — slash-command prompt structure.
- `AGENTS.md` — managed instruction block.

---

## Agent Definition Pattern

Platform agent Markdown files should include:

1. Frontmatter with `name`, `description`, and available `tools` when the platform supports it.
2. A clear role heading.
3. A recursion guard for Trellis implement/check agents.
4. A context-loading protocol that handles hook-injected and fallback/manual paths.
5. Core responsibilities.
6. Forbidden operations, especially no commits/pushes from sub-agents.
7. A concise workflow and report format.

Local example: `.claude/agents/trellis-implement.md` states that the agent is already the implement sub-agent, must not spawn another implement/check agent, must read task artifacts when hook injection is absent, and must not run `git commit`, `git push`, or `git merge`.

Channel agent files under `.trellis/agents/` use similar sections but mention `trellis channel spawn --agent <name>` and require an `Active task: <path>` line in the inbox.

---

## Skill Document Pattern

Skill directories use `SKILL.md` plus optional `references/` files.

Local examples:
- `.claude/skills/trellis-spec-bootstrap/SKILL.md` defines the spec bootstrap workflow and routes detailed topics to `references/repository-analysis.md`, `references/spec-task-planning.md`, and `references/spec-writing.md`.
- `.claude/skills/trellis-before-dev/SKILL.md` lists a mandatory pre-development sequence for reading task artifacts and specs.

Rules:
- Keep `SKILL.md` as the high-level workflow and put long reusable guidance in `references/`.
- Make the skill host-neutral unless the task is specifically about a platform.
- Use ordered steps for required flows and tables for reference routing.
- Do not hide required persistence steps; Trellis expects research/spec/task learnings to be written to files.

---

## Command Prompt Pattern

Slash command files are entrypoints for the main session, not implementation scripts.

Local example: `.claude/commands/trellis/continue.md` tells the agent to:
- run `get_context.py`,
- run `get_context.py --mode phase`,
- route by task status and artifact presence,
- load the specific phase step before acting.

Rules:
- Commands should point at canonical runtime files such as `.trellis/workflow.md`; do not duplicate every workflow detail unless needed for routing.
- Commands should not imply implementation approval when they only resume context.
- For task continuation, preserve the distinction between planning, implementation, check, spec update, commit, and archive.

---

## Managed Instruction Blocks

`AGENTS.md` uses managed markers:

```markdown
<!-- TRELLIS:START -->
...
<!-- TRELLIS:END -->
```

Rules:
- Preserve text outside managed blocks.
- Expect text inside managed blocks to be overwritten by future Trellis updates.
- Keep the block focused on how agents should orient to `.trellis/`, not project-specific implementation minutiae.

---

## Report Formats

Agent report formats should be specific enough for the main session to act:

- Files modified or checked.
- Implementation/check summary.
- Verification results with pass/fail/skipped and reasons.
- Open questions or issues not fixed.

Do not use vague reports like "done" when a sub-agent changed files or ran checks.

---

## Anti-Patterns

- Do not tell a Trellis sub-agent to spawn another Trellis implement/check agent.
- Do not let sub-agent docs own commits; the supervising main session owns commit planning.
- Do not include browser accessibility/styling guidance unless the repository gains an actual browser UI.
- Do not copy a skill's long reference content into every command or agent definition.
