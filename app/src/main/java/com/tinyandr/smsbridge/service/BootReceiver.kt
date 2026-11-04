package com.tinyandr.smsbridge.service

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import com.tinyandr.smsbridge.SmsBridge

class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        val action = intent?.action ?: return
        val interesting = setOf(
            Intent.ACTION_BOOT_COMPLETED,
            Intent.ACTION_LOCKED_BOOT_COMPLETED,
            Intent.ACTION_MY_PACKAGE_REPLACED,
            "android.intent.action.QUICKBOOT_POWERON",
            "com.htc.intent.action.QUICKBOOT_POWERON"
        )
        if (action !in interesting) return

        val s = SmsBridge.settings
        Log.i("BootReceiver", "action=$action enabled=${s.serviceEnabled} autoStart=${s.autoStart}")
        if (s.serviceEnabled && s.autoStart) {
            runCatching { BridgeService.start(context) }
        }
        if (s.serviceEnabled) KeepAliveScheduler.schedule(context)
    }
}
