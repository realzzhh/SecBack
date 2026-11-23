//! 桌面入口。
//!
//! 真正的逻辑全在 `lib.rs` 里——那是桌面和 Android 共用的一份。
//! 这里只保留"双击 exe 之后调一下 run()"，以及 bin 专属的那条 Windows 属性：
//! 加了它，release 版启动时不会弹出一个黑色控制台窗口。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    secback_lib::run()
}
