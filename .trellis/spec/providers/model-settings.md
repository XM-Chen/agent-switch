# Model Settings

## 1. Scope / Trigger

Use this contract when changing Claude model env fields, Codex top-level `model`, generated catalogs, or managed-account takeover defaults.

## 2. Signatures

- Claude subagent: `env.CLAUDE_CODE_SUBAGENT_MODEL?: string`, with optional `[1M]` suffix.
- Codex default: top-level TOML `model = "..."`; `useCodexConfigState` owns the form state and safe TOML rewrite.
- Codex OAuth Claude context: `CLAUDE_CODE_MAX_CONTEXT_TOKENS` and `CLAUDE_CODE_AUTO_COMPACT_WINDOW`.

## 3. Contracts

- Subagent is provider-specific, participates in quick-set/clear and mapper preservation, but is not a new aggregate tier.
- Common Config strips subagent, Fable, and context-window fields. Takeover removes stale values before copying target-provider values.
- An explicit Codex top-level `model` wins over model-catalog row zero. Row zero is only a fallback when top-level `model` is absent.
- TOML model writes escape quotes, backslashes, and control characters; section-local `model` keys are not top-level defaults.
- Codex OAuth providers targeting only GPT-5.6 receive 372000 live context defaults. Explicit provider values win, and injected defaults are stripped during backfill.

## 4. Validation & Error Matrix

| Condition | Behavior |
|---|---|
| Empty Claude subagent field | Remove the env key |
| Codex model contains hostile characters | Escape into one TOML basic-string line |
| Explicit Codex model plus catalog | Keep explicit model |
| Codex OAuth contains a non-GPT-5.6 model | Do not inject 372000 defaults |
| Backfill value equals an injected default | Remove it unless provider stored it explicitly |

## 5. Good / Base / Bad Cases

- Good: provider subagent and explicit Codex model round-trip without affecting other providers.
- Base: empty optional fields preserve existing provider behavior.
- Bad: copying provider model fields into shared Common Config or always replacing top-level model with catalog row zero.

## 6. Tests Required

- Hook hydrate/write/clear and Claude form quick-set/fallback 1M tests.
- Mapper and takeover stale-removal tests for subagent.
- TOML scope, escaping, removal, and hostile model ID tests.
- GPT-5.6 live injection and backfill tests.
- Typecheck, Clippy, and affected full suites.

## 7. Wrong vs Correct

Wrong: `model = "${remoteModelId}"` string interpolation and unconditional catalog-row overwrite.

Correct: use the shared safe TOML helper and backfill row zero only when no top-level model exists.
