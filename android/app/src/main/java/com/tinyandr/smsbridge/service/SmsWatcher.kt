package com.tinyandr.smsbridge.service

import android.content.Context
import android.database.ContentObserver
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.notify.ProactiveNotifier
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
            Log.d(TAG, "onChange uri=$uri self=$selfChange")
            // de-bounce bursts (provider often fires 2-3 times per message)
            if (now - lastTrigger.get() < DEBOUNCE_MS) return
            lastTrigger.set(now)
            executor.execute { drain(0) }
        }
    }

    /**
     * The provider occasionally notifies a moment before the new row becomes
     * readable, so an empty first pass is retried briefly. If nothing shows up
     * the debounce is released; holding it would let one mistimed notification
     * swallow the real one that follows it.
     */
    private fun drain(attempt: Int) {
        val fresh = try {
            // The push cursor is committed before this returns, so a retry can
            // never hand the same message over twice.
            SmsBridge.events.pendingPush()
        } catch (t: Throwable) {
            Log.w(TAG, "push poll failed", t)
            lastTrigger.set(0L)
            return
        }
        // Keep the API event stream current in the same pass. It has its own
        // cursor, so this can never race the notifier for a message.
        runCatching { SmsBridge.events.poll() }
        if (fresh.isNotEmpty()) {
            Log.i(TAG, "queued ${fresh.size} new sms event(s)")
            ProactiveNotifier.onNewMessages(context, fresh)
            return
        }
        if (attempt < MAX_ATTEMPTS) {
            Thread.sleep(RETRY_DELAY_MS)
            drain(attempt + 1)
        } else {
            lastTrigger.set(0L)
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
            executor.execute {
                // The API event stream stays current regardless of what the user
                // configured; it has its own cursor and is not the notifier's.
                runCatching { SmsBridge.events.poll() }
                // Baseline poll advances the push cursor past anything that
                // arrived while the process was dead, without pushing it. Only
                // when the periodic scan is off: the scan performs the same poll
                // but pushes what falls inside its window, and the two would race
                // for the same rows — whichever ran first would swallow the rest.
                if (!SmsBridge.settings.notifyScanEnabled) {
                    runCatching { SmsBridge.events.establishPushBaseline() }
                }
            }
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
        private const val DEBOUNCE_MS = 400L
        private const val RETRY_DELAY_MS = 700L
        private const val MAX_ATTEMPTS = 2
    }
}
