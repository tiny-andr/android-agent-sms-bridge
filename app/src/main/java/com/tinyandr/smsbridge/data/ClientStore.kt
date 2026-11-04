package com.tinyandr.smsbridge.data

import android.content.Context
import com.tinyandr.smsbridge.util.Crypto
import org.json.JSONArray
import org.json.JSONObject
import java.util.UUID

data class PairedClient(
    val id: String,
    val name: String,
    val token: String,
    val createdAt: Long,
    var lastActiveAt: Long,
    var lastIp: String,
    var requestCount: Long,
    var revoked: Boolean
)

class ClientStore(context: Context) {

    private val sp = context.applicationContext
        .getSharedPreferences("smsbridge_clients", Context.MODE_PRIVATE)

    @Synchronized
    fun list(): List<PairedClient> = load()

    @Synchronized
    fun find(id: String): PairedClient? = load().firstOrNull { it.id == id }

    @Synchronized
    fun pair(name: String, ip: String): Pair<PairedClient, String> {
        val token = Crypto.randomToken(32)
        val client = PairedClient(
            id = UUID.randomUUID().toString(),
            name = name.ifBlank { "agent" },
            token = token,
            createdAt = System.currentTimeMillis(),
            lastActiveAt = 0L,
            lastIp = ip,
            requestCount = 0L,
            revoked = false
        )
        val all = load().toMutableList()
        all.add(client)
        save(all)
        return client to token
    }

    @Synchronized
    fun revoke(id: String) {
        val all = load().toMutableList()
        val i = all.indexOfFirst { it.id == id }
        if (i >= 0) {
            all[i] = all[i].copy(revoked = true)
            save(all)
        }
    }

    @Synchronized
    fun remove(id: String) {
        val all = load().toMutableList()
        all.removeAll { it.id == id }
        save(all)
    }

    @Synchronized
    fun touch(id: String, ip: String): PairedClient? {
        val all = load().toMutableList()
        val i = all.indexOfFirst { it.id == id }
        if (i < 0) return null
        val c = all[i].copy(
            lastActiveAt = System.currentTimeMillis(),
            lastIp = ip,
            requestCount = all[i].requestCount + 1
        )
        all[i] = c
        save(all)
        return c
    }

    @Synchronized
    fun countActive(): Int = load().count { !it.revoked }

    private fun load(): List<PairedClient> {
        val raw = sp.getString("clients", "[]") ?: "[]"
        return try {
            val arr = JSONArray(raw)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                PairedClient(
                    id = o.getString("id"),
                    name = o.getString("name"),
                    token = o.getString("token"),
                    createdAt = o.getLong("createdAt"),
                    lastActiveAt = o.optLong("lastActiveAt", 0L),
                    lastIp = o.optString("lastIp", ""),
                    requestCount = o.optLong("requestCount", 0L),
                    revoked = o.optBoolean("revoked", false)
                )
            }
        } catch (t: Throwable) {
            emptyList()
        }
    }

    private fun save(clients: List<PairedClient>) {
        val arr = JSONArray()
        clients.forEach { c ->
            arr.put(
                JSONObject()
                    .put("id", c.id)
                    .put("name", c.name)
                    .put("token", c.token)
                    .put("createdAt", c.createdAt)
                    .put("lastActiveAt", c.lastActiveAt)
                    .put("lastIp", c.lastIp)
                    .put("requestCount", c.requestCount)
                    .put("revoked", c.revoked)
            )
        }
        sp.edit().putString("clients", arr.toString()).apply()
    }
}
