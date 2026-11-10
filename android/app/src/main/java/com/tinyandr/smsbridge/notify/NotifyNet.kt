package com.tinyandr.smsbridge.notify

import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder

/** Minimal result holder so channels can report a real error text to the UI. */
data class HttpResult(val code: Int, val body: String) {
    val ok: Boolean get() = code in 200..299
}

/**
 * Tiny HTTP helper. The app talks to exactly two endpoints (Telegram Bot API and
 * a Feishu bot webhook), so pulling in an HTTP client library for that would add
 * a megabyte to the APK for nothing.
 */
internal object NotifyNet {

    private const val TIMEOUT_MS = 15_000

    fun postJson(url: String, json: String): HttpResult =
        post(url, json.toByteArray(Charsets.UTF_8), "application/json; charset=utf-8")

    fun postForm(url: String, fields: Map<String, String>): HttpResult {
        val body = fields.entries.joinToString("&") { "${enc(it.key)}=${enc(it.value)}" }
        return post(
            url,
            body.toByteArray(Charsets.UTF_8),
            "application/x-www-form-urlencoded; charset=utf-8"
        )
    }

    fun get(url: String): HttpResult {
        val c = (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = TIMEOUT_MS
            readTimeout = TIMEOUT_MS
            requestMethod = "GET"
        }
        return try {
            read(c)
        } finally {
            c.disconnect()
        }
    }

    private fun post(url: String, bytes: ByteArray, contentType: String): HttpResult {
        val c = (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = TIMEOUT_MS
            readTimeout = TIMEOUT_MS
            requestMethod = "POST"
            doOutput = true
            setRequestProperty("Content-Type", contentType)
            // Content-Length is deliberately not set by hand: HttpURLConnection
            // derives it from the buffered body, and a manual value that drifts
            // from the real byte count is one way to get the stack to abort the
            // connection mid-write.
            setFixedLengthStreamingMode(bytes.size)
        }
        return try {
            c.outputStream.use { it.write(bytes) }
            read(c)
        } finally {
            c.disconnect()
        }
    }

    private fun read(c: HttpURLConnection): HttpResult {
        val code = c.responseCode
        val stream = if (code in 200..299) c.inputStream else c.errorStream
        val text = stream?.bufferedReader(Charsets.UTF_8)?.use { it.readText() } ?: ""
        return HttpResult(code, text)
    }

    private fun enc(s: String): String = URLEncoder.encode(s, "UTF-8")
}
