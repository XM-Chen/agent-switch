# Usage Cache and Accounting

## 1. Scope / Trigger

Use this contract when changing provider usage transport errors, React Query keep-last-good behavior, tray usage snapshots, cache read/write token parsing, rollups, or historical cost backfill.

## 2. Signatures

- Tauri usage commands return `Result<UsageResult, String>` or `Result<SubscriptionQuota, String>`.
- `input_token_semantics`: `0=LEGACY`, `1=TOTAL`, `2=FRESH` on `proxy_request_logs` and `usage_daily_rollups`.
- `resolveDisplayUsage(raw, dataUpdatedAt, lastGood, now, { rejected, keepMs })` owns the frontend display window.

## 3. Contracts

- Transport send/read failures return `Err`; HTTP/auth/complete invalid-response failures remain `Ok(success=false)` when credentials or upstream status are known.
- Only `Ok` results are emitted and stored as fresh usage snapshots. A transport `Err` invalidates the tray cache because it has no timestamped keep-last-good window; React Query may display its last success for 10 minutes.
- TOTAL input includes cache read and cache creation; FRESH includes neither; LEGACY preserves the historical Codex/Gemini rule of subtracting cache read only.
- Parser, calculator, logger, rollup, dashboard aggregation, and cost backfill must use the same semantics.

## 4. Validation & Error Matrix

| Condition | Result |
|---|---|
| Network timeout / response-body read failure | command `Err`, React Query retry, tray cache invalidated |
| HTTP 401/403 | `Ok(success=false)`, shown immediately, last-good cleared |
| HTTP 429 or 5xx | transient result; frontend may retain recent success |
| TOTAL row: input 100, read 10, write 20 | fresh input 70 |
| LEGACY Codex row: input 100, read 10, write 20 | fresh input 90 |
| FRESH row: input 100 | fresh input 100 |

## 5. Good / Base / Bad Cases

- Good: a transient reject keeps the frontend value briefly but removes an indefinitely stale tray suffix.
- Base: successful quota results update frontend events, cache, and tray normally.
- Bad: marking TOTAL rows as LEGACY, or fixing rollup while leaving historical cost backfill on the old formula.

## 6. Tests Required

- Frontend: 429, rejected query, 10-minute boundary, first failure, deterministic failure, and account scope reset.
- Rust transport: local HTTP listener covers send/read `Err` and HTTP/auth `Ok(success=false)`.
- Accounting: nested cache-write parser, cost calculator, TOTAL logger, TOTAL-to-FRESH rollup, and LEGACY/TOTAL backfill distinction.
- Run `pnpm typecheck`, Clippy with `-D warnings`, and the full Rust suite.

## 7. Wrong vs Correct

Wrong: `input_tokens - cache_read_tokens` in every Codex cost path.

Correct: branch on `input_token_semantics`; TOTAL subtracts read and write, LEGACY preserves the old read-only deduction, and FRESH subtracts neither.
