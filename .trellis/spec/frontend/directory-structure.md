# Directory Structure

> How agent-platform files are organized in this repository.

---

## Scope

This guide covers the platform-facing layer: instructions, prompts, hooks, settings, skills, commands, agent definitions, and plugin code. It does not cover browser components or UI assets because this repository has none.

---

## Platform Layout

```text
AGENTS.md                         # Cross-agent project instructions
.claude/
├── agents/                       # Claude Code Trellis sub-agent definitions
├── commands/trellis/             # Claude slash-command entrypoints
├── hooks/                        # Python hook adapters
├── settings.json                 # Claude project hooks/statusline/env
└── skills/                       # Trellis skills and references
.codex/
├── agents/                       # TOML sub-agent definitions
├── config.toml                   # Codex project defaults
└── hooks/                        # Python hook adapters
.cursor/, .trae/
├── agents/                       # Platform agent definitions
├── commands/                     # Trellis command markdown
├── hooks/                        # Python hook adapters
└── skills/                       # Bundled Trellis skills
.opencode/
├── agents/                       # Agent definitions
└── lib/trellis-context.js        # Plugin context helper
.pi/
├── agents/                       # Agent definitions
├── prompts/                      # Trellis command prompts
├── skills/                       # Bundled Trellis skills
├── settings.json                 # Pi settings
└── extensions/trellis/index.ts   # Pi extension and sub-agent tool
.reasonix/, .zcode/               # Platform-specific generated prompts/skills/agents
.trellis/agents/                  # Channel runtime agent definitions
```

---

## Ownership Boundaries

- `AGENTS.md` contains the managed Trellis instruction block shared by agent-compatible tools.
- `.trellis/workflow.md` is the source of workflow phase text and `[workflow-state:*]` breadcrumb bodies.
- `.trellis/scripts/common/workflow_phase.py` parses workflow step/platform blocks for command output.
- Hook-capable platforms use Python hook copies under their own directory, such as `.claude/hooks/inject-workflow-state.py` and `.trae/hooks/session-start.py`.
- OpenCode and Pi have JavaScript/TypeScript adapters because their plugin APIs run inside those runtimes.
- Channel-runtime agents live in `.trellis/agents/` and are not the same files as Claude/Cursor/Codex platform agents.

---

## Naming Conventions

- Trellis sub-agent names use the `trellis-` prefix in platform directories: `trellis-implement`, `trellis-check`, `trellis-research`.
- Channel-runtime agent definitions under `.trellis/agents/` use shorter names (`implement`, `check`) because the channel command supplies Trellis context.
- Slash command files are kebab-case and grouped by platform convention: `.claude/commands/trellis/continue.md`, `.cursor/commands/trellis-continue.md`, `.pi/prompts/trellis-continue.md`.
- Skills use kebab-case directories such as `trellis-before-dev`, `trellis-spec-bootstrap`, and `trellis-update-spec`.
- Hook files use descriptive snake_case or kebab-compatible names matching existing platform files: `inject-workflow-state.py`, `session-start.py`, `trellis-context.js`.

---

## Adding or Editing Platform Files

- Mirror an existing platform's shape before introducing a new pattern. For example, `.claude/agents/trellis-implement.md` and `.claude/agents/trellis-check.md` share frontmatter, recursion guard, context protocol, forbidden operations, and report format sections.
- Keep managed Trellis blocks explicit. `AGENTS.md` uses `<!-- TRELLIS:START -->` / `<!-- TRELLIS:END -->` and warns that edits inside may be overwritten.
- Put reusable skill references under `skills/<skill>/references/` and keep `SKILL.md` as the entrypoint.
- Do not mix runtime Python helpers into platform directories unless they are host hooks. Shared runtime behavior belongs under `.trellis/scripts/common/`.

---

## Examples to Follow

- `.claude/settings.json` shows project hook registration for `SessionStart`, `PreToolUse`, `UserPromptSubmit`, and status line.
- `.claude/commands/trellis/continue.md` is a command entrypoint that tells the main agent to load context and route to the correct workflow step.
- `.codex/config.toml` documents host limitations and avoids incompatible feature blocks.
- `.opencode/lib/trellis-context.js` mirrors Python active-task resolution in a plugin runtime.
- `.pi/extensions/trellis/index.ts` implements a tool, session startup injection, per-turn workflow breadcrumbs, and sub-agent prompt building.

---

## Anti-Patterns

- Do not add React-style `components/`, `hooks/`, or `pages/` directories; the project has no browser UI layer.
- Do not duplicate long workflow instructions independently per platform when the host can parse `.trellis/workflow.md`.
- Do not edit generated platform copies inconsistently; if a convention appears in multiple platform directories, update all relevant copies or document why one platform differs.
- Do not place secrets, user tokens, or machine-local paths in tracked settings files.
