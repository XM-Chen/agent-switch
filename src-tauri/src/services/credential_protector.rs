//! 网关上游凭据的本机保护。
//!
//! Windows 生产环境使用当前用户作用域 DPAPI。其它平台只允许通过测试注入的
//! 可逆保护器运行迁移，避免把明文伪装成“已加密”持久化。

use crate::error::AppError;

pub(crate) const DPAPI_CURRENT_USER_SCHEME: &str = "dpapi-current-user-v1";

pub(crate) trait CredentialProtector {
    fn scheme(&self) -> &'static str;
    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError>;
    fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError>;
}

#[cfg(target_os = "windows")]
pub(crate) struct PlatformCredentialProtector;

#[cfg(target_os = "windows")]
impl CredentialProtector for PlatformCredentialProtector {
    fn scheme(&self) -> &'static str {
        DPAPI_CURRENT_USER_SCHEME
    }

    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
        use std::ptr;
        use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
        use windows_sys::Win32::Security::Cryptography::{
            CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };

        let length = u32::try_from(plaintext.len())
            .map_err(|_| AppError::Config("上游凭据过长，无法使用 DPAPI 加密".to_string()))?;
        let input = CRYPT_INTEGER_BLOB {
            cbData: length,
            pbData: plaintext.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(AppError::Config(format!(
                "使用 Windows DPAPI 加密上游凭据失败: {}",
                unsafe { GetLastError() }
            )));
        }

        let protected = unsafe {
            let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            LocalFree(output.pbData.cast());
            bytes
        };
        Ok(protected)
    }

    fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
        use std::ptr;
        use windows_sys::Win32::Foundation::{GetLastError, LocalFree};
        use windows_sys::Win32::Security::Cryptography::{
            CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };

        let length = u32::try_from(ciphertext.len())
            .map_err(|_| AppError::Config("加密后的上游凭据过长".to_string()))?;
        let input = CRYPT_INTEGER_BLOB {
            cbData: length,
            pbData: ciphertext.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(AppError::Config(format!(
                "使用 Windows DPAPI 解密上游凭据失败: {}",
                unsafe { GetLastError() }
            )));
        }

        let plaintext = unsafe {
            let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
            LocalFree(output.pbData.cast());
            bytes
        };
        Ok(plaintext)
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) struct PlatformCredentialProtector;

#[cfg(not(target_os = "windows"))]
impl CredentialProtector for PlatformCredentialProtector {
    fn scheme(&self) -> &'static str {
        "unavailable"
    }

    fn protect(&self, _plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
        Err(AppError::Config(
            "当前平台没有可用的本机上游凭据保护器；凭据未迁移".to_string(),
        ))
    }

    fn unprotect(&self, _ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
        Err(AppError::Config(
            "当前平台没有可用的本机上游凭据保护器".to_string(),
        ))
    }
}
