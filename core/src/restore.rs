//! 恢复码：生成、编码、规范化。
//!
//! 用 Crockford Base32：字母表去掉了容易看错的 I / L / O / U，念出来也不歧义。
//! 20 字节 = 160 bit，正好编码成 32 个字符，按 4 个一组显示。
//!
//! 恢复码的作用是**另一把钥匙**：它不替代主密码，只是在密码忘了的时候
//! 还能解开同一个数据密钥。所以它必须和电脑分开保管（打印、抄纸）。

use crate::crypto;

/// 恢复码的原始字节数。160 bit 足够抵抗穷举。
pub const CODE_LEN: usize = 20;
/// 编码后的字符数。
pub const CODE_CHARS: usize = 32;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// 把 20 字节编码成 32 个字符（不含分隔符）。
pub fn encode(raw: &[u8; CODE_LEN]) -> String {
    let mut out = String::with_capacity(CODE_CHARS);
    let mut acc: u64 = 0;
    let mut bits: u32 = 0;

    for &b in raw {
        acc = (acc << 8) | b as u64;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 0x1F) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 0x1F) as usize] as char);
    }
    out
}

/// 每 4 个字符插一个短横，方便抄写和念。
pub fn pretty(code: &str) -> String {
    code.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("-")
}

fn char_value(c: char) -> Option<u8> {
    let up = c.to_ascii_uppercase();
    // 抄写时常见的形近字，替一下不至于让用户白跑一趟
    let up = match up {
        'O' => '0',
        'I' | 'L' => '1',
        _ => up,
    };
    ALPHABET
        .iter()
        .position(|&x| x as char == up)
        .map(|p| p as u8)
}

/// 宽松解析：忽略空白和短横，容忍大小写和 O/0、I/1 混淆。
/// 只有长度或字符确实不对才返回 None。
pub fn normalize(input: &str) -> Option<String> {
    let mut out = String::with_capacity(CODE_CHARS);
    for c in input.chars() {
        if c.is_whitespace() || c == '-' || c == '_' {
            continue;
        }
        let v = char_value(c)?;
        out.push(ALPHABET[v as usize] as char);
    }
    if out.len() != CODE_CHARS {
        return None;
    }
    Some(out)
}

pub fn decode(input: &str) -> Option<[u8; CODE_LEN]> {
    let norm = normalize(input)?;
    let mut out = [0u8; CODE_LEN];
    let mut acc: u64 = 0;
    let mut bits: u32 = 0;
    let mut idx = 0;

    for c in norm.chars() {
        let v = ALPHABET.iter().position(|&x| x as char == c)? as u64;
        acc = (acc << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            if idx < CODE_LEN {
                out[idx] = ((acc >> bits) & 0xFF) as u8;
                idx += 1;
            }
        }
    }
    if idx == CODE_LEN {
        Some(out)
    } else {
        None
    }
}

/// 生成一组新恢复码，返回（原始字节, 32 字符无分隔）。
pub fn generate() -> Option<([u8; CODE_LEN], String)> {
    let raw = crypto::random_fixed::<CODE_LEN>()?;
    let text = encode(&raw);
    Some((raw, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let mut raw = [0u8; CODE_LEN];
        for (i, b) in raw.iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        let text = encode(&raw);
        assert_eq!(text.len(), CODE_CHARS);
        assert_eq!(decode(&text), Some(raw));
    }

    #[test]
    fn all_zero_and_all_ff() {
        let z = [0u8; CODE_LEN];
        assert_eq!(decode(&encode(&z)), Some(z));
        let f = [0xFFu8; CODE_LEN];
        assert_eq!(decode(&encode(&f)), Some(f));
    }

    #[test]
    fn pretty_groups_by_four() {
        let code = "0123456789ABCDEFGHJKMNPQRSTVWXY"; // 31 字符
        assert_eq!(pretty("ABCDEFGH"), "ABCD-EFGH");
        assert!(code.len() < CODE_CHARS);
    }

    #[test]
    fn normalize_ignores_separators_and_case() {
        let raw = [0x5Au8; CODE_LEN];
        let text = encode(&raw);
        let messy = format!(" {} ", pretty(&text.to_lowercase()));
        assert_eq!(normalize(&messy), Some(text));
    }

    #[test]
    fn normalize_maps_lookalikes() {
        // O -> 0, I/L -> 1
        assert_eq!(normalize("OOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOO").unwrap(), "0".repeat(32));
        assert_eq!(normalize("IIIIIIIIIIIIIIIIIIIIIIIIIIIIIIII").unwrap(), "1".repeat(32));
        assert_eq!(normalize("LLLLLLLLLLLLLLLLLLLLLLLLLLLLLLLL").unwrap(), "1".repeat(32));
    }

    #[test]
    fn wrong_length_rejected() {
        assert_eq!(normalize("ABC"), None);
        assert_eq!(normalize(&"A".repeat(31)), None);
        assert_eq!(normalize(&"A".repeat(33)), None);
    }

    #[test]
    fn invalid_char_rejected() {
        // U 不在字母表里（Crockford 故意去掉）
        assert_eq!(normalize("UUUUUUUUUUUUUUUUUUUUUUUUUUUUUUUU"), None);
    }

    #[test]
    fn determinism() {
        let code = "0123456789ABCDEFGHJKMNPQRSTVWXY";
        assert_eq!(decode(code), decode(code));
    }

    #[test]
    fn generated_codes_differ() {
        let a = generate().unwrap().1;
        let b = generate().unwrap().1;
        assert_ne!(a, b, "两次生成不能一样");
        assert_eq!(a.len(), CODE_CHARS);
    }
}
