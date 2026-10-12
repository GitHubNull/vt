//! OS 凭据管理器封装（keyring，NFR-S-01）。
//!
//! 最小权限说明（AGENTS.md 3.5）：仅经系统 IPC 访问本应用名下（service =
//! [`SERVICE_NAME`]）的凭据条目，无任何其他系统能力。秘密本体只在进程内
//! 请求构造时使用，绝不进入日志与错误信息。

use crate::error::AppError;

use super::{CredentialBackend, CredentialStore};

/// keyring 服务名（本应用凭据条目的命名空间）。
pub const SERVICE_NAME: &str = "com.vibetype.app";

/// keyring 探测账号：探测 OS 凭据管理器可用性时临时写入后删除。
const PROBE_ACCOUNT: &str = "__vt_probe__";

/// E-PLAT 编号：OS 凭据管理器操作失败。
const ERR_CRED_KEYRING: u32 = 20;

/// keyring 凭据存储（Secret Service / Windows Credential Manager / macOS Keychain）。
pub struct KeyringStore;

/// 探测 OS 凭据管理器是否可用：写入并删除一条探测条目。
pub fn probe() -> bool {
    let Ok(entry) = keyring::Entry::new(SERVICE_NAME, PROBE_ACCOUNT) else {
        return false;
    };
    entry.set_password("probe").is_ok() && entry.delete_credential().is_ok()
}

fn map_set_error(ref_key: &str, err: keyring::Error) -> AppError {
    // 错误信息只含引用键（非秘密），绝不含秘密本体。
    AppError::Platform(
        ERR_CRED_KEYRING,
        format!("OS 凭据管理器写入失败（{ref_key}）: {err}"),
    )
}

fn map_get_error(ref_key: &str, err: keyring::Error) -> AppError {
    match err {
        keyring::Error::NoEntry => AppError::Platform(
            ERR_CRED_KEYRING,
            format!("OS 凭据管理器中不存在该凭据（{ref_key}）"),
        ),
        other => AppError::Platform(
            ERR_CRED_KEYRING,
            format!("OS 凭据管理器读取失败（{ref_key}）: {other}"),
        ),
    }
}

impl CredentialStore for KeyringStore {
    fn backend(&self) -> CredentialBackend {
        CredentialBackend::Keyring
    }

    fn set(&self, ref_key: &str, secret: &str) -> Result<(), AppError> {
        let entry = keyring::Entry::new(SERVICE_NAME, ref_key)
            .map_err(|err| map_set_error(ref_key, err))?;
        entry
            .set_password(secret)
            .map_err(|err| map_set_error(ref_key, err))
    }

    fn get(&self, ref_key: &str) -> Result<Option<String>, AppError> {
        let entry = keyring::Entry::new(SERVICE_NAME, ref_key)
            .map_err(|err| map_get_error(ref_key, err))?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(map_get_error(ref_key, err)),
        }
    }

    fn delete(&self, ref_key: &str) -> Result<(), AppError> {
        let entry = keyring::Entry::new(SERVICE_NAME, ref_key)
            .map_err(|err| map_get_error(ref_key, err))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            // 条目不存在视为成功（幂等删除）。
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(AppError::Platform(
                ERR_CRED_KEYRING,
                format!("OS 凭据管理器删除失败（{ref_key}）: {err}"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_never_contain_secret() {
        // 构造性验证（NFR-S-01/02）：keyring 错误信息（含底层 reason）不得泄露秘密本体。
        let secret = "sk-test-SECRET-9f8e7d6c";
        let err = AppError::Platform(
            ERR_CRED_KEYRING,
            "OS 凭据管理器写入失败（cred:provider:deepseek）: mocked io error".to_string(),
        );
        let text = format!("{err:?}{err}");
        assert!(!text.contains(secret));
    }
}
