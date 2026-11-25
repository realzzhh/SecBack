//! 密库读写层：加载、保存、改口令、预检、原子落盘。
//!
//! 两个刻意的取舍（与 C++ 版一致，不能改）：
//!   1. nonce 每次写盘重新生成 —— 内容一字未改，磁盘字节也完全不同，
//!      不会泄漏"这次只动了备注"这类元信息。salt 只在改口令时轮换。
//!   2. 写盘一律 .tmp + 原子重命名 —— 半途崩了最多留个 .tmp，
//!      不会把已经存好的数据写坏。

use crate::crypto::{self, KEY_LEN, NONCE_LEN, SALT_LEN, TAG_LEN};
use crate::restore;
use crate::format::{
    self, Header, Project, DEK_LEN, FLAG_RECOVERY, FORMAT_VERSION, FORMAT_VERSION_V2, HEADER_SIZE,
    WRAPPED_DEK_LEN,
};
use std::fs;
use zeroize::Zeroizing;
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
    /// 这个库没设恢复码
    NoRecovery,
    /// 恢复码不对
    BadRestoreCode,
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
            Err::NoRecovery => "这个密库没有设置恢复码",
            Err::BadRestoreCode => "恢复码不正确",
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
    /// 加密载荷用的密钥。
    /// v1：直接是 PBKDF2(口令) 的结果；v2：是那个被口令和恢复码包起来的随机 DEK。
    key: [u8; KEY_LEN],
    /// 写盘用的头模板。salt / iterations / version / 两处 wrap 材料都在里面，
    /// 每次 save 只更新 nonce 和 payload_size——这样保存时不需要口令。
    hdr: Header,
    unlocked: bool,
}

impl Store {
    pub fn new() -> Self {
        Self {
            projects: Vec::new(),
            key: [0u8; KEY_LEN],
            hdr: Header {
                version: FORMAT_VERSION,
                iterations: crypto::ITERATIONS,
                ..Default::default()
            },
            unlocked: false,
        }
    }

    /// 临时事务副本；失败时析构并擦除，成功时替换原对象。
    pub fn staged(&self) -> Self {
        Self { projects: self.projects.clone(), key: self.key, hdr: self.hdr.clone(), unlocked: self.unlocked }
    }

    pub fn commit(&mut self, path: &Path, mut candidate: Self) -> Result<(), Err> {
        candidate.save(path)?;
        std::mem::swap(self, &mut candidate);
        Ok(())
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
        crypto::wipe(&mut self.hdr.salt);
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
        // 新建的库是 v1：不带恢复槽，C++ 版也能读。
        // 用户主动去设恢复码时才升 v2。
        let mut candidate = Store::new();
        candidate.hdr = Header {
            version: FORMAT_VERSION,
            iterations: crypto::ITERATIONS,
            salt,
            ..Default::default()
        };
        candidate.key = crypto::derive_key(password, &salt, candidate.hdr.iterations);
        candidate.projects.clear();
        candidate.unlocked = true;

        self.commit(path, candidate)?;
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

        // parse 自己会按版本判断头有多长（v1=64 / v2=256）
        let h = Header::parse(&bytes).map_err(|_| Err::Corrupt)?;
        let hsize = h.size();

        if bytes.len() < hsize {
            return Err(Err::Corrupt);
        }

        // payload_size 含 16 字节 tag；文件总长必须正好是 头长 + payload_size
        let expect = hsize as u64 + h.payload_size;
        if h.payload_size < TAG_LEN as u64 || expect != bytes.len() as u64 {
            return Err(Err::Corrupt);
        }

        let key = Zeroizing::new(if h.version >= FORMAT_VERSION_V2 {
            // v2：口令只负责解开 DEK，真正的载荷密钥是那个随机 DEK
            let kek = Zeroizing::new(crypto::derive_key(password, &h.salt, h.iterations));
            let dek = Zeroizing::new(crypto::open(&kek, &h.wrap_nonce, &h.wrapped_dek).map_err(|_| Err::BadPassword)?);
            if dek.len() != DEK_LEN {
                return Err(Err::Corrupt);
            }
            let mut k = [0u8; KEY_LEN];
            k.copy_from_slice(&dek);
            k
        } else {
            crypto::derive_key(password, &h.salt, h.iterations)
        });

        let plain = Zeroizing::new(crypto::open(&key, &h.nonce, &bytes[hsize..]).map_err(|_| Err::BadPassword)?);

        let projects = format::deserialize(&plain).map_err(|_| Err::Corrupt)?;

        // 走到这里才算真的解开了：前面任何一步失败都不改现有状态。
        self.clear_secure();
        self.key = *key;
        self.hdr = h;
        self.projects = projects;
        self.unlocked = true;
        Ok(())
    }

    pub fn save(&mut self, path: &Path) -> Result<(), Err> {
        if !self.unlocked {
            return Err(Err::Locked);
        }
        let plain = Zeroizing::new(format::serialize(&self.projects));
        let nonce = crypto::random_fixed::<NONCE_LEN>().ok_or(Err::CryptoFail)?;
        let blob = crypto::seal(&self.key, &nonce, &plain).map_err(|_| Err::CryptoFail)?;

        // 头模板原样带过去（v2 的两处 wrap 材料必须保留），只换 nonce 和长度
        self.hdr.nonce = nonce;
        self.hdr.payload_size = blob.len() as u64;

        let head = self.hdr.encode();
        let mut out = Vec::with_capacity(head.len() + blob.len());
        out.extend_from_slice(&head);
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
        let plain = Zeroizing::new(format::serialize(projects));
        let nonce = crypto::random_fixed::<NONCE_LEN>().ok_or(Err::CryptoFail)?;
        let blob = crypto::seal(&self.key, &nonce, &plain).map_err(|_| Err::CryptoFail)?;

        self.hdr.nonce = nonce;
        self.hdr.payload_size = blob.len() as u64;

        let head = self.hdr.encode();
        let mut out = Vec::with_capacity(head.len() + blob.len());
        out.extend_from_slice(&head);
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

        if probe.hdr.version >= FORMAT_VERSION_V2 {
            // v2：DEK 不动，只用新口令把它重新包一遍。整个库不用重加密。
            let dek = Zeroizing::new(probe.key);
            let kek = Zeroizing::new(crypto::derive_key(new_pwd, &new_salt, probe.hdr.iterations));
            let wrap_nonce = crypto::random_fixed::<NONCE_LEN>().ok_or(Err::CryptoFail)?;
            let wrapped = crypto::seal(&kek, &wrap_nonce, &dek[..]).map_err(|_| Err::CryptoFail)?;
            if wrapped.len() != WRAPPED_DEK_LEN {
                return Err(Err::CryptoFail);
            }

            probe.hdr.salt = new_salt;
            probe.hdr.wrap_nonce = wrap_nonce;
            probe.hdr.wrapped_dek.copy_from_slice(&wrapped);
            probe.key = *dek;
        } else {
            // v1：密钥本身就是口令派生的，只能整库重新加密
            probe.hdr = Header {
                version: FORMAT_VERSION,
                iterations: probe.hdr.iterations,
                salt: new_salt,
                ..Default::default()
            };
            probe.key = crypto::derive_key(new_pwd, &new_salt, probe.hdr.iterations);
        }

        // 用磁盘上读到的内容写，而不是内存里的——避免"内存有未保存改动"时被静默覆盖
        probe.unlocked = true;
        self.commit(path, probe)
    }

    /// 这个库有没有设恢复码。
    pub fn has_recovery(&self) -> bool {
        self.hdr.has_recovery()
    }

    /// 当前库的格式版本，界面用来提示"这份还不是 v2"。
    pub fn version(&self) -> u16 {
        self.hdr.version
    }

    /// 设恢复码。没有就升级到 v2 并生成一组新码，已有就换一组新的。
    /// 返回给人抄写的那串（带短横）。
    ///
    /// 先按磁盘现状验一遍口令，顺便拿到真实的 DEK 和头，
    /// 免得界面状态和磁盘不一致时改坏文件。
    pub fn enable_recovery(&mut self, path: &Path, password: &str) -> Result<String, Err> {
        let mut probe = Store::new();
        probe.load(path, password)?;

        // v1 的 key 是口令派生出来的，当不了独立 DEK——升级时得换一个随机的
        let dek = Zeroizing::new(if probe.hdr.version < FORMAT_VERSION_V2 {
            crypto::random_fixed::<DEK_LEN>().ok_or(Err::CryptoFail)?
        } else {
            probe.key
        });

        let kek = Zeroizing::new(crypto::derive_key(password, &probe.hdr.salt, probe.hdr.iterations));
        let wrap_nonce = crypto::random_fixed::<NONCE_LEN>().ok_or(Err::CryptoFail)?;
        let wrapped = crypto::seal(&kek, &wrap_nonce, &dek[..]).map_err(|_| Err::CryptoFail)?;
        if wrapped.len() != WRAPPED_DEK_LEN {
            return Err(Err::CryptoFail);
        }

        let (raw, code) = restore::generate().ok_or(Err::CryptoFail)?;
        let _raw = Zeroizing::new(raw);
        let code = Zeroizing::new(code);
        let rec_salt = crypto::random_fixed::<SALT_LEN>().ok_or(Err::CryptoFail)?;
        let rec_kek = Zeroizing::new(crypto::derive_key(&code, &rec_salt, probe.hdr.iterations));
        let rec_nonce = crypto::random_fixed::<NONCE_LEN>().ok_or(Err::CryptoFail)?;
        let rec_wrapped = crypto::seal(&rec_kek, &rec_nonce, &dek[..]).map_err(|_| Err::CryptoFail)?;
        if rec_wrapped.len() != WRAPPED_DEK_LEN {
            return Err(Err::CryptoFail);
        }

        probe.hdr.version = FORMAT_VERSION_V2;
        probe.hdr.flags |= FLAG_RECOVERY;
        probe.hdr.wrap_nonce = wrap_nonce;
        probe.hdr.wrapped_dek.copy_from_slice(&wrapped);
        probe.hdr.recovery_salt = rec_salt;
        probe.hdr.recovery_nonce = rec_nonce;
        probe.hdr.recovery_wrapped_dek.copy_from_slice(&rec_wrapped);
        probe.key = *dek;
        probe.unlocked = true;

        self.commit(path, probe)?;
        Ok(restore::pretty(&code))
    }

    /// 用恢复码解锁。
    ///
    /// 成功之后库是「已解锁但还没设新口令」的状态——紧接着必须调
    /// [`set_password`](Self::set_password)，否则下次还得再用恢复码。
    pub fn recover(&mut self, path: &Path, code: &str) -> Result<(), Err> {
        let bytes = fs::read(path).map_err(|_| Err::Io)?;
        let h = Header::parse(&bytes).map_err(|_| Err::Corrupt)?;

        // 先看这个库有没有恢复槽，再看码的格式——
        // 反过来的话，"库压根没设恢复码"会被报成"恢复码不对"，容易误导。
        if !h.has_recovery() {
            return Err(Err::NoRecovery);
        }

        let norm = Zeroizing::new(restore::normalize(code).ok_or(Err::BadRestoreCode)?);
        let hsize = h.size();
        if bytes.len() != hsize + h.payload_size as usize {
            return Err(Err::Corrupt);
        }

        let kek = Zeroizing::new(crypto::derive_key(&norm, &h.recovery_salt, h.iterations));
        let dek = Zeroizing::new(crypto::open(&kek, &h.recovery_nonce, &h.recovery_wrapped_dek).map_err(|_| Err::BadRestoreCode)?);
        if dek.len() != DEK_LEN {
            return Err(Err::Corrupt);
        }
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        key.copy_from_slice(&dek);

        let plain = Zeroizing::new(crypto::open(&key, &h.nonce, &bytes[hsize..]).map_err(|_| Err::Corrupt)?);
        let projects = format::deserialize(&plain).map_err(|_| Err::Corrupt)?;

        self.clear_secure();
        self.key = *key;
        self.hdr = h;
        self.projects = projects;
        self.unlocked = true;
        Ok(())
    }

    /// 恢复之后设新口令：只重新包 DEK，不重加密内容。
    pub fn set_password(&mut self, path: &Path, new_pwd: &str) -> Result<(), Err> {
        if !self.unlocked {
            return Err(Err::Locked);
        }
        if new_pwd.is_empty() {
            return Err(Err::TooWeak);
        }
        if self.hdr.version < FORMAT_VERSION_V2 {
            return Err(Err::Corrupt);
        }

        let new_salt = crypto::random_fixed::<SALT_LEN>().ok_or(Err::CryptoFail)?;
        let kek = Zeroizing::new(crypto::derive_key(new_pwd, &new_salt, self.hdr.iterations));
        let wrap_nonce = crypto::random_fixed::<NONCE_LEN>().ok_or(Err::CryptoFail)?;
        let wrapped = crypto::seal(&kek, &wrap_nonce, &self.key).map_err(|_| Err::CryptoFail)?;
        if wrapped.len() != WRAPPED_DEK_LEN {
            return Err(Err::CryptoFail);
        }

        let mut candidate = self.staged();
        candidate.hdr.salt = new_salt;
        candidate.hdr.wrap_nonce = wrap_nonce;
        candidate.hdr.wrapped_dek.copy_from_slice(&wrapped);
        self.commit(path, candidate)
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

    match Header::parse(&bytes) {
        Ok(h) => {
            info.iterations = h.iterations;
            info.payload_size = h.payload_size;
            let expect = h.size() as u64 + h.payload_size;
            if h.payload_size < TAG_LEN as u64 || expect != bytes.len() as u64 {
                info.status = ProbeStatus::Corrupt;
            } else {
                info.status = ProbeStatus::Ok;
            }
        }
        Err(format::ParseError::Version(v)) => {
            info.status = if v > FORMAT_VERSION_V2 {
                ProbeStatus::VersionNewer(v)            } else {
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

    // 这些回归只测事务，不测 KDF 强度；降低轮数让故障矩阵保持快速。
    fn transaction_fixture(name: &str) -> (Store, PathBuf, PathBuf) {
        let path = tmp_path(name);
        let block = PathBuf::from(format!("{}.tmp", path.display()));
        let mut s = Store::new();
        s.hdr.salt = [4; SALT_LEN];
        s.hdr.iterations = 1000;
        s.key = crypto::derive_key("old-password", &s.hdr.salt, s.hdr.iterations);
        s.unlocked = true;
        s.projects.push(sample_project("原项目", "原账户", "原密码"));
        s.save(&path).unwrap();
        (s, path, block)
    }

    #[test]
    fn failed_project_transactions_keep_memory_and_disk() {
        for operation in ["create", "save", "delete", "reorder"] {
            let (mut s, path, block) = transaction_fixture(operation);
            s.projects.push(sample_project("第二项", "u", "p"));
            s.save(&path).unwrap();
            let before = fs::read(&path).unwrap();
            fs::create_dir(&block).unwrap();
            let mut candidate = s.staged();
            match operation {
                "create" => candidate.projects.push(sample_project("新增", "u", "p")),
                "save" => candidate.projects[0].name = "修改".into(),
                "delete" => { candidate.projects.remove(0); },
                _ => { candidate.move_item(0, 1); },
            }
            assert_eq!(s.commit(&path, candidate), Err(Err::Io));
            assert_eq!(s.projects.len(), 2);
            assert_eq!(s.projects[0].name, "原项目");
            assert_eq!(fs::read(&path).unwrap(), before);
            fs::remove_dir(block).unwrap();
            s.save(&path).unwrap();
            let mut disk = Store::new();
            disk.load(&path, "old-password").unwrap();
            assert_eq!(disk.projects[0].name, "原项目");
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn failed_rekey_does_not_activate_new_password_on_later_save() {
        for v2 in [false, true] {
            let (mut s, path, block) = transaction_fixture(if v2 {"rekey-v2-fail"} else {"rekey-v1-fail"});
            if v2 { s.enable_recovery(&path, "old-password").unwrap(); }
            fs::create_dir(&block).unwrap();
            assert_eq!(s.rekey(&path, "old-password", "new-password"), Err(Err::Io));
            fs::remove_dir(block).unwrap();
            s.save(&path).unwrap();
            let mut disk = Store::new();
            disk.load(&path, "old-password").unwrap();
            assert_eq!(disk.load(&path, "new-password"), Err(Err::BadPassword));
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn failed_recovery_change_keeps_original_header() {
        let (mut s, path, block) = transaction_fixture("recovery-fail");
        fs::create_dir(&block).unwrap();
        assert_eq!(s.enable_recovery(&path, "old-password"), Err(Err::Io));
        assert!(!s.has_recovery());
        fs::remove_dir(&block).unwrap();
        let code = s.enable_recovery(&path, "old-password").unwrap();
        let old_header = s.hdr.clone();
        fs::create_dir(&block).unwrap();
        assert_eq!(s.enable_recovery(&path, "old-password"), Err(Err::Io));
        assert_eq!(s.hdr, old_header);
        fs::remove_dir(block).unwrap();
        s.save(&path).unwrap();
        let mut disk = Store::new();
        disk.recover(&path, &code).unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_recovery_password_keeps_old_password_and_recovery() {
        let (mut s, path, block) = transaction_fixture("set-password-fail");
        let code = s.enable_recovery(&path, "old-password").unwrap();
        s.recover(&path, &code).unwrap();
        let header = s.hdr.clone();
        fs::create_dir(&block).unwrap();
        assert_eq!(s.set_password(&path, "new-password"), Err(Err::Io));
        assert_eq!(s.hdr, header);
        fs::remove_dir(block).unwrap();
        s.save(&path).unwrap();
        let mut disk = Store::new();
        disk.load(&path, "old-password").unwrap();
        disk.recover(&path, &code).unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn failed_create_keeps_store_locked() {
        let path = tmp_path("create-fail");
        let block = PathBuf::from(format!("{}.tmp", path.display()));
        fs::create_dir(&block).unwrap();
        let mut s = Store::new();
        assert_eq!(s.load_or_create(&path, "password", true), Err(Err::Io));
        assert!(!s.is_unlocked());
        assert_eq!(s.key, [0; KEY_LEN]);
        fs::remove_dir(block).unwrap();
    }

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

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "secback_rec_{}_{}.properties",
            name,
            std::process::id()
        ));
        let _ = fs::remove_file(&p);
        p
    }

    fn sample(name: &str, user: &str, pass: &str) -> Project {
        let mut p = Project::new_with_id([7u8; 16]);
        p.name = name.to_string();
        p.slots[0] = crate::format::Account {
            user: user.to_string(),
            pass: pass.to_string(),
            remark: String::new(),
        };
        p
    }

    fn fresh(path: &Path, pwd: &str, name: &str, user: &str, pass: &str) -> Store {
        let mut s = Store::new();
        s.load_or_create(path, pwd, true).unwrap();
        s.projects_mut().push(sample(name, user, pass));
        s.save(path).unwrap();
        s
    }

    #[test]
    fn enable_recovery_upgrades_to_v2_and_password_still_works() {
        let path = tmp_path("upgrade");
        let mut s = fresh(&path, "pwd-a", "邮箱", "u1", "p1");
        assert_eq!(s.version(), 1, "新建的库应该是 v1");

        let code = s.enable_recovery(&path, "pwd-a").unwrap();
        assert_eq!(code.len(), 39, "32 字符 + 7 个短横");
        assert_eq!(s.version(), 2);

        // 密码照旧能开
        let mut s2 = Store::new();
        s2.load(&path, "pwd-a").unwrap();
        assert_eq!(s2.projects()[0].slots[0].pass, "p1");
        assert!(s2.has_recovery());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn recover_with_code_opens_vault() {
        let path = tmp_path("recover");
        let mut s = fresh(&path, "forgotten", "银行", "u", "secret");
        let code = s.enable_recovery(&path, "forgotten").unwrap();
        s.clear_secure();

        // 假设密码彻底忘了，只有恢复码
        let mut s2 = Store::new();
        assert_eq!(s2.load(&path, "wrong-pwd"), Err(Err::BadPassword));

        s2.recover(&path, &code).unwrap();
        assert_eq!(s2.projects().len(), 1);
        assert_eq!(s2.projects()[0].name, "银行");
        assert_eq!(s2.projects()[0].slots[0].pass, "secret");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn recover_accepts_messy_code() {
        let path = tmp_path("messy");
        let mut s = fresh(&path, "pw", "A", "u", "p");
        let code = s.enable_recovery(&path, "pw").unwrap();
        s.clear_secure();

        // 用户抄的时候可能大小写混、多打空格
        let messy = format!("  {}  ", code.to_lowercase().replace('-', " "));
        let mut s2 = Store::new();
        s2.recover(&path, &messy).unwrap();
        assert_eq!(s2.projects().len(), 1);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn recover_then_set_new_password() {
        let path = tmp_path("newpwd");
        let mut s = fresh(&path, "old-pwd", "A", "u", "p");
        let code = s.enable_recovery(&path, "old-pwd").unwrap();
        s.clear_secure();

        let mut s2 = Store::new();
        s2.recover(&path, &code).unwrap();
        s2.set_password(&path, "brand-new-pwd").unwrap();
        s2.clear_secure();

        // 新密码能开
        let mut s3 = Store::new();
        s3.load(&path, "brand-new-pwd").unwrap();
        assert_eq!(s3.projects()[0].slots[0].pass, "p");

        // 旧密码不能
        let mut s4 = Store::new();
        assert_eq!(s4.load(&path, "old-pwd"), Err(Err::BadPassword));

        // 恢复码仍然有效（还是包着同一个 DEK）
        let mut s5 = Store::new();
        s5.recover(&path, &code).unwrap();
        assert_eq!(s5.projects().len(), 1);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn wrong_restore_code_rejected() {
        let path = tmp_path("badcode");
        let mut s = fresh(&path, "pw", "A", "u", "p");
        let code = s.enable_recovery(&path, "pw").unwrap();
        s.clear_secure();

        // 改掉最后一个字符
        let mut bad: Vec<char> = code.chars().collect();
        let last = bad.len() - 1;
        bad[last] = if bad[last] == 'A' { 'B' } else { 'A' };
        let bad: String = bad.into_iter().collect();

        let mut s2 = Store::new();
        assert_eq!(s2.recover(&path, &bad), Err(Err::BadRestoreCode));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn malformed_restore_code_rejected() {
        let path = tmp_path("malformed");
        let mut s = fresh(&path, "pw", "A", "u", "p");
        let _ = s.enable_recovery(&path, "pw").unwrap();
        s.clear_secure();

        let mut s2 = Store::new();
        assert_eq!(s2.recover(&path, "太短了"), Err(Err::BadRestoreCode));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn vault_without_recovery_slot() {
        let path = tmp_path("noslot");
        fresh(&path, "pw", "A", "u", "p");

        // v1 库没有恢复槽，随便给个格式合法的码也应报"没设恢复码"
        let mut s = Store::new();
        let code = "0123456789ABCDEFGHJKMNPQRSTVWXY";
        assert_eq!(s.recover(&path, code), Err(Err::NoRecovery));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn v2_rekey_keeps_content_and_recovery() {
        let path = tmp_path("v2rekey");
        let mut s = fresh(&path, "pwd-1", "A", "u", "p1");
        let code = s.enable_recovery(&path, "pwd-1").unwrap();
        s.clear_secure();

        let mut s2 = Store::new();
        s2.rekey(&path, "pwd-1", "pwd-2").unwrap();
        assert_eq!(s2.version(), 2, "改密码不该把库退回 v1");

        let mut s3 = Store::new();
        s3.load(&path, "pwd-2").unwrap();
        assert_eq!(s3.projects()[0].slots[0].pass, "p1");

        // 改密码后恢复码依然有效
        let mut s4 = Store::new();
        s4.recover(&path, &code).unwrap();
        assert_eq!(s4.projects().len(), 1);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn recovery_code_can_be_reissued() {
        let path = tmp_path("reissue");
        let mut s = fresh(&path, "pw", "A", "u", "p");
        let first = s.enable_recovery(&path, "pw").unwrap();
        let second = s.enable_recovery(&path, "pw").unwrap();
        assert_ne!(first, second, "重新生成应该换一组新码");
        s.clear_secure();

        // 旧码作废
        let mut s2 = Store::new();
        assert_eq!(s2.recover(&path, &first), Err(Err::BadRestoreCode));

        // 新码可用
        let mut s3 = Store::new();
        s3.recover(&path, &second).unwrap();
        assert_eq!(s3.projects().len(), 1);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn enable_recovery_rejects_wrong_password() {
        let path = tmp_path("badpwd");
        let mut s = fresh(&path, "real", "A", "u", "p");
        assert_eq!(s.enable_recovery(&path, "fake"), Err(Err::BadPassword));
        let _ = fs::remove_file(&path);
    }
}
