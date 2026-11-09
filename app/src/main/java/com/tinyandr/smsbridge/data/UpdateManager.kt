package com.tinyandr.smsbridge.data

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import androidx.core.content.FileProvider
import com.tinyandr.smsbridge.BuildConfig
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * Checks GitHub Releases for a newer build, downloads it (direct link first,
 * then mainland-friendly mirrors) and hands the APK to the system installer.
 *
 * The SHA-256 comes from the release metadata, so a mirror can only serve a
 * byte-identical file; a mismatch is deleted and the next mirror is tried.
 */
class UpdateManager(private val context: Context) {

    data class Asset(val name: String, val size: Long, val sha256: String, val url: String)

    data class Release(val version: String, val notes: String, val asset: Asset)

    data class Outcome(val release: Release?, val error: String?)

    private val sp = context.getSharedPreferences("smsbridge_update", Context.MODE_PRIVATE)

    private fun dir(): File = File(context.filesDir, "updates").apply { mkdirs() }

    fun check(force: Boolean): Outcome {
        if (!force) {
            val last = sp.getLong(KEY_LAST_CHECK, 0L)
            if (System.currentTimeMillis() - last < CHECK_INTERVAL_MS) {
                return Outcome(null, null)
            }
        }
        return try {
            val body = http(API, 8000)
            val json = JSONObject(body)
            sp.edit().putLong(KEY_LAST_CHECK, System.currentTimeMillis()).apply()

            val tag = json.optString("tag_name").trim().removePrefix("v")
            if (tag.isEmpty()) return Outcome(null, "发布信息不完整")
            if (compare(tag, BuildConfig.VERSION_NAME) <= 0) return Outcome(null, null)

            val assets = json.optJSONArray("assets")
            var picked: Asset? = null
            if (assets != null) {
                for (i in 0 until assets.length()) {
                    val a = assets.getJSONObject(i)
                    val name = a.optString("name")
                    if (!name.startsWith("SmsBridge-") || !name.endsWith(".apk")) continue
                    val digest = a.optString("digest")
                    picked = Asset(
                        name = name,
                        size = a.optLong("size", 0L),
                        sha256 = if (digest.startsWith("sha256:")) digest.removePrefix("sha256:") else "",
                        url = a.optString("browser_download_url")
                    )
                    break
                }
            }
            val asset = picked ?: return Outcome(null, "这个版本没有 APK 附件")
            Outcome(Release(tag, json.optString("body", ""), asset), null)
        } catch (t: Throwable) {
            Log.w(TAG, "check failed: ${t.message}")
            Outcome(null, t.message ?: "检查更新失败")
        }
    }

    /** Downloads [asset], trying every mirror in turn. Returns the verified file. */
    fun download(asset: Asset, onProgress: (Int, Int) -> Unit): File {
        val target = File(dir(), "update.apk")
        var lastError: Throwable? = null
        for (prefix in SOURCES) {
            val url = prefix + asset.url
            try {
                if (target.exists()) target.delete()
                val conn = (URL(url).openConnection() as HttpURLConnection).apply {
                    connectTimeout = 8000
                    readTimeout = 15000
                    instanceFollowRedirects = true
                    setRequestProperty("User-Agent", UA)
                }
                if (conn.responseCode !in 200..299) {
                    throw IllegalStateException("HTTP ${conn.responseCode}")
                }
                val total = conn.contentLengthLong.takeIf { it > 0 } ?: asset.size
                conn.inputStream.use { input ->
                    target.outputStream().use { output ->
                        val buf = ByteArray(64 * 1024)
                        var done = 0L
                        while (true) {
                            val n = input.read(buf)
                            if (n <= 0) break
                            output.write(buf, 0, n)
                            done += n
                            onProgress((done / 1024).toInt(), (total / 1024).toInt())
                        }
                    }
                }
                if (asset.size > 0 && target.length() != asset.size) {
                    throw IllegalStateException("大小不符 ${target.length()}/${asset.size}")
                }
                if (asset.sha256.isNotEmpty() && !sha256(target).equals(asset.sha256, ignoreCase = true)) {
                    throw IllegalStateException("SHA-256 不匹配")
                }
                return target
            } catch (t: Throwable) {
                Log.w(TAG, "download via ${prefix.ifEmpty { "github" }} failed: ${t.message}")
                lastError = t
            }
        }
        throw IllegalStateException(lastError?.message ?: "下载失败")
    }

    fun canInstall(): Boolean =
        context.packageManager.canRequestPackageInstalls()

    /** Opens the system installer. The user still has to tap "install" (no root). */
    fun install(file: File) {
        val uri: Uri = FileProvider.getUriForFile(
            context, "${context.packageName}.fileprovider", file
        )
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, "application/vnd.android.package-archive")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        context.startActivity(intent)
    }

    fun clear() {
        dir().listFiles()?.forEach { it.delete() }
    }

    private fun http(url: String, timeout: Int): String {
        val conn = (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = timeout
            readTimeout = timeout
            setRequestProperty("User-Agent", UA)
            setRequestProperty("Accept", "application/vnd.github+json")
        }
        return try {
            if (conn.responseCode !in 200..299) throw IllegalStateException("HTTP ${conn.responseCode}")
            conn.inputStream.bufferedReader().use { it.readText() }
        } finally {
            conn.disconnect()
        }
    }

    private fun sha256(file: File): String {
        val md = MessageDigest.getInstance("SHA-256")
        file.inputStream().use { input ->
            val buf = ByteArray(64 * 1024)
            while (true) {
                val n = input.read(buf)
                if (n <= 0) break
                md.update(buf, 0, n)
            }
        }
        return md.digest().joinToString("") { "%02x".format(it) }
    }

    private fun compare(a: String, b: String): Int {
        val pa = a.split('.').map { it.takeWhile(Char::isDigit).toIntOrNull() ?: 0 }
        val pb = b.split('.').map { it.takeWhile(Char::isDigit).toIntOrNull() ?: 0 }
        for (i in 0 until maxOf(pa.size, pb.size)) {
            val x = pa.getOrElse(i) { 0 }
            val y = pb.getOrElse(i) { 0 }
            if (x != y) return x.compareTo(y)
        }
        return 0
    }

    companion object {
        private const val TAG = "UpdateManager"
        private const val UA = "SmsBridge-Android"
        private const val API =
            "https://api.github.com/repos/tiny-andr/android-agent-sms-bridge/releases/latest"
        private const val CHECK_INTERVAL_MS = 6 * 3600_000L
        private const val KEY_LAST_CHECK = "lastCheck"

        /** Empty prefix = GitHub direct; the rest are public mainland mirrors. */
        private val SOURCES = listOf(
            "",
            "https://ghfast.top/",
            "https://gh-proxy.com/",
            "https://ghproxy.net/",
            "https://ghproxy.site/"
        )
    }
}
