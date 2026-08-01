//! Codex OAuth 数据面认证。
//!
//! 本模块只从 Agent Switch 自有的 `codex_oauth_auth.json` 读取账号凭据，
//! 并为网关请求提供 access token 缓存与 refresh token 刷新能力。
//! 它不会读取或写入任何客户端真实配置。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// OpenAI OAuth 客户端 ID（与官方 Codex CLI 相同）。
const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// OAuth Token URL，仅用于 refresh token 换取 access token。
const OAUTH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// Token 刷新提前量（毫秒）。
const TOKEN_REFRESH_BUFFER_MS: i64 = 60_000;

const DEFAULT_TOKEN_EXPIRES_IN_SECS: i64 = 3_600;
const CODEX_USER_AGENT: &str = "agent-switch-codex-oauth";

#[derive(Debug, thiserror::Error)]
pub enum CodexOAuthError {
    #[error("OAuth Token 获取失败: {0}")]
    TokenFetchFailed(String),

    #[error("Refresh Token 失效或已过期")]
    RefreshTokenInvalid,

    #[error("网络错误: {0}")]
    NetworkError(String),

    #[error("解析错误: {0}")]
    ParseError(String),

    #[error("IO 错误: {0}")]
    IoError(String),

    #[error("账号不存在: {0}")]
    AccountNotFound(String),
}

impl From<reqwest::Error> for CodexOAuthError {
    fn from(err: reqwest::Error) -> Self {
        Self::NetworkError(err.to_string())
    }
}

impl From<std::io::Error> for CodexOAuthError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}

#[derive(Debug, Clone, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Debug, Clone)]
struct CachedAccessToken {
    token: String,
    /// Unix 毫秒时间戳。
    expires_at_ms: i64,
}

impl CachedAccessToken {
    fn is_expiring_soon(&self) -> bool {
        self.is_expiring_soon_at(chrono::Utc::now().timestamp_millis())
    }

    fn is_expiring_soon_at(&self, now_ms: i64) -> bool {
        self.expires_at_ms.saturating_sub(now_ms) < TOKEN_REFRESH_BUFFER_MS
    }
}

/// Agent Switch 自有存储中的账号数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexAccountData {
    /// ChatGPT account ID；存储格式中同时也是 `accounts` 的 key。
    account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    refresh_token: String,
    /// 认证时间戳（秒），用于缺少有效默认账号时确定稳定回退项。
    authenticated_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CodexOAuthStore {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    accounts: HashMap<String, CodexAccountData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_account_id: Option<String>,
}

/// Codex OAuth 数据面 token 管理器。
pub struct CodexOAuthManager {
    accounts: RwLock<HashMap<String, CodexAccountData>>,
    default_account_id: RwLock<Option<String>>,
    /// access token 只缓存在内存中。
    access_tokens: RwLock<HashMap<String, CachedAccessToken>>,
    /// 每个账号一把锁，避免并发请求重复刷新同一 refresh token。
    refresh_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Agent Switch 自有数据文件，不是 Codex 客户端配置。
    storage_path: PathBuf,
}

impl CodexOAuthManager {
    pub fn new(data_dir: PathBuf) -> Self {
        let storage_path = data_dir.join("codex_oauth_auth.json");
        let store = match Self::read_store(&storage_path) {
            Ok(store) => store,
            Err(error) => {
                log::warn!("[CodexOAuth] 加载存储失败: {error}");
                CodexOAuthStore::default()
            }
        };

        let default_account_id =
            Self::resolve_default_from(&store.accounts, store.default_account_id.as_deref());

        if !store.accounts.is_empty() {
            log::info!(
                "[CodexOAuth] 从 Agent Switch 自有存储加载 {} 个账号",
                store.accounts.len()
            );
        }

        Self {
            accounts: RwLock::new(store.accounts),
            default_account_id: RwLock::new(default_account_id),
            access_tokens: RwLock::new(HashMap::new()),
            refresh_locks: Mutex::new(HashMap::new()),
            storage_path,
        }
    }

    /// 获取指定账号的有效 access token，必要时自动刷新。
    pub async fn get_valid_token_for_account(
        &self,
        account_id: &str,
    ) -> Result<String, CodexOAuthError> {
        if let Some(token) = self.cached_valid_token(account_id).await {
            return Ok(token);
        }

        let refresh_lock = self.get_refresh_lock(account_id).await;
        let _guard = refresh_lock.lock().await;

        // 等待刷新锁期间，其他请求可能已经完成刷新。
        if let Some(token) = self.cached_valid_token(account_id).await {
            return Ok(token);
        }

        let refresh_token = {
            let accounts = self.accounts.read().await;
            accounts
                .get(account_id)
                .map(|account| account.refresh_token.clone())
                .ok_or_else(|| CodexOAuthError::AccountNotFound(account_id.to_string()))?
        };

        log::info!("[CodexOAuth] 账号 {account_id} 的 access token 需要刷新");
        let refreshed = self.refresh_with_token(&refresh_token).await?;

        self.persist_rotated_refresh_token(
            account_id,
            &refresh_token,
            refreshed.refresh_token.clone(),
        )
        .await?;

        let access_token = refreshed.access_token;
        let expires_at_ms = compute_expires_at_ms(refreshed.expires_in);

        self.access_tokens.write().await.insert(
            account_id.to_string(),
            CachedAccessToken {
                token: access_token.clone(),
                expires_at_ms,
            },
        );

        Ok(access_token)
    }

    /// 获取默认账号的有效 access token。
    pub async fn get_valid_token(&self) -> Result<String, CodexOAuthError> {
        match self.resolve_default_account_id().await {
            Some(account_id) => self.get_valid_token_for_account(&account_id).await,
            None => Err(CodexOAuthError::AccountNotFound(
                "无可用的 ChatGPT 账号".to_string(),
            )),
        }
    }

    /// 获取默认账号 ID，供网关注入 `ChatGPT-Account-Id` 请求头。
    pub async fn default_account_id(&self) -> Option<String> {
        self.resolve_default_account_id().await
    }

    async fn cached_valid_token(&self, account_id: &str) -> Option<String> {
        self.access_tokens
            .read()
            .await
            .get(account_id)
            .filter(|cached| !cached.is_expiring_soon())
            .map(|cached| cached.token.clone())
    }

    /// 用 refresh token 换取新的 access token。
    async fn refresh_with_token(
        &self,
        refresh_token: &str,
    ) -> Result<OAuthTokenResponse, CodexOAuthError> {
        let response = crate::proxy::http_client::get()
            .post(OAUTH_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", CODEX_USER_AGENT)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CODEX_CLIENT_ID),
                ("scope", "openid profile email"),
            ])
            .send()
            .await?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(CodexOAuthError::RefreshTokenInvalid);
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(CodexOAuthError::TokenFetchFailed(format!(
                "Refresh 失败: {status} - {body}"
            )));
        }

        response
            .json()
            .await
            .map_err(|error| CodexOAuthError::ParseError(error.to_string()))
    }

    async fn persist_rotated_refresh_token(
        &self,
        account_id: &str,
        previous_refresh_token: &str,
        replacement: Option<String>,
    ) -> Result<(), CodexOAuthError> {
        let Some(replacement) = replacement else {
            return Ok(());
        };
        if replacement == previous_refresh_token {
            return Ok(());
        }

        {
            let mut accounts = self.accounts.write().await;
            let account = accounts
                .get_mut(account_id)
                .ok_or_else(|| CodexOAuthError::AccountNotFound(account_id.to_string()))?;
            account.refresh_token = replacement.clone();
        }

        if let Err(error) = self.save_to_disk().await {
            // 同一账号的刷新锁仍由调用方持有；失败时恢复内存，避免磁盘与内存分裂。
            let mut accounts = self.accounts.write().await;
            if let Some(account) = accounts.get_mut(account_id) {
                if account.refresh_token == replacement {
                    account.refresh_token = previous_refresh_token.to_string();
                }
            }
            return Err(error);
        }

        Ok(())
    }

    async fn resolve_default_account_id(&self) -> Option<String> {
        let stored = self.default_account_id.read().await.clone();
        let accounts = self.accounts.read().await;
        Self::resolve_default_from(&accounts, stored.as_deref())
    }

    fn resolve_default_from(
        accounts: &HashMap<String, CodexAccountData>,
        stored: Option<&str>,
    ) -> Option<String> {
        stored
            .filter(|account_id| accounts.contains_key(*account_id))
            .map(str::to_owned)
            .or_else(|| Self::fallback_default_account_id(accounts))
    }

    fn fallback_default_account_id(accounts: &HashMap<String, CodexAccountData>) -> Option<String> {
        accounts
            .iter()
            .max_by(|(id_a, account_a), (id_b, account_b)| {
                account_a
                    .authenticated_at
                    .cmp(&account_b.authenticated_at)
                    // 时间相同时选择字典序较小的 ID，避免依赖 HashMap 遍历顺序。
                    .then_with(|| id_b.cmp(id_a))
            })
            .map(|(account_id, _)| account_id.clone())
    }

    async fn get_refresh_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.refresh_locks.lock().await;
        Arc::clone(
            locks
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    fn read_store(storage_path: &Path) -> Result<CodexOAuthStore, CodexOAuthError> {
        if !storage_path.exists() {
            return Ok(CodexOAuthStore::default());
        }

        let content = fs::read_to_string(storage_path)?;
        serde_json::from_str(&content)
            .map_err(|error| CodexOAuthError::ParseError(error.to_string()))
    }

    async fn save_to_disk(&self) -> Result<(), CodexOAuthError> {
        let accounts = self.accounts.read().await.clone();
        let stored_default = self.default_account_id.read().await.clone();
        let default_account_id = Self::resolve_default_from(&accounts, stored_default.as_deref());

        let store = CodexOAuthStore {
            version: 1,
            accounts,
            default_account_id,
        };
        let content = serde_json::to_string_pretty(&store)
            .map_err(|error| CodexOAuthError::ParseError(error.to_string()))?;

        self.write_store_atomic(&content)?;
        log::info!(
            "[CodexOAuth] 刷新凭据已持久化（{} 个账号）",
            store.accounts.len()
        );
        Ok(())
    }

    fn write_store_atomic(&self, content: &str) -> Result<(), CodexOAuthError> {
        let parent = self
            .storage_path
            .parent()
            .ok_or_else(|| CodexOAuthError::IoError("无效的存储路径".to_string()))?;
        fs::create_dir_all(parent)?;

        let file_name = self
            .storage_path
            .file_name()
            .ok_or_else(|| CodexOAuthError::IoError("无效的存储文件名".to_string()))?
            .to_string_lossy();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary_path = parent.join(format!("{file_name}.tmp.{timestamp}"));

        let result = self.write_and_replace(&temporary_path, content);
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }

    fn write_and_replace(
        &self,
        temporary_path: &Path,
        content: &str,
    ) -> Result<(), CodexOAuthError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(temporary_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            fs::rename(temporary_path, &self.storage_path)?;
            fs::set_permissions(&self.storage_path, fs::Permissions::from_mode(0o600))?;
        }

        #[cfg(windows)]
        {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(temporary_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            if self.storage_path.exists() {
                fs::remove_file(&self.storage_path)?;
            }
            fs::rename(temporary_path, &self.storage_path)?;
        }

        #[cfg(not(any(unix, windows)))]
        {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(temporary_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;
            fs::rename(temporary_path, &self.storage_path)?;
        }

        Ok(())
    }
}

fn compute_expires_at_ms(expires_in: Option<i64>) -> i64 {
    compute_expires_at_ms_from(chrono::Utc::now().timestamp_millis(), expires_in)
}

fn compute_expires_at_ms_from(now_ms: i64, expires_in: Option<i64>) -> i64 {
    now_ms.saturating_add(
        expires_in
            .unwrap_or(DEFAULT_TOKEN_EXPIRES_IN_SECS)
            .saturating_mul(1_000),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(account_id: &str, refresh_token: &str, authenticated_at: i64) -> CodexAccountData {
        CodexAccountData {
            account_id: account_id.to_string(),
            email: Some(format!("{account_id}@example.com")),
            refresh_token: refresh_token.to_string(),
            authenticated_at,
        }
    }

    fn write_store(
        data_dir: &Path,
        accounts: HashMap<String, CodexAccountData>,
        default_account_id: Option<&str>,
    ) {
        let store = CodexOAuthStore {
            version: 1,
            accounts,
            default_account_id: default_account_id.map(str::to_owned),
        };
        fs::write(
            data_dir.join("codex_oauth_auth.json"),
            serde_json::to_string_pretty(&store).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn cached_token_expiry_respects_refresh_buffer() {
        let now_ms = 1_000_000;
        let token = |expires_at_ms| CachedAccessToken {
            token: "access-token".to_string(),
            expires_at_ms,
        };

        assert!(token(now_ms - 1).is_expiring_soon_at(now_ms));
        assert!(token(now_ms + 30_000).is_expiring_soon_at(now_ms));
        assert!(!token(now_ms + TOKEN_REFRESH_BUFFER_MS).is_expiring_soon_at(now_ms));
        assert!(!token(now_ms + TOKEN_REFRESH_BUFFER_MS + 1).is_expiring_soon_at(now_ms));
    }

    #[test]
    fn computes_expiry_from_server_or_default_lifetime() {
        let now_ms = 5_000;
        assert_eq!(
            compute_expires_at_ms_from(now_ms, Some(120)),
            now_ms + 120_000
        );
        assert_eq!(
            compute_expires_at_ms_from(now_ms, None),
            now_ms + DEFAULT_TOKEN_EXPIRES_IN_SECS * 1_000
        );
        assert_eq!(compute_expires_at_ms_from(now_ms, Some(-1)), now_ms - 1_000);
    }

    #[tokio::test]
    async fn missing_store_loads_empty_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = CodexOAuthManager::new(temp.path().to_path_buf());

        assert!(manager.accounts.read().await.is_empty());
        assert_eq!(manager.default_account_id().await, None);
    }

    #[tokio::test]
    async fn loads_accounts_and_stored_default_from_agent_switch_store() {
        let temp = tempfile::tempdir().unwrap();
        let mut accounts = HashMap::new();
        accounts.insert("acc-old".to_string(), account("acc-old", "rt-old", 10));
        accounts.insert("acc-new".to_string(), account("acc-new", "rt-new", 20));
        write_store(temp.path(), accounts, Some("acc-old"));

        let manager = CodexOAuthManager::new(temp.path().to_path_buf());
        let loaded = manager.accounts.read().await;

        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded
                .get("acc-old")
                .map(|data| data.refresh_token.as_str()),
            Some("rt-old")
        );
        drop(loaded);
        assert_eq!(
            manager.default_account_id().await.as_deref(),
            Some("acc-old")
        );
    }

    #[tokio::test]
    async fn invalid_stored_default_falls_back_to_latest_account() {
        let temp = tempfile::tempdir().unwrap();
        let mut accounts = HashMap::new();
        accounts.insert("acc-old".to_string(), account("acc-old", "rt-old", 10));
        accounts.insert("acc-new".to_string(), account("acc-new", "rt-new", 20));
        write_store(temp.path(), accounts, Some("missing-account"));

        let manager = CodexOAuthManager::new(temp.path().to_path_buf());

        assert_eq!(
            manager.default_account_id().await.as_deref(),
            Some("acc-new")
        );
    }

    #[test]
    fn default_fallback_is_deterministic_when_timestamps_match() {
        let mut accounts = HashMap::new();
        accounts.insert("acc-b".to_string(), account("acc-b", "rt-b", 10));
        accounts.insert("acc-a".to_string(), account("acc-a", "rt-a", 10));

        assert_eq!(
            CodexOAuthManager::resolve_default_from(&accounts, None).as_deref(),
            Some("acc-a")
        );
    }

    #[tokio::test]
    async fn rotated_refresh_token_is_persisted_without_losing_account_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let mut accounts = HashMap::new();
        accounts.insert("acc-1".to_string(), account("acc-1", "rt-old", 42));
        write_store(temp.path(), accounts, Some("acc-1"));

        let manager = CodexOAuthManager::new(temp.path().to_path_buf());
        manager
            .persist_rotated_refresh_token("acc-1", "rt-old", Some("rt-new".to_string()))
            .await
            .unwrap();

        let stored =
            CodexOAuthManager::read_store(&temp.path().join("codex_oauth_auth.json")).unwrap();
        let stored_account = stored.accounts.get("acc-1").unwrap();
        assert_eq!(stored_account.refresh_token, "rt-new");
        assert_eq!(stored_account.account_id, "acc-1");
        assert_eq!(stored_account.email.as_deref(), Some("acc-1@example.com"));
        assert_eq!(stored_account.authenticated_at, 42);
        assert_eq!(stored.default_account_id.as_deref(), Some("acc-1"));
    }
}
