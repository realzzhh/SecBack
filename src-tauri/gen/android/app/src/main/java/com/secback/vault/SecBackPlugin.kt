package com.secback.vault

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.WindowCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.UUID

@InvokeArg class CopyArgs { lateinit var text: String }
@InvokeArg class ThemeArgs { var dark: Boolean = true }
@InvokeArg class WatchArgs { lateinit var channel: Channel }
@TauriPlugin
class SecBackPlugin(private val activity: Activity): Plugin(activity) {
    private val clipboard get() = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    private val handler = Handler(Looper.getMainLooper())
    private var token: String? = null
    private var channel: Channel? = null
    private fun clearOwned() {
        try {
            if (token != null && clipboard.primaryClipDescription?.label?.toString() == token) {
                if (Build.VERSION.SDK_INT >= 28) clipboard.clearPrimaryClip()
                else clipboard.setPrimaryClip(ClipData.newPlainText("", ""))
            }
        } catch (_: Exception) { /* Android may deny clipboard access after backgrounding. */ }
        token = null
    }
    @Command fun copy(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(CopyArgs::class.java)
            val marker = "SecBack:" + UUID.randomUUID().toString()
            val clip = ClipData.newPlainText(marker, args.text)
            clip.description.extras = PersistableBundle().apply { putBoolean("android.content.extra.IS_SENSITIVE", true) }
            clipboard.setPrimaryClip(clip)
            token = marker
            handler.removeCallbacksAndMessages(null)
            handler.postDelayed({ clearOwned() }, 60_000)
            invoke.resolve(JSObject())
        } catch (_: Exception) { invoke.reject("写入系统剪贴板失败") }
    }
    @Command fun theme(invoke: Invoke) {
        val dark = invoke.parseArgs(ThemeArgs::class.java).dark
        activity.runOnUiThread {
            val color = android.graphics.Color.parseColor(if (dark) "#14161a" else "#f5f6f8")
            activity.window.decorView.setBackgroundColor(color)
            WindowCompat.getInsetsController(activity.window, activity.window.decorView).apply {
                isAppearanceLightStatusBars = !dark
                isAppearanceLightNavigationBars = !dark
            }
            invoke.resolve(JSObject())
        }
    }
    @Command fun watch(invoke: Invoke) { channel = invoke.parseArgs(WatchArgs::class.java).channel; invoke.resolve(JSObject()) }
    override fun onPause(activity: AppCompatActivity) { clearOwned() }
    override fun onStop(activity: AppCompatActivity) { channel?.send(JSObject().put("background", true)) }
}
