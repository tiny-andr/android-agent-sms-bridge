package com.tinyandr.smsbridge.data

import android.content.Context

class Settings(context: Context) {
    private val sp = context.applicationContext
        .getSharedPreferences("smsbridge_settings", Context.MODE_PRIVATE)

    /** HTTP listen port. */
    var port: Int
        get() = sp.getInt("port", 8787)
        set(v) = sp.edit().putInt("port", v).apply()

    /** Start the bridge automatically after boot / app update. */
    var autoStart: Boolean
        get() = sp.getBoolean("auto_start", true)
        set(v) = sp.edit().putBoolean("auto_start", v).apply()

    /** Hard cap on how many days back an agent may query. 0 = no cap. */
    var maxDays: Int
        get() = sp.getInt("max_days", 30)
        set(v) = sp.edit().putInt("max_days", v).apply()

    /** Hard cap on messages returned per request. */
    var maxLimit: Int
        get() = sp.getInt("max_limit", 200)
        set(v) = sp.edit().putInt("max_limit", v).apply()

    /** Requests per minute per client. 0 = unlimited. */
    var rateLimitPerMin: Int
        get() = sp.getInt("rate_limit", 60)
        set(v) = sp.edit().putInt("rate_limit", v).apply()

    /** Allow purpose=otp requests (body masked, only the code returned). */
    var otpModeAllowed: Boolean
        get() = sp.getBoolean("otp_mode", true)
        set(v) = sp.edit().putBoolean("otp_mode", v).apply()

    /** Comma separated numbers/prefixes that must never be returned. */
    var blockedAddresses: String
        get() = sp.getString("blocked", "") ?: ""
        set(v) = sp.edit().putString("blocked", v).apply()

    /** User-facing master switch (persisted across reboots). */
    var serviceEnabled: Boolean
        get() = sp.getBoolean("service_enabled", false)
        set(v) = sp.edit().putBoolean("service_enabled", v).apply()

    /** Loopback self-ping every 20s; cheap, and defeats vendor freezers. */
    var heartbeatEnabled: Boolean
        get() = sp.getBoolean("heartbeat_enabled", true)
        set(v) = sp.edit().putBoolean("heartbeat_enabled", v).apply()

    /** Hold PARTIAL_WAKE_LOCK + WifiLock while serving. Costs more battery. */
    var holdLocks: Boolean
        get() = sp.getBoolean("hold_locks", false)
        set(v) = sp.edit().putBoolean("hold_locks", v).apply()

    /** Require HMAC on every request (cannot be disabled from UI, kept for clarity). */
    val requireHmac: Boolean get() = true
}
