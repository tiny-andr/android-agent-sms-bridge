package com.tinyandr.smsbridge.http

import android.os.Build
import android.os.BatteryManager
import android.content.Context
import android.content.IntentFilter
import com.tinyandr.smsbridge.BuildConfig
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.data.AuditEntry
import com.tinyandr.smsbridge.data.PairedClient
import com.tinyandr.smsbridge.data.SmsQuery
import com.tinyandr.smsbridge.util.OtpExtractor
import fi.iki.elonen.NanoHTTPD
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayInputStream
import java.util.ArrayDeque

class BridgeServer(private val ctx: Context, port: Int) : NanoHTTPD(port) {

    private val startedAt = System.currentTimeMillis()

    init {
        // TLS is mandatory: the API carries SMS bodies and pairing tokens.
        val sc = TlsServer.sslContext(ctx)
        makeSecure(sc.serverSocketFactory, null)
    }

    override fun serve(session: IHTTPSession): Response {
        val method = session.method.name
        val path = session.uri ?: "/"
        val rawQuery = session.queryParameterString ?: ""
        val ip = session.remoteIpAddress ?: ""
        val params = session.parms ?: emptyMap()

        return try {
            route(session, method, path, rawQuery, ip, params)
        } catch (t: Throwable) {
            android.util.Log.e(TAG, "serve error", t)
            json(Response.Status.INTERNAL_ERROR, err("internal_error", t.message ?: t.javaClass.simpleName))
        }
    }

    private fun route(
        session: IHTTPSession,
        method: String,
        path: String,
        rawQuery: String,
        ip: String,
        params: Map<String, String>
    ): Response {
        // ---------- public ----------
        if (path == "/") {
            return json(
                Response.Status.OK,
                JSONObject().put("ok", true)
                    .put("app", "SmsBridge")
                    .put("version", BuildConfig.VERSION_NAME)
                    .put("hint", "POST /v1/pair with a code from the phone, then sign every request with HMAC-SHA256")
            )
        }
        if (path == "/v1/health" ) {
            return handleHealth()
        }
        if (method == "POST" && path == "/v1/pair") {
            return handlePair(session, ip)
        }

        // ---------- authenticated ----------
        val headers = session.headers ?: emptyMap()
        val auth = Auth.verify(
            method = method,
            path = path,
            rawQuery = rawQuery,
            clientId = headers["x-client-id"],
            timestamp = headers["x-timestamp"],
            nonce = headers["x-nonce"],
            signature = headers["x-signature"]
        )
        if (!auth.ok || auth.client == null) {
            return json(
                Response.Status.UNAUTHORIZED,
                err("unauthorized", auth.reason ?: "denied").put("hint", "POST /v1/pair first")
            )
        }
        val client = auth.client

        val rate = SmsBridge.settings.rateLimitPerMin
        if (!RateLimiter.allow(client.id, rate)) {
            return json(Response.Status.TOO_MANY_REQUESTS, err("rate_limited", "slow down"))
        }
        SmsBridge.clients.touch(client.id, ip)

        val response = when (path) {
            "/v1/sms" -> handleSms(client, params)
            "/v1/events" -> handleEvents(params)
            "/v1/stats" -> handleStats(params)
            "/v1/whoami" -> handleWhoami(client)
            "/v1/revoke-self" -> handleRevokeSelf(client)
            else -> json(Response.Status.NOT_FOUND, err("not_found", path))
        }

        // audit (skip health, keep it cheap)
        SmsBridge.audit.append(
            AuditEntry(
                ts = System.currentTimeMillis(),
                clientName = client.name,
                clientId = client.id,
                path = path,
                query = rawQuery,
                ip = ip,
                status = 200,
                count = countOf(response)
            )
        )
        return response
    }

    // ---------------------------------------------------------------- handlers

    private fun handlePair(session: IHTTPSession, ip: String): Response {
        val files = HashMap<String, String>()
        val raw = try {
            session.parseBody(files)
            files["postData"] ?: ""
        } catch (t: Throwable) {
            ""
        }
        val obj = try {
            JSONObject(raw)
        } catch (t: Throwable) {
            return json(Response.Status.BAD_REQUEST, err("bad_request", "body must be JSON"))
        }
        val code = obj.optString("code", "")
        val name = obj.optString("name", "agent")

        if (!SmsBridge.pairCodes.consume(code, ip)) {
            return json(Response.Status.UNAUTHORIZED, err("pair_failed", "wrong or expired code"))
        }
        val (client, token) = SmsBridge.clients.pair(name, ip)
        return json(
            Response.Status.OK,
            JSONObject()
                .put("ok", true)
                .put("client_id", client.id)
                .put("client_name", client.name)
                .put("token", token)
                .put("note", "store the token on the agent side; the phone never shows it again")
        )
    }

    private fun handleHealth(): Response {
        val bm = ctx.getSystemService(Context.BATTERY_SERVICE) as BatteryManager
        val level = bm.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY)
        val plugged = bm.isCharging
        val sims = JSONArray()
        SmsBridge.repo.sims(force = true).forEach {
            sims.put(
                JSONObject()
                    .put("slot", it.slot)
                    .put("sub_id", it.subId)
                    .put("carrier", it.carrier)
            )
        }
        return json(
            Response.Status.OK,
            JSONObject()
                .put("ok", true)
                .put("app", "SmsBridge")
                .put("version", BuildConfig.VERSION_NAME)
                .put("android", Build.VERSION.RELEASE)
                .put("sdk", Build.VERSION.SDK_INT)
                .put("model", Build.MODEL)
                .put("battery", level)
                .put("charging", plugged)
                .put("uptime_s", (System.currentTimeMillis() - startedAt) / 1000)
                .put("paired_clients", SmsBridge.clients.countActive())
                .put("pending_events", SmsBridge.events.count())
                .put("latest_seq", SmsBridge.events.latestSeq())
                .put("server_time", System.currentTimeMillis())
                .put("sims", sims)
        )
    }

    private fun handleSms(client: PairedClient, params: Map<String, String>): Response {
        fun p(n: String): String? = params[n]
        val s = SmsBridge.settings

        val otpMode = p("purpose")?.lowercase() == "otp"
        if (otpMode && !s.otpModeAllowed) {
            return json(Response.Status.FORBIDDEN, err("otp_mode_disabled", "OTP mode is disabled on the phone"))
        }

        var days = p("days")?.toIntOrNull()
        if (s.maxDays > 0) {
            days = if (days == null) s.maxDays else minOf(days, s.maxDays)
        }
        var limit = p("limit")?.toIntOrNull() ?: 50
        limit = limit.coerceIn(1, s.maxLimit)
        val offset = (p("offset")?.toIntOrNull() ?: 0).coerceAtLeast(0)
        val box = p("box")?.lowercase() ?: "inbox"
        val slot = p("slot")?.toIntOrNull()
        val sinceId = p("since_id")?.toLongOrNull()
        val search = p("q")
        val addresses = p("address")
            ?.split(',')
            ?.map { it.trim() }
            ?.filter { it.isNotEmpty() }
            ?: emptyList()

        val query = SmsQuery(
            days = days,
            addresses = addresses,
            slot = slot,
            box = box,
            limit = limit,
            offset = offset,
            sinceId = sinceId,
            search = search,
            otpOnly = otpMode
        )
        val rows = SmsBridge.repo.query(query)

        val arr = JSONArray()
        for (m in rows) {
            val o = JSONObject()
                .put("id", m.id)
                .put("address", m.address)
                .put("date", m.date)
                .put("date_iso", iso(m.date))
                .put("type", if (m.isInbox) "inbox" else "sent")
                .put("slot", m.slot)
                .put("sub_id", m.subId)
                .put("carrier", m.carrier)
                .put("read", m.read)
            if (otpMode) {
                val hit = OtpExtractor.extract(m.body) ?: continue
                o.put("otp", hit.code)
                o.put("matched_by", hit.matchedBy)
                o.put("masked_body", maskBody(m.body, hit.code))
            } else {
                o.put("body", m.body)
            }
            arr.put(o)
        }

        return json(
            Response.Status.OK,
            JSONObject()
                .put("ok", true)
                .put("query", JSONObject()
                    .put("days", days ?: JSONObject.NULL)
                    .put("box", box)
                    .put("address", if (addresses.isEmpty()) JSONObject.NULL else JSONArray(addresses))
                    .put("slot", slot ?: JSONObject.NULL)
                    .put("limit", limit)
                    .put("offset", offset)
                    .put("purpose", if (otpMode) "otp" else "full"))
                .put("count", arr.length())
                .put("messages", arr)
        )
    }

    private fun handleEvents(params: Map<String, String>): Response {
        val sinceSeq = params["since_seq"]?.toLongOrNull() ?: 0L
        SmsBridge.events.poll()
        val events = SmsBridge.events.since(sinceSeq)
        val arr = JSONArray()
        events.forEach { e ->
            arr.put(
                JSONObject()
                    .put("seq", e.seq)
                    .put("sms_id", e.smsId)
                    .put("address", e.address)
                    .put("date", e.date)
                    .put("date_iso", iso(e.date))
                    .put("type", if (e.type == 1) "inbox" else "other")
                    .put("sub_id", e.subId)
                    .put("preview", e.preview)
            )
        }
        return json(
            Response.Status.OK,
            JSONObject()
                .put("ok", true)
                .put("since_seq", sinceSeq)
                .put("latest_seq", SmsBridge.events.latestSeq())
                .put("count", arr.length())
                .put("events", arr)
        )
    }

    private fun handleStats(params: Map<String, String>): Response {
        fun p(n: String): String? = params[n]
        val s = SmsBridge.settings
        var days = p("days")?.toIntOrNull() ?: 7
        if (s.maxDays > 0) days = minOf(days, s.maxDays)

        val rows = SmsBridge.repo.query(
            SmsQuery(days = days, box = "all", limit = minOf(s.maxLimit, 500))
        )
        val grouped = rows.groupBy { it.address }
        val arr = JSONArray()
        grouped.entries.sortedByDescending { it.value.size }.forEach { (addr, list) ->
            arr.put(
                JSONObject()
                    .put("address", addr)
                    .put("count", list.size)
                    .put("last_date", list.maxOf { it.date })
                    .put("last_date_iso", iso(list.maxOf { it.date }))
            )
        }
        return json(
            Response.Status.OK,
            JSONObject()
                .put("ok", true)
                .put("days", days)
                .put("total", rows.size)
                .put("addresses", arr)
        )
    }

    private fun handleWhoami(client: PairedClient): Response =
        json(
            Response.Status.OK,
            JSONObject()
                .put("ok", true)
                .put("client_id", client.id)
                .put("name", client.name)
                .put("created_at", client.createdAt)
                .put("last_active_at", client.lastActiveAt)
                .put("request_count", client.requestCount)
                .put("phone_model", Build.MODEL)
        )

    private fun handleRevokeSelf(client: PairedClient): Response {
        SmsBridge.clients.revoke(client.id)
        return json(Response.Status.OK, JSONObject().put("ok", true).put("revoked", client.id))
    }

    // ------------------------------------------------------------------ utils

    private fun json(status: Response.IStatus, body: JSONObject): Response {
        val bytes = body.toString().toByteArray(Charsets.UTF_8)
        return newFixedLengthResponse(
            status,
            "application/json; charset=utf-8",
            ByteArrayInputStream(bytes),
            bytes.size.toLong()
        )
    }

    private fun err(code: String, message: String): JSONObject =
        JSONObject()
            .put("ok", false)
            .put("error", JSONObject().put("code", code).put("message", message))

    private fun iso(ms: Long): String =
        java.time.Instant.ofEpochMilli(ms)
            .atZone(java.time.ZoneId.systemDefault())
            .format(java.time.format.DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm:ss"))

    private fun countOf(@Suppress("UNUSED_PARAMETER") r: Response): Int = 0

    private fun maskBody(body: String, code: String): String {
        var kept = false
        return Regex("[0-9]{4,}").replace(body) { m ->
            if (!kept && m.value == code) {
                kept = true
                m.value
            } else {
                "*".repeat(minOf(m.value.length, 8))
            }
        }
    }

    private object RateLimiter {
        private val map = HashMap<String, ArrayDeque<Long>>()

        @Synchronized
        fun allow(clientId: String, perMinute: Int): Boolean {
            if (perMinute <= 0) return true
            val now = System.currentTimeMillis()
            val dq = map.getOrPut(clientId) { ArrayDeque() }
            while (dq.isNotEmpty() && now - dq.first() > 60_000L) dq.removeFirst()
            if (dq.size >= perMinute) return false
            dq.addLast(now)
            return true
        }
    }

    companion object {
        private const val TAG = "BridgeServer"
    }
}
