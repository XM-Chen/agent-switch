//! 从 Agent Switch 自身旧表到独立网关影子域的只读、幂等迁移。
//!
//! 本模块只接收已经打开的 SQLite 连接，不解析或访问任何客户端文件。旧表不删、
//! 不改、不双写；所有无法证明等价的数据都写入 gateway_migration_report。

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::str::FromStr;

use crate::app_config::AppType;
use crate::error::AppError;
use crate::provider::Provider;
use crate::proxy::providers::{
    get_adapter_for, module_canonical_protocol, ModuleProtocol, ProviderType,
};
use crate::services::credential_protector::{CredentialProtector, PlatformCredentialProtector};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const CANONICAL_APPS: &[&str] = &[
    "claude",
    "claude-desktop",
    "codex",
    "gemini",
    "opencode",
    "openclaw",
    "hermes",
];

#[derive(Debug)]
struct LegacyProviderRow {
    id: String,
    app_type: String,
    name: String,
    settings_config_raw: String,
    notes: Option<String>,
    created_at: Option<i64>,
    sort_index: Option<i64>,
    meta_raw: String,
    in_failover_queue: bool,
}

#[derive(Debug)]
struct LegacyModelRow {
    provider_id: String,
    app_type: String,
    model_id: String,
    source: String,
    owned_by: Option<String>,
    fetched_at: i64,
}

#[derive(Debug)]
struct LegacyAggregateRow {
    id: String,
    app_type: String,
    name: String,
    ordered_members_raw: String,
    created_at: Option<i64>,
    updated_at: Option<i64>,
}

#[derive(Debug, Clone)]
struct ExtractedProvider {
    upstream_id: String,
    protocol: String,
    adapter_type: String,
    base_url: Option<String>,
    credential: Option<(String, String)>,
    config_json: Value,
    ambiguous: bool,
}

pub(crate) fn migrate(conn: &Connection) -> Result<(), AppError> {
    let protector = PlatformCredentialProtector;
    migrate_with_protector(conn, &protector)
}

/// 在 v18 purge 前，将早期 v17 已写入的泛型 `api_key` 凭据按旧 DB 原文精确重分类。
///
/// 只读取 Agent Switch 自有 `providers` 表；无法精确分类的行保持不变，由 readiness
/// fail-closed 阻止 purge。已由用户录入的精确类型凭据绝不被旧值覆盖。
pub(crate) fn reclassify_v17_credentials(conn: &Connection) -> Result<(), AppError> {
    let protector = PlatformCredentialProtector;
    reclassify_v17_credentials_with_protector(conn, &protector)
}

fn reclassify_v17_credentials_with_protector(
    conn: &Connection,
    protector: &dyn CredentialProtector,
) -> Result<(), AppError> {
    if !legacy_domain_available(conn)? {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp_millis();
    for provider in read_legacy_providers(conn)? {
        let Some(app_type) = canonical_app_type(&provider.app_type) else {
            continue;
        };
        let upstream_id = stable_id("upstream", &[&app_type, &provider.id]);
        let Ok(extracted) = extract_provider(&provider, &app_type, &upstream_id) else {
            continue;
        };
        let Some((kind, plaintext)) = extracted.credential else {
            continue;
        };
        if !crate::gateway::credential::is_ready_kind(&kind) {
            continue;
        }
        let legacy_exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM upstream_credentials
                    WHERE upstream_id = ?1 AND credential_kind = ?2
                 )",
                params![upstream_id, crate::gateway::credential::LEGACY_API_KEY],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| AppError::Database(format!("检查 v17 泛型凭据失败: {e}")))?
            != 0;
        if !legacy_exists {
            continue;
        }
        let encrypted = protector.protect(plaintext.as_bytes()).map_err(|_| {
            AppError::Config(format!(
                "v18 净化被阻止：无法重新保护旧上游 {upstream_id} 的精确类型凭据"
            ))
        })?;
        conn.execute("SAVEPOINT reclassify_v17_credential", [])
            .map_err(|e| AppError::Database(format!("开启 v17 凭据重分类失败: {e}")))?;
        let result = (|| {
            let credential_id = stable_id("credential", &[&upstream_id, &kind]);
            let precise_exists: bool = conn
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM upstream_credentials
                        WHERE upstream_id = ?1 AND credential_kind = ?2
                     )",
                    params![upstream_id, kind],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| AppError::Database(format!("检查 v17 精确凭据失败: {e}")))?
                != 0;
            if precise_exists {
                conn.execute(
                    "DELETE FROM upstream_credentials
                     WHERE upstream_id = ?1 AND credential_kind = ?2",
                    params![upstream_id, crate::gateway::credential::LEGACY_API_KEY],
                )
                .map_err(|e| AppError::Database(format!("删除冗余 v17 泛型凭据失败: {e}")))?;
            } else {
                conn.execute(
                    "UPDATE upstream_credentials
                     SET id = ?1, credential_kind = ?2, encrypted_payload = ?3,
                         encryption_scheme = ?4, key_hint = ?5, updated_at = ?6
                     WHERE upstream_id = ?7 AND credential_kind = ?8",
                    params![
                        credential_id,
                        kind,
                        encrypted,
                        protector.scheme(),
                        credential_hint(&plaintext),
                        now,
                        upstream_id,
                        crate::gateway::credential::LEGACY_API_KEY,
                    ],
                )
                .map_err(|e| AppError::Database(format!("重分类 v17 泛型凭据失败: {e}")))?;
            }
            Ok::<(), AppError>(())
        })();
        match result {
            Ok(()) => {
                conn.execute("RELEASE reclassify_v17_credential", [])
                    .map_err(|e| AppError::Database(format!("提交 v17 凭据重分类失败: {e}")))?;
            }
            Err(error) => {
                let _ = conn.execute("ROLLBACK TO reclassify_v17_credential", []);
                let _ = conn.execute("RELEASE reclassify_v17_credential", []);
                return Err(error);
            }
        }
    }
    Ok(())
}

fn legacy_domain_available(conn: &Connection) -> Result<bool, AppError> {
    if !table_exists(conn, "providers")? {
        return Ok(false);
    }
    for column in [
        "id",
        "app_type",
        "name",
        "settings_config",
        "notes",
        "created_at",
        "sort_index",
        "meta",
        "in_failover_queue",
        "is_current",
    ] {
        if !column_exists(conn, "providers", column)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn migrate_with_protector(
    conn: &Connection,
    protector: &dyn CredentialProtector,
) -> Result<(), AppError> {
    if !legacy_domain_available(conn)? {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp_millis();
    let providers = read_legacy_providers(conn)?;
    let models = read_legacy_models(conn)?;
    let aggregates = read_legacy_aggregates(conn)?;

    report_unknown_app_types(conn, &providers, &models, &aggregates, now)?;
    report_multiple_current_providers(conn, now)?;
    report_legacy_failover_table(conn, now)?;
    report_legacy_column_conflicts(conn, now)?;
    report_endpoint_anomalies(conn, now)?;

    let legacy_provider_keys: HashSet<(String, String)> = providers
        .iter()
        .map(|provider| (provider.app_type.clone(), provider.id.clone()))
        .collect();

    let model_groups = group_models(&models);
    let mut migrated_upstreams = HashMap::<(String, String), String>::new();
    let mut migrated_upstream_enabled = HashMap::<(String, String), bool>::new();

    for provider in &providers {
        let Some(app_type) = canonical_app_type(&provider.app_type) else {
            continue;
        };
        let upstream_id = stable_id("upstream", &[&app_type, &provider.id]);
        let extracted = match extract_provider(provider, &app_type, &upstream_id) {
            Ok(extracted) => extracted,
            Err((code, details)) => {
                report_issue(
                    conn,
                    "error",
                    "provider",
                    Some(&app_type),
                    Some(&provider.id),
                    code,
                    details,
                    now,
                )?;
                continue;
            }
        };

        if provider.sort_index.is_some_and(|value| value < 0) {
            report_issue(
                conn,
                "warning",
                "provider",
                Some(&app_type),
                Some(&provider.id),
                "negative_sort_index",
                json!({"sortIndex": provider.sort_index}),
                now,
            )?;
        }
        if extracted.ambiguous {
            report_issue(
                conn,
                "warning",
                "provider",
                Some(&app_type),
                Some(&provider.id),
                "provider_requires_review",
                json!({
                    "protocol": extracted.protocol,
                    "hasBaseUrl": extracted.base_url.is_some(),
                    "hasCredential": extracted.credential.is_some()
                }),
                now,
            )?;
        }

        let mut upstream_enabled = (!extracted.ambiguous
            || (extracted.protocol != "unknown" && extracted.base_url.is_some()))
            && extracted.credential.is_some();
        conn.execute(
            "INSERT OR IGNORE INTO upstreams
                (id, name, enabled, base_url, protocol, adapter_type, config_json, notes,
                 legacy_app_type, legacy_provider_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                extracted.upstream_id,
                provider.name,
                i64::from(upstream_enabled),
                extracted.base_url,
                extracted.protocol,
                extracted.adapter_type,
                serde_json::to_string(&extracted.config_json)
                    .map_err(|e| AppError::Database(format!("序列化上游配置失败: {e}")))?,
                provider.notes,
                app_type,
                provider.id,
                provider.created_at.unwrap_or(now),
                now,
            ],
        )
        .map_err(|e| AppError::Database(format!("迁移上游 {} 失败: {e}", provider.id)))?;

        if let Some((kind, plaintext)) = extracted.credential.as_ref() {
            match protector.protect(plaintext.as_bytes()) {
                Ok(encrypted) => {
                    let credential_id = stable_id("credential", &[&upstream_id, kind]);
                    conn.execute(
                        "INSERT OR IGNORE INTO upstream_credentials
                            (id, upstream_id, credential_kind, encrypted_payload,
                             encryption_scheme, key_hint, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        params![
                            credential_id,
                            upstream_id,
                            kind,
                            encrypted,
                            protector.scheme(),
                            credential_hint(plaintext),
                            now,
                        ],
                    )
                    .map_err(|e| {
                        AppError::Database(format!("保存上游 {} 的加密凭据失败: {e}", provider.id))
                    })?;
                }
                Err(error) => {
                    upstream_enabled = false;
                    conn.execute(
                        "UPDATE upstreams SET enabled = 0, updated_at = ?1 WHERE id = ?2",
                        params![now, upstream_id],
                    )
                    .map_err(|e| {
                        AppError::Database(format!(
                            "禁用凭据保护失败的上游 {} 失败: {e}",
                            provider.id
                        ))
                    })?;
                    report_issue(
                        conn,
                        "error",
                        "credential",
                        Some(&app_type),
                        Some(&provider.id),
                        "credential_protection_unavailable",
                        json!({"message": error.to_string()}),
                        now,
                    )?;
                }
            }
        }

        migrated_upstreams.insert((app_type, provider.id.clone()), upstream_id);
        migrated_upstream_enabled.insert(
            (provider.app_type.clone(), provider.id.clone()),
            upstream_enabled,
        );
    }

    migrate_upstream_models(
        conn,
        &models,
        &legacy_provider_keys,
        &migrated_upstreams,
        now,
    )?;
    migrate_exact_gateway_models_and_routes(
        conn,
        &models,
        &providers,
        &migrated_upstreams,
        &migrated_upstream_enabled,
        now,
    )?;
    migrate_custom_aggregate_drafts(
        conn,
        &aggregates,
        &providers,
        &model_groups,
        &migrated_upstreams,
        now,
    )?;
    report_aggregate_settings(conn, &aggregates, &model_groups, now)?;

    Ok(())
}

fn read_legacy_providers(conn: &Connection) -> Result<Vec<LegacyProviderRow>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, app_type, name, settings_config, notes, created_at,
                    sort_index, meta, in_failover_queue
             FROM providers ORDER BY app_type ASC, id ASC",
        )
        .map_err(|e| AppError::Database(format!("准备读取旧 Provider 失败: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(LegacyProviderRow {
                id: row.get(0)?,
                app_type: row.get(1)?,
                name: row.get(2)?,
                settings_config_raw: row.get(3)?,
                notes: row.get(4)?,
                created_at: row.get(5)?,
                sort_index: row.get(6)?,
                meta_raw: row.get(7)?,
                in_failover_queue: row.get::<_, i64>(8)? != 0,
            })
        })
        .map_err(|e| AppError::Database(format!("读取旧 Provider 失败: {e}")))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(format!("解析旧 Provider 失败: {e}")))
}

fn read_legacy_models(conn: &Connection) -> Result<Vec<LegacyModelRow>, AppError> {
    if !table_exists(conn, "provider_models")? {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT provider_id, app_type, model_id, source, owned_by, fetched_at
             FROM provider_models ORDER BY app_type ASC, provider_id ASC, model_id ASC",
        )
        .map_err(|e| AppError::Database(format!("准备读取旧模型失败: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(LegacyModelRow {
                provider_id: row.get(0)?,
                app_type: row.get(1)?,
                model_id: row.get(2)?,
                source: row.get(3)?,
                owned_by: row.get(4)?,
                fetched_at: row.get(5)?,
            })
        })
        .map_err(|e| AppError::Database(format!("读取旧模型失败: {e}")))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(format!("解析旧模型失败: {e}")))
}

fn read_legacy_aggregates(conn: &Connection) -> Result<Vec<LegacyAggregateRow>, AppError> {
    if !table_exists(conn, "custom_aggregates")? {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, app_type, name, ordered_members, created_at, updated_at
             FROM custom_aggregates
             ORDER BY app_type ASC, COALESCE(sort_index, 999999) ASC, id ASC",
        )
        .map_err(|e| AppError::Database(format!("准备读取旧聚合定义失败: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(LegacyAggregateRow {
                id: row.get(0)?,
                app_type: row.get(1)?,
                name: row.get(2)?,
                ordered_members_raw: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .map_err(|e| AppError::Database(format!("读取旧聚合定义失败: {e}")))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(format!("解析旧聚合定义失败: {e}")))
}

fn extract_provider(
    provider: &LegacyProviderRow,
    app_type: &str,
    upstream_id: &str,
) -> Result<ExtractedProvider, (&'static str, Value)> {
    let settings: Value = serde_json::from_str(&provider.settings_config_raw).map_err(|error| {
        (
            "invalid_settings_json",
            json!({"message": error.to_string()}),
        )
    })?;
    let meta: Value = serde_json::from_str(&provider.meta_raw)
        .map_err(|error| ("invalid_meta_json", json!({"message": error.to_string()})))?;
    if !settings.is_object() {
        return Err((
            "invalid_settings_shape",
            json!({"actualType": json_type(&settings)}),
        ));
    }

    let app = AppType::from_str(app_type)
        .map_err(|error| ("unknown_app_type", json!({"message": error.to_string()})))?;
    let typed_meta = serde_json::from_value(meta.clone())
        .map_err(|error| ("invalid_meta_shape", json!({"message": error.to_string()})))?;
    let typed_provider = Provider {
        id: provider.id.clone(),
        name: provider.name.clone(),
        settings_config: settings.clone(),
        website_url: None,
        category: None,
        created_at: provider.created_at,
        sort_index: provider
            .sort_index
            .and_then(|value| usize::try_from(value).ok()),
        notes: provider.notes.clone(),
        meta: Some(typed_meta),
        icon: None,
        icon_color: None,
        in_failover_queue: provider.in_failover_queue,
    };

    let credential_candidate_count = count_credential_candidates(app_type, &settings);
    let codex_config_had_embedded_secret = app_type == "codex"
        && settings
            .get("config")
            .and_then(Value::as_str)
            .is_some_and(|config| config.contains("experimental_bearer_token"));
    let (usage_base_url, usage_api_key) = typed_provider.resolve_usage_credentials(&app);
    let adapter = get_adapter_for(&app, &typed_provider);
    let base_url = adapter
        .extract_base_url(&typed_provider)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| (!usage_base_url.trim().is_empty()).then(|| usage_base_url.trim().to_string()));
    let dynamic_credential = matches!(
        ProviderType::from_app_type_and_config(&app, &typed_provider),
        ProviderType::GitHubCopilot | ProviderType::CodexOAuth
    );
    let api_key = if dynamic_credential {
        String::new()
    } else {
        adapter
            .extract_auth(&typed_provider)
            .map(|auth| auth.api_key)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(usage_api_key)
    };
    let (protocol, adapter_type) = protocol_for_provider(&app, &typed_provider);
    let credential = (!api_key.trim().is_empty()).then(|| {
        crate::gateway::credential::classify_legacy_kind(
            app_type,
            &protocol,
            &adapter_type,
            &settings,
            api_key.trim(),
        )
        .map(|kind| (kind.to_string(), api_key.trim().to_string()))
    });
    let credential = credential.flatten();
    let mut ambiguous = protocol == "unknown"
        || base_url.is_none()
        || credential_candidate_count > 1
        || credential.is_none();

    let sanitized_settings = remove_known_credentials(app_type, settings);
    let mut sanitized_settings = sanitized_settings;
    redact_sensitive_json(&mut sanitized_settings);
    let sanitized_meta = remove_meta_credentials(meta);
    let mut migration_notes = Vec::new();
    if codex_config_had_embedded_secret {
        migration_notes.push("codex_config_omitted_to_remove_embedded_secret");
    }
    let config_json = json!({
        "legacySettings": sanitized_settings,
        "legacyMeta": sanitized_meta,
        "migrationNotes": migration_notes,
        "legacyIdentity": {
            "appType": app_type,
            "providerId": provider.id,
            "upstreamId": upstream_id,
        }
    });

    if credential.is_none() {
        ambiguous = true;
    }

    Ok(ExtractedProvider {
        upstream_id: upstream_id.to_string(),
        protocol,
        adapter_type,
        base_url,
        credential,
        config_json,
        ambiguous,
    })
}

fn protocol_for_provider(app: &AppType, provider: &Provider) -> (String, String) {
    match app {
        AppType::Claude | AppType::ClaudeDesktop => {
            let protocol = provider
                .meta
                .as_ref()
                .and_then(|meta| meta.api_format.as_deref())
                .unwrap_or("anthropic")
                .to_string();
            (protocol, "claude".to_string())
        }
        AppType::Codex => ("openai_responses".to_string(), "codex".to_string()),
        AppType::Gemini => ("gemini".to_string(), "gemini".to_string()),
        AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => {
            match module_canonical_protocol(app, provider) {
                Some(ModuleProtocol::Anthropic) => {
                    ("anthropic".to_string(), "module_anthropic".to_string())
                }
                Some(ModuleProtocol::OpenAiChat) => {
                    ("openai_chat".to_string(), "module_openai".to_string())
                }
                Some(ModuleProtocol::OpenAiResponses) => {
                    ("openai_responses".to_string(), "module_openai".to_string())
                }
                None => ("unknown".to_string(), "unsupported".to_string()),
            }
        }
    }
}

fn count_credential_candidates(app_type: &str, settings: &Value) -> usize {
    let non_empty = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    };
    let count_keys = |object: Option<&serde_json::Map<String, Value>>, keys: &[&str]| {
        object
            .map(|object| {
                keys.iter()
                    .filter(|key| non_empty(object.get(**key)))
                    .count()
            })
            .unwrap_or(0)
    };
    match app_type {
        "claude" | "claude-desktop" => {
            count_keys(
                settings.get("env").and_then(Value::as_object),
                &[
                    "ANTHROPIC_AUTH_TOKEN",
                    "ANTHROPIC_API_KEY",
                    "OPENROUTER_API_KEY",
                    "OPENAI_API_KEY",
                    "GOOGLE_API_KEY",
                    "GEMINI_API_KEY",
                ],
            ) + usize::from(non_empty(settings.get("apiKey")))
                + usize::from(non_empty(settings.get("api_key")))
        }
        "gemini" => {
            count_keys(
                settings.get("env").and_then(Value::as_object),
                &["GEMINI_API_KEY", "GOOGLE_API_KEY"],
            ) + usize::from(non_empty(settings.get("apiKey")))
                + usize::from(non_empty(settings.get("api_key")))
        }
        "codex" => {
            count_keys(
                settings.get("env").and_then(Value::as_object),
                &["OPENAI_API_KEY"],
            ) + usize::from(non_empty(settings.pointer("/auth/OPENAI_API_KEY")))
                + usize::from(non_empty(settings.get("apiKey")))
                + usize::from(non_empty(settings.get("api_key")))
                + usize::from(non_empty(settings.pointer("/config/apiKey")))
                + usize::from(non_empty(settings.pointer("/config/api_key")))
                + usize::from(
                    settings
                        .get("config")
                        .and_then(Value::as_str)
                        .and_then(crate::codex_config::extract_codex_experimental_bearer_token)
                        .is_some(),
                )
        }
        "opencode" => usize::from(non_empty(settings.pointer("/options/apiKey"))),
        "openclaw" => usize::from(non_empty(settings.get("apiKey"))),
        "hermes" => usize::from(non_empty(settings.get("api_key"))),
        _ => 0,
    }
}

fn remove_known_credentials(app_type: &str, mut settings: Value) -> Value {
    match app_type {
        "claude" | "claude-desktop" | "gemini" => {
            if let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) {
                for key in [
                    "ANTHROPIC_AUTH_TOKEN",
                    "ANTHROPIC_API_KEY",
                    "OPENROUTER_API_KEY",
                    "OPENAI_API_KEY",
                    "GOOGLE_API_KEY",
                    "GEMINI_API_KEY",
                ] {
                    env.remove(key);
                }
            }
        }
        "codex" => {
            if let Some(env) = settings.get_mut("env").and_then(Value::as_object_mut) {
                env.remove("OPENAI_API_KEY");
            }
            if let Some(auth) = settings.get_mut("auth").and_then(Value::as_object_mut) {
                auth.remove("OPENAI_API_KEY");
            }
            if let Some(config) = settings.get("config").and_then(Value::as_str) {
                if config.contains("experimental_bearer_token") {
                    // 无法在这里保证保留 toml_edit 的全部格式；避免将 secret 留在影子配置，
                    // 仅保留可证明不含该字段的 marker，原文仍留在只读旧表用于回滚。
                    settings["config"] = Value::String(
                        "# legacy Codex config retained in v16 providers table".into(),
                    );
                }
            }
        }
        "opencode" => {
            if let Some(options) = settings.get_mut("options").and_then(Value::as_object_mut) {
                options.remove("apiKey");
            }
        }
        "openclaw" => {
            settings
                .as_object_mut()
                .map(|object| object.remove("apiKey"));
        }
        "hermes" => {
            settings
                .as_object_mut()
                .map(|object| object.remove("api_key"));
        }
        _ => {}
    }
    settings
}

fn remove_meta_credentials(mut meta: Value) -> Value {
    redact_sensitive_json(&mut meta);
    meta
}

fn redact_sensitive_json(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                if is_sensitive_json_key(key) {
                    *child = Value::String("[redacted]".to_string());
                } else {
                    redact_sensitive_json(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_sensitive_json(item);
            }
        }
        _ => {}
    }
}

fn is_sensitive_json_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    normalized.ends_with("apikey")
        || normalized.ends_with("authtoken")
        || normalized.ends_with("accesstoken")
        || normalized.ends_with("refreshtoken")
        || normalized.ends_with("clientsecret")
        || normalized.ends_with("secretaccesskey")
        || matches!(normalized.as_str(), "authorization" | "password" | "cookie")
}

fn migrate_upstream_models(
    conn: &Connection,
    models: &[LegacyModelRow],
    providers: &HashSet<(String, String)>,
    upstreams: &HashMap<(String, String), String>,
    now: i64,
) -> Result<(), AppError> {
    for model in models {
        let Some(app_type) = canonical_app_type(&model.app_type) else {
            continue;
        };
        let canonical_key = (app_type.clone(), model.provider_id.clone());
        let legacy_key = (model.app_type.clone(), model.provider_id.clone());
        if !providers.contains(&legacy_key) {
            report_issue(
                conn,
                "error",
                "upstream_model",
                Some(&app_type),
                Some(&model.provider_id),
                "orphan_provider_model",
                json!({"modelId": model.model_id}),
                now,
            )?;
            continue;
        }
        if !matches!(model.source.as_str(), "manual" | "fetched") {
            report_issue(
                conn,
                "error",
                "upstream_model",
                Some(&app_type),
                Some(&model.provider_id),
                "invalid_model_source",
                json!({"modelId": model.model_id, "source": model.source}),
                now,
            )?;
            continue;
        }
        let Some(upstream_id) = upstreams.get(&canonical_key) else {
            continue;
        };
        conn.execute(
            "INSERT OR IGNORE INTO upstream_models
                (upstream_id, model_id, source, owned_by, refreshed_at,
                 legacy_app_type, legacy_provider_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                upstream_id,
                model.model_id,
                model.source,
                model.owned_by,
                model.fetched_at,
                app_type,
                model.provider_id,
            ],
        )
        .map_err(|e| AppError::Database(format!("迁移上游模型失败: {e}")))?;
    }
    Ok(())
}

fn migrate_exact_gateway_models_and_routes(
    conn: &Connection,
    models: &[LegacyModelRow],
    providers: &[LegacyProviderRow],
    upstreams: &HashMap<(String, String), String>,
    upstream_enabled: &HashMap<(String, String), bool>,
    now: i64,
) -> Result<(), AppError> {
    let provider_order = provider_order(providers);
    let mut groups = BTreeMap::<String, Vec<&LegacyModelRow>>::new();
    for model in models {
        if canonical_app_type(&model.app_type).is_some() {
            groups
                .entry(model.model_id.clone())
                .or_default()
                .push(model);
        }
    }

    for (model_id, mut members) in groups {
        members.sort_by_key(|model| {
            let app = canonical_app_type(&model.app_type).unwrap_or_else(|| model.app_type.clone());
            (
                *provider_order
                    .get(&(app.clone(), model.provider_id.clone()))
                    .unwrap_or(&i64::MAX),
                app,
                model.provider_id.clone(),
            )
        });
        let namespaces: BTreeSet<String> = members
            .iter()
            .filter_map(|member| canonical_app_type(&member.app_type))
            .collect();
        let conflict = namespaces.len() > 1;
        let gateway_model_id = stable_id("gateway-model", &["exact", &model_id]);
        let exact_route_count = members
            .iter()
            .filter(|member| {
                canonical_app_type(&member.app_type).is_some_and(|app_type| {
                    upstreams.contains_key(&(app_type.clone(), member.provider_id.clone()))
                        && upstream_enabled
                            .get(&(member.app_type.clone(), member.provider_id.clone()))
                            .copied()
                            .unwrap_or(false)
                })
            })
            .count();
        let active = !conflict && exact_route_count > 0;
        conn.execute(
            "INSERT OR IGNORE INTO gateway_models
                (id, model_id, display_name, enabled, source, migration_status,
                 metadata_json, created_at, updated_at)
             VALUES (?1, ?2, ?2, ?3, 'legacy_model', ?4, ?5, ?6, ?6)",
            params![
                gateway_model_id,
                model_id,
                i64::from(active),
                if conflict {
                    "conflict"
                } else if active {
                    "active"
                } else {
                    "draft"
                },
                serde_json::to_string(&json!({"legacyNamespaces": namespaces}))
                    .map_err(|e| AppError::Database(format!("序列化网关模型元数据失败: {e}")))?,
                now,
            ],
        )
        .map_err(|e| AppError::Database(format!("迁移网关模型失败: {e}")))?;

        if conflict {
            report_issue(
                conn,
                "warning",
                "gateway_model",
                None,
                Some(&model_id),
                "cross_namespace_model_conflict",
                json!({"namespaces": namespaces}),
                now,
            )?;
        }

        for (position, member) in members.into_iter().enumerate() {
            let Some(app_type) = canonical_app_type(&member.app_type) else {
                continue;
            };
            let Some(upstream_id) = upstreams.get(&(app_type.clone(), member.provider_id.clone()))
            else {
                continue;
            };
            let route_id = stable_id(
                "route-target",
                &[&gateway_model_id, upstream_id, &member.model_id],
            );
            let route_enabled = active
                && upstream_enabled
                    .get(&(member.app_type.clone(), member.provider_id.clone()))
                    .copied()
                    .unwrap_or(false);
            conn.execute(
                "INSERT OR IGNORE INTO route_targets
                    (id, gateway_model_id, upstream_id, target_model, position, enabled,
                     legacy_app_type, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    route_id,
                    gateway_model_id,
                    upstream_id,
                    member.model_id,
                    position as i64,
                    i64::from(route_enabled),
                    app_type,
                    now,
                ],
            )
            .map_err(|e| AppError::Database(format!("迁移精确模型路由候选失败: {e}")))?;
        }
    }
    Ok(())
}

fn migrate_custom_aggregate_drafts(
    conn: &Connection,
    aggregates: &[LegacyAggregateRow],
    providers: &[LegacyProviderRow],
    models: &HashMap<(String, String), Vec<&LegacyModelRow>>,
    upstreams: &HashMap<(String, String), String>,
    now: i64,
) -> Result<(), AppError> {
    let provider_order = provider_order(providers);
    for aggregate in aggregates {
        let Some(app_type) = canonical_app_type(&aggregate.app_type) else {
            continue;
        };
        let members: Vec<String> = match serde_json::from_str(&aggregate.ordered_members_raw) {
            Ok(members) => members,
            Err(error) => {
                report_issue(
                    conn,
                    "error",
                    "custom_aggregate",
                    Some(&app_type),
                    Some(&aggregate.id),
                    "invalid_ordered_members",
                    json!({"message": error.to_string()}),
                    now,
                )?;
                continue;
            }
        };
        let gateway_model_id = stable_id("gateway-model", &["aggregate", &app_type, &aggregate.id]);
        let created_at = aggregate.created_at.unwrap_or(now);
        conn.execute(
            "INSERT OR IGNORE INTO gateway_models
                (id, model_id, display_name, enabled, source, migration_status,
                 legacy_app_type, legacy_source_id, metadata_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, 0, 'legacy_aggregate', 'draft', ?4, ?5, ?6, ?7, ?8)",
            params![
                gateway_model_id,
                format!("legacy-aggregate:{}:{}", app_type, aggregate.id),
                aggregate.name,
                app_type,
                aggregate.id,
                serde_json::to_string(&json!({"orderedMembers": members}))
                    .map_err(|e| AppError::Database(format!("序列化聚合草稿失败: {e}")))?,
                created_at,
                aggregate.updated_at.unwrap_or(created_at),
            ],
        )
        .map_err(|e| AppError::Database(format!("迁移自定义聚合草稿失败: {e}")))?;

        let mut seen = HashSet::<(String, String)>::new();
        let mut position = 0_i64;
        for member_key in &members {
            let Some(candidates) = models.get(&(app_type.clone(), member_key.to_lowercase()))
            else {
                report_issue(
                    conn,
                    "warning",
                    "custom_aggregate",
                    Some(&app_type),
                    Some(&aggregate.id),
                    "missing_aggregate_member",
                    json!({"member": member_key}),
                    now,
                )?;
                continue;
            };
            let mut ordered = candidates.clone();
            ordered.sort_by_key(|model| {
                *provider_order
                    .get(&(app_type.clone(), model.provider_id.clone()))
                    .unwrap_or(&i64::MAX)
            });
            for candidate in ordered {
                let Some(upstream_id) =
                    upstreams.get(&(app_type.clone(), candidate.provider_id.clone()))
                else {
                    continue;
                };
                if !seen.insert((upstream_id.clone(), candidate.model_id.clone())) {
                    continue;
                }
                let route_id = stable_id(
                    "route-target",
                    &[&gateway_model_id, upstream_id, &candidate.model_id],
                );
                conn.execute(
                    "INSERT OR IGNORE INTO route_targets
                        (id, gateway_model_id, upstream_id, target_model, position, enabled,
                         legacy_app_type, legacy_aggregate_id, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?8)",
                    params![
                        route_id,
                        gateway_model_id,
                        upstream_id,
                        candidate.model_id,
                        position,
                        app_type,
                        aggregate.id,
                        now,
                    ],
                )
                .map_err(|e| AppError::Database(format!("迁移聚合路由草稿失败: {e}")))?;
                position += 1;
            }
        }
    }
    Ok(())
}

fn group_models(models: &[LegacyModelRow]) -> HashMap<(String, String), Vec<&LegacyModelRow>> {
    let mut groups = HashMap::new();
    for model in models {
        if let Some(app_type) = canonical_app_type(&model.app_type) {
            groups
                .entry((app_type, model.model_id.to_lowercase()))
                .or_insert_with(Vec::new)
                .push(model);
        }
    }
    groups
}

fn provider_order(providers: &[LegacyProviderRow]) -> HashMap<(String, String), i64> {
    let mut by_app = BTreeMap::<String, Vec<&LegacyProviderRow>>::new();
    for provider in providers
        .iter()
        .filter(|provider| provider.in_failover_queue)
    {
        if let Some(app_type) = canonical_app_type(&provider.app_type) {
            by_app.entry(app_type).or_default().push(provider);
        }
    }
    let mut result = HashMap::new();
    for (app_type, mut rows) in by_app {
        rows.sort_by_key(|provider| (provider.sort_index.unwrap_or(999_999), provider.id.as_str()));
        for (index, provider) in rows.into_iter().enumerate() {
            result.insert((app_type.clone(), provider.id.clone()), index as i64);
        }
    }
    result
}

fn report_unknown_app_types(
    conn: &Connection,
    providers: &[LegacyProviderRow],
    models: &[LegacyModelRow],
    aggregates: &[LegacyAggregateRow],
    now: i64,
) -> Result<(), AppError> {
    for (entity, app_type, id) in providers
        .iter()
        .map(|row| ("provider", row.app_type.as_str(), row.id.as_str()))
        .chain(models.iter().map(|row| {
            (
                "upstream_model",
                row.app_type.as_str(),
                row.provider_id.as_str(),
            )
        }))
        .chain(
            aggregates
                .iter()
                .map(|row| ("custom_aggregate", row.app_type.as_str(), row.id.as_str())),
        )
    {
        if canonical_app_type(app_type).is_none() {
            report_issue(
                conn,
                "error",
                entity,
                Some(app_type),
                Some(id),
                "unknown_app_type",
                json!({}),
                now,
            )?;
        }
    }
    Ok(())
}

fn report_multiple_current_providers(conn: &Connection, now: i64) -> Result<(), AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT app_type, GROUP_CONCAT(id), COUNT(*)
             FROM providers WHERE is_current = 1
             GROUP BY app_type HAVING COUNT(*) > 1",
        )
        .map_err(|e| AppError::Database(format!("准备检查 current Provider 失败: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| AppError::Database(format!("检查 current Provider 失败: {e}")))?;
    for row in rows {
        let (app, ids, count) =
            row.map_err(|e| AppError::Database(format!("解析 current Provider 失败: {e}")))?;
        report_issue(
            conn,
            "warning",
            "provider",
            Some(&app),
            None,
            "multiple_current_providers",
            json!({"ids": ids.split(',').collect::<Vec<_>>(), "count": count}),
            now,
        )?;
    }
    Ok(())
}

fn report_legacy_failover_table(conn: &Connection, now: i64) -> Result<(), AppError> {
    if table_exists(conn, "failover_queue")? {
        report_issue(
            conn,
            "warning",
            "failover",
            None,
            None,
            "legacy_failover_table_present",
            json!({"message": "未读取或猜测残留表内容"}),
            now,
        )?;
    }
    Ok(())
}

fn report_legacy_column_conflicts(conn: &Connection, now: i64) -> Result<(), AppError> {
    for column in [
        "cost_multiplier",
        "limit_daily_usd",
        "limit_monthly_usd",
        "provider_type",
    ] {
        if !column_exists(conn, "providers", column)? {
            continue;
        }
        let sql = format!(
            "SELECT id, app_type, {column} FROM providers
             WHERE {column} IS NOT NULL AND TRIM(CAST({column} AS TEXT)) NOT IN ('', '1.0')"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| AppError::Database(format!("准备检查旧 Provider 列失败: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| AppError::Database(format!("检查旧 Provider 列失败: {e}")))?;
        for row in rows {
            let (id, app, _value) =
                row.map_err(|e| AppError::Database(format!("解析旧 Provider 列失败: {e}")))?;
            report_issue(
                conn,
                "warning",
                "provider",
                Some(&app),
                Some(&id),
                "legacy_provider_column_present",
                json!({"column": column}),
                now,
            )?;
        }
    }
    Ok(())
}

fn report_endpoint_anomalies(conn: &Connection, now: i64) -> Result<(), AppError> {
    if !table_exists(conn, "provider_endpoints")? {
        return Ok(());
    }
    let mut stmt = conn
        .prepare(
            "SELECT provider_id, app_type, url, COUNT(*), GROUP_CONCAT(COALESCE(added_at, 0))
             FROM provider_endpoints
             GROUP BY provider_id, app_type, url HAVING COUNT(*) > 1",
        )
        .map_err(|e| AppError::Database(format!("准备检查重复上游端点失败: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| AppError::Database(format!("检查重复上游端点失败: {e}")))?;
    for row in rows {
        let (provider, app, url, count, timestamps) =
            row.map_err(|e| AppError::Database(format!("解析重复上游端点失败: {e}")))?;
        report_issue(
            conn,
            "warning",
            "provider_endpoint",
            Some(&app),
            Some(&provider),
            "duplicate_endpoint",
            json!({"url": url, "count": count, "addedAt": timestamps}),
            now,
        )?;
    }
    Ok(())
}

fn report_aggregate_settings(
    conn: &Connection,
    aggregates: &[LegacyAggregateRow],
    model_groups: &HashMap<(String, String), Vec<&LegacyModelRow>>,
    now: i64,
) -> Result<(), AppError> {
    let aggregate_ids: HashSet<(String, String)> = aggregates
        .iter()
        .filter_map(|row| canonical_app_type(&row.app_type).map(|app| (app, row.id.clone())))
        .collect();
    let mut stmt = conn
        .prepare(
            "SELECT key, value FROM settings WHERE key LIKE 'cc_aggregate_config:%' ORDER BY key",
        )
        .map_err(|e| AppError::Database(format!("准备读取旧聚合设置失败: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| AppError::Database(format!("读取旧聚合设置失败: {e}")))?;
    for row in rows {
        let (key, raw) = row.map_err(|e| AppError::Database(format!("解析旧聚合设置失败: {e}")))?;
        let app = key.trim_start_matches("cc_aggregate_config:");
        let Some(canonical_app) = canonical_app_type(app) else {
            report_issue(
                conn,
                "error",
                "aggregate_config",
                Some(app),
                Some(&key),
                "unknown_app_type",
                json!({}),
                now,
            )?;
            continue;
        };
        let value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(error) => {
                report_issue(
                    conn,
                    "error",
                    "aggregate_config",
                    Some(&canonical_app),
                    Some(&key),
                    "invalid_aggregate_config",
                    json!({"message": error.to_string()}),
                    now,
                )?;
                continue;
            }
        };
        for (tier, reference) in value
            .get("tierSelection")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|object| object.iter())
        {
            let Some(kind) = reference.get("type").and_then(Value::as_str) else {
                continue;
            };
            let Some(target) = reference.get("value").and_then(Value::as_str) else {
                continue;
            };
            let dangling = match kind {
                "custom" => !aggregate_ids.contains(&(canonical_app.clone(), target.to_string())),
                "auto" => {
                    !model_groups.contains_key(&(canonical_app.clone(), target.to_lowercase()))
                }
                _ => true,
            };
            if dangling {
                report_issue(
                    conn,
                    "warning",
                    "aggregate_config",
                    Some(&canonical_app),
                    Some(&key),
                    "dangling_aggregate_reference",
                    json!({"tier": tier, "type": kind, "value": target}),
                    now,
                )?;
            }
        }
        report_issue(
            conn,
            "info",
            "aggregate_config",
            Some(&canonical_app),
            Some(&key),
            "legacy_tier_selection_requires_review",
            json!({"config": value}),
            now,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn report_issue(
    conn: &Connection,
    severity: &str,
    entity_type: &str,
    app_type: Option<&str>,
    entity_id: Option<&str>,
    code: &str,
    details: Value,
    now: i64,
) -> Result<(), AppError> {
    let details_json = serde_json::to_string(&details)
        .map_err(|e| AppError::Database(format!("序列化迁移报告失败: {e}")))?;
    let migration_key = stable_id(
        "migration-issue",
        &[
            severity,
            entity_type,
            app_type.unwrap_or(""),
            entity_id.unwrap_or(""),
            code,
            &details_json,
        ],
    );
    conn.execute(
        "INSERT OR IGNORE INTO gateway_migration_report
            (migration_key, severity, entity_type, legacy_app_type,
             legacy_entity_id, code, details_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            migration_key,
            severity,
            entity_type,
            app_type,
            entity_id,
            code,
            details_json,
            now,
        ],
    )
    .map_err(|e| AppError::Database(format!("写入迁移报告失败: {e}")))?;
    Ok(())
}

fn canonical_app_type(value: &str) -> Option<String> {
    let app = AppType::from_str(value).ok()?;
    let canonical = app.as_str();
    CANONICAL_APPS
        .contains(&canonical)
        .then(|| canonical.to_string())
}

fn stable_id(namespace: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    for part in parts {
        hasher.update([0]);
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    format!("{namespace}-{}", hex_prefix(&digest, 24))
}

fn hex_prefix(bytes: &[u8], count: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(count * 2);
    for byte in bytes.iter().take(count) {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn credential_hint(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        "***".to_string()
    } else {
        format!(
            "{}...{}",
            chars[..4].iter().collect::<String>(),
            chars[chars.len() - 4..].iter().collect::<String>()
        )
    }
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(|e| AppError::Database(format!("检查表 {table} 失败: {e}")))
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, AppError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| AppError::Database(format!("读取表 {table} 结构失败: {e}")))?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| AppError::Database(format!("查询表 {table} 结构失败: {e}")))?;
    for name in names {
        if name.map_err(|e| AppError::Database(e.to_string()))? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::services::model_fetch::FetchedModel;

    struct TestProtector;

    impl CredentialProtector for TestProtector {
        fn scheme(&self) -> &'static str {
            "test-protector-v1"
        }

        fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
            Ok(plaintext.iter().map(|byte| byte ^ 0xa5).collect())
        }

        fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
            self.protect(ciphertext)
        }
    }

    struct FailingProtector;

    impl CredentialProtector for FailingProtector {
        fn scheme(&self) -> &'static str {
            "test-failing-v1"
        }

        fn protect(&self, _plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
            Err(AppError::Config("credential protection unavailable".into()))
        }

        fn unprotect(&self, _ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
            Err(AppError::Config("credential decrypt unavailable".into()))
        }
    }

    fn legacy_provider(app_type: &str, settings: Value, meta: Value) -> LegacyProviderRow {
        LegacyProviderRow {
            id: format!("provider-{app_type}"),
            app_type: app_type.into(),
            name: app_type.into(),
            settings_config_raw: settings.to_string(),
            notes: None,
            created_at: Some(1),
            sort_index: Some(0),
            meta_raw: meta.to_string(),
            in_failover_queue: true,
        }
    }

    #[test]
    fn reclassifies_existing_v17_generic_credential_without_overwriting_precise_kind() {
        let db = Database::memory().expect("memory db");
        let conn = db.conn.lock().expect("lock");
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, meta)
             VALUES ('v17-codex', 'codex', 'Codex', ?1, '{}')",
            [r#"{"config":"base_url = \"https://codex.invalid\"\nexperimental_bearer_token = \"codex-secret\""}"#],
        )
        .expect("seed provider");
        migrate_with_protector(&conn, &TestProtector).expect("initial v17 migration");
        let upstream_id = stable_id("upstream", &["codex", "v17-codex"]);
        let provider = read_legacy_providers(&conn)
            .expect("read provider")
            .into_iter()
            .next()
            .expect("provider row");
        let extracted =
            extract_provider(&provider, "codex", &upstream_id).expect("extract provider");
        assert_eq!(
            extracted.credential.as_ref().map(|value| value.0.as_str()),
            Some(crate::gateway::credential::BEARER_TOKEN)
        );
        let initial_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM upstream_credentials WHERE upstream_id = ?1",
                [&upstream_id],
                |row| row.get(0),
            )
            .expect("count initial credential");
        assert_eq!(initial_count, 1);
        conn.execute(
            "UPDATE upstream_credentials SET credential_kind = 'api_key'
             WHERE upstream_id = ?1",
            [&upstream_id],
        )
        .expect("simulate early v17 generic kind");

        reclassify_v17_credentials_with_protector(&conn, &TestProtector)
            .expect("reclassify generic credential");
        let kinds = conn
            .prepare(
                "SELECT credential_kind FROM upstream_credentials
                 WHERE upstream_id = ?1 ORDER BY credential_kind",
            )
            .expect("prepare kinds")
            .query_map([&upstream_id], |row| row.get::<_, String>(0))
            .expect("query kinds")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect kinds");
        assert_eq!(kinds, vec![crate::gateway::credential::BEARER_TOKEN]);

        reclassify_v17_credentials_with_protector(&conn, &TestProtector)
            .expect("idempotent reclassify");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM upstream_credentials WHERE upstream_id = ?1",
                [&upstream_id],
                |row| row.get(0),
            )
            .expect("count credentials");
        assert_eq!(count, 1);
    }

    #[test]
    fn adapter_readable_env_credentials_never_enter_config_json() {
        let cases = [
            (
                "claude",
                json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://openai-compatible.invalid",
                        "OPENAI_API_KEY": "claude-openai-secret"
                    }
                }),
                json!({"apiFormat": "openai_chat"}),
                "claude-openai-secret",
            ),
            (
                "claude",
                json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://gemini-compatible.invalid",
                        "GEMINI_API_KEY": "claude-gemini-secret"
                    }
                }),
                json!({"apiFormat": "gemini_native"}),
                "claude-gemini-secret",
            ),
            (
                "codex",
                json!({
                    "base_url": "https://responses.invalid/v1",
                    "env": {"OPENAI_API_KEY": "codex-env-secret"}
                }),
                json!({}),
                "codex-env-secret",
            ),
        ];

        for (app_type, settings, meta, secret) in cases {
            let row = legacy_provider(app_type, settings, meta);
            let extracted = extract_provider(&row, app_type, "upstream-test").expect("extract");
            assert_eq!(
                extracted
                    .credential
                    .as_ref()
                    .map(|(_, value)| value.as_str()),
                Some(secret)
            );
            assert!(!extracted.config_json.to_string().contains(secret));
        }
    }

    #[test]
    fn protect_failure_disables_migrated_upstream_and_routes() {
        let db = Database::memory().expect("memory db");
        let provider = Provider::with_id(
            "protect-fail".into(),
            "Protect Fail".into(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://upstream.invalid",
                    "ANTHROPIC_API_KEY": "must-not-be-routable"
                }
            }),
            None,
        );
        db.save_provider("claude", &provider)
            .expect("save provider");
        db.replace_fetched_models(
            "claude",
            &provider.id,
            &[FetchedModel {
                id: "model-1".into(),
                owned_by: None,
            }],
            1,
        )
        .expect("save model");

        let conn = db.conn.lock().expect("lock");
        migrate_with_protector(&conn, &FailingProtector).expect("migration remains inspectable");
        let upstream_enabled: i64 = conn
            .query_row(
                "SELECT enabled FROM upstreams
                 WHERE legacy_app_type = 'claude' AND legacy_provider_id = 'protect-fail'",
                [],
                |row| row.get(0),
            )
            .expect("read upstream");
        let enabled_routes: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM route_targets rt
                 JOIN upstreams u ON u.id = rt.upstream_id
                 WHERE u.legacy_provider_id = 'protect-fail' AND rt.enabled = 1",
                [],
                |row| row.get(0),
            )
            .expect("read routes");
        let credentials: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM upstream_credentials uc
                 JOIN upstreams u ON u.id = uc.upstream_id
                 WHERE u.legacy_provider_id = 'protect-fail'",
                [],
                |row| row.get(0),
            )
            .expect("read credentials");

        assert_eq!(upstream_enabled, 0);
        assert_eq!(enabled_routes, 0);
        assert_eq!(credentials, 0);
    }

    #[test]
    fn stable_ids_are_deterministic_and_namespaced() {
        assert_eq!(
            stable_id("upstream", &["claude", "p1"]),
            stable_id("upstream", &["claude", "p1"])
        );
        assert_ne!(
            stable_id("upstream", &["claude", "p1"]),
            stable_id("upstream", &["codex", "p1"])
        );
    }

    #[test]
    fn test_protector_round_trips_without_plaintext_storage() {
        let protector = TestProtector;
        let plaintext = b"database-secret";
        let encrypted = protector.protect(plaintext).expect("protect");
        assert_ne!(encrypted, plaintext);
        assert_eq!(
            protector.unprotect(&encrypted).expect("unprotect"),
            plaintext
        );
    }
}
