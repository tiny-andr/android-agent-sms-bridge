package com.tinyandr.smsbridge.service

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.SystemClock
import android.util.Log
import com.tinyandr.smsbridge.SmsBridge

/** Alarm based watchdog: if the OS killed us, bring the service back. */
object KeepAliveScheduler {

    private const val REQ = 2001
    const val INTERVAL_MS = 15 * 60_000L

    fun schedule(ctx: Context) {
        val am = ctx.getSystemService(AlarmManager::class.java) ?: return
        val pi = pending(ctx)
        try {
            am.setAndAllowWhileIdle(
                AlarmManager.ELAPSED_REALTIME_WAKEUP,
                SystemClock.elapsedRealtime() + INTERVAL_MS,
                pi
            )
        } catch (t: Throwable) {
            Log.w(TAG, "schedule failed", t)
        }
        // second, independent watchdog path
        KeepAliveJob.schedule(ctx)
    }

    fun cancel(ctx: Context) {
        val am = ctx.getSystemService(AlarmManager::class.java) ?: return
        try {
            am.cancel(pending(ctx))
        } catch (_: Throwable) {
        }
        KeepAliveJob.cancel(ctx)
    }

    private fun pending(ctx: Context): PendingIntent =
        PendingIntent.getBroadcast(
            ctx, REQ,
            Intent(ctx, KeepAliveReceiver::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

    private const val TAG = "KeepAlive"
}

class KeepAliveReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        val s = SmsBridge.settings
        if (s.serviceEnabled && !BridgeService.running) {
            Log.i("KeepAliveReceiver", "service was dead, restarting")
            runCatching { BridgeService.start(context) }
        }
        if (s.serviceEnabled) KeepAliveScheduler.schedule(context)
    }
}
