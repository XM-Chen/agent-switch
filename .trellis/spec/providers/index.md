# Provider Specifications

Contracts for provider forms, persisted settings, live configuration, and proxy takeover.

## Pre-Development Checklist

- Read [Model Settings](model-settings.md) before adding Claude role fields or Codex top-level model controls.
- Read [Proxy Protocol Bridges](proxy-bridges.md) before changing Codex Responses/Anthropic/Chat routing or client identity behavior.
- Read [Gateway Takeover](gateway-takeover.md) before changing proxy_config migrations, takeover lifecycle, gateway start/stop, route_mode, snapshot recovery, or proxy status IPC.
- Trace provider-specific values through form state, persistence, Common Config extraction, and takeover.

## Quality Check

- Explicit user values must win over inferred defaults.
- Provider-specific fields must not leak through Common Config.
- Add frontend state/TOML tests and Rust live/takeover tests.
- For gateway/takeover changes, verify all seven module states, migration idempotency, startup no-retakeover behavior, snapshot failure retention, and proxy-route stop protection.
