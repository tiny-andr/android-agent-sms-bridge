package com.tinyandr.smsbridge.notify

import android.content.Context
import android.util.Base64
import android.util.Log
import com.tinyandr.smsbridge.R
import com.tinyandr.smsbridge.data.SmsMessage
import org.json.JSONArray
import org.json.JSONObject
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * Feishu (Lark) custom-bot webhook.
 *
 * Two payload shapes: an interactive card (readable, keeps the sender in the
 * header) and plain text. The card is tried first and text is the fallback, so a
 * bot configured with restricted card permissions still gets the message through.
 *
 * Signature: HMAC-SHA256 keyed with "<timestamp>\n<secret>", signing an empty
 * body, base64 encoded. Only sent when a secret is configured.
 */
internal object FeishuChannel {

    private const val TAG = "FeishuChannel"

    fun sendSms(ctx: Context, webhook: String, secret: String, m: SmsMessage): String? {
        if (webhook.isBlank()) return "webhook is empty"
        // A dropped connection must stay a failed push. This runs on the push
        // worker, and an uncaught exception there kills the whole process —
        // losing the service, not just the message.
        return try {
            val fields = SmsMessageFormat.fields(ctx, m)
            val cardError = sendCard(webhook, secret, m.address, fields)
            if (cardError == null) return null
            Log.w(TAG, "card rejected ($cardError), falling back to text")
            val textError = sendText(webhook, secret, SmsMessageFormat.plain(ctx, m))
            if (textError == null) null else "$cardError / $textError"
        } catch (t: Throwable) {
            Log.w(TAG, "sendSms failed", t)
            t.message ?: t.javaClass.simpleName
        }
    }

    private fun sendCard(
        webhook: String,
        secret: String,
        sender: String,
        fields: List<SmsMessageFormat.Field>
    ): String? {
        val card = JSONObject().apply {
            put("config", JSONObject().put("wide_screen_mode", true))
            put(
                "header", JSONObject()
                    .put("template", "blue")
                    .put(
                        "title", JSONObject()
                            .put("tag", "plain_text")
                            .put("content", sender.ifBlank { "SMS" })
                    )
            )
            // Sender / time / card go into a compact two-column grid; the body
            // gets the full width underneath, separated by a divider.
            val grid = JSONArray()
            fields.take(3).forEach { f ->
                grid.put(
                    JSONObject().put("is_short", true).put(
                        "text", JSONObject()
                            .put("tag", "lark_md")
                            .put("content", "**${f.label}**\n${f.value}")
                    )
                )
            }
            val body = fields.last()
            val elements = JSONArray()
                .put(JSONObject().put("tag", "div").put("fields", grid))
                .put(JSONObject().put("tag", "hr"))
                .put(
                    JSONObject().put("tag", "div").put(
                        "text", JSONObject()
                            .put("tag", "lark_md")
                            .put("content", "**${body.label}**\n${body.value}")
                    )
                )
            put("elements", elements)
        }
        val payload = envelope(secret).put("msg_type", "interactive").put("card", card)
        return parse(NotifyNet.postJson(webhook, payload.toString()))
    }

    private fun sendText(webhook: String, secret: String, text: String): String? {
        val payload = envelope(secret)
            .put("msg_type", "text")
            .put("content", JSONObject().put("text", text))
        return parse(NotifyNet.postJson(webhook, payload.toString()))
    }

    private fun envelope(secret: String): JSONObject {
        val o = JSONObject()
        if (secret.isBlank()) return o
        val ts = (System.currentTimeMillis() / 1000L).toString()
        o.put("timestamp", ts)
        o.put("sign", sign(ts, secret))
        return o
    }

    fun sign(timestamp: String, secret: String): String {
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(
            SecretKeySpec(
                "$timestamp\n$secret".toByteArray(Charsets.UTF_8), "HmacSHA256"
            )
        )
        return Base64.encodeToString(mac.doFinal(ByteArray(0)), Base64.NO_WRAP)
    }

    private fun parse(r: HttpResult): String? {
        val json = runCatching { JSONObject(r.body) }.getOrNull()
            ?: return "HTTP ${r.code}"
        // Current API answers {"code":0}, the older one {"StatusCode":0}.
        val code = if (json.has("code")) json.optInt("code", -1) else json.optInt("StatusCode", -1)
        if (code == 0) return null
        val msg = json.optString("msg").ifBlank { json.optString("StatusMessage") }
        return if (msg.isBlank()) "code $code" else "$msg (code $code)"
    }

    fun sendTest(ctx: Context, webhook: String, secret: String): String? {
        if (webhook.isBlank()) return "webhook is empty"
        val text = "${ctx.getString(R.string.msg_from)}:SMS Bridge\n" +
            "${ctx.getString(R.string.msg_body)}:${ctx.getString(R.string.notify_test_body)}"
        return try {
            val cardError = sendCard(
                webhook, secret, "SMS Bridge",
                listOf(
                    SmsMessageFormat.Field(ctx.getString(R.string.msg_from), "SMS Bridge"),
                    SmsMessageFormat.Field(
                        ctx.getString(R.string.msg_time), SmsMessageFormat.time(System.currentTimeMillis())
                    ),
                    SmsMessageFormat.Field(ctx.getString(R.string.msg_sim), "SIM1"),
                    SmsMessageFormat.Field(ctx.getString(R.string.msg_body), ctx.getString(R.string.notify_test_body))
                )
            )
            if (cardError == null) return null
            val textError = sendText(webhook, secret, text)
            if (textError == null) null else "$cardError / $textError"
        } catch (t: Throwable) {
            Log.w(TAG, "sendTest failed", t)
            t.message ?: t.javaClass.simpleName
        }
    }
}
