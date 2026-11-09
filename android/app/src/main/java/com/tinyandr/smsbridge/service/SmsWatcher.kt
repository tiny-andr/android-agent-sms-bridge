package com.tinyandr.smsbridge.service

import android.content.Context
import android.database.ContentObserver
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.tinyandr.smsbridge.SmsBridge
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicLong

/**
 * Watches content://sms and pushes new rows into the incremental event queue.
 * This is event driven: no polling loop, no 10-second timer.
 */
class SmsWatcher(private val context: Context) {

    private var registered = false
    private val executor = Executors.newSingleThreadExecutor()
    private val lastTrigger = AtomicLong(0L)

    private val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
        override fun onChange(selfChange: Boolean, uri: Uri?) {
            val now = System.currentTimeMillis()
            // de-bounce bursts (provider often fires 2-3 times per message)
            val prev = lastTrigger.get()
            if (now - prev < 400L) return
            lastTrigger.set(now)
            executor.execute {
                try {
                    val n = SmsBridge.events.poll()
                    if (n > 0) Log.i(TAG, "queued $n new sms event(s)")
                } catch (t: Throwable) {
                    Log.w(TAG, "poll failed", t)
                }
            }
        }
    }

    fun start() {
        if (registered) return
        try {
            context.contentResolver.registerContentObserver(
                Uri.parse("content://sms"), true, observer
            )
            registered = true
            Log.i(TAG, "SMS observer registered")
            // Establish baseline: current head, history is served by /v1/sms.
            executor.execute { runCatching { SmsBridge.events.poll() } }
        } catch (t: Throwable) {
            Log.e(TAG, "cannot register observer", t)
        }
    }

    fun stop() {
        if (!registered) return
        try {
            context.contentResolver.unregisterContentObserver(observer)
        } catch (_: Throwable) {
        }
        registered = false
    }

    companion object {
        private const val TAG = "SmsWatcher"
    }
}
