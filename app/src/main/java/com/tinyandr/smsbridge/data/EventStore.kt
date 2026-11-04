package com.tinyandr.smsbridge.data

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

data class SmsEvent(
    val seq: Long,
    val smsId: Long,
    val address: String,
    val date: Long,
    val type: Int,
    val subId: Int,
    val preview: String
)

/**
 * Incremental event queue fed by [com.tinyandr.smsbridge.service.SmsWatcher].
 * Keeps the last 500 events so a paired agent can pull "what is new" with since_seq.
 */
class EventStore(context: Context, private val repo: SmsRepository) {

    private val sp = context.applicationContext
        .getSharedPreferences("smsbridge_events", Context.MODE_PRIVATE)

    init {
        if (!sp.contains("lastSeenId")) {
            // First ever run: treat current inbox head as baseline, do not replay history.
            sp.edit()
                .putLong("lastSeenId", runCatching { repo.maxId() }.getOrDefault(0L))
                .putLong("nextSeq", 1L)
                .apply()
        }
    }

    /** Poll the SMS provider for anything newer than lastSeenId and enqueue events. */
    @Synchronized
    fun poll(): Int {
        val last = sp.getLong("lastSeenId", 0L)
        val fresh = runCatching { repo.newerThan(last, 50) }.getOrDefault(emptyList())
        if (fresh.isEmpty()) return 0

        var seq = sp.getLong("nextSeq", 1L)
        val events = loadEvents().toMutableList()
        var maxId = last
        for (m in fresh) {
            events.add(
                SmsEvent(
                    seq = seq++,
                    smsId = m.id,
                    address = m.address,
                    date = m.date,
                    type = m.type,
                    subId = m.subId,
                    preview = preview(m.body)
                )
            )
            if (m.id > maxId) maxId = m.id
        }
        while (events.size > MAX_EVENTS) events.removeAt(0)
        saveEvents(events)
        sp.edit().putLong("lastSeenId", maxId).putLong("nextSeq", seq).apply()
        return fresh.size
    }

    @Synchronized
    fun since(seq: Long): List<SmsEvent> = loadEvents().filter { it.seq > seq }

    @Synchronized
    fun latestSeq(): Long = loadEvents().lastOrNull()?.seq ?: 0L

    @Synchronized
    fun count(): Int = loadEvents().size

    @Synchronized
    fun clear() {
        sp.edit().putString("events", "[]").apply()
    }

    private fun preview(body: String): String {
        val oneLine = body.replace(Regex("\\s+"), " ").trim()
        return if (oneLine.length <= 60) oneLine else oneLine.take(60) + "…"
    }

    private fun loadEvents(): List<SmsEvent> {
        val raw = sp.getString("events", "[]") ?: "[]"
        return try {
            val arr = JSONArray(raw)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                SmsEvent(
                    seq = o.getLong("seq"),
                    smsId = o.getLong("smsId"),
                    address = o.optString("address", ""),
                    date = o.optLong("date", 0L),
                    type = o.optInt("type", 1),
                    subId = o.optInt("subId", 0),
                    preview = o.optString("preview", "")
                )
            }
        } catch (t: Throwable) {
            emptyList()
        }
    }

    private fun saveEvents(events: List<SmsEvent>) {
        val arr = JSONArray()
        events.forEach { e ->
            arr.put(
                JSONObject()
                    .put("seq", e.seq)
                    .put("smsId", e.smsId)
                    .put("address", e.address)
                    .put("date", e.date)
                    .put("type", e.type)
                    .put("subId", e.subId)
                    .put("preview", e.preview)
            )
        }
        sp.edit().putString("events", arr.toString()).apply()
    }

    companion object {
        private const val MAX_EVENTS = 500
    }
}
