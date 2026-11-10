package com.tinyandr.smsbridge.notify

import android.content.Context
import com.tinyandr.smsbridge.R
import com.tinyandr.smsbridge.data.SmsMessage
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * The push payload. Field order and the `label:value` shape are deliberate:
 * four lines, nothing else, so a glance at the chat is enough to tell which
 * number sent it, which card received it, when, and what it said.
 */
object SmsMessageFormat {

    data class Field(val label: String, val value: String)

    fun fields(ctx: Context, m: SmsMessage): List<Field> = listOf(
        Field(ctx.getString(R.string.msg_from), m.address.ifBlank { "-" }),
        Field(ctx.getString(R.string.msg_time), time(m.date)),
        Field(ctx.getString(R.string.msg_sim), simLabel(m)),
        Field(ctx.getString(R.string.msg_body), m.body.trim().ifBlank { "-" })
    )

    /** Plain four-line text, used by Feishu's text fallback and for copy/paste. */
    fun plain(ctx: Context, m: SmsMessage): String =
        fields(ctx, m).joinToString("\n") { "${it.label}:${it.value}" }

    /** Same four lines with bold labels for Telegram's HTML parse mode. */
    fun html(ctx: Context, m: SmsMessage): String =
        fields(ctx, m).joinToString("\n") { "<b>${esc(it.label)}</b>:${esc(it.value)}" }

    /**
     * `SIM1_19973712132`: slot is 1-based for humans, and the number is reduced
     * to bare digits so it reads like a phone number rather than E.164. The
     * country code is dropped only when it is exactly the +86 prefix.
     */
    fun simLabel(m: SmsMessage): String {
        val slot = if (m.slot >= 0) "SIM${m.slot + 1}" else "SIM"
        val digits = m.simNumber.filter { it.isDigit() }.let {
            if (m.simNumber.trim().startsWith("+86") && it.length == 13) it.substring(2) else it
        }
        return if (digits.isEmpty()) slot else "${slot}_$digits"
    }

    fun time(ms: Long): String =
        SimpleDateFormat("yyyy-MM-dd HH:mm:ss", Locale.getDefault()).format(Date(ms))

    fun esc(s: String): String = s
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
}
