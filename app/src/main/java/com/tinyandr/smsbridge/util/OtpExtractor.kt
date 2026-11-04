package com.tinyandr.smsbridge.util

/** Extract one-time passwords / verification codes from SMS body. */
object OtpExtractor {

    private val PRIMARY_KEYWORDS = listOf(
        "验证码", "校验码", "动态码", "确认码", "激活码", "登录码", "一次性密码",
        "verification code", "verify code", "security code", "one-time", "one time",
        "passcode", "otp", "code", "验证"
    )

    private val DIGITS = Regex("(?<![0-9])([0-9]{4,8})(?![0-9])")
    private val ALNUM_CODE = Regex("(?<![A-Za-z0-9])([A-Za-z0-9]{4,8})(?![A-Za-z0-9])")

    fun extract(body: String): OtpHit? {
        if (body.isBlank()) return null
        val lower = body.lowercase()

        // 1) prefer digits that appear near a keyword
        for (kw in PRIMARY_KEYWORDS) {
            val idx = lower.indexOf(kw)
            if (idx < 0) continue
            val from = maxOf(0, idx - 24)
            val to = minOf(body.length, idx + kw.length + 64)
            val window = body.substring(from, to)
            DIGITS.find(window)?.let {
                return OtpHit(it.groupValues[1], kw)
            }
        }

        // 2) fall back to a standalone 4-8 digit run, but only when the message
        //    looks transactional (contains a keyword at all, even far away)
        val looksTransactional = PRIMARY_KEYWORDS.any { lower.contains(it) }
        if (looksTransactional) {
            DIGITS.find(body)?.let { return OtpHit(it.groupValues[1], "digit") }
        }

        // 3) alphanumeric codes for the rare "your code is A1B2C3" case
        if (lower.contains("code") || lower.contains("验证")) {
            ALNUM_CODE.findAll(body)
                .map { it.groupValues[1] }
                .firstOrNull { it.length in 4..8 && it.any { c -> c.isDigit() } && it.any { c -> c.isLetter() } }
                ?.let { return OtpHit(it, "alnum") }
        }
        return null
    }

    data class OtpHit(val code: String, val matchedBy: String)
}
