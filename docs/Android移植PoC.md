# Android 移植 PoC 记录

日期：2026-11-23
状态：**技术可行性已验证，运行验证受阻于模拟器稳定性**

---

## 一、结论

**PoC 已闭环：APK 在 Android 15 上跑起来了，登录、建库、主界面全部正常。**

加密核心（PBKDF2 20 万轮 + AES-GCM）在 Android 上跑通，密库正确落在
`/data/data/com.secback.vault/files/sec.properties`（84 字节空库）。

**真正的障碍不是"能不能跑"，而是界面完全没做移动适配**——见第六节。

---

## 二、已完成的部分

| 项目 | 结果 |
|---|---|
| NDK 29.0.14206865 | ✅ 装好 |
| cmdline-tools 12.0 | ✅ 装好（原来没有 sdkmanager） |
| Rust android targets | ✅ aarch64 / armv7 / x86_64 / i686 |
| 模拟器 | ✅ 能启动（4G 内存，30 秒起） |
| `core` 交叉编译 | ✅ **零改动通过**（纯 Rust 的回报） |
| Tauri 工程结构改造 | ✅ lib/bin 分离，桌面端未受影响 |
| 平台 API 隔离 | ✅ 只有 3 类需要条件编译 |
| APK 构建 | ✅ 137MB（debug，含 129MB .so） |
| APK 安装 | ✅ 装进模拟器 |
| **运行验证** | ✅ **登录页渲染、建库成功、主界面显示** |
| 界面适配 | ❌ 布局未改（预期内，属正式移植工作） |

---

## 三、最关键发现：平台差异只有 3 处

原本担心菜单、主题、拖放、路径全要重做，实际编译只报了 3 类错误：

1. **`arboard`**（系统剪贴板）—— 桌面专属，Android 要换 `tauri-plugin-clipboard-manager`
2. **`tauri-plugin-single-instance`** —— Android 一个应用本来就一个前台实例，整块去掉
3. **`tauri::menu`** —— **整个模块在移动端不存在**，菜单项全要搬进前端界面

**意外**：`set_theme`、`DragDrop`、`exe_dir` 这些居然都能编译通过（运行时行为还没验证）。

改造方式：加 `#[cfg(not(any(target_os = "android", target_os = "ios")))]`，
桌面代码一行没删，只是被条件编译挡住。**桌面版编译时间和测试结果完全没变。**

---

## 四、必须做的一项结构改造

Tauri 移动端要求**库目标**，桌面版的纯 bin 结构不行：

```
src/lib.rs   ← 全部逻辑，pub fn run()，带 #[cfg_attr(mobile, tauri::mobile_entry_point)]
src/main.rs  ← 三行薄壳：fn main() { secback_lib::run() }
```

`Cargo.toml` 加：

```toml
[lib]
name = "secback_lib"
crate-type = ["staticlib", "cdylib", "rlib"]
```

这一步绕不过去，但**桌面行为完全不变**（已用 85 项测试确认）。

---

## 五、踩到的坑（都跟代码无关，跟环境有关）

### 1. Windows 创建符号链接需要开发者模式

Tauri CLI 把 `.so` 软链到 `jniLibs`，普通权限做不到，报
`Creation symbolic link is not allowed for this system`。

**绕过**：手动复制 `.so`，直接调 `gradlew assembleDebug`。

**正解**：开启 Windows 开发者模式，一劳永逸。

### 2. Gradle 官方源连不上

`services.gradle.org` 10 秒超时。改用腾讯镜像（已验证可用）：

```
https://mirrors.cloud.tencent.com/gradle/gradle-9.6.1-bin.zip
```

### 3. Tauri 的 gradle 插件找不到 `npm.bat`

Node.js 在 Windows 上只提供 `npm.cmd`，**没有 `npm.bat`**，而插件硬编码找 `npm.bat`。

**绕过**：在 PATH 里的可写目录放一个包装脚本转调 `npm.cmd`。

### 4. `package.json` 缺 `tauri` 脚本

插件的实际调用是 `npm run -- tauri android android-studio-script --target aarch64`，
`package.json` 里没有 `"tauri": "tauri"` 就必然失败。标准模板里都有，我们这份是手写的所以漏了。

### 5. Android 17 模拟器不稳定（**当前卡点**）

镜像 `android-37.0/google_apis_playstore/x86_64`（Android 17，API 37）。

现象：`sys.boot_completed=1` 之后，`activity` / `window` / `package` 服务会陆续消失，
报 `cmd: Can't find service: package` 或 `DEAD_OBJECT`。system_server 反复重启。

已排除：内存不足（4G 可用 3.3G）、硬件加速（WHPX 正常）、镜像损坏。

**判断**：Android 17 是 2026 年的最新版本，模拟器在这台机器（WHPX + 软件 GPU）上
不够稳。**换 Android 14/15（API 34/35）镜像应该就正常了。**

---

## 六、运行后发现的真问题（真正的移植工作量在这里）

### 1. 密库路径落在只读系统目录（已修）

首次启动界面显示"位置：/system/bin/sec.properties"。

Android 上没有"exe 同目录"这个概念：进程的可执行文件是系统的 `app_process`，
`current_exe()` 指到 `/system/bin`——只读系统分区，写盘必然失败。

已改为 `/data/data/com.secback.vault/files`（等同 `Context.getFilesDir()`），
实测建库成功。

### 2. 软键盘遮挡（未修）

密码框一聚焦，键盘直接盖住「进入」按钮和下面的提示文字。
移动端必须处理：`windowSoftInputMode=adjustResize` 或 `visualViewport` + 自动滚动。

### 3. 布局完全不适配（未修，工作量最大）

证据在 `screenshots/android-main.png`：

- 桌面是左右分栏，手机上右侧详情被挤成一条窄缝，只看得见"在左侧选择…"竖排几个字
- 工具栏 7 个按钮挤在一行，「新增」两个字被折成上下两行
- 搜索框、排序箭头、导入、锁定、退出全是按桌面尺寸排的

**这不是调几行 CSS 的事**：列表→详情的导航方式、工具栏收纳（抽屉/底栏）、
触摸目标尺寸（至少 44px）都要重做。这部分才是原估「前端适配 4~6 天」的真实内容。

### 4. 菜单功能无处安放（未解决）

Android 版没有菜单（`tauri::menu` 模块根本不存在），而桌面靠菜单承载了
新增 / 删除 / 导入 / 改密 / 设恢复码 / 主题 / 锁定 / 退出。

这些必须变成界面元素，否则 Android 版等于只有"看"，不能操作。

---

## 六、对工期估算的修正

原估「宿主改造 5~7 天」，实际**只用了约 2 小时**——因为平台差异只有 3 处，
且都是小范围改。

**原估算里最不靠谱的是"环境搭建 1~2 天"，实际光排环境就花了半天以上**，
而且坑几乎全在 Windows 工具链上（符号链接、npm.bat、Gradle 源、模拟器）。

修正后：

| 阶段 | 原估 | 现在看 |
|---|---|---|
| 环境 | 1~2 天 | **已经做完了**（含踩坑记录） |
| core 交叉编译 | 0.5 天 | **已经做完了，零改动** |
| 宿主改造 | 5~7 天 | **已做 70%**（剩剪贴板换插件、路径改 app 私有目录） |
| 前端适配 | 4~6 天 | 没动（安全区、软键盘、列表→详情导航、抽屉菜单） |
| 测试 | 4~6 天 | 没动（且必须有稳定模拟器或实机） |
| 打包 | 1~2 天 | 没动（签名、arm64） |

---

## 七、下一步选项

**A. 换 Android 14/15 镜像重启 PoC（推荐，约 20 分钟）**

```bash
sdkmanager "system-images;android-35;google_apis;x86_64"
avdmanager create avd -n secback_35 -k "system-images;android-35;google_apis;x86_64" -d pixel_5
```

然后重装 APK、截图验证界面。

**B. 开 Windows 开发者模式**——解决符号链接，之后 `npx tauri android build` 一条命令搞定。

**C. 等 P10 能开 USB 调试**——真机验证最有说服力，但 P10 是 HarmonyOS 2（Android 10 基线），
反而更接近真实用户环境。

---

## 八、当前工程状态

- 桌面版：**完全正常**（debug 构建 27 秒，85 项测试全过）
- Android 工程：`src-tauri/gen/android` 已生成，已从 `.gitignore` 里放出来（它是源码）
- 未提交的改动：lib/bin 分离、Cargo.toml、.gitignore、图标、`package.json` 的 tauri 脚本

**注意**：`src-tauri/gen/android/local.properties` 含本机 SDK 路径，不该提交；
`gen/android/app/src/main/jniLibs/` 里的 `.so` 也不该提交。
