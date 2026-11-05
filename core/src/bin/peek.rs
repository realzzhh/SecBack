//! 调试用：直接读写一个密库，验证与 C++ 版的互通性。
//!
//!   peek <路径> <口令>                读出来打印
//!   peek <路径> <口令> --append <名字> 加一个项目再存盘
//!
//! 这不是产品的一部分，只在开发期对拍用。

use secback_core::format::Account;
use secback_core::vault::{make_id, Store};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("用法: peek <路径> <口令> [--append <名字>]");
        std::process::exit(2);
    }

    let path = PathBuf::from(&args[0]);
    let pwd = &args[1];

    let probe = secback_core::probe_file(&path);
    println!("probe: {:?}  size={} ver={}", probe.status, probe.file_size, probe.version);

    let mut store = Store::new();
    match store.load(&path, pwd) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("读取失败: {}", e.text());
            std::process::exit(1);
        }
    }

    println!("== 内容 {} 个项目 ==", store.projects().len());
    for p in store.projects() {
        println!(
            "[{}] 备注={} id={} created={}",
            p.name,
            p.remark,
            hex16(&p.id),
            p.created_utc
        );
        for (i, s) in p.slots.iter().enumerate() {
            if !s.is_empty() {
                println!("    slot{} user={} pass={} note={}", i, s.user, s.pass, s.remark);
            }
        }
    }

    if args.len() >= 4 && args[2] == "--append" {
        let name = &args[3];
        let id = make_id().unwrap();
        let mut p = secback_core::format::Project::new_with_id(id);
        p.name = name.clone();
        p.remark = "由 Rust 写入".to_string();
        p.slots[0] = Account {
            user: "rust_user".to_string(),
            pass: "RustPass#1".to_string(),
            remark: String::new(),
        };
        store.projects_mut().push(p);
        match store.save(&path) {
            Ok(()) => println!("已追加并保存：{}", name),
            Err(e) => {
                eprintln!("保存失败: {}", e.text());
                std::process::exit(1);
            }
        }
    }
}

fn hex16(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}
