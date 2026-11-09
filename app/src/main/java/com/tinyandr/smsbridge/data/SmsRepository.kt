package com.tinyandr.smsbridge.data

import android.content.Context
import android.provider.Telephony
import android.telephony.SubscriptionManager
import android.util.Log
import com.tinyandr.smsbridge.util.OtpExtractor

data class SmsMessage(
    val id: Long,
    val address: String,
    val body: String,
    val date: Long,
    val type: Int,
    val subId: Int,
    val slot: Int,
    val carrier: String,
    val simNumber: String,
    val read: Boolean
) {
    val isInbox: Boolean get() = type == Telephony.Sms.MESSAGE_TYPE_INBOX
}

data class SmsQuery(
    val days: Int? = null,
    val fromMs: Long? = null,
    val toMs: Long? = null,
    val addresses: List<String> = emptyList(),
    val slot: Int? = null,
    val box: String = "inbox",
    val limit: Int = 50,
    val offset: Int = 0,
    val sinceId: Long? = null,
    val search: String? = null,
    val otpOnly: Boolean = false
)

class SmsRepository(private val context: Context) {

    data class SimInfo(
        val subId: Int,
        val slot: Int,
        val carrier: String,
        val number: String = ""
    )

    @Volatile
    private var cachedSims: List<SimInfo> = emptyList()
    @Volatile
    private var simsAt = 0L

    fun sims(force: Boolean = false): List<SimInfo> {
        val now = System.currentTimeMillis()
        if (!force && now - simsAt < 15_000L && cachedSims.isNotEmpty()) return cachedSims
        cachedSims = try {
            val sm = context.getSystemService(Context.TELEPHONY_SUBSCRIPTION_SERVICE) as SubscriptionManager
            sm.activeSubscriptionInfoList?.map {
                SimInfo(
                    subId = it.subscriptionId,
                    slot = it.simSlotIndex,
                    carrier = it.carrierName?.toString() ?: "",
                    number = try {
                        it.number?.trim() ?: ""
                    } catch (t: Throwable) {
                        Log.w(TAG, "sim number unavailable: ${t.message}")
                        ""
                    }
                )
            } ?: emptyList()
        } catch (t: Throwable) {
            Log.w(TAG, "sims() failed: ${t.message}")
            emptyList()
        }
        simsAt = now
        return cachedSims
    }

    private fun simFor(subId: Int): SimInfo? = sims().firstOrNull { it.subId == subId }

    fun query(q: SmsQuery): List<SmsMessage> {
        val now = System.currentTimeMillis()
        val where = StringBuilder()
        val args = mutableListOf<String>()

        when (q.box.lowercase()) {
            "inbox" -> where.append("type = ?").also { args.add(Telephony.Sms.MESSAGE_TYPE_INBOX.toString()) }
            "sent" -> where.append("type = ?").also { args.add(Telephony.Sms.MESSAGE_TYPE_SENT.toString()) }
            "draft" -> where.append("type = ?").also { args.add(Telephony.Sms.MESSAGE_TYPE_DRAFT.toString()) }
            else -> where.append("1 = 1")
        }

        val from = q.fromMs ?: q.days?.let { now - it.toLong() * 86_400_000L }
        from?.let { where.append(" AND date >= ?"); args.add(it.toString()) }
        q.toMs?.let { where.append(" AND date <= ?"); args.add(it.toString()) }
        q.sinceId?.let { where.append(" AND _id > ?"); args.add(it.toString()) }

        if (q.addresses.isNotEmpty()) {
            where.append(" AND (")
            q.addresses.forEachIndexed { i, a ->
                if (i > 0) where.append(" OR ")
                // exact match, plus suffix match so "10086" hits "+8610086"
                where.append("(address = ? OR address LIKE ?)")
                args.add(a)
                args.add("%$a")
            }
            where.append(")")
        }

        if (q.slot != null) {
            val subIds = sims().filter { it.slot == q.slot }.map { it.subId }
            if (subIds.isNotEmpty()) {
                where.append(" AND sub_id IN (")
                where.append(subIds.joinToString(",") { "?" })
                where.append(")")
                subIds.forEach { args.add(it.toString()) }
            }
        }

        q.search?.takeIf { it.isNotBlank() }?.let {
            where.append(" AND body LIKE ?")
            args.add("%$it%")
        }

        val blocked = blockedPrefixes()
        if (blocked.isNotEmpty()) {
            where.append(" AND (")
            blocked.forEachIndexed { i, b ->
                if (i > 0) where.append(" AND ")
                where.append("address NOT LIKE ?")
                args.add("%$b")
            }
            where.append(")")
        }

        val sort = "date DESC LIMIT ${q.limit.coerceAtLeast(1)} OFFSET ${q.offset.coerceAtLeast(0)}"

        val rows = rawQuery(where.toString(), args.toTypedArray(), sort)
        return rows.map { m ->
            val otp = if (q.otpOnly) OtpExtractor.extract(m.body) else null
            if (q.otpOnly && otp == null) return@map null
            m
        }.filterNotNull()
    }

    /**
     * Returns events strictly newer than [sinceId], oldest first, capped at [limit].
     */
    fun newerThan(sinceId: Long, limit: Int = 50): List<SmsMessage> {
        return rawQuery("_id > ?", arrayOf(sinceId.toString()), "date ASC LIMIT $limit")
    }

    fun maxId(): Long {
        val rows = rawQuery("1 = 1", emptyArray(), "date DESC LIMIT 1")
        return rows.firstOrNull()?.id ?: 0L
    }

    private fun rawQuery(
        where: String,
        args: Array<String>,
        sort: String
    ): List<SmsMessage> {
        val projectionFull = arrayOf("_id", "address", "body", "date", "type", "sub_id", "read")
        val projectionNoSub = arrayOf("_id", "address", "body", "date", "type", "read")

        fun run(proj: Array<String>): List<SmsMessage> {
            val out = mutableListOf<SmsMessage>()
            context.contentResolver.query(
                Telephony.Sms.CONTENT_URI, proj, where, args, sort
            )?.use { c ->
                val iId = c.getColumnIndex("_id")
                val iAddr = c.getColumnIndex("address")
                val iBody = c.getColumnIndex("body")
                val iDate = c.getColumnIndex("date")
                val iType = c.getColumnIndex("type")
                val iSub = c.getColumnIndex("sub_id")
                val iRead = c.getColumnIndex("read")
                while (c.moveToNext()) {
                    val subId = if (iSub >= 0) c.getInt(iSub) else 0
                    val sim = simFor(subId)
                    out.add(
                        SmsMessage(
                            id = c.getLong(iId),
                            address = c.getString(iAddr) ?: "",
                            body = c.getString(iBody) ?: "",
                            date = c.getLong(iDate),
                            type = c.getInt(iType),
                            subId = subId,
                            slot = sim?.slot ?: -1,
                            carrier = sim?.carrier ?: "",
                            simNumber = sim?.number ?: "",
                            read = iRead >= 0 && c.getInt(iRead) == 1
                        )
                    )
                }
            }
            return out
        }

        return try {
            run(projectionFull)
        } catch (t: Throwable) {
            Log.w(TAG, "query with sub_id failed (${t.message}), retrying without")
            run(projectionNoSub)
        }
    }

    private fun blockedPrefixes(): List<String> =
        com.tinyandr.smsbridge.SmsBridge.settings.blockedAddresses
            .split(',', '\n', ';')
            .map { it.trim() }
            .filter { it.isNotEmpty() }

    companion object {
        private const val TAG = "SmsRepository"
    }
}
