//! 导出包容器结构。
//!
//! 定义 `ExportPackage`（版本化容器）、`KdfParams`（Argon2id 参数）与
//! `Payload`（明文数据载荷，各表 `*Export` 行结构）。容器格式见 design.md §2。
//!
//! 关键契约：
//! - `format_version = 1`，不匹配拒绝导入。
//! - 加密算法固定 `AES-256-GCM`。
//! - `kdf = "none"`（完整备份，用系统主密钥）或 `"argon2id"`（脱敏迁移，密码派生）。
//! - 凭据列以加密 BLOB 的 base64 表示装入（完整备份原样装包，脱敏模式置 None）。

use serde::{Deserialize, Serialize};

/// 当前导出包格式版本。
pub const FORMAT_VERSION: u32 = 1;

/// 包级 AAD，固定串，非记录级 AAD。
pub const PACKAGE_AAD: &[u8] = b"agent-switch-export-v1";

/// 导出模式标识。
pub const MODE_FULL_BACKUP: &str = "full_backup";
pub const MODE_PORTABLE: &str = "portable";

/// KDF / 算法标识。
pub const KDF_NONE: &str = "none";
pub const KDF_ARGON2ID: &str = "argon2id";
pub const ALGO_AES_GCM: &str = "AES-256-GCM";

/// 可迁移的 app_metadata 偏好键白名单。
///
/// 只导出偏好类键，排除本机运行状态快照（`last_model_sync_at` /
/// `last_model_sync_error`）。full_backup 与 portable 均只导出白名单偏好键。
/// 后续新增 app_metadata 键默认不导出，需显式加入白名单。
pub const PORTABLE_METADATA_KEYS: &[&str] = &["auto_model_refresh_enabled"];

/// 导出包容器（序列化为 `.asbak` / `.ascfg` 文本）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportPackage {
    /// 容器格式版本，当前 = 1。
    pub format_version: u32,
    /// "full_backup" | "portable"。
    pub mode: String,
    /// 固定 "AES-256-GCM"。
    pub algo: String,
    /// "none"（主密钥）| "argon2id"（密码）。
    pub kdf: String,
    /// 仅 argon2id 模式写入；主密钥模式为 None。
    pub kdf_params: Option<KdfParams>,
    /// base64(12 字节 nonce)。
    pub nonce: String,
    /// 创建时间，ISO 8601。
    pub created_at: String,
    /// 应用版本，`env!("CARGO_PKG_VERSION")`。
    pub app_version: String,
    /// base64(AES-GCM(gzip(payload_json)))。
    pub ciphertext: String,
}

/// Argon2id KDF 参数（脱敏迁移模式）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    /// base64(16 字节随机 salt)。
    pub salt: String,
    /// 内存（KiB），默认 19456（19 MiB）。
    pub m_cost: u32,
    /// 迭代，默认 2。
    pub t_cost: u32,
    /// 并行度，默认 1。
    pub p_cost: u32,
}

impl KdfParams {
    /// 默认 Argon2id 参数（19 MiB / t=2 / p=1）。
    pub const DEFAULT_M_COST: u32 = 19_456;
    pub const DEFAULT_T_COST: u32 = 2;
    pub const DEFAULT_P_COST: u32 = 1;
}

/// 明文 payload：各表数据。不含 request_logs / model_locks / 测试数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payload {
    pub accounts: Vec<AccountExport>,
    pub endpoints: Vec<EndpointExport>,
    pub endpoint_models: Vec<ModelExport>,
    pub model_aliases: Vec<AliasExport>,
    pub route_settings: Vec<RouteSettingExport>,
    pub tool_takeover: Vec<ToolTakeoverExport>,
    /// app_metadata 中可迁移偏好项（白名单）。
    pub ui_settings: Vec<(String, String)>,
}

/// 账号导出行。`credentials_b64` 为加密 BLOB 的 base64（full_backup 原样，portable 为 None）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountExport {
    pub id: String,
    pub name: String,
    pub account_type: String,
    pub platform: String,
    pub status: String,
    /// base64(credentials_encrypted BLOB)，脱敏模式为 None。
    pub credentials_b64: Option<String>,
    pub extra_json: Option<String>,
    pub priority: i64,
}

/// 端点导出行。`api_key_b64` 为加密 BLOB 的 base64（full_backup 原样，portable 为 None）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointExport {
    pub id: String,
    pub account_id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub protocol_type: String,
    /// base64(api_key_encrypted BLOB)，脱敏模式为 None。
    pub api_key_b64: Option<String>,
    pub auth_mode: String,
    pub enabled: bool,
    pub priority: i64,
    pub extra_json: Option<String>,
}

/// 模型导出行（custom + synced）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelExport {
    pub id: String,
    pub endpoint_id: String,
    pub model_name: String,
    pub display_name: String,
    pub source: String,
    pub capabilities: Option<String>,
    pub context_window: Option<i64>,
    pub is_available: bool,
    pub last_seen_at: Option<String>,
}

/// 别名导出行。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasExport {
    pub id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub alias_name: String,
    pub target_endpoint_id: Option<String>,
    pub target_model_name: String,
    pub priority: i64,
    pub enabled: bool,
    pub invalid_reason: Option<String>,
}

/// 路由设置导出行。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSettingExport {
    pub id: String,
    pub label: String,
    pub strategy: String,
    pub protocol_type: String,
    pub failover_enabled: bool,
    pub max_switches: i64,
    pub same_account_retries: i64,
    pub cooldown_multiplier: f64,
}

/// 工具接管状态导出行。仅保留 enabled 标记，导入后强制关闭。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolTakeoverExport {
    pub tool: String,
    /// 仅用于记录原始状态，导入时一律置 0，不写工具配置。
    pub was_enabled: bool,
}
