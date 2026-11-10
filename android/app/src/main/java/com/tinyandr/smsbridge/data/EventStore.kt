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
 *
 * Two independent cursors live here. [KEY_SEEN] drives the event queue that the
 * HTTP API serves; [KEY_PUSH_SEEN] drives the proactive push. They must not be
 * shared: a paired agent polling /v1/events would otherwise advance the cursor
 * past a message before the notifier ever looked at it.
 */
class EventStore(context: Context, private val repo: SmsRepository) {

    private val sp = context.applicationContext
        .getSharedPreferences("smsbridge_events", Context.MODE_PRIVATE)

    init {
        if (!sp.contains(KEY_SEEN)) {
            // First ever run: treat current inbox head as baseline, do not replay history.
            sp.edit()
                .putLong(KEY_SEEN, runCatching { repo.maxId() }.getOrDefault(0L))
                .putLong(KEY_NEXT_SEQ, 1L)
                .apply()
        }
    }

    /** Poll the SMS provider for anything newer than lastSeenId and enqueue events. */
    fun poll(): Int = pollDetailed().size

    /**
     * Same as [poll], but hands the full messages back. Returns an empty list when
     * nothing is new.
     */
    @Synchronized
    fun pollDetailed(): List<SmsMessage> {
        val last = sp.getLong(KEY_SEEN, 0L)
        val fresh = runCatching { repo.newerThan(last, 50) }.getOrDefault(emptyList())
        if (fresh.isEmpty()) return emptyList()

        var seq = sp.getLong(KEY_NEXT_SEQ, 1L)
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
        sp.edit().putLong(KEY_SEEN, maxId).putLong(KEY_NEXT_SEQ, seq).apply()
        return fresh
    }

    // ------------------------------------------------------------ push stream

    /**
     * Messages that still have to be pushed, oldest first.
     *
     * The cursor is committed to disk *before* this returns, and therefore before
     * the caller starts any network call. A process that is killed mid-push can
     * lose that one message, but a restart can never replay it — a duplicate card
     * on every reboot would be far worse than a missed one.
     */
    @Synchronized
    fun pendingPush(): List<SmsMessage> {
        val last = sp.getLong(KEY_PUSH_SEEN, -1L)
        if (last < 0L) {
            // Notifications just became active: start where the API stream stands
            // so the existing inbox is not replayed as a wall of cards.
            sp.edit().putLong(KEY_PUSH_SEEN, sp.getLong(KEY_SEEN, 0L)).commit()
            return emptyList()
        }
        val fresh = runCatching { repo.newerThan(last, 50) }.getOrDefault(emptyList())
        if (fresh.isEmpty()) return emptyList()
        sp.edit().putLong(KEY_PUSH_SEEN, fresh.maxOf { it.id }).commit()
        return fresh
    }

    /**
     * Move the push cursor to the newest row without pushing anything. Used when
     * the periodic scan is off and the process may have been dead for hours: the
     * alternative is a burst of stale cards on the next start.
     */
    @Synchronized
    fun establishPushBaseline() {
        val max = runCatching { repo.maxId() }.getOrDefault(0L)
        sp.edit().putLong(KEY_PUSH_SEEN, max).commit()
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
        private const val KEY_SEEN = "lastSeenId"
        private const val KEY_PUSH_SEEN = "pushSeenId"
        private const val KEY_NEXT_SEQ = "nextSeq"
    }
}
