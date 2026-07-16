# Coding Plan Quotas

## 1. Scope / Trigger

Use this contract when adding or changing a Coding Plan provider, credential field, quota endpoint, or `get_coding_plan_quota` caller.

## 2. Signatures

Frontend and Tauri command arguments use camelCase over IPC:

```text
get_coding_plan_quota(baseUrl, apiKey, accessKeyId?, secretAccessKey?,
                      codingPlanProvider?, teamOrganizationId?, teamProjectId?)
```

The Rust service receives the same optional values as `Option<&str>`. Persisted `UsageScript` fields are `teamOrganizationId` and `teamProjectId`; both must remain optional for old provider records.

## 3. Contracts

- `zhipu` remains before `zhipu_team` in `CODING_PLAN_PROVIDERS`, so base-URL auto-detection continues to select the personal plan.
- Team mode is entered only when `codingPlanProvider` equals `zhipu_team` case-insensitively.
- Team requests use `GET https://open.bigmodel.cn/api/monitor/usage/quota/limit?type=2` with `Authorization`, `bigmodel-organization`, and `bigmodel-project` headers.
- Personal and team responses share `zhipu_quota_from_body` and produce the existing `SubscriptionQuota` shape.

## 4. Validation & Error Matrix

| Condition | Result |
|---|---|
| Team API key, organization ID, or project ID is blank | `Ok(SubscriptionQuota)` with `credential_status=not_found` and `success=false` |
| HTTP 401/403 | `credential_status=expired` and `success=false` |
| Other non-2xx or invalid response | `success=false` with the existing Coding Plan error format |
| Valid response | Normalized tiers with `credential_status=valid` |

## 5. Good / Base / Bad Cases

- Good: explicit `zhipu_team` plus all three credentials sends the team headers and returns normalized tiers.
- Base: an existing personal `zhipu` record has no team fields and continues through personal detection.
- Bad: inferring team mode from `open.bigmodel.cn`; personal and team plans share that host.

## 6. Tests Required

- Missing team credentials return `not_found` before network access.
- A local HTTP server asserts `?type=2` and all three required headers.
- The returned body is parsed through the shared Zhipu tier parser.
- Frontend type-check and Rust Clippy pass.

## 7. Wrong vs Correct

Wrong: add `zhipu_team` before `zhipu` and let the shared host auto-select it.

Correct: keep personal detection first and require the explicit `codingPlanProvider: "zhipu_team"` marker for team queries.
