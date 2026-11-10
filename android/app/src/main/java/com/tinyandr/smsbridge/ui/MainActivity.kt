package com.tinyandr.smsbridge.ui

import com.tinyandr.smsbridge.util.applySystemBarPadding
import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.View
import android.widget.Toast
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatDelegate
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import androidx.core.os.LocaleListCompat
import com.tinyandr.smsbridge.BuildConfig
import com.tinyandr.smsbridge.R
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.data.UpdateManager
import com.tinyandr.smsbridge.databinding.ActivityMainBinding
import com.tinyandr.smsbridge.service.BridgeService
import java.net.Inet4Address
import java.net.NetworkInterface

class MainActivity : AppCompatActivity() {

    private lateinit var b: ActivityMainBinding
    private lateinit var updater: UpdateManager
    private var suppressSwitch = false
    private var askedOnce = false
    private var pendingRelease: UpdateManager.Release? = null
    private var updating = false

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
        updater = UpdateManager(this)

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
        b.btnNotify.setOnClickListener { startActivity(Intent(this, NotifyActivity::class.java)) }
        b.btnSelfCheck.setOnClickListener { startActivity(Intent(this, SelfCheckActivity::class.java)) }
        b.btnAudit.setOnClickListener { startActivity(Intent(this, AuditActivity::class.java)) }
        b.btnLang.setOnClickListener { showLanguageDialog() }
        b.btnUpdate.setOnClickListener { startUpdate() }
    }

    /** Silent check on foreground; [force] is the manual "check update" path. */
    fun checkForUpdate(force: Boolean) {
        if (updating) return
        Thread {
            val outcome = updater.check(force)
            runOnUiThread {
                val rel = outcome.release
                if (rel != null) {
                    pendingRelease = rel
                    b.cardUpdate.visibility = View.VISIBLE
                    b.tvUpdateTitle.text = getString(R.string.update_found, rel.version)
                    val first = rel.notes.lineSequence().firstOrNull { it.isNotBlank() }?.trim()
                    b.tvUpdateMsg.text = first ?: getString(R.string.update_size, rel.asset.size / 1024 / 1024)
                    b.btnUpdate.text = getString(R.string.update_action)
                    b.btnUpdate.isEnabled = true
                } else if (force) {
                    toast(outcome.error ?: getString(R.string.update_latest))
                }
            }
        }.start()
    }

    private fun startUpdate() {
        val rel = pendingRelease ?: return
        if (updating) return
        if (!updater.canInstall()) {
            toast(getString(R.string.update_allow_install))
            runCatching {
                startActivity(
                    Intent(
                        Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                        Uri.parse("package:$packageName")
                    )
                )
            }
            return
        }
        updating = true
        b.btnUpdate.isEnabled = false
        b.tvUpdateMsg.text = getString(R.string.update_downloading, 0)
        Thread {
            try {
                val file = updater.download(rel.asset) { done, total ->
                    runOnUiThread {
                        b.tvUpdateMsg.text = if (total > 0) {
                            getString(R.string.update_downloading, (done * 100 / total).coerceIn(0, 100))
                        } else {
                            getString(R.string.update_downloaded_kb, done)
                        }
                    }
                }
                runOnUiThread {
                    b.tvUpdateMsg.text = getString(R.string.update_opening_installer)
                    runCatching { updater.install(file) }
                        .onFailure { toast(getString(R.string.update_failed, it.message ?: "")) }
                    updating = false
                    b.btnUpdate.isEnabled = true
                }
            } catch (t: Throwable) {
                runOnUiThread {
                    toast(getString(R.string.update_failed, t.message ?: ""))
                    b.btnUpdate.text = getString(R.string.update_retry)
                    b.btnUpdate.isEnabled = true
                    updating = false
                }
            }
        }.start()
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
        val force = intent.getBooleanExtra(EXTRA_FORCE_CHECK, false)
        if (force) intent.removeExtra(EXTRA_FORCE_CHECK)
        checkForUpdate(force)
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
            if (running) "https://${localIp()}:$port" else getString(R.string.endpoint_unknown)
        val paired = SmsBridge.clients.countActive()
        b.tvPaired.text = if (paired == 0) {
            getString(R.string.paired_none_hint)
        } else {
            getString(R.string.paired_count, paired)
        }
        b.tvIpHint.text =
            "v${BuildConfig.VERSION_NAME} · ${getString(R.string.ip_hint)}: ${localIp()}"

        val s = SmsBridge.settings
        val channels = s.notifyChannelCount
        val status = if (s.notifyAnyTrigger && channels > 0) {
            getString(R.string.notify_status_on, channels)
        } else {
            getString(R.string.notify_status_off)
        }
        b.btnNotify.text = "${getString(R.string.btn_notify)} · $status"

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

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
    }

    companion object {
        /** Set when the settings page asked for a manual update check. */
        const val EXTRA_FORCE_CHECK = "forceUpdateCheck"
    }
}
