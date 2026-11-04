package com.tinyandr.smsbridge.service

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import android.util.Log
import com.tinyandr.smsbridge.SmsBridge

/**
 * Second watchdog path next to [KeepAliveReceiver]. JobScheduler survives
 * reboots (persisted) and is handled differently from AlarmManager by vendor
 * ROMs, so the two together cover more of the "killed in background" cases.
 */
class KeepAliveJob : JobService() {

    override fun onStartJob(params: JobParameters?): Boolean {
        val s = SmsBridge.settings
        if (s.serviceEnabled && !BridgeService.running) {
            Log.i(TAG, "job watchdog: service dead, restarting")
            runCatching { BridgeService.start(this) }
        }
        // re-arm only the one-shot alarm; the periodic job reschedules itself
        if (s.serviceEnabled) KeepAliveScheduler.schedule(this)
        return false
    }

    override fun onStopJob(params: JobParameters?): Boolean = true

    companion object {
        private const val TAG = "KeepAliveJob"
        private const val JOB_ID = 3001

        fun schedule(ctx: Context) {
            try {
                val js = ctx.getSystemService(Context.JOB_SCHEDULER_SERVICE) as JobScheduler
                // Scheduling is idempotent on the system side, but every call burns
                // a slot in the "schedulePersisted" quota (250/min). Skip if pending.
                if (js.getPendingJob(JOB_ID) != null) return
                val info = JobInfo.Builder(
                    JOB_ID,
                    ComponentName(ctx, KeepAliveJob::class.java)
                )
                    .setPeriodic(15 * 60_000L)
                    .setPersisted(true)
                    .setRequiredNetworkType(JobInfo.NETWORK_TYPE_NONE)
                    .build()
                js.schedule(info)
                Log.i(TAG, "job watchdog scheduled")
            } catch (t: Throwable) {
                Log.w(TAG, "cannot schedule job", t)
            }
        }

        fun cancel(ctx: Context) {
            try {
                val js = ctx.getSystemService(Context.JOB_SCHEDULER_SERVICE) as JobScheduler
                js.cancel(JOB_ID)
            } catch (_: Throwable) {
            }
        }
    }
}
