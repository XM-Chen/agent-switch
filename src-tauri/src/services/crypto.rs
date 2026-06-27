use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rand::RngCore;

/// AES-256-GCM 主密钥字节数。
pub const KEY_LEN: usize = 32;
/// AES-GCM nonce 字节数。
pub const NONCE_LEN: usize = 12;

/// 凭据加密服务。
///
/// 主密钥从 `keychain` 模块获取；加密时使用随机 nonce 和记录 ID 作为 AAD，
/// 防止密文被挪用到其他记录。
pub struct CryptoService {
    master_key: [u8; KEY_LEN],
}

impl CryptoService {
    pub fn new(master_key: [u8; KEY_LEN]) -> Self {
        Self { master_key }
    }

    /// 加密明文，返回 `nonce || ciphertext`（含 tag）。
    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, String> {
        let cipher = Aes256Gcm::new_from_slice(&self.master_key)
            .map_err(|e| format!("加密初始化失败: {}", e))?;

        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(
                nonce,
                aes_gcm::aead::Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|e| format!("加密失败: {}", e))?;

        let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// 解密 `nonce || ciphertext`，返回明文。
    ///
    /// 供后续路由子任务解密凭据使用。
    #[allow(dead_code)]
    pub fn decrypt(&self, blob: &[u8], aad: &[u8]) -> Result<Vec<u8>, String> {
        if blob.len() < NONCE_LEN {
            return Err("密文过短，无法解密".to_string());
        }
        let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
        let cipher = Aes256Gcm::new_from_slice(&self.master_key)
            .map_err(|e| format!("解密初始化失败: {}", e))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(
                nonce,
                aes_gcm::aead::Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map_err(|e| format!("解密失败: {}", e))
    }
}

/// 生成 32 字节随机主密钥。
pub fn generate_master_key() -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

/// Base64 编码工具（供其他模块复用）。
pub fn b64_encode(input: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(input)
}

pub fn b64_decode(input: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| format!("Base64 解码失败: {}", e))
}
