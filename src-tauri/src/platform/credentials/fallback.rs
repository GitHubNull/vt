//! Linux 无 Secret Service 的降级加密存储（tech/08 8.3，NFR-S-01）。
//!
//! 保护级别（设置页须明示，见 [`CredentialBackend::protection_level`]）：
//! 机器绑定的对称加密文件——密钥派生自机器标识（/etc/machine-id 等），密文存于
//! 应用数据目录。可抵御密文文件被直接拷贝 / 查看（异机不可解），但同机可读取
//! 机器标识的进程理论上可解密，**保护级别低于 OS 凭据管理器**。
//!
//! 加密方案：PBKDF2-HMAC-SHA256（21 万次迭代）派生 64 字节密钥材料，
//! 前 32 字节为 AES-256-CBC 加密密钥，后 32 字节为 HMAC-SHA256 完整性密钥
//! （encrypt-then-MAC，篡改密文会被检出）。
//!
//! 文件格式（JSON，字段均为 base64）：
//! ```json
//! {"version":1,"kdf":"pbkdf2-hmac-sha256","iterations":210000,
//!  "salt":"…","cipher":"aes-256-cbc","iv":"…","ct":"…","mac":"…"}
//! ```

use std::path::{Path, PathBuf};

use aes::Aes256;
use cbc::cipher::{Block, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use cbc::{Decryptor, Encryptor};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::AppError;

use super::{CredentialBackend, CredentialStore};

/// E-PLAT 编号：降级加密存储 I/O 失败。
const ERR_CRED_IO: u32 = 21;
/// E-PLAT 编号：降级加密 / 解密 / 完整性校验失败。
const ERR_CRED_CRYPTO: u32 = 22;

/// KDF 迭代次数（OWASP 2023 对 PBKDF2-HMAC-SHA256 的建议值）。
const PBKDF2_ITERATIONS: u32 = 210_000;
/// KDF 输出长度：32 字节加密密钥 + 32 字节 MAC 密钥。
const KEY_MATERIAL_LEN: usize = 64;
/// 加密密钥 = 派生材料前 32 字节。
const ENC_KEY_LEN: usize = 32;
/// 应用域分离串：与机器标识拼接作为 KDF 口令，避免跨应用复用同一派生。
const DOMAIN: &[u8] = b"vibetype/credentials/v1";

type Aes256CbcEnc = Encryptor<Aes256>;
type Aes256CbcDec = Decryptor<Aes256>;
type HmacSha256 = Hmac<Sha256>;

/// 降级加密文件存储。
pub struct FallbackStore {
    path: PathBuf,
    machine_secret: Vec<u8>,
}

impl FallbackStore {
    /// `path`：加密文件位置（数据目录下）；`machine_secret`：机器标识材料。
    pub fn new(path: PathBuf, machine_secret: Vec<u8>) -> Self {
        Self {
            path,
            machine_secret,
        }
    }

    /// 加密文件路径（测试 / 诊断用；不返回任何秘密内容）。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 读取全部条目并解密；文件不存在视为空表。
    fn load_map(&self) -> Result<serde_json::Map<String, serde_json::Value>, AppError> {
        let content = match std::fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(serde_json::Map::new());
            }
            Err(err) => {
                return Err(AppError::Platform(
                    ERR_CRED_IO,
                    format!("读取降级凭据文件失败: {err}"),
                ));
            }
        };
        let envelope: Envelope = serde_json::from_str(&content).map_err(|err| {
            AppError::Platform(
                ERR_CRED_IO,
                format!("降级凭据文件格式损坏（非本应用写入或已损坏）: {err}"),
            )
        })?;
        let payload = envelope.decrypt(&self.derived_password())?;
        let map: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&payload)
            .map_err(|err| {
                AppError::Platform(ERR_CRED_CRYPTO, format!("凭据明文解析失败: {err}"))
            })?;
        Ok(map)
    }

    /// 加密并原子写入全部条目（tmp → rename，与 tech/08 8.2 同思路）。
    fn save_map(&self, map: &serde_json::Map<String, serde_json::Value>) -> Result<(), AppError> {
        let payload = serde_json::to_vec(map)
            .map_err(|err| AppError::Platform(ERR_CRED_CRYPTO, format!("凭据序列化失败: {err}")))?;
        let envelope = Envelope::encrypt(&payload, &self.derived_password())?;
        let parent = self
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        std::fs::create_dir_all(&parent).map_err(|err| {
            AppError::Platform(
                ERR_CRED_IO,
                format!("创建降级凭据目录失败({}) : {err}", parent.display()),
            )
        })?;
        let tmp = self.path.with_extension("enc.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec(&envelope).map_err(|err| {
                AppError::Platform(ERR_CRED_CRYPTO, format!("凭据封套序列化失败: {err}"))
            })?,
        )
        .and_then(|_| std::fs::rename(&tmp, &self.path))
        .map_err(|err| {
            let _ = std::fs::remove_file(&tmp);
            AppError::Platform(ERR_CRED_IO, format!("写入降级凭据文件失败: {err}"))
        })
    }

    /// KDF 口令 = 域分离串 || 机器标识（机器绑定：换机后密文不可解）。
    fn derived_password(&self) -> Vec<u8> {
        let mut password = Vec::with_capacity(DOMAIN.len() + self.machine_secret.len());
        password.extend_from_slice(DOMAIN);
        password.extend_from_slice(&self.machine_secret);
        password
    }
}

impl CredentialStore for FallbackStore {
    fn backend(&self) -> CredentialBackend {
        CredentialBackend::EncryptedFile
    }

    fn set(&self, ref_key: &str, secret: &str) -> Result<(), AppError> {
        let mut map = self.load_map()?;
        map.insert(
            ref_key.to_string(),
            serde_json::Value::String(secret.to_string()),
        );
        self.save_map(&map)
    }

    fn get(&self, ref_key: &str) -> Result<Option<String>, AppError> {
        let map = self.load_map()?;
        Ok(map
            .get(ref_key)
            .and_then(|value| value.as_str())
            .map(str::to_string))
    }

    fn delete(&self, ref_key: &str) -> Result<(), AppError> {
        let mut map = self.load_map()?;
        if map.remove(ref_key).is_some() {
            self.save_map(&map)?;
        }
        Ok(())
    }
}

/// 加密文件封套（磁盘格式，见模块文档）。
#[derive(serde::Serialize, serde::Deserialize)]
struct Envelope {
    version: u32,
    kdf: String,
    iterations: u32,
    salt: String,
    cipher: String,
    iv: String,
    ct: String,
    mac: String,
}

impl Envelope {
    fn encrypt(payload: &[u8], password: &[u8]) -> Result<Self, AppError> {
        let mut rng = rand::thread_rng();
        use rand::RngCore;
        let mut salt = [0u8; 16];
        let mut iv = [0u8; 16];
        rng.fill_bytes(&mut salt);
        rng.fill_bytes(&mut iv);

        let key_material = derive_key_material(password, &salt);
        // 手动 PKCS7 填充 + 逐块 CBC 加密（cipher 0.4 的 padded 便捷 API 跨版本
        // 签名不稳定，这里用最基础的 encrypt_blocks 保证依赖小版本升级不破坏）。
        let pad = 16 - (payload.len() % 16);
        let mut buffer = Vec::with_capacity(payload.len() + pad);
        buffer.extend_from_slice(payload);
        buffer.resize(payload.len() + pad, pad as u8);
        let mut enc =
            Aes256CbcEnc::new_from_slices(&key_material[..ENC_KEY_LEN], &iv).map_err(crypto_err)?;
        for chunk in buffer.chunks_exact_mut(16) {
            enc.encrypt_blocks_mut(std::slice::from_mut(Block::<Aes256>::from_mut_slice(chunk)));
        }
        let ct = buffer;
        let mac = compute_mac(&key_material[ENC_KEY_LEN..], &iv, &ct);

        Ok(Self {
            version: 1,
            kdf: "pbkdf2-hmac-sha256".to_string(),
            iterations: PBKDF2_ITERATIONS,
            salt: base64_encode(&salt),
            cipher: "aes-256-cbc".to_string(),
            iv: base64_encode(&iv),
            ct: base64_encode(&ct),
            mac: base64_encode(&mac),
        })
    }

    fn decrypt(&self, password: &[u8]) -> Result<Vec<u8>, AppError> {
        if self.version != 1 {
            return Err(AppError::Platform(
                ERR_CRED_CRYPTO,
                format!("不支持的凭据文件版本: {}", self.version),
            ));
        }
        let salt = base64_decode(&self.salt)?;
        let iv = base64_decode(&self.iv)?;
        let ct = base64_decode(&self.ct)?;
        let mac = base64_decode(&self.mac)?;

        let key_material = derive_key_material(password, &salt);
        let expected = compute_mac(&key_material[ENC_KEY_LEN..], &iv, &ct);
        // 恒定时间比较，避免 MAC 侧信道。
        if !mac_ct_eq(&mac, &expected) {
            return Err(AppError::Platform(
                ERR_CRED_CRYPTO,
                "凭据文件完整性校验失败（文件被篡改或机器标识已变更）".to_string(),
            ));
        }
        let mut ct_buffer = ct.clone();
        let mut dec =
            Aes256CbcDec::new_from_slices(&key_material[..ENC_KEY_LEN], &iv).map_err(crypto_err)?;
        for chunk in ct_buffer.chunks_exact_mut(16) {
            dec.decrypt_blocks_mut(std::slice::from_mut(Block::<Aes256>::from_mut_slice(chunk)));
        }
        let plaintext = unpad_pkcs7(&ct_buffer)?;
        Ok(plaintext.to_vec())
    }
}

/// PKCS7 去填充：尾字节为填充长度，且填充字节必须全部一致；非法即密文损坏。
fn unpad_pkcs7(padded: &[u8]) -> Result<&[u8], AppError> {
    let Some(&last) = padded.last() else {
        return Err(AppError::Platform(
            ERR_CRED_CRYPTO,
            "凭据解密失败（密钥或密文损坏）".to_string(),
        ));
    };
    let pad = last as usize;
    let valid = pad > 0
        && pad <= 16
        && pad <= padded.len()
        && padded[padded.len() - pad..]
            .iter()
            .all(|&byte| byte == last);
    if !valid {
        return Err(AppError::Platform(
            ERR_CRED_CRYPTO,
            "凭据解密失败（密钥或密文损坏）".to_string(),
        ));
    }
    Ok(&padded[..padded.len() - pad])
}

fn crypto_err(err: cbc::cipher::InvalidLength) -> AppError {
    AppError::Platform(ERR_CRED_CRYPTO, format!("密钥 / IV 长度非法: {err}"))
}

fn derive_key_material(password: &[u8], salt: &[u8]) -> [u8; KEY_MATERIAL_LEN] {
    let mut out = [0u8; KEY_MATERIAL_LEN];
    pbkdf2::pbkdf2_hmac::<Sha256>(password, salt, PBKDF2_ITERATIONS, &mut out);
    out
}

fn compute_mac(mac_key: &[u8], iv: &[u8], ct: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(mac_key).expect("HMAC 接受任意长度密钥");
    mac.update(iv);
    mac.update(ct);
    mac.finalize().into_bytes().to_vec()
}

fn mac_ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn base64_decode(text: &str) -> Result<Vec<u8>, AppError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|err| AppError::Platform(ERR_CRED_CRYPTO, format!("base64 解码失败: {err}")))
}

/// 机器标识材料（机器绑定密钥派生的输入）。
///
/// - Linux：/etc/machine-id（无桌面 Secret Service 的主要降级场景），
///   回退 /var/lib/dbus/machine-id，再回退主机名 + 用户名；
/// - 其他平台：OS + 主机名 + 用户名组合（正常不会走到——keyring 可用）。
pub fn machine_secret() -> Vec<u8> {
    for candidate in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(id) = std::fs::read_to_string(candidate) {
            let id = id.trim();
            if !id.is_empty() {
                return id.as_bytes().to_vec();
            }
        }
    }
    let mut secret = std::env::consts::OS.as_bytes().to_vec();
    if let Ok(host) = std::env::var("HOSTNAME") {
        secret.extend_from_slice(host.as_bytes());
    }
    if let Ok(user) = std::env::var("USER") {
        secret.extend_from_slice(user.as_bytes());
    }
    secret
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> (FallbackStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "vt-cred-fallback-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("测试临时目录");
        (
            FallbackStore::new(dir.join("secrets.enc"), b"test-machine-id".to_vec()),
            dir,
        )
    }

    #[test]
    fn set_get_delete_roundtrip() {
        let (store, dir) = temp_store("roundtrip");
        store
            .set("cred:provider:deepseek", "sk-live-key")
            .expect("写入");
        assert_eq!(
            store
                .get("cred:provider:deepseek")
                .expect("读取")
                .as_deref(),
            Some("sk-live-key")
        );
        store.delete("cred:provider:deepseek").expect("删除");
        assert_eq!(store.get("cred:provider:deepseek").expect("再读"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ciphertext_file_never_contains_plaintext_secret() {
        // 构造性验证（NFR-S-01/02）：密文文件字节中不得出现秘密明文。
        let (store, dir) = temp_store("cipher");
        let secret = "sk-live-SECRET-0123456789abcdef";
        store.set("cred:provider:openai", secret).expect("写入");
        let raw = std::fs::read(store.path()).expect("读取密文文件");
        let text = String::from_utf8_lossy(&raw);
        assert!(
            !text.contains(secret),
            "降级加密文件中不得出现秘密明文: {text}"
        );
        assert_eq!(
            store.get("cred:provider:openai").expect("读取").as_deref(),
            Some(secret),
            "加解密往返必须保真"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_machine_secret_fails_to_decrypt() {
        // 机器绑定：异机（异机器标识）拿到密文文件也无法解密。
        let (store, dir) = temp_store("binding");
        store.set("cred:provider:kimi", "kimi-key").expect("写入");
        let foreign =
            FallbackStore::new(store.path().to_path_buf(), b"another-machine-id".to_vec());
        let err = foreign
            .get("cred:provider:kimi")
            .expect_err("异机必须解密失败");
        assert_eq!(err.code(), "E-PLAT-022");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        // 密文被篡改：HMAC 完整性校验必须拒绝，而非静默返回脏数据。
        let (store, dir) = temp_store("tamper");
        store.set("cred:provider:qwen", "qwen-key").expect("写入");
        let mut envelope: Envelope =
            serde_json::from_slice(&std::fs::read(store.path()).expect("读取")).expect("解析封套");
        let mut ct = base64_decode(&envelope.ct).expect("解码密文");
        ct[0] ^= 0x01;
        envelope.ct = base64_encode(&ct);
        std::fs::write(store.path(), serde_json::to_vec(&envelope).expect("序列化"))
            .expect("写回篡改文件");
        let err = store.get("cred:provider:qwen").expect_err("篡改必须被拒绝");
        assert_eq!(err.code(), "E-PLAT-022");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
