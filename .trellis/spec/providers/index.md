# Provider Specifications

Contracts for provider forms, persisted settings, live configuration, and proxy takeover.

## Pre-Development Checklist

- Read [Model Settings](model-settings.md) before adding Claude role fields or Codex top-level model controls.
- Read [Proxy Protocol Bridges](proxy-bridges.md) before changing Codex Responses/Anthropic/Chat routing or client identity behavior.
- Read [Gateway Takeover](gateway-takeover.md) before changing proxy_config migrations, takeover lifecycle, gateway start/stop, route_mode, snapshot recovery, proxy status IPC, external-config detection/conflicts, managed-write suppression, or the frontend gateway/takeover/route_mode + conflict-dialog consumption.
- Trace provider-specific values through form state, persistence, Common Config extraction, and takeover.

## Quality Check

- Explicit user values must win over inferred defaults.
- Provider-specific fields must not leak through Common Config.
- Add frontend state/TOML tests and Rust live/takeover tests.
- For gateway/takeover changes, verify all seven module states, migration idempotency, startup no-retakeover behavior, snapshot failure retention, and proxy-route stop protection.
- For external-config / conflict work, verify hands-off no reverse write, immutable snapshot separation, generation/stale errors, accept/reject atomicity, managed-write suppression, worker start-once/stop-join, and OpenCode never touches `opencode.db`.
- For frontend gateway/takeover consumption, verify the three controls stay orthogonal (gateway switch never toggles takeover), the seven-module region renders regardless of gateway run state, targeted per-appType invalidation (no global thrash), single event subscription, and the blocking conflict dialog only clears after backend success.
