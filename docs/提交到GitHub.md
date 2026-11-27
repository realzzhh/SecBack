# 版本控制与发布

## 提交范围

提交 core、src、src-tauri 宿主及 Android 手工工程、scripts、tests、CI 配置和文档（包括保留迁移说明的 docs/archive）。提交 package-lock.json、Cargo.lock，以及非敏感的 Gradle 配置与 wrapper 版本／校验和。gen/android 是受维护源码，不能整体忽略或通过 android init 覆盖。自动生成 schema、Rust/Gradle 缓存、JNI .so 和 node_modules 不提交。图标 SVG、ICO、PNG、ICNS 及 Android 资源属于产品资源，即使通过脚本生成也应提交，保证拉取源码后可直接构建并保持各平台图标一致。

keystore/、签名配置、真实 secback.cfg、密库、备份及调试日志不提交。发布密码通过 keystore/signing.properties 或 SECBACK_STOREPASSWORD、SECBACK_KEYALIAS、SECBACK_KEYPASSWORD 环境变量提供，不写回 Gradle 源码。

## 交付物

APK、exe 和随包生成的 manifest 均属于交付产物，不进入源码提交。2026-11-27 起，历史已跟踪的 delivery/android/SecBack-Android-arm64.apk 从 Git 索引移除，磁盘文件继续保留；不重写历史提交。需要对外提供安装包时，单独上传发行附件。

发布同时生成 manifest，记录版本、versionCode、ABI、APK/.so SHA-256、签名指纹、源提交和工作区是否含修改。若是在提交前构建，sourceDirty=true 是真实状态，不能将其伪装为干净提交产物。

任何推送、发布到外部服务等对外动作需获得用户授权。本轮仅本地提交。个人脚本 `清理剩余构建缓存.ps1` 保留本机并已忽略。

## 持续检查

.github/workflows/check.yml 配置 Windows/Linux 核心回归、Windows 协议回归、版本及 JS 语法检查，并运行 npm audit 与 cargo audit。工作流存在不代表本轮已在 GitHub 执行通过；在线数据库结果以实际运行时为准。Android Gradle 依赖更新需单独做构建与真机回归，已提供解析实际依赖的 Gradle/OSV 审计脚本与 CI 步骤；已知剩余项及本地结果见 Android导入与依赖审计-2026-11-26.md。工作流未在远端运行时不能声称通过。
