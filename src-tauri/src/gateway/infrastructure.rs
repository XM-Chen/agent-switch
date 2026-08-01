//! 将影子域 Upstream 投影成现有数据面可消费的临时 Provider。
//!
//! 这是阶段 3 的适配桥：持久层只保存脱敏配置和受保护凭据，运行时在内存中解密并
//! 注入成熟 Provider adapter 所需的字段。临时 Provider 不得写回数据库或返回 DTO。

use serde_json::{json, Map, Value};

use crate::app_config::AppType;
use crate::database::{Database, UpstreamRecord};
use crate::error::AppError;
use crate::gateway::credential;
use crate::provider::{Provider, ProviderMeta};
use crate::services::credential_protector::{CredentialProtector, PlatformCredentialProtector};

pub(crate) fn load_upstream_provider(
    db: &Database,
    upstream: &UpstreamRecord,
) -> Result<(AppType, Provider), AppError> {
    let protector = PlatformCredentialProtector;
    load_upstream_provider_with_protector(db, upstream, &protector)
}

fn load_upstream_provider_with_protector(
    db: &Database,
    upstream: &UpstreamRecord,
    protector: &dyn CredentialProtector,
) -> Result<(AppType, Provider), AppError> {
    let credentials = db.list_upstream_credentials(&upstream.id)?;
    if credentials.len() != 1 {
        return Err(unavailable_credential(
            &upstream.id,
            if credentials.is_empty() {
                "missing"
            } else {
                "ambiguous multiple credentials"
            },
        ));
    }
    let credential = &credentials[0];
    if !credential::is_supported_kind(&credential.credential_kind) {
        return Err(unavailable_credential(&upstream.id, "unsupported kind"));
    }
    if !credential::kind_can_serve(
        &credential.credential_kind,
        &upstream.protocol,
        &upstream.adapter_type,
    ) {
        return Err(unavailable_credential(
            &upstream.id,
            "unsupported adapter/kind",
        ));
    }
    if credential.encryption_scheme != protector.scheme() {
        return Err(unavailable_credential(&upstream.id, "unsupported scheme"));
    }

    let plaintext = protector
        .unprotect(&credential.encrypted_payload)
        .map_err(|_| unavailable_credential(&upstream.id, "decrypt failed"))?;
    if !credential::validate_payload(&credential.credential_kind, &plaintext) {
        return Err(unavailable_credential(&upstream.id, "invalid payload"));
    }
    let secret = std::str::from_utf8(&plaintext)
        .map_err(|_| unavailable_credential(&upstream.id, "invalid plaintext"))?
        .trim();
    let oauth_access_token;
    let runtime_secret = if credential.credential_kind == credential::GOOGLE_OAUTH {
        oauth_access_token = credential::google_oauth_access_token(secret).ok_or_else(|| {
            unavailable_credential(&upstream.id, "oauth access token unavailable")
        })?;
        oauth_access_token.as_str()
    } else {
        secret
    };

    project_upstream_to_provider(upstream, &credential.credential_kind, runtime_secret)
        .ok_or_else(|| unavailable_credential(&upstream.id, "unsupported adapter/kind"))
}

fn unavailable_credential(upstream_id: &str, reason: &str) -> AppError {
    AppError::Config(format!("上游 {upstream_id} 的运行时凭据不可用（{reason}）"))
}

pub(crate) fn project_upstream_to_provider(
    upstream: &UpstreamRecord,
    credential_kind: &str,
    secret: &str,
) -> Option<(AppType, Provider)> {
    let app_type = adapter_app_type(&upstream.adapter_type, &upstream.protocol)?;
    let mut settings = upstream
        .config_json
        .get("legacySettings")
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Map::new()));
    apply_base_url(&mut settings, &app_type, upstream.base_url.as_deref());

    let meta = upstream
        .config_json
        .get("legacyMeta")
        .cloned()
        .and_then(|value| serde_json::from_value::<ProviderMeta>(value).ok())
        .unwrap_or_default();
    inject_runtime_credential(
        &mut settings,
        &app_type,
        &upstream.adapter_type,
        &upstream.protocol,
        &meta,
        credential_kind,
        secret,
    )?;

    let provider = Provider {
        id: upstream.id.clone(),
        name: upstream.name.clone(),
        settings_config: settings,
        website_url: None,
        category: None,
        created_at: Some(upstream.created_at),
        sort_index: None,
        notes: upstream.notes.clone(),
        meta: Some(meta),
        icon: None,
        icon_color: None,
        in_failover_queue: true,
    };
    Some((app_type, provider))
}

fn adapter_app_type(adapter_type: &str, protocol: &str) -> Option<AppType> {
    match adapter_type {
        "claude" | "module_anthropic" => Some(AppType::Claude),
        "codex" | "module_openai" => Some(AppType::Codex),
        "gemini" => Some(AppType::Gemini),
        _ => match protocol {
            "anthropic" | "anthropic_messages" => Some(AppType::Claude),
            "openai_chat" | "openai_responses" => Some(AppType::Codex),
            "gemini" | "gemini_native" => Some(AppType::Gemini),
            _ => None,
        },
    }
}

fn apply_base_url(settings: &mut Value, app_type: &AppType, base_url: Option<&str>) {
    let Some(base_url) = base_url else {
        return;
    };
    match app_type {
        AppType::Claude | AppType::ClaudeDesktop => {
            insert_env_value(settings, "ANTHROPIC_BASE_URL", base_url);
        }
        AppType::Gemini => {
            insert_env_value(settings, "GOOGLE_GEMINI_BASE_URL", base_url);
        }
        AppType::Codex => {
            settings
                .as_object_mut()
                .expect("settings is object")
                .insert("base_url".into(), Value::String(base_url.into()));
        }
        AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => {}
    }
}

fn inject_runtime_credential(
    settings: &mut Value,
    app_type: &AppType,
    adapter_type: &str,
    protocol: &str,
    meta: &ProviderMeta,
    credential_kind: &str,
    secret: &str,
) -> Option<()> {
    match credential_kind {
        credential::BEARER_TOKEN => match app_type {
            AppType::Claude | AppType::ClaudeDesktop => {
                insert_env_value(settings, "ANTHROPIC_AUTH_TOKEN", secret)
            }
            AppType::Codex => insert_env_value(settings, "OPENAI_API_KEY", secret),
            _ => return None,
        },
        credential::X_API_KEY => match app_type {
            AppType::Claude | AppType::ClaudeDesktop => {
                insert_env_value(settings, "ANTHROPIC_API_KEY", secret)
            }
            _ => return None,
        },
        credential::GOOGLE_API_KEY | credential::GOOGLE_OAUTH => {
            if *app_type != AppType::Gemini && !matches!(protocol, "gemini" | "gemini_native") {
                return None;
            }
            insert_env_value(settings, "GEMINI_API_KEY", secret);
        }
        credential::LEGACY_API_KEY => match app_type {
            AppType::Claude | AppType::ClaudeDesktop => {
                let key = if adapter_type == "module_anthropic" {
                    "ANTHROPIC_AUTH_TOKEN"
                } else if matches!(protocol, "gemini" | "gemini_native") {
                    "GEMINI_API_KEY"
                } else if meta.api_key_field.as_deref() == Some("ANTHROPIC_AUTH_TOKEN")
                    || settings.get("auth_mode").and_then(Value::as_str) == Some("bearer_only")
                    || settings.pointer("/env/AUTH_MODE").and_then(Value::as_str)
                        == Some("bearer_only")
                {
                    "ANTHROPIC_AUTH_TOKEN"
                } else if meta.provider_type.as_deref() == Some("openrouter") {
                    "OPENROUTER_API_KEY"
                } else {
                    "ANTHROPIC_API_KEY"
                };
                insert_env_value(settings, key, secret);
            }
            AppType::Codex => insert_env_value(settings, "OPENAI_API_KEY", secret),
            AppType::Gemini => insert_env_value(settings, "GEMINI_API_KEY", secret),
            AppType::OpenCode | AppType::OpenClaw | AppType::Hermes => return None,
        },
        _ => return None,
    }
    Some(())
}

fn insert_env_value(settings: &mut Value, key: &str, value: &str) {
    let object = settings.as_object_mut().expect("settings is object");
    let env = object.entry("env").or_insert_with(|| json!({}));
    if let Some(env) = env.as_object_mut() {
        env.insert(key.into(), Value::String(value.into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

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
            "test-protector-v1"
        }

        fn protect(&self, _plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
            Err(AppError::Config("protect failed".into()))
        }

        fn unprotect(&self, _ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
            Err(AppError::Config("decrypt failed".into()))
        }
    }

    fn upstream(adapter_type: &str, protocol: &str) -> UpstreamRecord {
        UpstreamRecord {
            id: format!("up-{adapter_type}-{protocol}"),
            name: "One".into(),
            enabled: true,
            base_url: Some("https://upstream.invalid/v1".into()),
            protocol: protocol.into(),
            adapter_type: adapter_type.into(),
            config_json: json!({"legacySettings": {}, "legacyMeta": {}}),
            notes: None,
            legacy_app_type: None,
            legacy_provider_id: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn save_credential(
        db: &Database,
        upstream: &UpstreamRecord,
        kind: &str,
        plaintext: &str,
        scheme: &str,
    ) {
        let encrypted = TestProtector
            .protect(plaintext.as_bytes())
            .expect("protect");
        let now = chrono::Utc::now().timestamp_millis();
        let conn = db.conn.lock().expect("lock");
        conn.execute(
            "INSERT OR IGNORE INTO upstreams
                (id, name, enabled, base_url, protocol, adapter_type, config_json,
                 created_at, updated_at)
             VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                upstream.id,
                upstream.name,
                upstream.base_url,
                upstream.protocol,
                upstream.adapter_type,
                upstream.config_json.to_string(),
                now
            ],
        )
        .expect("save upstream");
        conn.execute(
            "INSERT INTO upstream_credentials
                    (id, upstream_id, credential_kind, encrypted_payload, encryption_scheme,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                format!("cred-{}", upstream.id),
                upstream.id,
                kind,
                encrypted,
                scheme,
                now
            ],
        )
        .expect("save credential");
    }

    #[test]
    fn runtime_loader_injects_credentials_for_all_adapter_families() {
        let db = Database::memory().expect("memory db");
        let cases = [
            (
                "claude",
                "anthropic",
                credential::X_API_KEY,
                AppType::Claude,
                "/env/ANTHROPIC_API_KEY",
            ),
            (
                "codex",
                "openai_responses",
                credential::BEARER_TOKEN,
                AppType::Codex,
                "/env/OPENAI_API_KEY",
            ),
            (
                "gemini",
                "gemini",
                credential::GOOGLE_API_KEY,
                AppType::Gemini,
                "/env/GEMINI_API_KEY",
            ),
            (
                "module_anthropic",
                "anthropic",
                credential::BEARER_TOKEN,
                AppType::Claude,
                "/env/ANTHROPIC_AUTH_TOKEN",
            ),
            (
                "module_openai",
                "openai_chat",
                credential::BEARER_TOKEN,
                AppType::Codex,
                "/env/OPENAI_API_KEY",
            ),
        ];

        for (adapter_type, protocol, kind, expected_app, pointer) in cases {
            let upstream = upstream(adapter_type, protocol);
            let secret = format!("secret-{adapter_type}");
            save_credential(&db, &upstream, kind, &secret, TestProtector.scheme());

            let (adapter_app, provider) =
                load_upstream_provider_with_protector(&db, &upstream, &TestProtector)
                    .expect("load runtime provider");
            assert_eq!(adapter_app, expected_app);
            assert_eq!(
                provider
                    .settings_config
                    .pointer(pointer)
                    .and_then(Value::as_str),
                Some(secret.as_str())
            );
            assert!(!upstream.config_json.to_string().contains(&secret));
        }
    }

    #[test]
    fn codex_projection_injects_runtime_base_url_without_persisting_secret() {
        let db = Database::memory().expect("memory db");
        let upstream = upstream("codex", "openai_responses");
        save_credential(
            &db,
            &upstream,
            credential::BEARER_TOKEN,
            "codex-secret",
            TestProtector.scheme(),
        );

        let (_, provider) =
            load_upstream_provider_with_protector(&db, &upstream, &TestProtector).expect("load");
        assert_eq!(
            provider
                .settings_config
                .get("base_url")
                .and_then(Value::as_str),
            upstream.base_url.as_deref()
        );
        assert!(!upstream.config_json.to_string().contains("codex-secret"));
    }

    #[test]
    fn google_oauth_projection_injects_only_access_token() {
        let db = Database::memory().expect("memory db");
        let upstream = upstream("gemini", "gemini");
        let oauth = r#"{"access_token":"ya29.runtime","refresh_token":"must-stay-encrypted"}"#;
        save_credential(
            &db,
            &upstream,
            credential::GOOGLE_OAUTH,
            oauth,
            TestProtector.scheme(),
        );

        let (_, provider) =
            load_upstream_provider_with_protector(&db, &upstream, &TestProtector).expect("load");
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/GEMINI_API_KEY")
                .and_then(Value::as_str),
            Some("ya29.runtime")
        );
        assert!(!provider
            .settings_config
            .to_string()
            .contains("must-stay-encrypted"));
    }

    #[test]
    fn missing_wrong_scheme_and_decrypt_failure_are_fail_closed_and_redacted() {
        let db = Database::memory().expect("memory db");
        let upstream = upstream("claude", "anthropic");

        let missing = load_upstream_provider_with_protector(&db, &upstream, &TestProtector)
            .expect_err("missing credential must fail");
        assert!(!missing.to_string().contains("secret"));

        save_credential(
            &db,
            &upstream,
            credential::X_API_KEY,
            "never-log-this-secret",
            "wrong-scheme",
        );
        let wrong_scheme = load_upstream_provider_with_protector(&db, &upstream, &TestProtector)
            .expect_err("wrong scheme must fail");
        assert!(!wrong_scheme.to_string().contains("never-log-this-secret"));

        db.conn
            .lock()
            .expect("lock")
            .execute(
                "UPDATE upstream_credentials SET encryption_scheme = ?1 WHERE upstream_id = ?2",
                params![FailingProtector.scheme(), upstream.id],
            )
            .expect("update scheme");
        let decrypt = load_upstream_provider_with_protector(&db, &upstream, &FailingProtector)
            .expect_err("decrypt failure must fail");
        assert!(!decrypt.to_string().contains("never-log-this-secret"));
    }
}
