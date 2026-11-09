package com.tinyandr.smsbridge.util

import android.util.Base64
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

object Crypto {
    private val rnd = SecureRandom()

    fun sha256Hex(input: String): String = sha256Hex(input.toByteArray(Charsets.UTF_8))

    fun sha256Hex(input: ByteArray): String {
        val md = MessageDigest.getInstance("SHA-256")
        return md.digest(input).joinToString("") { "%02x".format(it) }
    }

    fun hmacSha256Hex(secret: String, data: String): String {
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(secret.toByteArray(Charsets.UTF_8), "HmacSHA256"))
        return mac.doFinal(data.toByteArray(Charsets.UTF_8)).joinToString("") { "%02x".format(it) }
    }

    fun randomToken(byteLen: Int = 32): String {
        val b = ByteArray(byteLen)
        rnd.nextBytes(b)
        return Base64.encodeToString(b, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
    }

    fun randomCode(len: Int = 6): String {
        val sb = StringBuilder()
        repeat(len) { sb.append(rnd.nextInt(10)) }
        return sb.toString()
    }

    fun constantTimeEquals(a: String, b: String): Boolean {
        if (a.length != b.length) return false
        var r = 0
        for (i in a.indices) r = r or (a[i].code xor b[i].code)
        return r == 0
    }
}
