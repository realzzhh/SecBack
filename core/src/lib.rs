//! SecBack 密码本核心。
//!
//! 这里不依赖任何 GUI / 平台 UI 框架，只做三件事：
//!   1. 按格式契约读写 `sec.properties`
//!   2. 用 PBKDF2 + AES-256-GCM 保护内容
//!   3. 提供内存模型给上层（桌面宿主或手机 App）使用
//!
//! 分开的理由很实际：同一份核心要同时供 Windows 桌面和 Android/iOS 使用，
//! 而 UI 那层每端都不一样。

pub mod config;
pub mod crypto;
pub mod format;
pub mod merge;
pub mod restore;
pub mod vault;

pub use format::{Account, Project, MAX_SLOTS};
pub use vault::{probe_file, Err, ProbeInfo, ProbeStatus, Store};
