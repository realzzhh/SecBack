//! 密码学原语。
//!
//! 与 C++ 版保持完全一致的行为：
//!   PBKDF2-HMAC-SHA256（轮数从文件头读） -> AES-256-GCM（12B nonce，16B tag，无 AAD）
//!
//! 选纯 Rust 实现而不是调用系统 CNG，是为了同一份代码能同时在 Windows / macOS /
//! Linux / Android / iOS 上产出完全相同的字节——这是"密库跨设备互通"的前提。

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use sha2::Sha256;
use zeroize::Zeroize;

pub const KEY_LEN: usize = 32;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
pub const ITERATIONS: u32 = 200_000;

/// 口令 -> 主密钥。口令按 **UTF-8 字节**参与派生（不是 UTF-16），
/// 否则同一个人在 Windows 和 Android 上输入同样的字会得到不同的密钥。
pub fn derive_key(password: &str, salt: &[u8; SALT_LEN], iterations: u32) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, iterations, &mut key);
    key
}

/// 加密。返回 ciphertext || tag(16B)，与格式契约一致。
pub fn seal(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    plain: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError::BadKey)?;
    cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plain,
                aad: &[],
            },
        )
        .map_err(|_| CryptoError::SealFailed)
}

/// 解密。失败即"密钥不对"或"数据被改过"，两者故意不做区分。
pub fn open(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    blob: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError::BadKey)?;
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: blob,
                aad: &[],
            },
        )
        .map_err(|_| CryptoError::AuthFailed)
}

/// 系统熵源。失败就返回 None，调用方必须放弃而不是退化成固定值。
pub fn random_bytes(n: usize) -> Option<Vec<u8>> {
    let mut v = vec![0u8; n];
    getrandom::getrandom(&mut v).ok()?;
    Some(v)
}

pub fn random_fixed<const N: usize>() -> Option<[u8; N]> {
    let mut out = [0u8; N];
    getrandom::getrandom(&mut out).ok()?;
    Some(out)
}

/// 擦内存。zeroize 会插入编译器屏障，不会被优化掉。
pub fn wipe(buf: &mut [u8]) {
    buf.zeroize();
}

pub fn wipe_string(s: &mut String) {
    // SAFETY: 只是就地覆盖字节，不改变长度和编码（全 0 是合法的 UTF-8）
    unsafe {
        let bytes = s.as_bytes_mut();
        bytes.zeroize();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    BadKey,
    SealFailed,
    AuthFailed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_key_is_stable() {
        // 只验证"同样的输入永远给同样的输出"，与 C++ 的逐字节对拍在
        // tests/vectors.rs 里做（那里直接读 format_vectors.txt，避免手抄）。
        let salt = [0xABu8; SALT_LEN];
        let a = derive_key("same-password", &salt, 1000);
        let b = derive_key("same-password", &salt, 1000);
        assert_eq!(a, b);

        let c = derive_key("same-password", &salt, 1001);
        assert_ne!(a, c, "轮数变了密钥必须变");
    }

    #[test]
    fn roundtrip() {
        let key = [7u8; KEY_LEN];
        let nonce = [9u8; NONCE_LEN];
        let blob = seal(&key, &nonce, b"hello").unwrap();
        assert_eq!(blob.len(), 5 + TAG_LEN, "密文尾部必须带 16 字节 tag");
        assert_eq!(open(&key, &nonce, &blob).unwrap(), b"hello");
    }

    #[test]
    fn empty_plaintext_ok() {
        let key = [3u8; KEY_LEN];
        let nonce = [4u8; NONCE_LEN];
        let blob = seal(&key, &nonce, b"").unwrap();
        assert_eq!(blob.len(), TAG_LEN);
        assert_eq!(open(&key, &nonce, &blob).unwrap(), b"");
    }

    #[test]
    fn tampered_blob_rejected() {
        let key = [7u8; KEY_LEN];
        let nonce = [9u8; NONCE_LEN];
        let mut blob = seal(&key, &nonce, b"hello").unwrap();
        blob[0] ^= 0xff;
        assert!(open(&key, &nonce, &blob).is_err());
    }

    #[test]
    fn truncated_tag_rejected() {
        let key = [7u8; KEY_LEN];
        let nonce = [9u8; NONCE_LEN];
        let blob = seal(&key, &nonce, b"hello").unwrap();
        assert!(open(&key, &nonce, &blob[..blob.len() - 1]).is_err());
    }

    #[test]
    fn wrong_key_rejected() {
        let nonce = [9u8; NONCE_LEN];
        let blob = seal(&[1u8; KEY_LEN], &nonce, b"hello").unwrap();
        assert!(open(&[2u8; KEY_LEN], &nonce, &blob).is_err());
    }

    #[test]
    fn random_is_not_constant() {
        let a = random_fixed::<12>().unwrap();
        let b = random_fixed::<12>().unwrap();
        assert_ne!(a, b);
    }
}
