# Proxy Protocol Bridges

## 1. Scope / Trigger

Use this contract when adding or changing Codex Responses bridges, especially native Anthropic Messages upstreams, streaming tool/reasoning conversion, or provider identity headers.

## 2. Signatures

- Provider meta: `apiFormat?: "openai_responses" | "openai_chat" | "anthropic"`.
- Anthropic auth selector: `apiKeyField?: "ANTHROPIC_AUTH_TOKEN" | "ANTHROPIC_API_KEY"`.
- Output override: `maxOutputTokens?: number`, persisted only when positive.
- Request conversion: `responses_request_to_anthropic(body, default_max_tokens) -> Result<Value, ProxyError>`.
- Response conversion: Anthropic JSON/SSE must return Codex Responses JSON/SSE, including usage and terminal errors.

## 3. Contracts

- Route only `/responses`, `/v1/responses`, and compact variants when the provider explicitly declares an Anthropic alias in meta/settings/TOML. Never infer from host.
- Rewrite the upstream endpoint to `/v1/messages`, preserve query parameters, and do not double-append a pasted full endpoint.
- `ANTHROPIC_AUTH_TOKEN` sends only `Authorization: Bearer`; `ANTHROPIC_API_KEY` sends only `x-api-key`.
- Explicit provider model and positive `maxOutputTokens` win. Default Anthropic output is 8192.
- Preserve `[1m]` through catalog selection, strip it from the final upstream model, and add the context beta.
- Bridge caching is enabled by default but honors `cache_injection` and configured TTL.
- The bridge never owns a separate impersonation flag. L1 headers use `ClaudeClientProfileConfig.enabled`; L2 body identity additionally requires `body_identity`. Both exclude Copilot.
- Strip incoming Codex/OpenAI identity headers. Add protocol-required `anthropic-version` and JSON Accept independently of identity simulation.
- Keep AGS names, port `42567`, and `agent-switch-model-catalog.json`.

## 4. Validation & Error Matrix

| Condition | Behavior |
|---|---|
| Unknown/implicit Anthropic gateway | Do not route through the bridge |
| Unsupported client history/tool arguments | Return `InvalidRequest`; do not fail over |
| Anthropic HTTP 2xx error envelope | Convert to retryable transform failure before success accounting |
| Responses `failed`/`cancelled` envelope | Surface a failure, never an empty success |
| Truncated stream or terminal error | Emit a failed/incomplete terminal, not `response.completed` |
| L1 disabled | No Claude UA, x-app, stainless, or claude-code beta injection |
| L1 enabled, L2 disabled | Normalize headers only; do not modify body identity |
| L1 and L2 enabled | Normalize headers and apply the shared body identity helper |

## 5. Good / Base / Bad Cases

- Good: an explicit Anthropic provider receives one auth scheme, stable tool IDs/order, signed reasoning round-trips, and AGS-controlled identity.
- Base: native Responses and Chat providers retain their existing routes and payloads.
- Bad: a bridge-local `impersonateClaudeCode` field, hard-coded UA/system prompt, generated UUID cache key, `15721`, or extra locale changes.

## 6. Tests Required

- Format/TOML/path/catalog/auth tests in `providers::codex`.
- Request, JSON response, SSE, tool/media/reasoning/usage/error tests in the four bridge modules.
- Forwarder tests for endpoint rewrite, full URL guard, 2xx semantic errors, cache sub-switch, fingerprint filtering, and L1/L2 four-state guards.
- Handler tests for JSON, SSE, mislabeled content type, compact, fallback, and usage logging.
- `pnpm typecheck`, provider action/config tests, Clippy with `-D warnings`, and full Rust/frontend suites.

## 7. Wrong vs Correct

Wrong: enable Claude identity merely because `apiFormat == "anthropic"`, or copy a CCS fixed client identity into the bridge.

Correct: compute native Anthropic routing separately, then call the shared L1/L2 guards and `cc_client_profile` helpers only when the AGS settings enable them.
