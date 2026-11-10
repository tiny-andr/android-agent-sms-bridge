package com.tinyandr.smsbridge.notify

import android.content.Context
import android.util.Log
import com.tinyandr.smsbridge.BuildConfig
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.data.SmsMessage
import java.util.concurrent.Executors

/**
 * Fans a batch of new messages out to every enabled channel.
 *
 * Everything runs on one serial worker: webhooks are rate limited (Feishu allows
 * 100 calls a minute, Telegram one message a second per chat), and a burst of ten
 * messages arriving at once should not fire ten parallel requests. The queue is
 * in memory only; a failed push is logged, not retried, because the next scan
 * would otherwise keep replaying old messages.
 */
object ProactiveNotifier {

    private const val TAG = "ProactiveNotifier"

    private val worker = Executors.newSingleThreadExecutor { r ->
        Thread(r, "proactive-push").apply { isDaemon = true }
    }

    /** Called from the SMS observer and from the periodic scan. */
    fun onNewMessages(ctx: Context, msgs: List<SmsMessage>) {
        if (msgs.isEmpty()) return
        val s = SmsBridge.settings
        if (!s.notifyAnyTrigger) return
        if (!s.notifyAnyChannel) return
        val keys = keywords(s.notifyKeywords)
        val hits = msgs.filter { matches(it, keys) }
        if (hits.isEmpty()) {
            Log.i(TAG, "keyword filter dropped ${msgs.size} message(s)")
            return
        }
        val app = ctx.applicationContext
        worker.execute {
            hits.forEach { m ->
                // An uncaught exception on this worker terminates the process,
                // which would take the foreground service down with it. A failed
                // push must never cost more than the push itself.
                try {
                    push(app, m)
                } catch (t: Throwable) {
                    Log.w(TAG, "push failed for sms#${m.id}", t)
                }
            }
        }
    }

    private fun push(ctx: Context, m: SmsMessage) {
        val s = SmsBridge.settings
        if (BuildConfig.DEBUG) {
            // Debug builds only: seeing the exact payload is the difference
            // between "some message arrived" and "the right fields arrived".
            Log.d(TAG, "payload sms#${m.id}\n${SmsMessageFormat.plain(ctx, m)}")
        }
        if (s.tgEnabled && s.tgToken.isNotBlank() && s.tgChatId.isNotBlank()) {
            val err = sendChannel { TelegramChannel.sendSms(s.tgToken, s.tgChatId, ctx, m) }
            Log.i(TAG, "telegram sms#${m.id}: ${err ?: "ok"}")
        }
        if (s.feishuEnabled && s.feishuWebhook.isNotBlank()) {
            val err = sendChannel { FeishuChannel.sendSms(ctx, s.feishuWebhook, s.feishuSecret, m) }
            Log.i(TAG, "feishu sms#${m.id}: ${err ?: "ok"}")
        }
    }

    /**
     * A connection that the stack aborts is retried once; a provider rejection
     * is not, because the request clearly reached the server and replaying it
     * would only produce a duplicate card. The abort case usually means the body
     * never left the device, so the retry is the difference between a delivered
     * notification and a silently missed one.
     */
    private fun sendChannel(block: () -> String?): String? {
        var threw = false
        val first = try {
            block()
        } catch (t: Throwable) {
            threw = true
            Log.w(TAG, "channel threw", t)
            t.message ?: t.javaClass.simpleName
        }
        if (!threw) return first
        Thread.sleep(RETRY_DELAY_MS)
        return attempt(block)
    }

    /** Runs one channel, turning every failure into a message for the log. */
    private inline fun attempt(block: () -> String?): String? = try {
        block()
    } catch (t: Throwable) {
        Log.w(TAG, "channel failed", t)
        t.message ?: t.javaClass.simpleName
    }

    // ------------------------------------------------------------------ tests

    /** Blocking; call from a worker thread. Returns null on success. */
    fun sendTest(ctx: Context, channel: String): String? = attempt {
        val s = SmsBridge.settings
        val app = ctx.applicationContext
        when (channel) {
            CHANNEL_TELEGRAM -> TelegramChannel.sendTest(s.tgToken, s.tgChatId, app)
            CHANNEL_FEISHU -> FeishuChannel.sendTest(app, s.feishuWebhook, s.feishuSecret)
            else -> "unknown channel"
        }
    }

    /** Blocking; call from a worker thread. (chatId, error) */
    fun fetchTelegramChatId(token: String): Pair<String?, String?> =
        TelegramChannel.fetchChatId(token)

    // -------------------------------------------------------------- filtering

    /** null when delivered, otherwise the provider's own words. */
    fun keywords(raw: String): List<String> = raw
        .split(',', '，', ';', '；', '\n', ' ')
        .map { it.trim().lowercase() }
        .filter { it.isNotEmpty() }

    fun matches(m: SmsMessage, keys: List<String>): Boolean {
        if (keys.isEmpty()) return true
        val address = m.address.lowercase()
        val body = m.body.lowercase()
        return keys.any { address.contains(it) || body.contains(it) }
    }

    const val CHANNEL_TELEGRAM = "telegram"
    const val CHANNEL_FEISHU = "feishu"

    private const val RETRY_DELAY_MS = 900L
}
