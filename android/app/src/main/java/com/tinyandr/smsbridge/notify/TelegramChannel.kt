package com.tinyandr.smsbridge.notify

import android.content.Context
import android.util.Log
import com.tinyandr.smsbridge.R
import com.tinyandr.smsbridge.data.SmsMessage
import org.json.JSONObject

/**
 * Telegram Bot API. Two calls only: sendMessage, and getUpdates (used once, to
 * learn the chat id from the last message the bot received).
 */
internal object TelegramChannel {

    private const val TAG = "TelegramChannel"
    private const val API = "https://api.telegram.org/bot"

    /** Telegram rejects anything over 4096 characters; leave room for the markup. */
    private const val MAX_TEXT = 3800

    /** null means delivered, otherwise a human readable reason. */
    fun sendSms(token: String, chatId: String, ctx: Context, m: SmsMessage): String? =
        send(token, chatId, SmsMessageFormat.html(ctx, m))

    fun send(token: String, chatId: String, html: String): String? {
        if (token.isBlank()) return "bot token is empty"
        if (chatId.isBlank()) return "chat id is empty"
        val text = if (html.length <= MAX_TEXT) html else html.take(MAX_TEXT) + "…"
        return try {
            val r = NotifyNet.postForm(
                "$API$token/sendMessage",
                mapOf(
                    "chat_id" to chatId,
                    "text" to text,
                    "parse_mode" to "HTML",
                    "disable_web_page_preview" to "true"
                )
            )
            val json = runCatching { JSONObject(r.body) }.getOrNull()
            if (r.ok && json?.optBoolean("ok") == true) return null
            val desc = json?.optString("description").orEmpty()
            if (desc.isNotBlank()) desc else "HTTP ${r.code}"
        } catch (t: Throwable) {
            Log.w(TAG, "send failed", t)
            t.message ?: t.javaClass.simpleName
        }
    }

    /**
     * The chat id of the most recent update. A bot can never start a conversation,
     * so the user has to say something to it first; this turns that one manual
     * step into a button instead of asking for a numeric id.
     */
    fun fetchChatId(token: String): Pair<String?, String?> {
        if (token.isBlank()) return null to "bot token is empty"
        return try {
            val r = NotifyNet.get("$API$token/getUpdates?limit=100&timeout=0")
            val json = JSONObject(r.body)
            if (!json.optBoolean("ok")) {
                val desc = json.optString("description").ifBlank { "HTTP ${r.code}" }
                return null to desc
            }
            val arr = json.optJSONArray("result")
            if (arr == null || arr.length() == 0) return null to "no message yet"
            val last = arr.getJSONObject(arr.length() - 1)
            val chat = last.optJSONObject("message")?.optJSONObject("chat")
                ?: last.optJSONObject("edited_message")?.optJSONObject("chat")
                ?: last.optJSONObject("channel_post")?.optJSONObject("chat")
            val id = chat?.optLong("id") ?: 0L
            if (id == 0L) null to "the last update has no chat"
            else id.toString() to null
        } catch (t: Throwable) {
            Log.w(TAG, "getUpdates failed", t)
            null to (t.message ?: t.javaClass.simpleName)
        }
    }

    /** Used by the UI's test button; a fixed payload so the shape is obvious. */
    fun sendTest(token: String, chatId: String, ctx: Context): String? {
        val html = "<b>${SmsMessageFormat.esc(ctx.getString(R.string.msg_from))}</b>:SMS Bridge\n" +
            "<b>${SmsMessageFormat.esc(ctx.getString(R.string.msg_body))}</b>:" +
            SmsMessageFormat.esc(ctx.getString(R.string.notify_test_body))
        return send(token, chatId, html)
    }
}
