//! 秘密凭据存储（T-531，SPEC: doc/specs/v0.5/T-531-ai-providers.md）。
//!
//! 密钥 / 令牌只进 OS 凭据管理器（NFR-S-01）：优先 `keyring`（Linux Secret Service /
//! Windows 凭据管理器 / macOS Keychain）；Linux 无 Secret Service 时降级为机器绑定的
//! 对称加密文件（tech/08 8.3），保护级别低于 OS 凭据管理器，必须向用户明示。
//!
//! 安全红线：
//! - 秘密本体（API Key / 令牌）永不写入日志、错误信息、配置与导出文件（NFR-S-02）；
//! - `get` 只允许服务层在发起请求的进程内使用，命令层不得把秘密回传前端；
//! - 错误信息只含操作结果与原因，不含秘密内容。
//!
//! 最小权限说明（AGENTS.md 3.5）：本模块仅在用户配置 AI 供应商 / MCP 令牌时
//! 访问 OS 凭据管理器或数据目录下的加密文件，无其他系统能力。

mod fallback;
mod keyring_store;

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

use crate::error::AppError;

pub use fallback::FallbackStore;
pub use keyring_store::{KeyringStore, SERVICE_NAME};

/// 凭据后端类型（命令层可查询，用于设置页明示保护级别）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialBackend {
    /// OS 凭据管理器（keyring）。
    Keyring,
    /// 降级：机器绑定对称加密文件（Linux 无 Secret Service）。
    EncryptedFile,
}

impl CredentialBackend {
    /// 保护级别说明（用户可见，tech/08 8.3"明示保护级别"要求）。
    pub fn protection_level(self) -> &'static str {
        match self {
            Self::Keyring => "OS 凭据管理器：秘密由系统加密保管，应用仅可读写本应用名下的条目",
            Self::EncryptedFile => {
                "降级加密（低于 OS 凭据管理器）：秘密经机器标识派生密钥加密后存于本机数据目录，\
                 可抵御文件被直接拷贝 / 查看；但可读取本机机器标识的同一用户进程理论上可解密，\
                 建议启用系统全盘加密获得完整保护"
            }
        }
    }
}

/// 后端信息（命令返回给设置页展示）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CredentialBackendInfo {
    pub backend: CredentialBackend,
    pub protection_level: &'static str,
}

/// 凭据存储抽象：引用键（如 `cred:provider:deepseek`）→ 秘密本体。
///
/// 实现必须保证错误信息中不出现秘密内容。
pub trait CredentialStore: Send + Sync {
    /// 当前后端类型。
    fn backend(&self) -> CredentialBackend;

    /// 保存 / 覆盖一条秘密。
    fn set(&self, ref_key: &str, secret: &str) -> Result<(), AppError>;

    /// 读取秘密；条目不存在返回 `Ok(None)`（本地供应商无 Key 时走此分支）。
    fn get(&self, ref_key: &str) -> Result<Option<String>, AppError>;

    /// 删除秘密；条目不存在视为成功。
    fn delete(&self, ref_key: &str) -> Result<(), AppError>;
}

/// 选择本机默认可用的凭据存储：先探测 OS 凭据管理器，不可用则降级加密文件。
///
/// - `data_dir`：应用数据目录（降级加密文件的存放位置，tech/08 8.1）。
pub fn default_credential_store(data_dir: &Path) -> Arc<dyn CredentialStore> {
    if keyring_store::probe() {
        tracing::info!(backend = "keyring", "凭据存储使用 OS 凭据管理器");
        Arc::new(KeyringStore)
    } else {
        tracing::warn!(
            backend = "encrypted_file",
            "OS 凭据管理器不可用，凭据存储降级为机器绑定加密文件（保护级别较低，已按 tech/08 8.3 明示）"
        );
        Arc::new(FallbackStore::new(
            data_dir.join("credentials").join("secrets.enc"),
            fallback::machine_secret(),
        ))
    }
}

/// 后端信息查询（命令层用）。
pub fn backend_info(store: &Arc<dyn CredentialStore>) -> CredentialBackendInfo {
    CredentialBackendInfo {
        backend: store.backend(),
        protection_level: store.backend().protection_level(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protection_level_mentions_degraded_for_fallback() {
        // tech/08 8.3：降级方案的保护级别必须可被用户识别（含"降级"提示）。
        let text = CredentialBackend::EncryptedFile.protection_level();
        assert!(text.contains("降级"), "降级保护级别须明示: {text}");
        assert!(CredentialBackend::Keyring
            .protection_level()
            .contains("OS 凭据管理器"));
    }

    #[test]
    fn default_store_falls_back_without_secret_service() {
        // 本 CI/开发机无 Secret Service 时：默认后端为降级加密文件，且可用。
        let dir = std::env::temp_dir().join(format!("vt-cred-default-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("测试临时目录");
        let store = default_credential_store(&dir);
        if store.backend() == CredentialBackend::EncryptedFile {
            store.set("test:ref", "s3cr3t-value").expect("写入降级存储");
            assert_eq!(
                store.get("test:ref").expect("读取降级存储").as_deref(),
                Some("s3cr3t-value")
            );
            store.delete("test:ref").expect("删除降级存储");
            assert_eq!(store.get("test:ref").expect("再读"), None);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
