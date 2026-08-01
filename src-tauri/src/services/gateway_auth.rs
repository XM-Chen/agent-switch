use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::database::{Database, GatewayApiKeyRecord};
use crate::error::AppError;

const KEY_PREFIX: &str = "agsk_";
const DISPLAY_PREFIX_LEN: usize = 12;
const RANDOM_KEY_BYTES: usize = 32;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayApiKeySummary {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedGatewayApiKey {
    pub key: GatewayApiKeySummary,
    pub secret: String,
}

pub struct GatewayAuthService;

impl GatewayAuthService {
    pub fn migrate_legacy_token(db: &Database) -> Result<bool, AppError> {
        let Some(token) = db.get_setting("claude_desktop_gateway_token")? else {
            return Ok(false);
        };
        if token.trim().is_empty() {
            let _ = db.take_legacy_gateway_token()?;
            return Ok(false);
        }
        let created_at = chrono::Utc::now().timestamp_millis();
        let record = GatewayApiKeyRecord {
            id: deterministic_legacy_id(&token),
            name: "迁移的网关密钥".to_string(),
            key_hash: hash_secret(&token)?,
            key_prefix: display_prefix(&token),
            created_at,
            revoked_at: None,
            last_used_at: None,
        };
        if db.get_gateway_api_key(&record.id)?.is_none() {
            db.insert_gateway_api_key(&record)?;
        }
        db.take_legacy_gateway_token()?;
        Ok(true)
    }

    pub fn create_key(db: &Database, name: &str) -> Result<CreatedGatewayApiKey, AppError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::InvalidInput("API Key 名称不能为空".to_string()));
        }
        let secret = generate_secret()?;
        let created_at = chrono::Utc::now().timestamp_millis();
        let record = GatewayApiKeyRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            key_hash: hash_secret(&secret)?,
            key_prefix: display_prefix(&secret),
            created_at,
            revoked_at: None,
            last_used_at: None,
        };
        db.insert_gateway_api_key(&record)?;
        Ok(CreatedGatewayApiKey {
            key: record_to_summary(record),
            secret,
        })
    }

    pub fn list_keys(db: &Database) -> Result<Vec<GatewayApiKeySummary>, AppError> {
        db.list_gateway_api_keys()
            .map(|records| records.into_iter().map(record_to_summary).collect())
    }

    pub fn revoke_key(db: &Database, key_id: &str) -> Result<bool, AppError> {
        db.revoke_gateway_api_key(key_id, chrono::Utc::now().timestamp_millis())
    }

    pub fn verify(db: &Database, secret: &str) -> Result<Option<String>, AppError> {
        if secret.is_empty() {
            return Ok(None);
        }
        for record in db.list_gateway_api_keys()? {
            if record.revoked_at.is_some() {
                continue;
            }
            let parsed = PasswordHash::new(&record.key_hash)
                .map_err(|e| AppError::Config(format!("网关 API Key 哈希无效: {e}")))?;
            if Argon2::default()
                .verify_password(secret.as_bytes(), &parsed)
                .is_ok()
            {
                return Ok(Some(record.id));
            }
        }
        Ok(None)
    }
}

fn record_to_summary(record: GatewayApiKeyRecord) -> GatewayApiKeySummary {
    GatewayApiKeySummary {
        id: record.id,
        name: record.name,
        key_prefix: record.key_prefix,
        created_at: record.created_at,
        revoked_at: record.revoked_at,
        last_used_at: record.last_used_at,
    }
}

fn generate_secret() -> Result<String, AppError> {
    use rand::{rngs::OsRng, RngCore};

    let mut bytes = [0_u8; RANDOM_KEY_BYTES];
    OsRng.fill_bytes(&mut bytes);
    Ok(format!("{KEY_PREFIX}{}", hex_encode(&bytes)))
}

fn hash_secret(secret: &str) -> Result<String, AppError> {
    use rand::rngs::OsRng;

    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| AppError::Config(format!("哈希网关 API Key 失败: {e}")))
}

fn deterministic_legacy_id(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    format!("legacy-{}", hex_encode(&digest[..8]))
}

fn display_prefix(secret: &str) -> String {
    secret.chars().take(DISPLAY_PREFIX_LEN).collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_verify_and_revoke_key() {
        let db = Database::memory().unwrap();
        let created = GatewayAuthService::create_key(&db, "test").unwrap();
        assert!(created.secret.starts_with(KEY_PREFIX));
        assert_eq!(
            GatewayAuthService::verify(&db, &created.secret).unwrap(),
            Some(created.key.id.clone())
        );
        assert!(GatewayAuthService::revoke_key(&db, &created.key.id).unwrap());
        assert_eq!(
            GatewayAuthService::verify(&db, &created.secret).unwrap(),
            None
        );
    }

    #[test]
    fn legacy_token_moves_out_of_plaintext_settings() {
        let db = Database::memory().unwrap();
        db.set_setting("claude_desktop_gateway_token", "legacy-secret")
            .unwrap();
        assert!(GatewayAuthService::migrate_legacy_token(&db).unwrap());
        assert_eq!(
            db.get_setting("claude_desktop_gateway_token").unwrap(),
            None
        );
        assert!(GatewayAuthService::verify(&db, "legacy-secret")
            .unwrap()
            .is_some());
    }
}
