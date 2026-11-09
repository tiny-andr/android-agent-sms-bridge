package com.tinyandr.smsbridge.data

import android.content.Context
import com.tinyandr.smsbridge.util.Crypto

/**
 * One active 6-digit pairing code at a time, valid 5 minutes, one-shot.
 * Attempts are rate limited to stop brute forcing.
 */
class PairCodeManager {

    private var code: String = ""
    private var expiresAt: Long = 0L
    private val attempts = ArrayDeque<Long>()

    @Synchronized
    fun current(): Pair<String, Long>? {
        val now = System.currentTimeMillis()
        return if (code.isNotEmpty() && now < expiresAt) code to expiresAt else null
    }

    @Synchronized
    fun generate(): Pair<String, Long> {
        code = Crypto.randomCode(6)
        expiresAt = System.currentTimeMillis() + 5 * 60_000L
        // Logged so headless setup / adb driven testing can read the code.
        android.util.Log.i("SmsBridge", "PAIR_CODE=$code expiresAt=$expiresAt")
        return code to expiresAt
    }

    @Synchronized
    fun clear() {
        code = ""
        expiresAt = 0L
    }

    /** Returns true when [attempt] matches the live code; code is consumed on success. */
    @Synchronized
    fun consume(attempt: String, ip: String): Boolean {
        val now = System.currentTimeMillis()
        while (attempts.isNotEmpty() && now - attempts.first() > 60_000L) attempts.removeFirst()
        if (attempts.size >= 20) return false
        attempts.addLast(now)

        if (code.isEmpty() || now > expiresAt) return false
        if (Crypto.constantTimeEquals(attempt.trim(), code)) {
            code = ""
            expiresAt = 0L
            return true
        }
        return false
    }
}
