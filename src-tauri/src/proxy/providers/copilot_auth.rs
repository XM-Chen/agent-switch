//! GitHub Copilot 网关认证数据面。
//!
//! 仅负责加载 Agent Switch 自有 `copilot_auth.json`、惰性迁移旧本地格式，
//! 并为网关请求提供有效 token、模型元数据与动态 API endpoint。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// 默认 GitHub 域名
const DEFAULT_GITHUB_DOMAIN: &str = "github.com";

fn default_github_domain() -> String {
    DEFAULT_GITHUB_DOMAIN.to_string()
}

/// GitHub API 基础 URL（github.com 用 api.github.com，GHES 用 {domain}/api/v3）
fn github_api_base(domain: &str) -> String {
    if domain == DEFAULT_GITHUB_DOMAIN {
        "https://api.github.com".to_string()
    } else {
        format!("https://{domain}/api/v3")
    }
}

/// Copilot Token URL
fn copilot_token_url(domain: &str) -> String {
    format!("{}/copilot_internal/v2/token", github_api_base(domain))
}

/// GitHub User API URL
fn github_user_url(domain: &str) -> String {
    format!("{}/user", github_api_base(domain))
}

/// Copilot 内部用户 API URL（用于动态获取 API endpoint）
fn copilot_internal_user_url(domain: &str) -> String {
    format!("{}/copilot_internal/user", github_api_base(domain))
}

/// Copilot API 基础地址（github.com 用 api.githubcopilot.com，GHES 用 copilot-api.{domain}）
fn copilot_api_base(domain: &str) -> String {
    if domain == DEFAULT_GITHUB_DOMAIN {
        "https://api.githubcopilot.com".to_string()
    } else {
        format!("https://copilot-api.{domain}")
    }
}

/// Token 刷新提前量（秒）
const TOKEN_REFRESH_BUFFER_SECONDS: i64 = 60;

/// 判断是否为 GitHub Enterprise Server（非 github.com）
fn is_ghes(domain: &str) -> bool {
    domain != DEFAULT_GITHUB_DOMAIN
}

/// Copilot API Header 常量
pub const COPILOT_EDITOR_VERSION: &str = "vscode/1.110.1";
pub const COPILOT_PLUGIN_VERSION: &str = "copilot-chat/0.38.2";
pub const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.38.2";
pub const COPILOT_API_VERSION: &str = "2025-10-01";
pub const COPILOT_INTEGRATION_ID: &str = "vscode-chat";

/// Copilot 内部用户 API 的数据面响应
#[derive(Debug, Deserialize)]
struct CopilotUserResponse {
    #[serde(default)]
    endpoints: Option<CopilotEndpoints>,
}

#[derive(Debug, Deserialize)]
struct CopilotEndpoints {
    api: String,
}

/// Copilot 可用模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotModel {
    /// 模型 ID（用于 API 调用）
    pub id: String,
    /// 模型显示名称
    pub name: String,
    /// 模型供应商
    pub vendor: String,
    /// 是否在模型选择器中显示
    pub model_picker_enabled: bool,
}

/// Copilot Models API 响应
#[derive(Debug, Deserialize)]
struct CopilotModelsResponse {
    data: Vec<CopilotModelsResponseItem>,
}

/// Copilot Models API 响应项
#[derive(Debug, Deserialize)]
struct CopilotModelsResponseItem {
    id: String,
    name: String,
    vendor: String,
    model_picker_enabled: bool,
}

/// Copilot 认证错误
#[derive(Debug, thiserror::Error)]
pub enum CopilotAuthError {
    #[error("GitHub 令牌无效或已过期")]
    GitHubTokenInvalid,

    #[error("Copilot 令牌获取失败: {0}")]
    CopilotTokenFetchFailed(String),

    #[error("网络错误: {0}")]
    NetworkError(String),

    #[error("解析错误: {0}")]
    ParseError(String),

    #[error("IO 错误: {0}")]
    IoError(String),

    #[error("用户未订阅 Copilot")]
    NoCopilotSubscription,

    #[error("账号不存在: {0}")]
    AccountNotFound(String),
}

impl From<reqwest::Error> for CopilotAuthError {
    fn from(err: reqwest::Error) -> Self {
        CopilotAuthError::NetworkError(err.to_string())
    }
}

impl From<std::io::Error> for CopilotAuthError {
    fn from(err: std::io::Error) -> Self {
        CopilotAuthError::IoError(err.to_string())
    }
}

/// Copilot Token
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CopilotToken {
    token: String,
    expires_at: i64,
}

impl CopilotToken {
    /// 检查令牌是否即将过期（提前 60 秒）
    fn is_expiring_soon(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        self.expires_at - now < TOKEN_REFRESH_BUFFER_SECONDS
    }
}

/// Copilot Token API 响应
#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: String,
    expires_at: i64,
}

/// GitHub 用户信息（仅用于旧本地格式惰性迁移）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubUser {
    login: String,
    id: u64,
    avatar_url: Option<String>,
}

/// 账号数据（内部存储结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubAccountData {
    /// GitHub OAuth Token
    ///
    /// 安全说明：为了复用登录状态，本地会持久化该令牌。
    /// 当前实现未接入系统钥匙串，依赖私有文件权限（Unix 下 0600）保护。
    github_token: String,
    /// 用户信息
    user: GitHubUser,
    /// 认证时间戳
    authenticated_at: i64,
    /// GitHub 域名（github.com 或 GHES 域名）
    #[serde(default = "default_github_domain")]
    github_domain: String,
}

/// 持久化存储结构（v3 多账号 + 默认账号格式）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CopilotAuthStore {
    /// 存储格式版本（3 = 多账号 + 默认账号格式）
    #[serde(default)]
    version: u32,
    /// 多账号数据（key = GitHub user ID）
    #[serde(default)]
    accounts: HashMap<String, GitHubAccountData>,
    /// 默认账号 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    default_account_id: Option<String>,
    /// 兼容 v1 单账号格式的字段
    #[serde(skip_serializing_if = "Option::is_none")]
    github_token: Option<String>,
}

/// Copilot 认证管理器（支持多账号）
pub struct CopilotAuthManager {
    /// 所有 GitHub 账号（key = GitHub user ID）
    accounts: Arc<RwLock<HashMap<String, GitHubAccountData>>>,
    /// 默认账号 ID
    default_account_id: Arc<RwLock<Option<String>>>,
    /// 每个账号的刷新锁，避免并发刷新重复打 GitHub API
    refresh_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
    /// Copilot Token 缓存（key = GitHub user ID，内存缓存，自动刷新）
    copilot_tokens: Arc<RwLock<HashMap<String, CopilotToken>>>,
    /// Copilot Models 缓存（key = GitHub user ID，仅进程内复用）
    copilot_models: Arc<RwLock<HashMap<String, Vec<CopilotModel>>>>,
    /// Copilot API 端点缓存（key = GitHub user ID，从 /copilot_internal/user 获取）
    api_endpoints: Arc<RwLock<HashMap<String, String>>>,
    /// 每个账号的端点拉取锁，避免并发拉取重复打 GitHub API
    endpoint_locks: Arc<RwLock<HashMap<String, Arc<Mutex<()>>>>>,
    /// 存储路径
    storage_path: PathBuf,
    /// 待迁移的旧格式 token
    pending_migration: Arc<RwLock<Option<String>>>,
}

impl CopilotAuthManager {
    /// 创建新的认证管理器
    pub fn new(data_dir: PathBuf) -> Self {
        let storage_path = data_dir.join("copilot_auth.json");

        let manager = Self {
            accounts: Arc::new(RwLock::new(HashMap::new())),
            default_account_id: Arc::new(RwLock::new(None)),
            refresh_locks: Arc::new(RwLock::new(HashMap::new())),
            copilot_tokens: Arc::new(RwLock::new(HashMap::new())),
            copilot_models: Arc::new(RwLock::new(HashMap::new())),
            api_endpoints: Arc::new(RwLock::new(HashMap::new())),
            endpoint_locks: Arc::new(RwLock::new(HashMap::new())),
            storage_path,
            pending_migration: Arc::new(RwLock::new(None)),
        };

        // 尝试从磁盘加载（同步，不发起网络请求）
        if let Err(e) = manager.load_from_disk_sync() {
            log::warn!("[CopilotAuth] 加载存储失败: {e}");
        }

        manager
    }

    // ==================== 旧格式迁移 ====================

    /// 将 v1 单账号 token 转换并持久化为当前多账号格式
    async fn persist_migrated_account(
        &self,
        github_token: String,
        user: GitHubUser,
    ) -> Result<(), CopilotAuthError> {
        let account_id = user.id.to_string();
        let account_data = GitHubAccountData {
            github_token,
            user,
            authenticated_at: chrono::Utc::now().timestamp(),
            github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
        };

        self.accounts
            .write()
            .await
            .insert(account_id.clone(), account_data);

        let mut default_account_id = self.default_account_id.write().await;
        if default_account_id.is_none() {
            *default_account_id = Some(account_id);
        }
        drop(default_account_id);

        self.save_to_disk().await
    }

    // ==================== Token 获取方法 ====================

    /// 获取指定账号的有效 Copilot Token（自动刷新）
    pub async fn get_valid_token_for_account(
        &self,
        account_id: &str,
    ) -> Result<String, CopilotAuthError> {
        // 确保迁移完成
        self.ensure_migration_complete().await?;

        // GHES 账号直接使用 GitHub OAuth token，无需 Copilot token 交换
        let domain = self.get_account_domain(account_id).await;
        if is_ghes(&domain) {
            let accounts = self.accounts.read().await;
            return accounts
                .get(account_id)
                .map(|a| a.github_token.clone())
                .ok_or_else(|| CopilotAuthError::AccountNotFound(account_id.to_string()));
        }

        // 检查缓存的 token
        {
            let tokens = self.copilot_tokens.read().await;
            if let Some(copilot_token) = tokens.get(account_id) {
                if !copilot_token.is_expiring_soon() {
                    return Ok(copilot_token.token.clone());
                }
            }
        }

        // 需要刷新
        log::info!("[CopilotAuth] 账号 {account_id} 的 Copilot Token 需要刷新");

        let refresh_lock = self.get_refresh_lock(account_id).await;
        let _refresh_guard = refresh_lock.lock().await;

        // double-check：等待锁期间可能已由其他请求刷新完成
        {
            let tokens = self.copilot_tokens.read().await;
            if let Some(copilot_token) = tokens.get(account_id) {
                if !copilot_token.is_expiring_soon() {
                    return Ok(copilot_token.token.clone());
                }
            }
        }

        // 获取账号的 GitHub token
        let (github_token, domain) = {
            let accounts = self.accounts.read().await;
            let account = accounts
                .get(account_id)
                .ok_or_else(|| CopilotAuthError::AccountNotFound(account_id.to_string()))?;
            (account.github_token.clone(), account.github_domain.clone())
        };

        // 刷新 Copilot token
        self.fetch_copilot_token_with_github_token(&github_token, account_id, &domain)
            .await?;

        // 返回新 token
        let tokens = self.copilot_tokens.read().await;
        tokens.get(account_id).map(|t| t.token.clone()).ok_or(
            CopilotAuthError::CopilotTokenFetchFailed("刷新后仍无令牌".to_string()),
        )
    }

    /// 获取有效的 Copilot Token（向后兼容：使用第一个账号）
    pub async fn get_valid_token(&self) -> Result<String, CopilotAuthError> {
        // 确保迁移完成
        self.ensure_migration_complete().await?;

        match self.resolve_default_account_id().await {
            Some(id) => self.get_valid_token_for_account(&id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    // ==================== 模型与 vendor ====================

    /// 获取指定账号的 Copilot 可用模型列表
    pub async fn fetch_models_for_account(
        &self,
        account_id: &str,
    ) -> Result<Vec<CopilotModel>, CopilotAuthError> {
        self.ensure_migration_complete().await?;

        {
            let models = self.copilot_models.read().await;
            if let Some(cached) = models.get(account_id) {
                return Ok(cached.clone());
            }
        }

        let models = self.fetch_models_for_account_uncached(account_id).await?;
        {
            let mut cache = self.copilot_models.write().await;
            cache.insert(account_id.to_string(), models.clone());
        }
        Ok(models)
    }

    async fn fetch_models_for_account_uncached(
        &self,
        account_id: &str,
    ) -> Result<Vec<CopilotModel>, CopilotAuthError> {
        let copilot_token = self.get_valid_token_for_account(account_id).await?;

        // 使用 get_api_endpoint() 动态解析 Copilot API 基础 URL。
        // 对于 github.com 账号，会查询 /copilot_internal/user 获取 endpoints.api 字段。
        // 对于 GHES 账号，/copilot_internal/user 可能不返回 endpoints——此时
        // get_api_endpoint() 会回退到 copilot_api_base(&domain)，与之前的静态 URL
        // 拼接结果一致。该回退行为是安全且符合预期的。
        let api_base = self.get_api_endpoint(account_id).await;
        let models_url = format!("{}/models", api_base);

        log::info!("[CopilotAuth] 获取账号 {account_id} 的 Copilot 可用模型");

        let response = crate::proxy::http_client::get()
            .get(&models_url)
            .header("Authorization", format!("Bearer {copilot_token}"))
            .header("Content-Type", "application/json")
            .header("copilot-integration-id", COPILOT_INTEGRATION_ID)
            .header("editor-version", COPILOT_EDITOR_VERSION)
            .header("editor-plugin-version", COPILOT_PLUGIN_VERSION)
            .header("user-agent", COPILOT_USER_AGENT)
            .header("x-github-api-version", COPILOT_API_VERSION)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "获取模型列表失败: {status} - {text}"
            )));
        }

        let models_response: CopilotModelsResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        let models: Vec<CopilotModel> = models_response
            .data
            .into_iter()
            .filter(|m| m.model_picker_enabled)
            .map(|m| CopilotModel {
                id: m.id,
                name: m.name,
                vendor: m.vendor,
                model_picker_enabled: m.model_picker_enabled,
            })
            .collect();

        log::info!("[CopilotAuth] 获取到 {} 个可用模型", models.len());

        Ok(models)
    }

    pub async fn get_model_vendor_for_account(
        &self,
        account_id: &str,
        model_id: &str,
    ) -> Result<Option<String>, CopilotAuthError> {
        let models = self.fetch_models_for_account(account_id).await?;
        Ok(models
            .into_iter()
            .find(|model| model.id == model_id)
            .map(|model| model.vendor))
    }

    /// 获取默认账号的 Copilot 可用模型列表
    pub async fn fetch_models(&self) -> Result<Vec<CopilotModel>, CopilotAuthError> {
        self.ensure_migration_complete().await?;

        match self.resolve_default_account_id().await {
            Some(id) => self.fetch_models_for_account(&id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    pub async fn get_model_vendor(
        &self,
        model_id: &str,
    ) -> Result<Option<String>, CopilotAuthError> {
        self.ensure_migration_complete().await?;

        match self.resolve_default_account_id().await {
            Some(id) => self.get_model_vendor_for_account(&id, model_id).await,
            None => Err(CopilotAuthError::GitHubTokenInvalid),
        }
    }

    // ==================== 动态 API endpoint ====================

    /// 获取指定账号的 API 端点（缓存命中直接返回，未命中则从 API 惰性拉取）
    pub async fn get_api_endpoint(&self, account_id: &str) -> String {
        let _ = self.ensure_migration_complete().await;

        {
            let endpoints = self.api_endpoints.read().await;
            if let Some(endpoint) = endpoints.get(account_id) {
                return endpoint.clone();
            }
        }

        // 用锁串行化同一账号的并发拉取，避免对 GitHub API 的重复请求
        let lock = self.get_endpoint_lock(account_id).await;
        let _guard = lock.lock().await;

        // 持锁后二次检查：可能已由其他请求填充
        {
            let endpoints = self.api_endpoints.read().await;
            if let Some(endpoint) = endpoints.get(account_id) {
                return endpoint.clone();
            }
        }

        match self.fetch_and_cache_endpoint(account_id).await {
            Ok(endpoint) => endpoint,
            Err(e) => {
                log::debug!(
                    "[CopilotAuth] 获取账号 {account_id} 动态 API 端点失败: {e}，使用默认值"
                );
                let domain = self.get_account_domain(account_id).await;
                copilot_api_base(&domain)
            }
        }
    }

    /// 获取默认账号的 API 端点
    pub async fn get_default_api_endpoint(&self) -> String {
        let _ = self.ensure_migration_complete().await;

        match self.resolve_default_account_id().await {
            Some(id) => self.get_api_endpoint(&id).await,
            None => {
                // 无账号时回退到 github.com 的默认端点
                copilot_api_base(DEFAULT_GITHUB_DOMAIN)
            }
        }
    }

    async fn fetch_and_cache_endpoint(&self, account_id: &str) -> Result<String, CopilotAuthError> {
        let (github_token, domain) = {
            let accounts = self.accounts.read().await;
            let account = accounts
                .get(account_id)
                .ok_or_else(|| CopilotAuthError::AccountNotFound(account_id.to_string()))?;
            (account.github_token.clone(), account.github_domain.clone())
        };

        log::debug!("[CopilotAuth] 为账号 {account_id} 惰性拉取动态 API 端点");

        let response = crate::proxy::http_client::get()
            .get(copilot_internal_user_url(&domain))
            .header("Authorization", format!("token {github_token}"))
            .header("Content-Type", "application/json")
            .header("editor-version", COPILOT_EDITOR_VERSION)
            .header("editor-plugin-version", COPILOT_PLUGIN_VERSION)
            .header("user-agent", COPILOT_USER_AGENT)
            .header("x-github-api-version", COPILOT_API_VERSION)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        if !response.status().is_success() {
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "获取 API 端点失败: {}",
                response.status()
            )));
        }

        let user_response: CopilotUserResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        let endpoint = match user_response.endpoints {
            Some(endpoints) => endpoints.api,
            None => copilot_api_base(&domain),
        };

        // 缓存端点（包括默认值），避免重复请求
        let mut api_endpoints = self.api_endpoints.write().await;
        api_endpoints.insert(account_id.to_string(), endpoint.clone());
        log::debug!("[CopilotAuth] 账号 {account_id} 已缓存 API 端点");

        Ok(endpoint)
    }

    async fn get_endpoint_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        {
            let locks = self.endpoint_locks.read().await;
            if let Some(lock) = locks.get(account_id) {
                return Arc::clone(lock);
            }
        }

        let mut locks = self.endpoint_locks.write().await;
        Arc::clone(
            locks
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    // ==================== 内部方法 ====================

    fn fallback_default_account_id(
        accounts: &HashMap<String, GitHubAccountData>,
    ) -> Option<String> {
        accounts
            .iter()
            .max_by(|(id_a, a), (id_b, b)| {
                a.authenticated_at
                    .cmp(&b.authenticated_at)
                    .then_with(|| id_b.cmp(id_a))
            })
            .map(|(id, _)| id.clone())
    }

    async fn resolve_default_account_id(&self) -> Option<String> {
        let stored_default = self.default_account_id.read().await.clone();
        let accounts = self.accounts.read().await;

        if let Some(default_id) = stored_default {
            if accounts.contains_key(&default_id) {
                return Some(default_id);
            }
        }

        Self::fallback_default_account_id(&accounts)
    }

    /// 获取指定账号的 GitHub 域名
    async fn get_account_domain(&self, account_id: &str) -> String {
        let accounts = self.accounts.read().await;
        accounts
            .get(account_id)
            .map(|a| a.github_domain.clone())
            .unwrap_or_else(|| DEFAULT_GITHUB_DOMAIN.to_string())
    }

    async fn get_refresh_lock(&self, account_id: &str) -> Arc<Mutex<()>> {
        {
            let refresh_locks = self.refresh_locks.read().await;
            if let Some(lock) = refresh_locks.get(account_id) {
                return Arc::clone(lock);
            }
        }

        let mut refresh_locks = self.refresh_locks.write().await;
        Arc::clone(
            refresh_locks
                .entry(account_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    fn write_store_atomic(&self, content: &str) -> Result<(), CopilotAuthError> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let parent = self
            .storage_path
            .parent()
            .ok_or_else(|| CopilotAuthError::IoError("无效的存储路径".to_string()))?;
        let file_name = self
            .storage_path
            .file_name()
            .ok_or_else(|| CopilotAuthError::IoError("无效的存储文件名".to_string()))?
            .to_string_lossy()
            .to_string();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let tmp_path = parent.join(format!("{file_name}.tmp.{ts}"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            fs::rename(&tmp_path, &self.storage_path)?;
            fs::set_permissions(&self.storage_path, fs::Permissions::from_mode(0o600))?;
        }

        #[cfg(windows)]
        {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.flush()?;

            if self.storage_path.exists() {
                let _ = fs::remove_file(&self.storage_path);
            }
            fs::rename(&tmp_path, &self.storage_path)?;
        }

        Ok(())
    }

    /// 使用指定 token 获取 GitHub 用户信息
    async fn fetch_user_info_with_token(
        &self,
        github_token: &str,
        domain: &str,
    ) -> Result<GitHubUser, CopilotAuthError> {
        let response = crate::proxy::http_client::get()
            .get(github_user_url(domain))
            .header("Authorization", format!("token {github_token}"))
            .header("User-Agent", COPILOT_USER_AGENT)
            .header("Editor-Version", COPILOT_EDITOR_VERSION)
            .header("Editor-Plugin-Version", COPILOT_PLUGIN_VERSION)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        let user: GitHubUser = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        log::info!("[CopilotAuth] 获取用户信息成功: {}", user.login);

        Ok(user)
    }

    /// 使用 GitHub token 获取 Copilot Token
    async fn fetch_copilot_token_with_github_token(
        &self,
        github_token: &str,
        account_id: &str,
        domain: &str,
    ) -> Result<(), CopilotAuthError> {
        log::debug!("[CopilotAuth] 获取账号 {account_id} 的 Copilot Token (domain: {domain})");

        let response = crate::proxy::http_client::get()
            .get(copilot_token_url(domain))
            .header("Authorization", format!("token {github_token}"))
            .header("User-Agent", COPILOT_USER_AGENT)
            .header("Editor-Version", COPILOT_EDITOR_VERSION)
            .header("Editor-Plugin-Version", COPILOT_PLUGIN_VERSION)
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CopilotAuthError::GitHubTokenInvalid);
        }

        if response.status() == reqwest::StatusCode::FORBIDDEN {
            return Err(CopilotAuthError::NoCopilotSubscription);
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(CopilotAuthError::CopilotTokenFetchFailed(format!(
                "{status}: {text}"
            )));
        }

        let token_response: CopilotTokenResponse = response
            .json()
            .await
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        log::info!(
            "[CopilotAuth] 账号 {} 的 Copilot Token 获取成功，过期时间: {}",
            account_id,
            token_response.expires_at
        );

        let copilot_token = CopilotToken {
            token: token_response.token,
            expires_at: token_response.expires_at,
        };

        let mut tokens = self.copilot_tokens.write().await;
        tokens.insert(account_id.to_string(), copilot_token);

        Ok(())
    }

    // ==================== 存储和迁移 ====================

    /// 从磁盘加载（仅加载 token，不发起网络请求）
    fn load_from_disk_sync(&self) -> Result<(), CopilotAuthError> {
        if !self.storage_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.storage_path)?;
        let store: CopilotAuthStore = serde_json::from_str(&content)
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        if store.version >= 2 {
            // v2 多账号格式
            if let Ok(mut accounts) = self.accounts.try_write() {
                *accounts = store.accounts;
                log::info!("[CopilotAuth] 从磁盘加载 {} 个账号", accounts.len());
            }
            if let Ok(mut default_account_id) = self.default_account_id.try_write() {
                *default_account_id = store.default_account_id;
                if default_account_id.is_none() {
                    if let Ok(accounts) = self.accounts.try_read() {
                        *default_account_id = Self::fallback_default_account_id(&accounts);
                    }
                }
            }
        } else if store.github_token.is_some() {
            // v1 单账号格式，标记待迁移
            log::info!("[CopilotAuth] 检测到旧格式，将在首次访问时迁移");
            if let Ok(mut pending) = self.pending_migration.try_write() {
                *pending = store.github_token;
            }
        }

        Ok(())
    }

    /// 确保迁移完成
    async fn ensure_migration_complete(&self) -> Result<(), CopilotAuthError> {
        let pending = {
            let guard = self.pending_migration.read().await;
            guard.clone()
        };

        if let Some(legacy_token) = pending {
            log::info!("[CopilotAuth] 执行旧格式迁移");

            // 获取用户信息
            match self
                .fetch_user_info_with_token(&legacy_token, DEFAULT_GITHUB_DOMAIN)
                .await
            {
                Ok(user) => {
                    let account_id = user.id.to_string();

                    // 尝试获取 Copilot token 验证订阅
                    if let Err(e) = self
                        .fetch_copilot_token_with_github_token(
                            &legacy_token,
                            &account_id,
                            DEFAULT_GITHUB_DOMAIN,
                        )
                        .await
                    {
                        log::warn!("[CopilotAuth] 迁移时验证 Copilot 订阅失败: {e}");
                    }

                    self.persist_migrated_account(legacy_token, user).await?;
                    log::info!("[CopilotAuth] 旧格式迁移完成");
                }
                Err(e) => {
                    log::warn!("[CopilotAuth] 迁移失败，旧 token 可能已失效: {e}");
                }
            }

            // 清除待迁移标记
            {
                let mut pending = self.pending_migration.write().await;
                *pending = None;
            }
        }

        Ok(())
    }

    /// 保存到磁盘
    async fn save_to_disk(&self) -> Result<(), CopilotAuthError> {
        let accounts = self.accounts.read().await.clone();
        let default_account_id = self.resolve_default_account_id().await;

        let store = CopilotAuthStore {
            version: 3,
            accounts,
            default_account_id,
            github_token: None,
        };

        let content = serde_json::to_string_pretty(&store)
            .map_err(|e| CopilotAuthError::ParseError(e.to_string()))?;

        self.write_store_atomic(&content)?;

        log::info!(
            "[CopilotAuth] 保存到磁盘成功（{} 个账号）",
            store.accounts.len()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_copilot_token_expiry() {
        let now = chrono::Utc::now().timestamp();

        // 未过期的 token (1小时后过期，不在60秒缓冲期内)
        let token = CopilotToken {
            token: "test".to_string(),
            expires_at: now + 3600,
        };
        assert!(!token.is_expiring_soon());

        // 即将过期的 token (30秒后过期，在60秒缓冲期内)
        let token = CopilotToken {
            token: "test".to_string(),
            expires_at: now + 30,
        };
        assert!(token.is_expiring_soon());

        // 已过期的 token (也在缓冲期内)
        let token = CopilotToken {
            token: "test".to_string(),
            expires_at: now - 100,
        };
        assert!(token.is_expiring_soon());
    }

    #[test]
    fn test_multi_account_store_serialization() {
        let mut accounts = HashMap::new();
        accounts.insert(
            "12345".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token".to_string(),
                user: GitHubUser {
                    login: "alice".to_string(),
                    id: 12345,
                    avatar_url: Some("https://example.com/alice.png".to_string()),
                },
                authenticated_at: 1700000000,
                github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
            },
        );
        accounts.insert(
            "67890".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token_2".to_string(),
                user: GitHubUser {
                    login: "bob".to_string(),
                    id: 67890,
                    avatar_url: None,
                },
                authenticated_at: 1700000001,
                github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
            },
        );

        let store = CopilotAuthStore {
            version: 3,
            accounts,
            default_account_id: Some("67890".to_string()),
            github_token: None,
        };

        let json = serde_json::to_string_pretty(&store).unwrap();
        let parsed: CopilotAuthStore = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.version, 3);
        assert_eq!(parsed.default_account_id, Some("67890".to_string()));
        assert_eq!(parsed.accounts.len(), 2);
        assert!(parsed.accounts.contains_key("12345"));
        assert!(parsed.accounts.contains_key("67890"));
        assert_eq!(parsed.accounts["12345"].user.login, "alice");
        assert_eq!(parsed.accounts["67890"].user.login, "bob");
    }

    #[test]
    fn test_legacy_format_detection() {
        // 旧格式（v1）
        let legacy_json = r#"{
            "github_token": "gho_legacy_token",
            "authenticated_at": 1700000000
        }"#;

        let store: CopilotAuthStore = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(store.version, 0); // 默认值
        assert!(store.github_token.is_some());
        assert!(store.accounts.is_empty());
    }

    #[test]
    fn test_fallback_default_account_prefers_latest_authenticated() {
        let mut accounts = HashMap::new();
        accounts.insert(
            "12345".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token".to_string(),
                user: GitHubUser {
                    login: "alice".to_string(),
                    id: 12345,
                    avatar_url: None,
                },
                authenticated_at: 1700000000,
                github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
            },
        );
        accounts.insert(
            "67890".to_string(),
            GitHubAccountData {
                github_token: "gho_test_token_2".to_string(),
                user: GitHubUser {
                    login: "bob".to_string(),
                    id: 67890,
                    avatar_url: None,
                },
                authenticated_at: 1700000001,
                github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
            },
        );

        assert_eq!(
            CopilotAuthManager::fallback_default_account_id(&accounts),
            Some("67890".to_string())
        );
    }

    #[tokio::test]
    async fn test_get_model_vendor_from_cache() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        {
            let mut default_account_id = manager.default_account_id.write().await;
            *default_account_id = Some("12345".to_string());
        }
        {
            let mut accounts = manager.accounts.write().await;
            accounts.insert(
                "12345".to_string(),
                GitHubAccountData {
                    github_token: "gho_test".to_string(),
                    user: GitHubUser {
                        login: "alice".to_string(),
                        id: 12345,
                        avatar_url: None,
                    },
                    authenticated_at: 1700000000,
                    github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
                },
            );
        }
        {
            let mut models = manager.copilot_models.write().await;
            models.insert(
                "12345".to_string(),
                vec![
                    CopilotModel {
                        id: "gpt-5.4".to_string(),
                        name: "GPT-5.4".to_string(),
                        vendor: "OpenAI".to_string(),
                        model_picker_enabled: true,
                    },
                    CopilotModel {
                        id: "claude-sonnet-4".to_string(),
                        name: "Claude Sonnet 4".to_string(),
                        vendor: "Anthropic".to_string(),
                        model_picker_enabled: true,
                    },
                ],
            );
        }

        let vendor = manager
            .get_model_vendor_for_account("12345", "gpt-5.4")
            .await
            .unwrap();
        assert_eq!(vendor.as_deref(), Some("OpenAI"));

        let default_vendor = manager.get_model_vendor("claude-sonnet-4").await.unwrap();
        assert_eq!(default_vendor.as_deref(), Some("Anthropic"));
    }

    #[tokio::test]
    async fn test_get_api_endpoint_returns_cached_value() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // 手动设置 api_endpoints 缓存
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.enterprise.example.com".to_string(),
            );
        }

        let endpoint = manager.get_api_endpoint("12345").await;
        assert_eq!(endpoint, "https://copilot-api.enterprise.example.com");
    }

    #[tokio::test]
    async fn test_get_api_endpoint_returns_default_when_not_cached() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let endpoint = manager.get_api_endpoint("99999").await;
        assert_eq!(endpoint, "https://api.githubcopilot.com");
    }

    #[tokio::test]
    async fn test_get_default_api_endpoint_uses_default_account() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        // 设置默认账号
        {
            let mut default_account_id = manager.default_account_id.write().await;
            *default_account_id = Some("12345".to_string());
        }
        // 添加账号数据
        {
            let mut accounts = manager.accounts.write().await;
            accounts.insert(
                "12345".to_string(),
                GitHubAccountData {
                    github_token: "gho_test".to_string(),
                    user: GitHubUser {
                        login: "alice".to_string(),
                        id: 12345,
                        avatar_url: None,
                    },
                    authenticated_at: 1700000000,
                    github_domain: DEFAULT_GITHUB_DOMAIN.to_string(),
                },
            );
        }
        // 设置 API endpoint 缓存
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert(
                "12345".to_string(),
                "https://copilot-api.corp.example.com".to_string(),
            );
        }

        let endpoint = manager.get_default_api_endpoint().await;
        assert_eq!(endpoint, "https://copilot-api.corp.example.com");
    }

    #[tokio::test]
    async fn test_get_api_endpoint_cache_hit_skips_fetch() {
        // 缓存命中时应直接返回，不发起网络请求
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let enterprise_endpoint = "https://copilot-api.enterprise.example.com".to_string();
        {
            let mut api_endpoints = manager.api_endpoints.write().await;
            api_endpoints.insert("12345".to_string(), enterprise_endpoint.clone());
        }

        // 即使没有账号数据，缓存命中也应直接返回
        let endpoint = manager.get_api_endpoint("12345").await;
        assert_eq!(endpoint, enterprise_endpoint);
    }

    #[tokio::test]
    async fn test_get_api_endpoint_returns_default_for_unknown_account() {
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let endpoint = manager.get_api_endpoint("12345").await;
        assert_eq!(endpoint, copilot_api_base(DEFAULT_GITHUB_DOMAIN));
    }

    #[tokio::test]
    async fn test_fetch_and_cache_endpoint_requires_account() {
        // 账号不存在时 fetch_and_cache_endpoint 应返回 AccountNotFound 错误
        let temp_dir = tempdir().unwrap();
        let manager = CopilotAuthManager::new(temp_dir.path().to_path_buf());

        let result = manager.fetch_and_cache_endpoint("nonexistent").await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CopilotAuthError::AccountNotFound(id) => assert_eq!(id, "nonexistent"),
            other => panic!("期望 AccountNotFound 错误，实际: {other:?}"),
        }
    }
}
