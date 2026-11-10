package com.tinyandr.smsbridge.data

import android.content.Context
import com.tinyandr.smsbridge.BuildConfig

class Settings(context: Context) {
    private val sp = context.applicationContext
        .getSharedPreferences("smsbridge_settings", Context.MODE_PRIVATE)

    /**
     * HTTP listen port. The debug build is installed next to the released one,
     * so it must not fight it for the same port on first run.
     */
    var port: Int
        get() = sp.getInt("port", if (BuildConfig.DEBUG) 8788 else 8787)
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

    // ------------------------------------------------------- proactive push
    // Everything below is entered by the user on the device and lives only in
    // this app's private storage. Nothing is baked into the build, so a clone
    // of the repository never carries a token, a webhook or a chat id.

    /** Push an event the moment a new SMS row appears (ContentObserver). */
    var notifyEventEnabled: Boolean
        get() = sp.getBoolean("notify_event", false)
        set(v) = sp.edit().putBoolean("notify_event", v).apply()

    /** Periodically re-scan the inbox to catch what the observer missed. */
    var notifyScanEnabled: Boolean
        get() = sp.getBoolean("notify_scan", false)
        set(v) = sp.edit().putBoolean("notify_scan", v).apply()

    /** Scan period in minutes, clamped to the values the UI offers. */
    var notifyScanIntervalMin: Int
        get() = sp.getInt("notify_scan_interval", 15)
        set(v) = sp.edit().putInt("notify_scan_interval", v.coerceIn(5, 60)).apply()

    /**
     * Comma separated keywords. Empty means "push everything". When set, a
     * message is only pushed if any keyword occurs in the sender or the body.
     */
    var notifyKeywords: String
        get() = sp.getString("notify_keywords", "") ?: ""
        set(v) = sp.edit().putString("notify_keywords", v).apply()

    var tgEnabled: Boolean
        get() = sp.getBoolean("tg_enabled", false)
        set(v) = sp.edit().putBoolean("tg_enabled", v).apply()

    var tgToken: String
        get() = sp.getString("tg_token", "") ?: ""
        set(v) = sp.edit().putString("tg_token", v.trim()).apply()

    var tgChatId: String
        get() = sp.getString("tg_chat_id", "") ?: ""
        set(v) = sp.edit().putString("tg_chat_id", v.trim()).apply()

    var feishuEnabled: Boolean
        get() = sp.getBoolean("fs_enabled", false)
        set(v) = sp.edit().putBoolean("fs_enabled", v).apply()

    var feishuWebhook: String
        get() = sp.getString("fs_webhook", "") ?: ""
        set(v) = sp.edit().putString("fs_webhook", v.trim()).apply()

    /** Empty when the bot's signature check is turned off. */
    var feishuSecret: String
        get() = sp.getString("fs_secret", "") ?: ""
        set(v) = sp.edit().putString("fs_secret", v.trim()).apply()

    /** At least one trigger is selected. */
    val notifyAnyTrigger: Boolean get() = notifyEventEnabled || notifyScanEnabled

    /** At least one channel is selected and complete enough to send. */
    val notifyAnyChannel: Boolean
        get() = (tgEnabled && tgToken.isNotBlank() && tgChatId.isNotBlank()) ||
            (feishuEnabled && feishuWebhook.isNotBlank())

    val notifyChannelCount: Int
        get() = (if (tgEnabled && tgToken.isNotBlank() && tgChatId.isNotBlank()) 1 else 0) +
            (if (feishuEnabled && feishuWebhook.isNotBlank()) 1 else 0)
}
