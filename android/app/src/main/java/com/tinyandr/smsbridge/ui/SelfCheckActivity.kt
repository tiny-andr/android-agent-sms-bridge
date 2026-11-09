package com.tinyandr.smsbridge.ui

import com.tinyandr.smsbridge.util.applySystemBarPadding
import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import android.view.View
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import com.tinyandr.smsbridge.R
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.databinding.ActivitySelfcheckBinding
import com.tinyandr.smsbridge.databinding.ItemCheckBinding
import com.tinyandr.smsbridge.service.BridgeService
import java.net.InetSocketAddress
import java.net.Socket

class SelfCheckActivity : AppCompatActivity() {

    private lateinit var b: ActivitySelfcheckBinding
    private val permLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { runChecks() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        b = ActivitySelfcheckBinding.inflate(layoutInflater)
        setContentView(b.root)
        applySystemBarPadding(b.root)
    }

    override fun onResume() {
        super.onResume()
        runChecks()
    }

    private fun runChecks() {
        b.checkList.removeAllViews()

        header()

        // 1) READ_SMS
        val smsOk = ContextCompat.checkSelfPermission(this, Manifest.permission.READ_SMS) ==
            PackageManager.PERMISSION_GRANTED
        addCheck(
            getString(R.string.selfcheck_read_sms), smsOk, null,
            getString(R.string.selfcheck_fix)
        ) {
            permLauncher.launch(arrayOf(Manifest.permission.READ_SMS))
        }

        // 2) notifications
        val notifOk = Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        addCheck(
            getString(R.string.selfcheck_notif), notifOk, null,
            getString(R.string.selfcheck_fix)
        ) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                permLauncher.launch(arrayOf(Manifest.permission.POST_NOTIFICATIONS))
            }
        }

        // 2b) ColorOS verification-code protection: blocks OTP SMS for every reader
        val otpBlocked = SmsBridge.repo.verificationCodeBlocked()
        addCheck(
            getString(R.string.selfcheck_otp), !otpBlocked,
            if (otpBlocked) getString(R.string.selfcheck_otp_hint) else null,
            if (otpBlocked) getString(R.string.selfcheck_otp_btn) else null
        ) {
            runCatching { startActivity(Intent(Settings.ACTION_SETTINGS)) }
        }

        // 3) battery optimisation
        val pm = getSystemService(PowerManager::class.java)
        val batteryOk = pm.isIgnoringBatteryOptimizations(packageName)
        addCheck(
            getString(R.string.selfcheck_battery), batteryOk,
            if (batteryOk) null else "Doze 下网络会被挂起，必须加入白名单",
            getString(R.string.selfcheck_fix)
        ) {
            runCatching {
                startActivity(
                    Intent(
                        Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                        Uri.parse("package:$packageName")
                    )
                )
            }.onFailure {
                runCatching {
                    startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
                }
            }
        }

        // 4) service running
        val running = BridgeService.running
        addCheck(
            getString(R.string.selfcheck_service), running, null,
            getString(R.string.selfcheck_fix)
        ) {
            if (!BridgeService.running) {
                runCatching {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                        startForegroundService(
                            Intent(this, BridgeService::class.java)
                                .setAction(BridgeService.ACTION_START)
                        )
                    } else {
                        startService(
                            Intent(this, BridgeService::class.java)
                                .setAction(BridgeService.ACTION_START)
                        )
                    }
                }
            }
        }

        // 5) port reachable (async)
        val port = SmsBridge.settings.port
        val portItem = addCheck(getString(R.string.selfcheck_network), null, null, null, null)
        portItem.tvStatus.text = "检查中…"
        checkPort(port) { ok ->
            portItem.tvStatus.text =
                if (ok) getString(R.string.selfcheck_ok) else getString(R.string.selfcheck_fail)
            portItem.tvStatus.setTextColor(
                ContextCompat.getColor(this, if (ok) R.color.ok else R.color.bad)
            )
            portItem.tvDetail.text = "127.0.0.1:$port"
        }

        // 6) vendor background whitelist (cannot be read programmatically)
        addCheck(
            getString(R.string.selfcheck_rom), null,
            getString(R.string.selfcheck_rom_hint),
            getString(R.string.selfcheck_rom_btn)
        ) {
            runCatching {
                startActivity(
                    Intent(
                        Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                        Uri.parse("package:$packageName")
                    )
                )
            }
        }
    }

    private fun header() {
        val tv = com.google.android.material.textview.MaterialTextView(this).apply {
            text = getString(R.string.selfcheck_title)
            setTextAppearance(com.google.android.material.R.style.TextAppearance_Material3_HeadlineSmall)
            setPadding(0, 0, 0, 12)
        }
        b.checkList.addView(tv)
    }

    private fun addCheck(
        title: String,
        ok: Boolean?,
        detail: String?,
        actionLabel: String?,
        action: (() -> Unit)?
    ): ItemCheckBinding {
        val item = ItemCheckBinding.inflate(layoutInflater, b.checkList, false)
        item.tvTitle.text = title
        if (ok != null) {
            item.tvStatus.text =
                if (ok) getString(R.string.selfcheck_ok) else getString(R.string.selfcheck_fail)
            item.tvStatus.setTextColor(
                ContextCompat.getColor(this, if (ok) R.color.ok else R.color.bad)
            )
        }
        if (detail.isNullOrBlank()) {
            item.tvDetail.visibility = View.GONE
        } else {
            item.tvDetail.text = detail
        }
        if (actionLabel == null || action == null) {
            item.btnFix.visibility = View.GONE
        } else {
            item.btnFix.text = actionLabel
            item.btnFix.setOnClickListener { action() }
        }
        b.checkList.addView(item.root)
        return item
    }

    private fun checkPort(port: Int, cb: (Boolean) -> Unit) {
        Thread {
            val ok = try {
                Socket().use {
                    it.connect(InetSocketAddress("127.0.0.1", port), 800)
                }
                true
            } catch (t: Throwable) {
                false
            }
            runOnUiThread { cb(ok) }
        }.start()
    }
}
