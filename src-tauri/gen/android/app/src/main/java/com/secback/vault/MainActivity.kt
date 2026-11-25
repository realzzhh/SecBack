package com.secback.vault

import android.os.Bundle

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    // 注意：这里**故意**不调用 enableEdgeToEdge()。
    //
    // Tauri 的模板默认会调它，让 WebView 铺满整个屏幕（包括状态栏底下）。
    // 我们这套界面是按"有边距的文档流"设计的，不是沉浸式全屏，
    // 全屏的后果是：状态栏的时间和电量被工具栏压住，
    // 页面里的 100vh 把状态栏和导航栏也算进去，弹层底部超出屏幕。
    //
    // 让系统把内容区放在状态栏下方更合适。
    //
    // 另外 targetSdk 35+ 会强制 edge-to-edge，光不调这个函数还不够，
    // 要配合 themes.xml 里的 windowOptOutEdgeToEdgeEnforcement。
    super.onCreate(savedInstanceState)
  }
}
