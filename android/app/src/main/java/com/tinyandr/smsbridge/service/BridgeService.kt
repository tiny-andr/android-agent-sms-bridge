package com.tinyandr.smsbridge.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.R
import com.tinyandr.smsbridge.http.BridgeServer
import com.tinyandr.smsbridge.notify.ProactiveNotifier
import com.tinyandr.smsbridge.ui.MainActivity

class BridgeService : Service() {

    private var server: BridgeServer? = null
    private var watcher: SmsWatcher? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private var cpuLock: PowerManager.WakeLock? = null
    private val uiHandler = Handler(Looper.getMainLooper())
    @Volatile
    private var heartbeatRunning = false
    @Volatile
    private var currentPort = -1

    override fun onCreate() {
        super.onCreate()
        createChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                SmsBridge.settings.serviceEnabled = false
                stopSelfSafely()
                return START_NOT_STICKY
            }
            else -> {
                SmsBridge.settings.serviceEnabled = true
                startInForeground()
                if (SmsBridge.settings.holdLocks) acquireLocks() else releaseLocks()
                startServer()
                startWatcher()
                startScanner()
                if (SmsBridge.settings.heartbeatEnabled) startHeartbeat() else stopHeartbeat()
                KeepAliveScheduler.schedule(this)
                running = true
            }
        }
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onTaskRemoved(rootIntent: Intent?) {
        // user swiped the app from recents: keep serving, re-arm both watchdogs
        if (SmsBridge.settings.serviceEnabled) {
            KeepAliveScheduler.schedule(this)
            runCatching { BridgeService.start(this) }
        }
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        stopHeartbeat()
        stopScanner()
        releaseLocks()
        stopServer()
        stopWatcher()
        KeepAliveScheduler.cancel(this)
        running = false
        super.onDestroy()
    }

    // --------------------------------------------------------------- internals

    /**
     * Lightweight loopback self-ping every 20s. Keeps the process visibly active
     * and the WiFi link warm, which makes aggressive vendor freezers less likely
     * to freeze the UID. Traffic never leaves the device.
     */
    private fun startHeartbeat() {
        if (!SmsBridge.settings.heartbeatEnabled) {
            Log.i(TAG, "heartbeat disabled by setting")
            return
        }
        if (heartbeatRunning) return
        heartbeatRunning = true
        uiHandler.post(heartbeatTick)
    }

    private fun stopHeartbeat() {
        heartbeatRunning = false
        uiHandler.removeCallbacks(heartbeatTick)
    }

    private val heartbeatTick = object : Runnable {
        override fun run() {
            if (!heartbeatRunning) return
            Thread {
                try {
                    java.net.Socket().use { s ->
                        s.connect(
                            java.net.InetSocketAddress("127.0.0.1", SmsBridge.settings.port), 800
                        )
                        s.getOutputStream().write(
                            "GET /v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
                                .toByteArray()
                        )
                        s.getInputStream().read()
                    }
                } catch (_: Throwable) {
                }
            }.start()
            uiHandler.postDelayed(this, HEARTBEAT_MS)
        }
    }

    /**
     * Screen-off lets the WiFi chip enter power save, and an idle AP drops its
     * ARP entry for the phone. The result is that a LAN agent cannot open a
     * socket to us even though the service is alive. Holding a high-perf WiFi
     * lock keeps the link hot; the partial CPU lock makes sure we can answer the
     * moment a packet lands.
     */
    private fun acquireLocks() {
        if (!SmsBridge.settings.holdLocks) {
            Log.i(TAG, "locks disabled by setting")
            return
        }
        try {
            if (wifiLock == null) {
                val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
                @Suppress("DEPRECATION")
                wifiLock = wm.createWifiLock(
                    WifiManager.WIFI_MODE_FULL_HIGH_PERF, "SmsBridge:wifi"
                ).also {
                    it.setReferenceCounted(false)
                    it.acquire()
                }
                Log.i(TAG, "wifi lock acquired")
            }
        } catch (t: Throwable) {
            Log.w(TAG, "wifi lock failed", t)
        }
        try {
            if (cpuLock == null) {
                val pm = applicationContext.getSystemService(Context.POWER_SERVICE) as PowerManager
                cpuLock = pm.newWakeLock(
                    PowerManager.PARTIAL_WAKE_LOCK, "SmsBridge:cpu"
                ).also {
                    it.setReferenceCounted(false)
                    it.acquire()
                }
                Log.i(TAG, "cpu wakelock acquired")
            }
        } catch (t: Throwable) {
            Log.w(TAG, "cpu wakelock failed", t)
        }
    }

    private fun releaseLocks() {
        try {
            wifiLock?.takeIf { it.isHeld }?.release()
        } catch (_: Throwable) {
        }
        wifiLock = null
        try {
            cpuLock?.takeIf { it.isHeld }?.release()
        } catch (_: Throwable) {
        }
        cpuLock = null
    }

    private fun startInForeground() {
        val n = buildNotification()
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                startForeground(NOTIF_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
            } else {
                startForeground(NOTIF_ID, n)
            }
        } catch (t: Throwable) {
            Log.e(TAG, "startForeground failed", t)
            // Keep running as a plain background service as a last resort.
        }
    }

    private fun startServer() {
        val port = SmsBridge.settings.port
        if (server != null && currentPort == port) return
        if (server != null) stopServer()
        try {
            val s = BridgeServer(applicationContext, port)
            s.start(NanoHTTPD_SOCKET_READ_TIMEOUT, true)
            server = s
            currentPort = port
            Log.i(TAG, "HTTPS listening on 0.0.0.0:$port")
        } catch (t: Throwable) {
            Log.e(TAG, "cannot bind port $port", t)
            server = null
        }
        refreshNotification()
    }

    private fun stopServer() {
        try {
            server?.stop()
        } catch (_: Throwable) {
        }
        server = null
    }

    private fun startWatcher() {
        if (watcher != null) return
        watcher = SmsWatcher(applicationContext).also { it.start() }
    }

    private fun stopWatcher() {
        watcher?.stop()
        watcher = null
    }

    // ------------------------------------------------------ proactive scanner
    //
    // The ContentObserver is the primary trigger and fires within a second, but a
    // vendor freezer or a killed process silently drops it. This timer is the
    // safety net: it replays whatever landed since the last pass. It only runs
    // while the service is alive, which is fine — nothing could be pushed then.

    private fun startScanner() {
        uiHandler.removeCallbacks(scanTick)
        val s = SmsBridge.settings
        if (!s.notifyScanEnabled) return
        val intervalMs = s.notifyScanIntervalMin * 60_000L
        // Service start is itself a catch-up: the process may have been gone for
        // a while, so push what arrived inside one interval and silently treat
        // anything older as already seen.
        Thread {
            runCatching { scan(intervalMs) }
                .onFailure { Log.w(TAG, "initial scan failed", it) }
        }.start()
        uiHandler.postDelayed(scanTick, intervalMs)
    }

    private fun stopScanner() {
        uiHandler.removeCallbacks(scanTick)
    }

    private val scanTick = object : Runnable {
        override fun run() {
            uiHandler.removeCallbacks(this)
            tickScan()
        }
    }

    private fun tickScan() {
        val s = SmsBridge.settings
        if (!s.notifyScanEnabled) return
        val intervalMs = s.notifyScanIntervalMin * 60_000L
        Thread {
            runCatching { scan(intervalMs) }
                .onFailure { Log.w(TAG, "scan failed", it) }
        }.start()
        uiHandler.postDelayed(scanTick, intervalMs)
    }

    /**
     * Advances the event cursor past everything that is new, but only pushes
     * messages that arrived within [windowMs]. Without the window a phone that
     * was off for two days would greet the user with 50 cards at once.
     */
    private fun scan(windowMs: Long) {
        val fresh = SmsBridge.events.pendingPush()
        // The API event stream has its own cursor; keep it current as well.
        runCatching { SmsBridge.events.poll() }
        if (fresh.isEmpty()) return
        val cutoff = System.currentTimeMillis() - windowMs
        val recent = fresh.filter { it.date >= cutoff }
        Log.i(TAG, "scan: ${fresh.size} new, ${recent.size} inside ${windowMs / 60_000} min")
        if (recent.isNotEmpty()) ProactiveNotifier.onNewMessages(this, recent)
    }

    private fun stopSelfSafely() {
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun createChannel() {
        val nm = getSystemService(NotificationManager::class.java)
        val ch = NotificationChannel(CHANNEL_ID, getString(R.string.channel_name), NotificationManager.IMPORTANCE_LOW)
        ch.description = getString(R.string.channel_desc)
        ch.setShowBadge(false)
        nm.createNotificationChannel(ch)
    }

    private fun buildNotification(): Notification {
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val stop = PendingIntent.getService(
            this, 1, Intent(this, BridgeService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        val port = SmsBridge.settings.port
        val paired = SmsBridge.clients.countActive()
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.notif_title))
            .setContentText(getString(R.string.notif_text, port, paired))
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setContentIntent(open)
            .addAction(0, getString(R.string.notif_stop), stop)
            .build()
    }

    private fun refreshNotification() {
        try {
            val nm = getSystemService(NotificationManager::class.java)
            nm.notify(NOTIF_ID, buildNotification())
        } catch (_: Throwable) {
        }
    }

    companion object {
        private const val TAG = "BridgeService"
        const val CHANNEL_ID = "smsbridge_service"
        const val NOTIF_ID = 1001
        const val ACTION_START = "com.tinyandr.smsbridge.action.START"
        const val ACTION_STOP = "com.tinyandr.smsbridge.action.STOP"
        private const val NanoHTTPD_SOCKET_READ_TIMEOUT = 10_000
        private const val HEARTBEAT_MS = 20_000L

        @Volatile
        var running: Boolean = false
            private set

        fun start(ctx: Context) {
            val i = Intent(ctx, BridgeService::class.java).setAction(ACTION_START)
            try {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    ctx.startForegroundService(i)
                } else {
                    ctx.startService(i)
                }
            } catch (t: Throwable) {
                // e.g. ForegroundServiceStartNotAllowedException when we are in the background
                Log.w("BridgeService", "startForegroundService refused: ${t.message}")
                runCatching { ctx.startService(i) }
            }
        }

        fun stop(ctx: Context) {
            ctx.startService(Intent(ctx, BridgeService::class.java).setAction(ACTION_STOP))
        }

        fun restart(ctx: Context) {
            stop(ctx)
            start(ctx)
        }
    }
}
