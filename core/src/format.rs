//! 磁盘格式：64 字节明文头 + AES-GCM 密文负载。
//!
//! 这个文件是 `docs/SecBack_格式契约.md` 的 Rust 实现。任何改动都必须同步
//! 那份文档，并且能过 `tests/vectors.rs` 的逐字节对拍——否则 PC 端和手机端
//! 就会互相读不了，而那是最难查的一类故障。

use crate::crypto::{NONCE_LEN, SALT_LEN};

pub const MAGIC: [u8; 8] = [b'S', b'E', b'C', b'B', b'K', 0x00, 0x01, 0x00];
pub const HEADER_SIZE: usize = 64;
pub const FORMAT_VERSION: u16 = 1;
pub const MAX_SLOTS: usize = 3;
pub const MAX_PROJECTS: u32 = 100_000;

const OFF_VERSION: usize = 8;
const OFF_ITERATIONS: usize = 12;
const OFF_SALT: usize = 16;
const OFF_NONCE: usize = 32;
const OFF_PAYLOAD: usize = 48;

pub type ProjectId = [u8; 16];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub user: String,
    pub pass: String,
    pub remark: String,
}

impl Account {
    pub fn empty() -> Self {
        Self {
            user: String::new(),
            pass: String::new(),
            remark: String::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.user.is_empty() && self.pass.is_empty() && self.remark.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: ProjectId,
    /// FILETIME（100ns 自 1601）。只用于合并时判断谁更新，展示时换算成毫秒。
    pub created_utc: u64,
    pub modified_utc: u64,
    pub name: String,
    pub remark: String,
    pub slots: [Account; MAX_SLOTS],
}

impl Project {
    pub fn new_with_id(id: ProjectId) -> Self {
        let now = now_filetime();
        Self {
            id,
            created_utc: now,
            modified_utc: now,
            name: String::new(),
            remark: String::new(),
            slots: [Account::empty(), Account::empty(), Account::empty()],
        }
    }

    pub fn slot_empty(&self, i: usize) -> bool {
        i >= MAX_SLOTS || self.slots[i].is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: u16,
    pub iterations: u32,
    pub salt: [u8; SALT_LEN],
    pub nonce: [u8; NONCE_LEN],
    pub payload_size: u64,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut h = [0u8; HEADER_SIZE];
        h[..8].copy_from_slice(&MAGIC);
        h[OFF_VERSION..OFF_VERSION + 2].copy_from_slice(&self.version.to_le_bytes());
        h[OFF_ITERATIONS..OFF_ITERATIONS + 4].copy_from_slice(&self.iterations.to_le_bytes());
        h[OFF_SALT..OFF_SALT + SALT_LEN].copy_from_slice(&self.salt);
        h[OFF_NONCE..OFF_NONCE + NONCE_LEN].copy_from_slice(&self.nonce);
        h[OFF_PAYLOAD..OFF_PAYLOAD + 8].copy_from_slice(&self.payload_size.to_le_bytes());
        h
    }

    pub fn parse(buf: &[u8]) -> Result<Header, ParseError> {
        if buf.len() < HEADER_SIZE {
            return Err(ParseError::TooSmall);
        }
        if buf[..8] != MAGIC {
            return Err(ParseError::NotVault);
        }

        let version = u16::from_le_bytes([buf[OFF_VERSION], buf[OFF_VERSION + 1]]);
        if version != FORMAT_VERSION {
            return Err(ParseError::Version(version));
        }

        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&buf[OFF_SALT..OFF_SALT + SALT_LEN]);
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&buf[OFF_NONCE..OFF_NONCE + NONCE_LEN]);

        let iterations = u32::from_le_bytes([
            buf[OFF_ITERATIONS],
            buf[OFF_ITERATIONS + 1],
            buf[OFF_ITERATIONS + 2],
            buf[OFF_ITERATIONS + 3],
        ]);
        if iterations == 0 {
            return Err(ParseError::Corrupt);
        }

        let mut ps = [0u8; 8];
        ps.copy_from_slice(&buf[OFF_PAYLOAD..OFF_PAYLOAD + 8]);
        let payload_size = u64::from_le_bytes(ps);

        Ok(Header {
            version,
            iterations,
            salt,
            nonce,
            payload_size,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    TooSmall,
    NotVault,
    Version(u16),
    Corrupt,
}

// ---------------------------------------------------------------- 负载序列化

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// 字符串 = u32(UTF-8 字节数) + UTF-8 内容。
/// 关键：长度是**字节数**，不是字符数。中文一个字 3 字节，写错整个数组就错位。
fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

pub fn serialize(projects: &[Project]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + projects.len() * 128);
    put_u32(&mut out, projects.len() as u32);
    for p in projects {
        out.extend_from_slice(&p.id);
        put_str(&mut out, &p.name);
        put_str(&mut out, &p.remark);
        put_u64(&mut out, p.created_utc);
        put_u64(&mut out, p.modified_utc);
        put_u32(&mut out, MAX_SLOTS as u32);
        for s in &p.slots {
            put_str(&mut out, &s.user);
            put_str(&mut out, &s.pass);
            put_str(&mut out, &s.remark);
        }
    }
    out
}

pub fn deserialize(data: &[u8]) -> Result<Vec<Project>, ParseError> {
    let mut r = Reader::new(data);
    let count = r.u32()?;
    if count > MAX_PROJECTS {
        return Err(ParseError::Corrupt);
    }

    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let mut id = [0u8; 16];
        id.copy_from_slice(r.take(16)?);
        let name = r.string()?;
        let remark = r.string()?;
        let created_utc = r.u64()?;
        let modified_utc = r.u64()?;
        let slots_n = r.u32()?;
        if slots_n as usize != MAX_SLOTS {
            return Err(ParseError::Corrupt);
        }
        let mut slots = [Account::empty(), Account::empty(), Account::empty()];
        for s in slots.iter_mut() {
            s.user = r.string()?;
            s.pass = r.string()?;
            s.remark = r.string()?;
        }
        out.push(Project {
            id,
            created_utc,
            modified_utc,
            name,
            remark,
            slots,
        });
    }

    if !r.at_end() {
        // 尾部还有垃圾字节，说明数据被拼接或格式理解错了——宁可拒绝。
        return Err(ParseError::Corrupt);
    }
    Ok(out)
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        let end = self.pos.checked_add(n).ok_or(ParseError::Corrupt)?;
        if end > self.buf.len() {
            return Err(ParseError::Corrupt);
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, ParseError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, ParseError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn string(&mut self) -> Result<String, ParseError> {
        let n = self.u32()? as usize;
        // 上限保护：防止构造出的超大长度导致内存爆炸
        if n > 64 * 1024 * 1024 {
            return Err(ParseError::Corrupt);
        }
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| ParseError::Corrupt)
    }

    fn at_end(&self) -> bool {
        self.pos == self.buf.len()
    }
}

// ---------------------------------------------------------------- 时间

/// 当前时间转 FILETIME。
pub fn now_filetime() -> u64 {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u64)
        .unwrap_or(0);
    // 11644473600 = 1601-01-01 到 1970-01-01 的秒数
    (unix + 11_644_473_600) * 10_000_000
}

/// FILETIME -> Unix 毫秒。算错不会报错，只会安静地总是选错边，所以单独抽出来测。
pub fn filetime_to_unix_ms(ft: u64) -> i64 {
    (ft / 10_000) as i64 - 11_644_473_600_000
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Project {
        let mut p = Project::new_with_id([0x10; 16]);
        p.name = "项目".into();
        p.remark = "跨机器测试用户".into();
        p.created_utc = 0x01A0_0000_0000_0000;
        p.modified_utc = 0x01A0_0000_0000_0001;
        p.slots[0] = Account {
            user: "a@corp.com".into(),
            pass: "P@ss-1".into(),
            remark: "主账户".into(),
        };
        p
    }

    #[test]
    fn header_roundtrip() {
        let h = Header {
            version: FORMAT_VERSION,
            iterations: 200_000,
            salt: [1u8; 16],
            nonce: [2u8; 12],
            payload_size: 146,
        };
        let bytes = h.encode();
        assert_eq!(bytes.len(), HEADER_SIZE);
        let back = Header::parse(&bytes).unwrap();
        assert_eq!(back.iterations, 200_000);
        assert_eq!(back.salt, [1u8; 16]);
        assert_eq!(back.payload_size, 146);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = [0u8; HEADER_SIZE];
        bytes[..8].copy_from_slice(b"NOTAVAUL");
        assert_eq!(Header::parse(&bytes), Err(ParseError::NotVault));
    }

    #[test]
    fn rejects_future_version() {
        let h = Header {
            version: 2,
            iterations: 200_000,
            salt: [0; 16],
            nonce: [0; 12],
            payload_size: 0,
        };
        assert_eq!(Header::parse(&h.encode()), Err(ParseError::Version(2)));
    }

    #[test]
    fn string_length_is_utf8_bytes() {
        let mut out = Vec::new();
        put_str(&mut out, "项目");
        // 2 个汉字 = 6 字节，长度前缀必须是 6 而不是 2
        assert_eq!(&out[..4], &6u32.to_le_bytes());
        assert_eq!(out.len(), 4 + 6);
    }

    #[test]
    fn serialize_roundtrip() {
        let projects = vec![sample()];
        let bytes = serialize(&projects);
        let back = deserialize(&bytes).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].name, "项目");
        assert_eq!(back[0].slots[0].user, "a@corp.com");
        assert_eq!(back[0].slots[0].pass, "P@ss-1");
    }

    #[test]
    fn empty_vault_roundtrip() {
        let bytes = serialize(&[]);
        assert_eq!(bytes, vec![0, 0, 0, 0]);
        assert!(deserialize(&bytes).unwrap().is_empty());
    }

    #[test]
    fn truncated_payload_rejected() {
        let bytes = serialize(&vec![sample()]);
        assert!(deserialize(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn trailing_garbage_rejected() {
        let mut bytes = serialize(&[]);
        bytes.push(0xAA);
        assert!(deserialize(&bytes).is_err());
    }

    #[test]
    fn empty_slots_preserved() {
        let mut p = Project::new_with_id([0; 16]);
        p.name = "x".into();
        let bytes = serialize(&vec![p]);
        let back = deserialize(&bytes).unwrap();
        assert!(back[0].slots[0].is_empty());
        assert_eq!(back[0].slots[2].pass, "");
    }

    #[test]
    fn filetime_conversion() {
        // 2020-01-01 00:00:00 UTC = 1577836800000 ms
        let ft = (1_577_836_800u64 + 11_644_473_600) * 10_000_000;
        assert_eq!(filetime_to_unix_ms(ft), 1_577_836_800_000);
    }
}
