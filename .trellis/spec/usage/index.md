# Usage Specifications

Project contracts for usage accounting, subscription quotas, and coding-plan integrations.

## Pre-Development Checklist

- Read [Coding Plan Quotas](coding-plan-quotas.md) before changing coding-plan provider detection, credentials, commands, or quota UI.
- Read [Usage Cache and Accounting](usage-cache-accounting.md) before changing transport errors, tray snapshots, cache-write tokens, or historical cost backfill.
- Preserve existing personal-plan auto-detection when adding a provider variant that shares the same base URL.

## Quality Check

- Trace new fields through persisted `UsageScript`, frontend API arguments, Tauri commands, service validation, and UI.
- Add a request-shape test for provider-specific URL queries and headers.
- Run focused quota tests, `pnpm typecheck`, and Rust Clippy.
- For token semantics, verify parser, calculator, logger, rollup, and backfill as one chain.
