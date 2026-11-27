import java.util.Properties
import groovy.json.JsonSlurper
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

val secbackSourceRoot = file(project.findProperty("secback.sourceRoot")?.toString() ?: "../../../..")
val signingSecrets = Properties().apply {
    val secrets = secbackSourceRoot.resolve("keystore/signing.properties")
    if (secrets.exists()) secrets.inputStream().use { load(it) }
}
fun signingValue(key: String): String? = System.getenv("SECBACK_" + key.uppercase()) ?: signingSecrets.getProperty(key)
if (gradle.startParameter.taskNames.any { it.contains("Release", ignoreCase = true) }) {
    require(listOf("storePassword", "keyAlias", "keyPassword").all { !signingValue(it).isNullOrBlank() }) {
        "发布签名缺失：配置 keystore/signing.properties 或 SECBACK_STOREPASSWORD / SECBACK_KEYALIAS / SECBACK_KEYPASSWORD"
    }
}

val productMetadata = JsonSlurper().parse(secbackSourceRoot.resolve("package.json")) as Map<*, *>
val productVersion = productMetadata["version"].toString()
val versionParts = productVersion.split('.').map { it.toInt() }
require(versionParts.size == 3 && versionParts.all { it in 0..999 }) { "版本号须为三个 0～999 的整数" }
// 展示版本与安装升级序号独立，允许 1.0.0 调试包安全过渡到 0.1.0。
val productVersionCode = (productMetadata["androidVersionCode"] as Number).toInt()
require(productVersionCode in 1..2_100_000_000) { "androidVersionCode 必须为正整数且不超过 2100000000" }

android {
    // release 包必须签名才能装到手机上。这里指向项目根目录 keystore/ 下那一个，
    // 换成别的证书会导致「已安装的旧版无法覆盖升级」，所以别随便换。
    // 证书和密码都在 .gitignore 里，不入库。
    signingConfigs {
        create("release") {
            storeFile = secbackSourceRoot.resolve("keystore/secback-release.keystore")
            storePassword = signingValue("storePassword")
            keyAlias = signingValue("keyAlias")
            keyPassword = signingValue("keyPassword")
        }
    }

    compileSdk = 37
    namespace = "com.secback.vault"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.secback.vault"
        minSdk = 24
        targetSdk = 37
        versionCode = productVersionCode
        versionName = productVersion
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            signingConfig = signingConfigs.getByName("release")
            // 关掉 R8 混淆和资源压缩。
            // Tauri 的 Java 桥靠 JNI 反射找类名，混淆后很容易变成「一启动就闪退」，
            // 而这种崩溃没有可读堆栈、极难定位。省下的那点体积（大头是 .so）
            // 不值得拿稳定性换。等有稳定的测试链路再考虑打开。
            isMinifyEnabled = false
            isShrinkResources = false
            optimization {
               enable = false
            }
            proguardFiles(
                *fileTree(".") {
                  include("**/*.pro")
                  exclude("build/**")
                }.files.toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = file("tauri.build.gradle.kts"))
