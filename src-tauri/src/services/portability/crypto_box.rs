//! 导出包加密封装箱。
//!
//! 双密钥策略（design.md §3）：
//! - 完整备份 `kdf=none`：用系统 Keychain 主密钥直接作为 AES-256-GCM key。
//! - 脱敏迁移 `kdf=argon2id`：用户密码 → Argon2id → 32 字节 key。
//!
//! 压缩在加密前：payload JSON → flate2 gzip → AES-GCM。
//! 解密反向：AES-GCM → gunzip → 反序列化。
//!
//! AES-GCM 输出结构为 `nonce(12) || ciphertext || tag`，与 `services/crypto.rs` 一致。
//! AAD 用包级常量 `PACKAGE_AAD`。

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rand::rngs::OsRng;
use rand::RngCore;
use std::io::{Read, Write};

use super::package::{KdfParams, PACKAGE_AAD};

use crate::services::crypto::b64_decode;
use crate::services::crypto::b64_encode;

/// 生成随机 16 字节 salt（Argon2id）。
pub fn random_salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    salt
}

/// 生成随机 12 字节 nonce（AES-GCM）。
pub fn random_nonce() -> [u8; 12] {
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    nonce
}

/// 弱密码检测：长度 < 8 或全同字符类（仅一种字符类）。
///
/// 返回 Some(警告文案) 表示弱密码，调用方给 UI 警告但不强制阻止。
pub fn weak_password_warning(password: &str) -> Option<&'static str> {
    if password.len() < 8 {
        return Some("密码少于 8 个字符，安全性较低，建议使用更长的密码");
    }
    let mut has_lower = false;
    let mut has_upper = false;
    let mut has_digit = false;
    let mut has_other = false;
    for c in password.chars() {
        if c.is_ascii_lowercase() {
            has_lower = true;
        } else if c.is_ascii_uppercase() {
            has_upper = true;
        } else if c.is_ascii_digit() {
            has_digit = true;
        } else {
            has_other = true;
        }
    }
    let classes = [has_lower, has_upper, has_digit, has_other]
        .iter()
        .filter(|b| **b)
        .count();
    if classes <= 1 {
        return Some("密码仅包含单一字符类，安全性较低，建议混合字母、数字与符号");
    }
    None
}

/// 用 Argon2id 从密码派生 32 字节 key。
pub fn derive_key_argon2id(
    password: &str,
    salt: &[u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
) -> Result<[u8; 32], String> {
    let params = Params::new(m_cost, t_cost, p_cost, Some(32))
        .map_err(|e| format!("Argon2id 参数无效: {}", e))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| format!("Argon2id 派生失败: {}", e))?;
    Ok(out)
}

/// 用给定 key 加密 gzip(payload_bytes)，返回 `nonce || ciphertext`（含 tag）。
///
/// `key` 必须为 32 字节。AAD 为包级常量。
pub fn seal(key: &[u8], payload_bytes: &[u8]) -> Result<(Vec<u8>, [u8; 12]), String> {
    if key.len() != 32 {
        return Err("加密密钥必须为 32 字节".to_string());
    }
    let nonce_bytes = random_nonce();
    // 压缩在加密前。
    let compressed = gzip(payload_bytes)?;

    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("加密初始化失败: {}", e))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(
            nonce,
            Payload {
                msg: &compressed,
                aad: PACKAGE_AAD,
            },
        )
        .map_err(|e| format!("加密失败: {}", e))?;

    let mut out = Vec::with_capacity(12 + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok((out, nonce_bytes))
}

/// 解密 `nonce || ciphertext`（含 tag），返回 gunzip 后的 payload 明文。
///
/// 解密失败（密钥不匹配 / 包损坏）返回可读错误，调用方转 400。
pub fn open(key: &[u8], blob: &[u8]) -> Result<Vec<u8>, String> {
    if key.len() != 32 {
        return Err("解密密钥必须为 32 字节".to_string());
    }
    if blob.len() < 12 {
        return Err("密文过短，无法解密".to_string());
    }
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("解密初始化失败: {}", e))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let compressed = cipher
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad: PACKAGE_AAD,
            },
        )
        .map_err(|_| "解密失败：密钥不匹配或包损坏".to_string())?;
    gunzip(&compressed)
}

/// gzip 压缩。
fn gzip(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(input)
        .map_err(|e| format!("gzip 压缩失败: {}", e))?;
    encoder
        .finish()
        .map_err(|e| format!("gzip 压缩完成失败: {}", e))
}

/// gunzip 解压。
fn gunzip(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = GzDecoder::new(input);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|_| "解压失败：包损坏或格式不正确".to_string())?;
    Ok(out)
}

/// 构造默认 Argon2id 的 `KdfParams`（salt 已生成）。
pub fn default_kdf_params(salt: &[u8; 16]) -> KdfParams {
    KdfParams {
        salt: b64_encode(salt),
        m_cost: KdfParams::DEFAULT_M_COST,
        t_cost: KdfParams::DEFAULT_T_COST,
        p_cost: KdfParams::DEFAULT_P_COST,
    }
}

/// 从 `KdfParams` 复现 Argon2id 派生。
pub fn derive_key_from_params(password: &str, params: &KdfParams) -> Result<[u8; 32], String> {
    let salt = b64_decode(&params.salt)?;
    if salt.len() != 16 {
        return Err("导出包 salt 长度无效".to_string());
    }
    derive_key_argon2id(password, &salt, params.m_cost, params.t_cost, params.p_cost)
}
