fn main() {
    // generate_context! embeds these files. Cargo must rerun even when Rust is unchanged.
    println!("cargo:rerun-if-changed=../src");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=../package.json");
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
