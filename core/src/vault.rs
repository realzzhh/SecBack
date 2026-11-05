//! 密库读写层：加载、保存、改口令、预检、原子落盘。
//!
//! 两个刻意的取舍（与 C++ 版一致，不能改）：
//!   1. nonce 每次写盘重新生成 —— 内容一字未改，磁盘字节也完全不同，
//!      不会泄漏"这次只动了备注"这类元信息。salt 只在改口令时轮换。
//!   2. 写盘一律 .tmp + 原子重命名 —— 半途崩了最多留个 .tmp，
//!      不会把已经存好的数据写坏。

use crate::crypto::{self, KEY_LEN, NONCE_LEN, SALT_LEN, TAG_LEN};
use crate::format::{self, Header, Project, FORMAT_VERSION, HEADER_SIZE};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Err {
    Missing,
    BadPassword,
    Corrupt,
    Io,
    CryptoFail,
    TooWeak,
    Locked,
}

impl Err {
    /// 面向用户的中文说明。不出现内部组件名。
    pub fn text(self) -> &'static str {
        match self {
            Err::Missing => "密库文件不存在",
            Err::BadPassword => "密码不正确",
            Err::Corrupt => "密库文件已损坏或不是本程序的文件",
            Err::Io => "读写文件失败",
            Err::CryptoFail => "加密运算失败",
            Err::TooWeak => "密码不能为空",
            Err::Locked => "密库未解锁",
        }
    }
}

impl std::fmt::Display for Err {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.text())
    }
}

impl std::error::Error for Err {}

/// 只读文件头做的前置判断结果。不需要口令、不改变任何状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus {
    Ok,
    Missing,
    Io,
    TooSmall,
    NotVault,
    VersionNewer(u16),
    VersionOlder(u16),
    Corrupt,
}

#[derive(Debug, Clone)]
pub struct ProbeInfo {
    pub status: ProbeStatus,
    pub version: u16,
    pub iterations: u32,
    pub payload_size: u64,
    pub file_size: u64,
}

pub struct Store {
    projects: Vec<Project>,
    key: [u8; KEY_LEN],
    salt: [u8; SALT_LEN],
    iterations: u32,
    unlocked: bool,
}

impl Store {
    pub fn new() -> Self {
        Self {
            projects: Vec::new(),
            key: [0u8; KEY_LEN],
            salt: [0u8; SALT_LEN],
            iterations: crypto::ITERATIONS,
            unlocked: false,
        }
    }

    pub fn projects(&self) -> &[Project] {
        &self.projects
    }

    pub fn projects_mut(&mut self) -> &mut Vec<Project> {
        &mut self.projects
    }

    pub fn is_unlocked(&self) -> bool {
        self.unlocked
    }

    /// 抹掉内存里的模型和主密钥。锁定时必须调用。
    pub fn clear_secure(&mut self) {
        for p in self.projects.iter_mut() {
            for s in p.slots.iter_mut() {
                crypto::wipe_string(&mut s.user);
                crypto::wipe_string(&mut s.pass);
                crypto::wipe_string(&mut s.remark);
            }
            crypto::wipe_string(&mut p.name);
            crypto::wipe_string(&mut p.remark);
        }
        self.projects.clear();
        crypto::wipe(&mut self.key);
        crypto::wipe(&mut self.salt);
        self.unlocked = false;
    }

    /// 文件不存在时用该口令创建空库；存在则正常校验。
    pub fn load_or_create(
        &mut self,
        path: &Path,
        password: &str,
        create_if_missing: bool,
    ) -> Result<bool, Err> {
        if path.exists() {
            self.load(path, password)?;
            return Ok(false);
        }
        if !create_if_missing {
            return Err(Err::Missing);
        }
        if password.is_empty() {
            return Err(Err::TooWeak);
        }

        let salt = crypto::random_fixed::<SALT_LEN>().ok_or(Err::CryptoFail)?;
        self.salt = salt;
        self.iterations = crypto::ITERATIONS;
        self.key = crypto::derive_key(password, &salt, self.iterations);
        self.projects.clear();
        self.unlocked = true;

        self.save(path)?;
        Ok(true)
    }

    pub fn load(&mut self, path: &Path, password: &str) -> Result<(), Err> {
        if password.is_empty() {
            return Err(Err::TooWeak);
        }

        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Err::Missing),
            Err(_) => return Err(Err::Io),
        };

        if bytes.len() < HEADER_SIZE {
            return Err(Err::Corrupt);
        }

        let h = Header::parse(&bytes[..HEADER_SIZE]).map_err(|_| Err::Corrupt)?;

        // payload_size 含 16 字节 tag；文件总长必须正好是 64 + payload_size
        let expect = HEADER_SIZE as u64 + h.payload_size;
        if h.payload_size < TAG_LEN as u64 || expect != bytes.len() as u64 {
            return Err(Err::Corrupt);
        }

        let key = crypto::derive_key(password, &h.salt, h.iterations);
        let plain = crypto::open(&key, &h.nonce, &bytes[HEADER_SIZE..])
            .map_err(|_| Err::BadPassword)?;

        let projects = format::deserialize(&plain).map_err(|_| Err::Corrupt)?;

        // 走到这里才算真的解开了：前面任何一步失败都不改现有状态。
        self.key = key;
        self.salt = h.salt;
        self.iterations = h.iterations;
        self.projects = projects;
        self.unlocked = true;
        Ok(())
    }

    pub fn save(&mut self, path: &Path) -> Result<(), Err> {
        if !self.unlocked {
            return Err(Err::Locked);
        }
        let plain = format::serialize(&self.projects);

        let nonce = crypto::random_fixed::<NONCE_LEN>().ok_or(Err::CryptoFail)?;
        let blob = crypto::seal(&self.key, &nonce, &plain).map_err(|_| Err::CryptoFail)?;

        let header = Header {
            version: FORMAT_VERSION,
            iterations: self.iterations,
            salt: self.salt,
            nonce,
            payload_size: blob.len() as u64,
        };

        let mut out = Vec::with_capacity(HEADER_SIZE + blob.len());
        out.extend_from_slice(&header.encode());
        out.extend_from_slice(&blob);

        write_atomic(path, &out)
    }

    /// 把【外部】列表写盘，不动内存里的 projects。
    /// 导入流程用它：合并结果先落盘成功，调用方才换进内存，
    /// 这样任何一步失败，内存和磁盘都还是原样。
    pub fn write_out(&mut self, path: &Path, projects: &[Project]) -> Result<(), Err> {
        if !self.unlocked {
            return Err(Err::Locked);
        }
        let plain = format::serialize(projects);
        let nonce = crypto::random_fixed::<NONCE_LEN>().ok_or(Err::CryptoFail)?;
        let blob = crypto::seal(&self.key, &nonce, &plain).map_err(|_| Err::CryptoFail)?;

        let header = Header {
            version: FORMAT_VERSION,
            iterations: self.iterations,
            salt: self.salt,
            nonce,
            payload_size: blob.len() as u64,
        };
        let mut out = Vec::with_capacity(HEADER_SIZE + blob.len());
        out.extend_from_slice(&header.encode());
        out.extend_from_slice(&blob);

        write_atomic(path, &out)
    }

    /// 改主密码：换 salt + 新口令，整库重新加密落盘。
    /// 旧口令先验证一遍，避免"忘记旧密码也能改"。
    pub fn rekey(&mut self, path: &Path, old_pwd: &str, new_pwd: &str) -> Result<(), Err> {
        if new_pwd.is_empty() {
            return Err(Err::TooWeak);
        }
        // 重新加载一遍验证旧口令（也顺便确认磁盘上的就是当前内容）
        let mut probe = Store::new();
        probe.load(path, old_pwd)?;

        let new_salt = crypto::random_fixed::<SALT_LEN>().ok_or(Err::CryptoFail)?;
        let new_key = crypto::derive_key(new_pwd, &new_salt, self.iterations);

        self.salt = new_salt;
        self.key = new_key;
        // 用磁盘上读到的内容写，而不是内存里的——避免"内存有未保存改动"时被静默覆盖
        self.projects = std::mem::take(&mut probe.projects);
        self.unlocked = true;
        self.save(path)
    }

    pub fn find_index(&self, id: &[u8; 16]) -> Option<usize> {
        self.projects.iter().position(|p| &p.id == id)
    }

    /// 把第 index 个项目挪到 new_index。列表顺序 = 落盘顺序。
    pub fn move_item(&mut self, index: usize, new_index: usize) -> bool {
        if index >= self.projects.len() || new_index >= self.projects.len() || index == new_index {
            return false;
        }
        let p = self.projects.remove(index);
        self.projects.insert(new_index, p);
        true
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        // 析构时也要擦，不能指望调用方记得
        self.clear_secure();
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

/// 写 .tmp 再原子重命名。Windows 上 fs::rename 内部就是 MoveFileExW(REPLACE_EXISTING)。
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Err> {
    let mut tmp: PathBuf = path.as_os_str().to_os_string().into();
    let mut name = tmp.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    tmp.set_file_name(name);

    {
        let mut f = fs::File::create(&tmp).map_err(|_| Err::Io)?;
        f.write_all(bytes).map_err(|_| Err::Io)?;
        f.sync_all().map_err(|_| Err::Io)?;
    }

    fs::rename(&tmp, path).map_err(|_| {
        let _ = fs::remove_file(&tmp);
        Err::Io
    })
}

/// 只读文件头判断"能不能读"，不需要口令。
pub fn probe_file(path: &Path) -> ProbeInfo {
    let mut info = ProbeInfo {
        status: ProbeStatus::Missing,
        version: 0,
        iterations: 0,
        payload_size: 0,
        file_size: 0,
    };

    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return info,
        Err(_) => {
            info.status = ProbeStatus::Io;
            return info;
        }
    };
    info.file_size = meta.len();

    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => {
            info.status = ProbeStatus::Io;
            return info;
        }
    };

    if bytes.len() < HEADER_SIZE {
        info.status = ProbeStatus::TooSmall;
        return info;
    }
    if bytes[..8] != format::MAGIC {
        info.status = ProbeStatus::NotVault;
        return info;
    }

    let ver = u16::from_le_bytes([bytes[8], bytes[9]]);
    info.version = ver;

    match Header::parse(&bytes[..HEADER_SIZE]) {
        Ok(h) => {
            info.iterations = h.iterations;
            info.payload_size = h.payload_size;
            let expect = HEADER_SIZE as u64 + h.payload_size;
            if h.payload_size < TAG_LEN as u64 || expect != bytes.len() as u64 {
                info.status = ProbeStatus::Corrupt;
            } else {
                info.status = ProbeStatus::Ok;
            }
        }
        Err(format::ParseError::Version(v)) => {
            info.status = if v > FORMAT_VERSION {
                ProbeStatus::VersionNewer(v)
            } else {
                ProbeStatus::VersionOlder(v)
            };
        }
        Err(format::ParseError::TooSmall) => info.status = ProbeStatus::TooSmall,
        Err(format::ParseError::NotVault) => info.status = ProbeStatus::NotVault,
        Err(format::ParseError::Corrupt) => info.status = ProbeStatus::Corrupt,
    }

    info
}

/// 只解密、不碰任何已有对象：把另一个密库读进 out。
pub fn load_from(path: &Path, password: &str) -> Result<Vec<Project>, Err> {
    let mut s = Store::new();
    s.load(path, password)?;
    let mut projects = std::mem::take(&mut s.projects);
    // s 析构会 clear_secure，已经把 key 擦掉了；这里把数据拿出来
    for p in projects.iter_mut() {
        let _ = p;
    }
    Ok(projects)
}

/// 生成一个随机项目 id。
pub fn make_id() -> Option<[u8; 16]> {
    crypto::random_fixed::<16>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::Account;

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("secback_test_{}_{}.properties", name, std::process::id()));
        let _ = fs::remove_file(&p);
        p
    }

    fn sample_project(name: &str, user: &str, pass: &str) -> Project {
        let mut p = Project::new_with_id(make_id().unwrap());
        p.name = name.into();
        p.slots[0] = Account {
            user: user.into(),
            pass: pass.into(),
            remark: String::new(),
        };
        p
    }

    #[test]
    fn create_then_load() {
        let path = tmp_path("create");
        let mut s = Store::new();
        let created = s.load_or_create(&path, "pwd-1", true).unwrap();
        assert!(created);

        s.projects_mut().push(sample_project("项目A", "u1", "p1"));
        s.save(&path).unwrap();
        s.clear_secure();

        let mut s2 = Store::new();
        s2.load(&path, "pwd-1").unwrap();
        assert_eq!(s2.projects().len(), 1);
        assert_eq!(s2.projects()[0].name, "项目A");
        assert_eq!(s2.projects()[0].slots[0].pass, "p1");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn wrong_password_rejected_without_touching_state() {
        let path = tmp_path("wrongpwd");
        let mut s = Store::new();
        s.load_or_create(&path, "good", true).unwrap();
        s.projects_mut().push(sample_project("保留我", "u", "p"));
        s.save(&path).unwrap();

        let mut s2 = Store::new();
        let e = s2.load(&path, "bad").unwrap_err();
        assert_eq!(e, Err::BadPassword);
        // 失败不改变任何状态
        assert!(!s2.is_unlocked());
        assert_eq!(s2.projects().len(), 0);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn tampered_file_rejected() {
        let path = tmp_path("tamper");
        let mut s = Store::new();
        s.load_or_create(&path, "p", true).unwrap();
        s.projects_mut().push(sample_project("x", "u", "p"));
        s.save(&path).unwrap();

        let mut bytes = fs::read(&path).unwrap();
        let n = bytes.len();
        bytes[n - 1] ^= 0xFF;
        fs::write(&path, &bytes).unwrap();

        let mut s2 = Store::new();
        assert_eq!(s2.load(&path, "p").unwrap_err(), Err::BadPassword);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn nonce_changes_every_save() {
        let path = tmp_path("nonce");
        let mut s = Store::new();
        s.load_or_create(&path, "p", true).unwrap();
        s.projects_mut().push(sample_project("x", "u", "p"));

        s.save(&path).unwrap();
        let a = fs::read(&path).unwrap();
        s.save(&path).unwrap();
        let b = fs::read(&path).unwrap();

        assert_eq!(a.len(), b.len());
        assert_ne!(a, b, "内容没变，但 nonce 每次都要重新随机");
        // salt 必须保持不变（只在改口令时轮换）
        assert_eq!(&a[16..32], &b[16..32]);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn no_tmp_left_behind() {
        let path = tmp_path("notmp");
        let mut s = Store::new();
        s.load_or_create(&path, "p", true).unwrap();
        s.save(&path).unwrap();

        let mut tmp: PathBuf = path.as_os_str().to_os_string().into();
        let mut name = tmp.file_name().unwrap().to_os_string();
        name.push(".tmp");
        tmp.set_file_name(name);
        assert!(!tmp.exists(), "成功后不能留 .tmp");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn rekey_roundtrip() {
        let path = tmp_path("rekey");
        let mut s = Store::new();
        s.load_or_create(&path, "old-pwd", true).unwrap();
        s.projects_mut().push(sample_project("改密测试", "u", "p"));
        s.save(&path).unwrap();
        let old_salt = fs::read(&path).unwrap()[16..32].to_vec();

        s.rekey(&path, "old-pwd", "new-pwd").unwrap();

        let mut s2 = Store::new();
        assert_eq!(s2.load(&path, "old-pwd").unwrap_err(), Err::BadPassword);
        s2.load(&path, "new-pwd").unwrap();
        assert_eq!(s2.projects()[0].name, "改密测试");

        let new_salt = fs::read(&path).unwrap()[16..32].to_vec();
        assert_ne!(old_salt, new_salt, "改密码必须换 salt");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn probe_detects_states() {
        let path = tmp_path("probe");
        assert_eq!(probe_file(&path).status, ProbeStatus::Missing);

        let mut s = Store::new();
        s.load_or_create(&path, "p", true).unwrap();
        assert_eq!(probe_file(&path).status, ProbeStatus::Ok);

        // 短文件
        let short = tmp_path("probe_short");
        fs::write(&short, b"short").unwrap();
        assert_eq!(probe_file(&short).status, ProbeStatus::TooSmall);

        // 不是密库
        let notvault = tmp_path("probe_notvault");
        fs::write(&notvault, [0xAAu8; 100]).unwrap();
        assert_eq!(probe_file(&notvault).status, ProbeStatus::NotVault);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&short);
        let _ = fs::remove_file(&notvault);
    }

    #[test]
    fn move_item_reorders() {
        let mut s = Store::new();
        let path = tmp_path("move");
        s.load_or_create(&path, "p", true).unwrap();
        for n in ["a", "b", "c"] {
            s.projects_mut().push(sample_project(n, "", ""));
        }
        assert!(s.move_item(2, 0));
        let names: Vec<_> = s.projects().iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, vec!["c", "a", "b"]);

        assert!(!s.move_item(0, 0), "原地不动返回 false");
        assert!(!s.move_item(9, 0), "越界返回 false");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn clear_secure_wipes() {
        let mut s = Store::new();
        let path = tmp_path("wipe");
        s.load_or_create(&path, "p", true).unwrap();
        s.projects_mut().push(sample_project("x", "user", "secret"));
        s.clear_secure();
        assert!(s.projects().is_empty());
        assert!(!s.is_unlocked());
        assert_eq!(s.key, [0u8; KEY_LEN]);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn locked_store_refuses_save() {
        let mut s = Store::new();
        let path = tmp_path("locked");
        assert_eq!(s.save(&path).unwrap_err(), Err::Locked);
    }

    #[test]
    fn empty_password_rejected() {
        let path = tmp_path("emptypwd");
        let mut s = Store::new();
        assert_eq!(
            s.load_or_create(&path, "", true).unwrap_err(),
            Err::TooWeak
        );
        let _ = fs::remove_file(&path);
    }
}
