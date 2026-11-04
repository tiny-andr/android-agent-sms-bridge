package com.tinyandr.smsbridge.data

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject

data class AuditEntry(
    val ts: Long,
    val clientName: String,
    val clientId: String,
    val path: String,
    val query: String,
    val ip: String,
    val status: Int,
    val count: Int
)

/** In-app audit trail: who queried what, when. Keeps the last 500 entries. */
class AuditLog(context: Context) {

    private val sp = context.applicationContext
        .getSharedPreferences("smsbridge_audit", Context.MODE_PRIVATE)

    @Synchronized
    fun append(entry: AuditEntry) {
        val list = load().toMutableList()
        list.add(entry)
        while (list.size > MAX) list.removeAt(0)
        save(list)
    }

    @Synchronized
    fun recent(limit: Int = 100): List<AuditEntry> =
        load().takeLast(limit).reversed()

    @Synchronized
    fun clear() {
        sp.edit().putString("log", "[]").apply()
    }

    private fun load(): List<AuditEntry> {
        val raw = sp.getString("log", "[]") ?: "[]"
        return try {
            val arr = JSONArray(raw)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                AuditEntry(
                    ts = o.getLong("ts"),
                    clientName = o.optString("clientName", ""),
                    clientId = o.optString("clientId", ""),
                    path = o.optString("path", ""),
                    query = o.optString("query", ""),
                    ip = o.optString("ip", ""),
                    status = o.optInt("status", 0),
                    count = o.optInt("count", 0)
                )
            }
        } catch (t: Throwable) {
            emptyList()
        }
    }

    private fun save(list: List<AuditEntry>) {
        val arr = JSONArray()
        list.forEach { e ->
            arr.put(
                JSONObject()
                    .put("ts", e.ts)
                    .put("clientName", e.clientName)
                    .put("clientId", e.clientId)
                    .put("path", e.path)
                    .put("query", e.query)
                    .put("ip", e.ip)
                    .put("status", e.status)
                    .put("count", e.count)
            )
        }
        sp.edit().putString("log", arr.toString()).apply()
    }

    companion object {
        private const val MAX = 500
    }
}
