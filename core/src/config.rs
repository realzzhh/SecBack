//! 密库位置解析。
//!
//! 规则（与 C++ 版逐条对齐）：
//!   1. 读 exe 同目录的 `secback.cfg`，找 `vault = ...`
//!   2. 支持 `%环境变量%` 展开，变量不存在就原样保留（用户能一眼看出写错了）
//!   3. 绝对路径直接使用；相对路径相对 exe 所在目录
//!   4. 分隔符规范化（`/` -> `\`，压掉重复的 `\`）
//!   5. 指向目录 → 报错；父目录不存在 → 自动创建
//!
//! 为什么要留这个机制：密库放固定位置（比如文档目录），以后更新程序、
//! 换目录、重装，都不会因为"只换了 exe 没带密库"而从一个空库开始。

use std::path::{Path, PathBuf};

pub const CONFIG_NAME: &str = "secback.cfg";
pub const DEFAULT_VAULT: &str = "sec.properties";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub path: PathBuf,
    /// 是否来自配置文件里显式写的 vault=（用于提示文案）
    pub from_config: bool,
}

/// 解析结果。配置写错时不是"悄悄退回默认"，而是明确报错——
/// 否则用户会以为自己打开的是原来那个库，实际上是新建了一个空库。
pub fn resolve(exe_dir: &Path) -> Result<Resolved, String> {
    let text = std::fs::read_to_string(exe_dir.join(CONFIG_NAME)).ok();
    resolve_with(exe_dir, text.as_deref(), &|k| std::env::var(k).ok())
}

pub fn resolve_with(
    exe_dir: &Path,
    cfg_text: Option<&str>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Resolved, String> {
    let configured = cfg_text
        .and_then(parse_vault_key)
        .unwrap_or_default();
    let expanded = expand_env(&configured, env);
    let trimmed = expanded.trim();

    let from_config = !trimmed.is_empty();
    let rel = if from_config { trimmed } else { DEFAULT_VAULT };

    let joined = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        exe_dir.join(rel)
    };

    let norm = normalize_seps(&joined.to_string_lossy());
    let path = PathBuf::from(&norm);

    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => {
            let mut msg = format!("密库路径不像一个文件名：\n{}", norm);
            if from_config {
                msg.push_str("\n\n（来自配置文件里的 vault=）");
            }
            return Err(msg);
        }
    };

    if path.is_dir() {
        return Err(format!("密库路径指向的是一个目录，不是文件：\n{}", norm));
    }

    if !parent.exists() {
        if let Err(e) = std::fs::create_dir_all(&parent) {
            return Err(format!(
                "密库所在的目录不存在，而且建不出来：\n{}\n\n{}",
                parent.display(),
                e
            ));
        }
    }

    Ok(Resolved { path, from_config })
}

/// 从配置文件里挑出 `vault=`。其它键一律忽略——新版本加选项时老配置不该报错。
fn parse_vault_key(text: &str) -> Option<String> {
    // 记事本存的 UTF-8 常带 BOM，不剥掉的话第一行的 `#` 判断会失效
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut configured: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim().to_lowercase() == "vault" {
            configured = Some(v.trim().to_string());
        }
    }
    configured
}

/// `%VAR%` -> 环境变量的值。变量不存在就原样保留。
fn expand_env(input: &str, env: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] != '%' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        // 找配对的下一个 %
        match chars[i + 1..].iter().position(|c| *c == '%') {
            None => {
                // 没有配对的 %，后面整段原样保留
                out.extend(&chars[i..]);
                break;
            }
            Some(off) => {
                let name: String = chars[i + 1..i + 1 + off].iter().collect();
                match if name.is_empty() { None } else { env(&name) } {
                    Some(v) => {
                        out.push_str(&v);
                        i = i + off + 2;
                    }
                    None => {
                        // 原样保留 %VAR%，包括两个 %
                        out.extend(&chars[i..=i + off]);
                        i = i + off + 1;
                    }
                }
            }
        }
    }
    out
}

#[cfg(windows)]
fn normalize_seps(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = if c == '/' { '\\' } else { c };
        if c == '\\' && out.ends_with('\\') {
            continue; // 压掉重复分隔符
        }
        out.push(c);
    }
    out
}

#[cfg(not(windows))]
fn normalize_seps(s: &str) -> String {
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn exe() -> PathBuf {
        // 用临时目录当基准：resolve 会真的去创建父目录，
        // 测试基准要是写成 D:/App 就会在 D 盘根目录留个文件夹。
        std::env::temp_dir().join("secback_config_test")
    }

    #[test]
    fn defaults_to_sec_properties_next_to_exe() {
        let r = resolve_with(&exe(), None, &no_env).unwrap();
        assert!(!r.from_config);
        assert!(r.path.to_string_lossy().ends_with("sec.properties"));
        assert!(r.path.starts_with(exe()));
    }

    #[test]
    fn reads_absolute_path_from_config() {
        let cfg = "vault = D:\\我的密库\\sec.properties\n";
        let r = resolve_with(&exe(), Some(cfg), &no_env).unwrap();
        assert!(r.from_config);
        assert_eq!(r.path.to_string_lossy(), "D:\\我的密库\\sec.properties");
    }

    #[test]
    fn relative_path_is_relative_to_exe() {
        let cfg = "vault = data\\vault.properties";
        let r = resolve_with(&exe(), Some(cfg), &no_env).unwrap();
        assert!(r
            .path
            .to_string_lossy()
            .ends_with("secback_config_test\\data\\vault.properties"));
    }

    #[test]
    fn expands_environment_variables() {
        // 只测展开本身：走完整 resolve 会真的去建目录，不该在单测里做
        let env = |k: &str| {
            if k == "DOCS" {
                Some("C:\\Users\\me\\Documents".to_string())
            } else {
                None
            }
        };
        assert_eq!(
            expand_env("%DOCS%\\sec.properties", &env),
            "C:\\Users\\me\\Documents\\sec.properties"
        );
    }

    #[test]
    fn unknown_variable_kept_verbatim() {
        let got = expand_env("A%NOPE%B", &no_env);
        assert_eq!(got, "A%NOPE%B");
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let cfg = "# 注释\n; 也是注释\n\nvault = real.properties\n";
        let r = resolve_with(&exe(), Some(cfg), &no_env).unwrap();
        assert!(r.path.to_string_lossy().ends_with("real.properties"));
    }

    #[test]
    fn key_is_case_insensitive() {
        let cfg = "VAULT = upper.properties";
        let r = resolve_with(&exe(), Some(cfg), &no_env).unwrap();
        assert!(r.path.to_string_lossy().ends_with("upper.properties"));
    }

    #[test]
    fn other_keys_ignored() {
        let cfg = "timeout = 30\nvault = mine.properties\nunknown = x\n";
        let r = resolve_with(&exe(), Some(cfg), &no_env).unwrap();
        assert!(r.path.to_string_lossy().ends_with("mine.properties"));
    }

    #[test]
    fn handles_utf8_bom() {
        let cfg = "\u{feff}vault = bom.properties";
        let r = resolve_with(&exe(), Some(cfg), &no_env).unwrap();
        assert!(r.path.to_string_lossy().ends_with("bom.properties"));
    }

    #[test]
    fn commented_out_vault_falls_back_to_default() {
        let cfg = "#vault = D:\\other\\sec.properties\n";
        let r = resolve_with(&exe(), Some(cfg), &no_env).unwrap();
        assert!(!r.from_config);
        assert!(r.path.to_string_lossy().ends_with("sec.properties"));
    }

    #[test]
    fn forward_slashes_are_normalized() {
        let cfg = "vault = D:/a//b///sec.properties";
        let r = resolve_with(&exe(), Some(cfg), &no_env).unwrap();
        #[cfg(windows)]
        assert_eq!(r.path.to_string_lossy(), "D:\\a\\b\\sec.properties");
    }

    #[test]
    fn empty_value_means_default() {
        let cfg = "vault =";
        let r = resolve_with(&exe(), Some(cfg), &no_env).unwrap();
        assert!(!r.from_config);
    }
}
