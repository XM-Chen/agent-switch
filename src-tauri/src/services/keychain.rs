use keyring::Entry;

/// 系统 Keychain 中主密钥的服务名和账户名。
const SERVICE_NAME: &str = "agent-switch";
const ACCOUNT_NAME: &str = "master-key";

/// 从系统 Keychain 读取主密钥。
///
/// 返回 `Ok(Some(key))` 表示存在；`Ok(None)` 表示尚未创建；
/// `Err` 表示 Keychain 不可用。
pub fn load_master_key() -> Result<Option<[u8; 32]>, String> {
    let entry = Entry::new(SERVICE_NAME, ACCOUNT_NAME)
        .map_err(|e| format!("系统凭据管理器不可用: {}", e))?;

    match entry.get_password() {
        Ok(stored) => {
            let mut key = [0u8; 32];
            let bytes = stored.as_bytes();
            if bytes.len() != 32 {
                // 存储的可能是 base64 字符串。
                match super::crypto::b64_decode(&stored) {
                    Ok(decoded) if decoded.len() == 32 => {
                        key.copy_from_slice(&decoded);
                        return Ok(Some(key));
                    }
                    _ => {
                        return Err("系统凭据管理器中的主密钥格式无效".to_string());
                    }
                }
            }
            key.copy_from_slice(bytes);
            Ok(Some(key))
        }
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("读取主密钥失败: {}", e)),
    }
}

/// 将主密钥写入系统 Keychain。
pub fn save_master_key(key: &[u8; 32]) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, ACCOUNT_NAME)
        .map_err(|e| format!("系统凭据管理器不可用: {}", e))?;
    let encoded = super::crypto::b64_encode(key);
    entry
        .set_password(&encoded)
        .map_err(|e| format!("写入主密钥失败: {}", e))
}

/// 获取或创建主密钥。
///
/// 若 Keychain 中不存在，生成新的 32 字节随机密钥并写入。
/// Keychain 不可用时返回错误，调用方应进入明确降级模式。
pub fn ensure_master_key() -> Result<[u8; 32], String> {
    if let Some(existing) = load_master_key()? {
        return Ok(existing);
    }
    let new_key = super::crypto::generate_master_key();
    save_master_key(&new_key)?;
    tracing::info!("已生成并写入主密钥到系统凭据管理器");
    Ok(new_key)
}
