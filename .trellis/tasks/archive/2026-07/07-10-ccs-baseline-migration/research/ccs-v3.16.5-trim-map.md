# CCS v3.16.5 Trim Map: Windows + Simplified Chinese + Claude Code Only

- **Scope**: internal (agent-switch repo, tag v3.16.5)
- **Date**: 2026-07-10
- **Retained Features**: Provider switching, proxy/failover/translation, usage/cost, MCP, Prompts, Skills, Sessions, Deep Link

---

## 1. Apps to Remove

| App | Frontend ID | Backend enum | Proxy adapter |
|-----|-------------|-------------|---------------|
| Codex | `codex` | `AppType::Codex` | `CodexAdapter` |
| Gemini | `gemini` | `AppType::Gemini` | `GeminiAdapter` |
| OpenCode | `opencode` | `AppType::OpenCode` | N/A (fallback) |
| OpenClaw | `openclaw` | `AppType::OpenClaw` | N/A (fallback) |
| Hermes | `hermes` | `AppType::Hermes` | N/A (fallback) |
| Claude Desktop | `claude-desktop` | `AppType::ClaudeDesktop` | shares ClaudeAdapter |

**Retain**: `claude` only (uses `ClaudeAdapter`).

---

## 2. Locale Trim

**Keep**: `src/i18n/locales/zh.json` (2920 lines)
**Delete**: `en.json`, `ja.json`, `zh-TW.json` (all ~2920 lines each)
**Adapt**: `src/i18n/index.ts` - hardcode `zh`, remove language detection logic

---

## 3. Platform Trim

**Target**: Windows only.

### 3.1 Backend (src-tauri/)
- DELETE: `src-tauri/src/linux_fix.rs` + its `#[cfg(target_os = "linux")]` import
- ADAPT: `src-tauri/src/tray.rs` - remove macOS tray icon templates
- DELETE: `src-tauri/icons/ios/`, `src-tauri/icons/android/`, `src-tauri/icons/tray/macos/`
- DELETE: `src-tauri/Info.plist` (macOS only)

### 3.2 Frontend
- ADAPT: `src/lib/platform.ts` - simplify (only isWindows matters)

### 3.3 CI / Build
- DELETE: `.github/workflows/release.yml` matrix entries for ubuntu, macos
- DELETE: `flatpak/` directory (Linux Flatpak packaging)

### 3.4 Docs
- DELETE: `README_DE.md`, `README_JA.md`
- ADAPT: `README_ZH.md` -> rename to `README.md`
- DELETE: docs with `-en.md` and `-ja.md` suffixes

---

## 4. Frontend Deletion Map

### 4.1 Whole directories to delete
| Directory | Files | Reason |
|---|---|---|
| `src/components/openclaw/` | 5 | OpenClaw env/tools/agents UI |
| `src/components/hermes/` | 1 | HermesMemoryPanel |
| `src/components/universal/` | 4 | Cross-app universal providers |

### 4.2 Provider form files to delete (28 files)

**Components** (16):
- `providers/forms/Codex{FormFields,CommonConfigModal,ConfigEditor,ConfigSections,OAuthSection}.tsx`
- `providers/forms/Gemini{FormFields,CommonConfigModal,ConfigEditor,ConfigSections}.tsx`
- `providers/forms/OpenCodeFormFields.tsx`, `OpenClawFormFields.tsx`, `OmoFormFields.tsx`
- `providers/forms/HermesFormFields.tsx`, `CopilotAuthSection.tsx`
- `providers/forms/ClaudeDesktopProviderForm.tsx`
- `providers/forms/helpers/opencodeFormUtils.ts`

**Hooks** (12):
- `hooks/useCodex{CommonConfig,ConfigState,Oauth,TomlValidation}.ts`
- `hooks/useGemini{CommonConfig,ConfigState}.ts`
- `hooks/use{CopilotAuth,HermesFormState,OmoDraftState,OmoModelSource,OpenclawFormState,OpencodeFormState}.ts`

### 4.3 Individual files to delete
| File | Reason |
|---|---|
| `CodexOauthQuotaFooter.tsx` | Codex OAuth |
| `CopilotQuotaFooter.tsx` | Copilot (Codex) |
| `SubscriptionQuotaFooter.tsx` | Codex subscription |
| `proxy/ClaudeDesktopRouteToggle.tsx` | Claude Desktop 3P route |
| `settings/CodexAuthSettings.tsx` | Codex auth |
| `settings/AppVisibilitySettings.tsx` | Multi-app toggle |

### 4.4 Hooks to delete (src/hooks/)
- `useHermes.ts`, `useOpenClaw.ts`

### 4.5 Config presets to delete (src/config/)
- `codexProviderPresets.ts`, `codexTemplates.ts`
- `geminiProviderPresets.ts`, `hermesProviderPresets.ts`
- `openclawProviderPresets.ts`, `opencodeProviderPresets.ts`
- `claudeDesktopProviderPresets.ts`, `universalProviderPresets.ts`

### 4.6 API layer (src/lib/api/)
- DELETE: `copilot.ts`, `vscode.ts`, `openclaw.ts`, `hermes.ts`, `subscription.ts`
- ADAPT: `types.ts` - remove non-claude AppId variants

### 4.7 Types (src/types.ts + src/types/)
- DELETE: `src/types/omo.ts`, `src/types/subscription.ts`
- ADAPT: main `types.ts` - strip Codex/Gemini/OpenCode/OpenClaw/Hermes types
- Remove: `VisibleApps`, `CodexChat*`, `OpenCode*`, `OpenClaw*`, `Hermes*`, `UniversalProvider*`

### 4.8 App.tsx changes
- Strip `VALID_APPS` to `["claude"]`
- Remove Views: `openclawEnv`, `openclawTools`, `openclawAgents`, `hermesMemory`, `universal`
- Remove workspace view if OpenClaw-only

---

## 5. Backend Deletion Map (src-tauri/src/)

### 5.1 Config modules to delete
| File | Reason |
|---|---|
| `codex_config.rs` | Codex config reader/writer |
| `codex_history_migration.rs` | Codex session history migration |
| `gemini_config.rs` | Gemini config reader/writer |
| `gemini_mcp.rs` | Gemini MCP sync |
| `opencode_config.rs` | OpenCode config reader/writer |
| `openclaw_config.rs` | OpenClaw config reader/writer |
| `hermes_config.rs` | Hermes config reader/writer |
| `claude_desktop_config.rs` | Claude Desktop 3P config |
| `linux_fix.rs` | Linux-only UI fix |

### 5.2 Commands：按能力归属裁剪

- **DELETE**：仅由被删除客户端使用的命令，如 `hermes.rs`、`openclaw.rs`、`omo.rs`、`subscription.rs` 中无 Claude 引用的部分（实施时再按引用闭包确认）。
- **KEEP**：`commands/codex_oauth.rs`、`commands/copilot.rs`。它们服务于 Claude Provider 的 ChatGPT Codex OAuth / GitHub Copilot 托管账号上游，不属于独立 Codex 客户端专属代码。


### 5.3 Services to delete (services/)
- `codex_oauth_models.rs`, `omo.rs`
- `session_usage_codex.rs`, `session_usage_gemini.rs`, `session_usage_opencode.rs`

### 5.4 Proxy modules to delete (proxy/providers/)
- `codex.rs`, `codex_chat_common.rs`, `codex_chat_history.rs`, `codex_oauth_auth.rs`
- `copilot_auth.rs`, `copilot_model_map.rs`
- `gemini.rs`, `gemini_schema.rs`, `gemini_shadow.rs`
- `streaming_codex_chat.rs`, `streaming_gemini.rs`
- `transform_codex_chat.rs`, `transform_gemini.rs`

**KEEP** (used by Claude apiFormat translation):
- `streaming_responses.rs`, `transform_responses.rs` (openai_responses format for Claude)

### 5.5 Session manager providers to delete
- `codex.rs`, `gemini.rs`, `opencode.rs`, `openclaw.rs`, `hermes.rs`
- **KEEP**: `claude.rs`, `mod.rs`, `utils.rs`

### 5.6 MCP modules to delete (mcp/)
- `codex.rs`, `gemini.rs`, `opencode.rs`, `hermes.rs`
- **KEEP**: `claude.rs`, `mod.rs`, `validation.rs`

---

## 6. Database Schema

Schema version: 11. Tables with multi-app columns:

| Table | Multi-app mechanism | Trim action |
|---|---|---|
| `providers` | `app_type` column in PK | Keep structure, only store claude rows |
| `prompts` | `app_type` column in PK | Same |
| `proxy_config` | `app_type` PK with CHECK | Adapt CHECK to claude-only |
| `mcp_servers` | `enabled_claude/codex/gemini/opencode/hermes` | Drop or ignore non-claude cols |
| `skills` | `enabled_claude/codex/gemini/opencode/hermes` | Same |
| `provider_health` | `app_type` column | Keep |
| `proxy_request_logs` | `app_type` column | Keep |
| `stream_check_logs` | `app_type` column | Keep |

**Recommendation**: Leave schema intact initially. No migration needed. Ignore non-claude values in code.

---

## 7. Hidden Cross-App Dependencies

| Dependency | Location | Risk |
|---|---|---|
| `ProviderType` enum | `proxy/providers/mod.rs` | Has Codex/Gemini/Copilot/CodexOAuth variants. **KEEP** CodexOAuth+GitHubCopilot+OpenRouter (used by Claude providers routing through these APIs) |
| `get_adapter()` | `proxy/providers/mod.rs` | Dispatches by AppType; simplify to Claude-only but keep ProviderType logic |
| Provider presets seed | `database/dao/providers_seed.rs` | May seed defaults for all apps |
| Import/Export | `commands/import_export.rs` | Multi-app JSON; adapt |
| DeepLink parser | `deeplink/` | May handle Codex/Gemini deep links |
| `AppSwitcher` component | `src/components/AppSwitcher.tsx` | Tab bar; DELETE or reduce to single-app |
| `postChangeSync` | `src/utils/postChangeSync.ts` | Syncs to multiple live apps; simplify |

---

## 8. Tests Impacted

**Delete entirely** (~10 files):
- `tests/config/codex*.test.ts`, `tests/config/opencodeProviderPresets.test.ts`
- `tests/components/OmoFormFields.*.test.ts`
- `tests/hooks/useCodexConfigState.catalog.test.ts`
- `tests/components/ProviderForm.codexCatalog.test.ts`

**Adapt** (~5 files):
- `tests/hooks/useSettings.test.tsx` - VisibleApps references
- `tests/integration/App.test.tsx` - multi-app navigation
- `tests/components/ProviderList.test.tsx` - may pass non-claude app

---

## 9. Recommended Deletion Order

1. **Locale files** (safe, no imports): en.json, ja.json, zh-TW.json
2. **Platform assets** (safe, no code refs): iOS/Android/macOS icons, flatpak/
3. **Frontend leaf components** (minimal importers): openclaw/, hermes/, universal/ dirs
4. **Provider form files** (only imported by ProviderForm.tsx)
5. **Config preset files** (leaf modules)
6. **Hooks** (useHermes, useOpenClaw)
7. **API layer files** (copilot, vscode, openclaw, hermes, subscription)
8. **App.tsx + AppSwitcher** (strip VALID_APPS, Views)
9. **Types** (strip from types.ts)
10. **Backend commands** (codex_oauth, copilot, hermes, openclaw, omo)
11. **Backend services** (session_usage_codex/gemini/opencode, omo, codex_oauth_models)
12. **Backend configs** (codex/gemini/opencode/openclaw/hermes/claude_desktop _config)
13. **Proxy adapters** (codex, gemini provider files + streaming/transform)
14. **Session manager providers** (all except claude)
15. **MCP modules** (codex, gemini, opencode, hermes)
16. **lib.rs + commands/mod.rs** (remove deleted module declarations)
17. **Database** (adapt proxy_config CHECK, optionally drop columns)
18. **Tests** (delete/adapt)
19. **CI/Docs** (release.yml, README cleanup)

---

## 10. Key Safety Notes

- **Proxy ProviderType::CodexOAuth and GitHubCopilot**: Used by Claude providers routing through Copilot/Codex OAuth. DO NOT remove from enum.
- **streaming_responses.rs / transform_responses.rs**: Used when Claude providers have `apiFormat: "openai_responses"`. KEEP.
- **Icons** (`src/icons/extracted/`): ~110 provider brand icons. Content-neutral. Keep all.
- **Database schema**: Leave at version 11. No need for new migration if unused columns remain.
- **Total estimated deletions**: ~130 files, ~25k-30k lines removed.
