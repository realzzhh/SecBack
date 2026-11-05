//! 与 C++ 实现的逐字节对拍。
//!
//! 输入来自 `format_vectors.txt`——那是**产品自己**（C++ 版）按规范拼出文件、
//! 再用自己的加载路径读回去验证过的向量。所以这里对得上，就意味着
//! "Rust 产出的文件，C++ 版能读"，反之亦然。
//!
//! 手抄十六进制太容易错，所以直接读文件，不抄。

use secback_core::crypto::{derive_key, open, seal};
use secback_core::format::{self, Header, HEADER_SIZE};

const VECTORS: &str = include_str!("format_vectors.txt");

fn field(name: &str) -> String {
    for line in VECTORS.lines() {
        let line = line.trim();
        if line.starts_with('#') || !line.contains('=') {
            continue;
        }
        let (k, v) = line.split_once('=').unwrap();
        if k.trim() == name {
            return v.trim().to_string();
        }
    }
    panic!("向量文件里没有字段 {}", name);
}

fn hex_of(name: &str) -> Vec<u8> {
    hex::decode(field(name)).unwrap_or_else(|_| panic!("{} 不是合法十六进制", name))
}

fn arr<const N: usize>(v: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&v[..N]);
    out
}

#[test]
fn password_and_salt_are_stable() {
    assert_eq!(field("password"), "test-password-1021");
    assert_eq!(field("salt"), "000102030405060708090a0b0c0d0e0f");
    assert_eq!(field("iterations"), "200000");
}

/// 最关键的一条：派生出的主密钥必须与 C++（走 Windows CNG）算出的完全相同。
/// 这条不过，说明口令编码、轮数或盐的用法有偏差，后面全部无从谈起。
#[test]
fn derived_key_matches_cpp() {
    let salt: [u8; 16] = arr(&hex_of("salt"));
    let iterations: u32 = field("iterations").parse().unwrap();
    let key = derive_key(&field("password"), &salt, iterations);
    assert_eq!(
        hex::encode(key),
        field("key"),
        "PBKDF2 结果与 C++ 不一致：检查口令是否按 UTF-8 参与派生、轮数是否来自文件头"
    );
}

/// 用固定 nonce 加密，密文必须逐字节等于 C++ 的结果。
#[test]
fn sealed_blob_matches_cpp() {
    let key: [u8; 32] = arr(&hex_of("key"));
    let nonce: [u8; 12] = arr(&hex_of("nonce"));
    let plain = hex_of("plain");

    let blob = seal(&key, &nonce, &plain).unwrap();
    assert_eq!(
        hex::encode(&blob),
        field("blob"),
        "AES-GCM 结果与 C++ 不一致：检查 nonce 长度(12)、tag 长度(16)、是否用了 AAD"
    );
}

/// 文件头字段必须落在契约规定的位置上。
#[test]
fn header_layout_matches_cpp() {
    let expected = hex_of("header");
    assert_eq!(expected.len(), HEADER_SIZE);

    let h = Header::parse(&expected).unwrap();
    assert_eq!(h.version, 1);
    assert_eq!(h.iterations, 200_000);
    assert_eq!(h.salt, arr::<16>(&hex_of("salt")));
    assert_eq!(h.nonce, arr::<12>(&hex_of("nonce")));
    assert_eq!(h.payload_size, hex_of("blob").len() as u64);

    // 反向：我们拼出来的头必须一模一样
    assert_eq!(h.encode().to_vec(), expected);
}

/// 完整文件 = 头 + 密文负载。
#[test]
fn whole_file_matches_cpp() {
    let header = hex_of("header");
    let blob = hex_of("blob");
    let mut file = header.clone();
    file.extend_from_slice(&blob);
    assert_eq!(hex::encode(&file), field("file"));
}

#[test]
fn can_open_cpp_blob() {
    let key: [u8; 32] = arr(&hex_of("key"));
    let nonce: [u8; 12] = arr(&hex_of("nonce"));
    let blob = hex_of("blob");
    let plain = open(&key, &nonce, &blob).expect("必须能解开 C++ 产出的密文");
    assert_eq!(plain, hex_of("plain"));
}

/// 负载反序列化得到的模型，要和向量生成时写进去的内容一致。
#[test]
fn deserialize_cpp_payload() {
    let plain = hex_of("plain");
    let projects = format::deserialize(&plain).unwrap();
    assert_eq!(projects.len(), 1);

    let p = &projects[0];
    assert_eq!(hex::encode(p.id), "101112131415161718191a1b1c1d1e1f");
    assert_eq!(p.name, "邮箱");
    assert_eq!(p.remark, "跨端测试用");
    assert_eq!(p.created_utc, 132_000_000_000_000_000);
    assert_eq!(p.modified_utc, 133_000_000_000_000_000);
    assert_eq!(p.slots[0].user, "a@corp.com");
    assert_eq!(p.slots[0].pass, "P@ss-1");
    assert_eq!(p.slots[0].remark, "主账号");
    assert!(p.slots[1].is_empty());
    assert!(p.slots[2].is_empty());
}

/// 反过来：Rust 序列化出来的负载，必须和 C++ 的逐字节相同。
/// 只有这条过了，"手机存的文件 PC 能开"才成立。
#[test]
fn serialize_matches_cpp_payload() {
    let plain = hex_of("plain");
    let projects = format::deserialize(&plain).unwrap();
    let back = format::serialize(&projects);
    assert_eq!(hex::encode(&back), field("plain"));
}

#[test]
fn tampered_cpp_blob_rejected() {
    let key: [u8; 32] = arr(&hex_of("key"));
    let nonce: [u8; 12] = arr(&hex_of("nonce"));
    let mut blob = hex_of("blob");
    blob[0] ^= 0x01;
    assert!(open(&key, &nonce, &blob).is_err(), "篡改必须被拒绝");
}

#[test]
fn wrong_password_key_rejected() {
    let salt: [u8; 16] = arr(&hex_of("salt"));
    let bad_key = derive_key("wrong-password", &salt, 200_000);
    let nonce: [u8; 12] = arr(&hex_of("nonce"));
    let blob = hex_of("blob");
    assert!(open(&bad_key, &nonce, &blob).is_err(), "错口令必须被拒绝");
}
