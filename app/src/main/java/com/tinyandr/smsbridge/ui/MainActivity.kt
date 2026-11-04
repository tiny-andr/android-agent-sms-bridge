package com.tinyandr.smsbridge.ui

import com.tinyandr.smsbridge.util.applySystemBarPadding
import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatDelegate
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.core.os.LocaleListCompat
import com.tinyandr.smsbridge.R
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.databinding.ActivityMainBinding
import com.tinyandr.smsbridge.service.BridgeService
import java.net.Inet4Address
import java.net.NetworkInterface

class MainActivity : AppCompatActivity() {

    private lateinit var b: ActivityMainBinding
    private var suppressSwitch = false
    private var askedOnce = false

    private val smsPermLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions()
    ) { _ ->
        if (hasReadSms()) {
            BridgeService.start(this)
        } else {
            toast(getString(R.string.permission_denied))
        }
        b.root.postDelayed({ refresh() }, 500)
    }

    private val notifPermLauncher = registerForActivityResult(
        ActivityResultContracts.RequestPermission()
    ) { refresh() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        b = ActivityMainBinding.inflate(layoutInflater)
        setContentView(b.root)
        applySystemBarPadding(b.root)

        b.swService.setOnCheckedChangeListener { _, checked ->
            if (suppressSwitch) return@setOnCheckedChangeListener
            if (checked) {
                if (!hasReadSms()) {
                    smsPermLauncher.launch(requiredSmsPermissions())
                    return@setOnCheckedChangeListener
                }
                askNotification()
                BridgeService.start(this)
            } else {
                BridgeService.stop(this)
            }
            b.root.postDelayed({ refresh() }, 700)
        }

        b.btnPair.setOnClickListener { startActivity(Intent(this, PairActivity::class.java)) }
        b.btnSettings.setOnClickListener { startActivity(Intent(this, SettingsActivity::class.java)) }
        b.btnSelfCheck.setOnClickListener { startActivity(Intent(this, SelfCheckActivity::class.java)) }
        b.btnAudit.setOnClickListener { startActivity(Intent(this, AuditActivity::class.java)) }
        b.btnLang.setOnClickListener { showLanguageDialog() }
    }

    private fun showLanguageDialog() {
        val tags = arrayOf("", "zh", "en", "ja")
        val labels = arrayOf(
            getString(R.string.lang_system),
            getString(R.string.lang_zh),
            getString(R.string.lang_en),
            getString(R.string.lang_ja)
        )
        val current = AppCompatDelegate.getApplicationLocales().toLanguageTags()
        val checked = tags.indexOfFirst { it.isNotEmpty() && current.startsWith(it) }
        AlertDialog.Builder(this)
            .setTitle(R.string.lang_title)
            .setSingleChoiceItems(labels, checked) { dialog, which ->
                val tag = tags[which]
                AppCompatDelegate.setApplicationLocales(
                    if (tag.isEmpty()) LocaleListCompat.getEmptyLocaleList()
                    else LocaleListCompat.forLanguageTags(tag)
                )
                dialog.dismiss()
            }
            .show()
    }

    override fun onResume() {
        super.onResume()
        refresh()
        // unless-stopped semantics: if the user did not press stop, the service
        // must be running. Re-issue START on every foreground, it is idempotent.
        if (SmsBridge.settings.serviceEnabled) {
            if (!BridgeService.running) {
                runCatching { BridgeService.start(this) }
            }
            b.root.postDelayed({ refresh() }, 900)
        }
        if (!askedOnce) {
            askedOnce = true
            val missing = missingPermissions()
            if (missing.isNotEmpty()) {
                smsPermLauncher.launch(missing)
            }
        }
    }

    private fun refresh() {
        val running = BridgeService.running
        b.tvStatus.setText(if (running) R.string.status_running else R.string.status_stopped)
        b.tvStatus.setTextColor(
            ContextCompat.getColor(this, if (running) R.color.ok else R.color.bad)
        )
        val port = SmsBridge.settings.port
        b.tvEndpoint.text =
            if (running) "http://${localIp()}:$port" else getString(R.string.endpoint_unknown)
        val paired = SmsBridge.clients.countActive()
        b.tvPaired.text = if (paired == 0) {
            getString(R.string.paired_none_hint)
        } else {
            getString(R.string.paired_count, paired)
        }
        b.tvIpHint.text = "${getString(R.string.ip_hint)}: ${localIp()}"

        suppressSwitch = true
        b.swService.isChecked = running
        suppressSwitch = false
    }

    private fun hasReadSms(): Boolean =
        ContextCompat.checkSelfPermission(this, Manifest.permission.READ_SMS) ==
            PackageManager.PERMISSION_GRANTED

    private fun requiredSmsPermissions(): Array<String> {
        val list = mutableListOf(
            Manifest.permission.READ_SMS,
            Manifest.permission.READ_PHONE_STATE
        )
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            list.add(Manifest.permission.POST_NOTIFICATIONS)
        }
        return list.toTypedArray()
    }

    private fun missingPermissions(): Array<String> =
        requiredSmsPermissions().filter {
            ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED
        }.toTypedArray()

    private fun askNotification() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            notifPermLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }

    private fun localIp(): String {
        return try {
            val all = NetworkInterface.getNetworkInterfaces().toList()
                .filter { it.isUp && !it.isLoopback }
            val ordered = all.sortedByDescending { if (it.name.startsWith("wlan")) 1 else 0 }
            for (nif in ordered) {
                for (addr in nif.inetAddresses) {
                    if (addr is Inet4Address && !addr.isLoopbackAddress) {
                        return addr.hostAddress ?: continue
                    }
                }
            }
            "0.0.0.0"
        } catch (t: Throwable) {
            "0.0.0.0"
        }
    }

    private fun toast(msg: String) = Toast.makeText(this, msg, Toast.LENGTH_SHORT).show()
}
