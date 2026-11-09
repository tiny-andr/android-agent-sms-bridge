package com.tinyandr.smsbridge.http

import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.data.PairedClient
import com.tinyandr.smsbridge.util.Crypto
import java.util.LinkedHashMap
import kotlin.math.abs

/**
 * HMAC-SHA256 request authentication.
 *
 * Canonical signing string:
 *   METHOD \n PATH \n RAW_QUERY_STRING \n TIMESTAMP_MS \n NONCE
 */
object Auth {

    private const val WINDOW_MS = 5 * 60_000L
    private const val MAX_NONCES = 4000
    private val nonces = LinkedHashMap<String, Long>()

    data class Result(val ok: Boolean, val client: PairedClient?, val reason: String?)

    fun verify(
        method: String,
        path: String,
        rawQuery: String,
        clientId: String?,
        timestamp: String?,
        nonce: String?,
        signature: String?
    ): Result {
        if (clientId.isNullOrBlank()) return Result(false, null, "missing X-Client-Id")
        if (timestamp.isNullOrBlank()) return Result(false, null, "missing X-Timestamp")
        if (nonce.isNullOrBlank()) return Result(false, null, "missing X-Nonce")
        if (signature.isNullOrBlank()) return Result(false, null, "missing X-Signature")

        val client = SmsBridge.clients.find(clientId)
            ?: return Result(false, null, "unknown client")
        if (client.revoked) return Result(false, null, "client revoked")

        val ts = timestamp.toLongOrNull() ?: return Result(false, null, "bad timestamp")
        val now = System.currentTimeMillis()
        if (abs(now - ts) > WINDOW_MS) return Result(false, null, "timestamp out of window")
        if (!storeNonce(nonce, now)) return Result(false, null, "nonce replayed")

        val signing = "$method\n$path\n$rawQuery\n$timestamp\n$nonce"
        val expected = Crypto.hmacSha256Hex(client.token, signing)
        if (!Crypto.constantTimeEquals(expected, signature.lowercase())) {
            return Result(false, null, "bad signature")
        }
        return Result(true, client, null)
    }

    fun signingString(
        method: String,
        path: String,
        rawQuery: String,
        timestamp: String,
        nonce: String
    ): String = "$method\n$path\n$rawQuery\n$timestamp\n$nonce"

    @Synchronized
    private fun storeNonce(nonce: String, now: Long): Boolean {
        val it = nonces.entries.iterator()
        while (it.hasNext()) {
            val e = it.next()
            if (now - e.value > WINDOW_MS) it.remove() else break
        }
        if (nonces.containsKey(nonce)) return false
        nonces[nonce] = now
        while (nonces.size > MAX_NONCES) {
            nonces.remove(nonces.keys.first())
        }
        return true
    }
}
