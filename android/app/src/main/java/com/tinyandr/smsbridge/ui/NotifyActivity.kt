package com.tinyandr.smsbridge.ui

import com.tinyandr.smsbridge.util.applySystemBarPadding
import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import com.google.android.material.chip.Chip
import com.tinyandr.smsbridge.R
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.databinding.ActivityNotifyBinding
import com.tinyandr.smsbridge.notify.ProactiveNotifier
import com.tinyandr.smsbridge.service.BridgeService

/**
 * Proactive push configuration: which events are pushed, which messages pass the
 * keyword filter, and where they go.
 *
 * Credentials are typed here and stored in this app's private preferences. They
 * are never part of the build, so a clone of the repository has no token, no
 * webhook and no chat id in it.
 */
class NotifyActivity : AppCompatActivity() {

    private lateinit var b: ActivityNotifyBinding

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        b = ActivityNotifyBinding.inflate(layoutInflater)
        setContentView(b.root)
        applySystemBarPadding(b.root)

        buildIntervalChips()
        fill()

        b.swScan.setOnCheckedChangeListener { _, _ -> refreshIntervalState() }
        b.btnTgFetch.setOnClickListener { fetchChatId() }
        b.btnTgTest.setOnClickListener { sendTest(ProactiveNotifier.CHANNEL_TELEGRAM) }
        b.btnFsTest.setOnClickListener { sendTest(ProactiveNotifier.CHANNEL_FEISHU) }
        b.btnSave.setOnClickListener { save() }
    }

    // ------------------------------------------------------------------- view

    private fun buildIntervalChips() {
        if (b.cgInterval.childCount > 0) return
        val inflater = LayoutInflater.from(this)
        INTERVALS.forEach { minutes ->
            val chip = inflater.inflate(R.layout.item_chip, b.cgInterval, false) as Chip
            chip.id = View.generateViewId()
            chip.tag = minutes
            chip.text = getString(R.string.notify_interval_value, minutes)
            b.cgInterval.addView(chip)
        }
    }

    private fun fill() {
        val s = SmsBridge.settings
        b.swEvent.isChecked = s.notifyEventEnabled
        b.swScan.isChecked = s.notifyScanEnabled
        b.etKeywords.setText(s.notifyKeywords)
        b.swTg.isChecked = s.tgEnabled
        b.etTgToken.setText(s.tgToken)
        b.etTgChatId.setText(s.tgChatId)
        b.swFeishu.isChecked = s.feishuEnabled
        b.etFsHook.setText(s.feishuWebhook)
        b.etFsSecret.setText(s.feishuSecret)
        selectInterval(s.notifyScanIntervalMin)
    }

    private fun selectInterval(minutes: Int) {
        // Values saved by an older build may not be one of the chips; snap up to
        // the next offered option rather than silently changing the stored value.
        val target = INTERVALS.firstOrNull { it >= minutes } ?: INTERVALS.last()
        for (i in 0 until b.cgInterval.childCount) {
            val chip = b.cgInterval.getChildAt(i) as? Chip ?: continue
            chip.isChecked = chip.tag == target
        }
        refreshIntervalState()
    }

    private fun selectedInterval(): Int {
        for (i in 0 until b.cgInterval.childCount) {
            val chip = b.cgInterval.getChildAt(i) as? Chip ?: continue
            if (chip.isChecked) (chip.tag as? Int)?.let { return it }
        }
        return 15
    }

    private fun refreshIntervalState() {
        val on = b.swScan.isChecked
        for (i in 0 until b.cgInterval.childCount) {
            val chip = b.cgInterval.getChildAt(i) ?: continue
            chip.isEnabled = on
            chip.alpha = if (on) 1f else 0.4f
        }
    }

    // ------------------------------------------------------------------ save

    private fun save(quiet: Boolean = false) {
        val s = SmsBridge.settings
        s.notifyEventEnabled = b.swEvent.isChecked
        s.notifyScanEnabled = b.swScan.isChecked
        s.notifyScanIntervalMin = selectedInterval()
        s.notifyKeywords = b.etKeywords.text?.toString()?.trim().orEmpty()
        s.tgEnabled = b.swTg.isChecked
        s.tgToken = b.etTgToken.text?.toString().orEmpty()
        s.tgChatId = b.etTgChatId.text?.toString().orEmpty()
        s.feishuEnabled = b.swFeishu.isChecked
        s.feishuWebhook = b.etFsHook.text?.toString().orEmpty()
        s.feishuSecret = b.etFsSecret.text?.toString().orEmpty()

        if (s.notifyAnyTrigger) {
            // The observer lives in the bridge service. Re-issuing START is
            // idempotent and re-arms the scan timer with the new interval, so
            // the feature keeps working without asking the user to flip the
            // main switch as well.
            BridgeService.start(this)
        }
        if (quiet) return
        if (s.notifyAnyTrigger && !s.notifyAnyChannel) {
            toast(getString(R.string.notify_need_channel))
        } else {
            toast(getString(R.string.notify_saved))
        }
    }

    // ------------------------------------------------------- telegram chat id

    private fun fetchChatId() {
        val token = b.etTgToken.text?.toString()?.trim().orEmpty()
        if (token.isBlank()) {
            toast(getString(R.string.notify_need_token))
            return
        }
        b.btnTgFetch.isEnabled = false
        Thread {
            val (chatId, error) = ProactiveNotifier.fetchTelegramChatId(token)
            runOnUiThread {
                b.btnTgFetch.isEnabled = true
                if (chatId != null) {
                    b.etTgChatId.setText(chatId)
                    SmsBridge.settings.tgToken = token
                    SmsBridge.settings.tgChatId = chatId
                    toast(getString(R.string.notify_tg_fetch_ok, chatId))
                } else if (error == "no message yet") {
                    toast(getString(R.string.notify_tg_fetch_need_message))
                } else {
                    toast(getString(R.string.notify_tg_fetch_fail, error ?: "?"))
                }
            }
        }.start()
    }

    // ------------------------------------------------------------------ test

    /** Tests always run against whatever is on screen, not the last saved state. */
    private fun sendTest(channel: String) {
        save(quiet = true)
        val s = SmsBridge.settings
        val ready = when (channel) {
            ProactiveNotifier.CHANNEL_TELEGRAM ->
                s.tgToken.isNotBlank() && s.tgChatId.isNotBlank()
            ProactiveNotifier.CHANNEL_FEISHU -> s.feishuWebhook.isNotBlank()
            else -> false
        }
        if (!ready) {
            toast(getString(R.string.notify_need_channel))
            return
        }
        val button = if (channel == ProactiveNotifier.CHANNEL_TELEGRAM) b.btnTgTest else b.btnFsTest
        button.isEnabled = false
        button.text = getString(R.string.notify_test_running)
        Thread {
            val error = ProactiveNotifier.sendTest(this, channel)
            runOnUiThread {
                button.isEnabled = true
                button.text = getString(R.string.notify_test)
                if (error == null) {
                    toast(getString(R.string.notify_test_ok))
                } else {
                    toast(getString(R.string.notify_test_fail, error))
                }
            }
        }.start()
    }

    private fun toast(msg: String) = Toast.makeText(this, msg, Toast.LENGTH_SHORT).show()

    companion object {
        /** Must match the chips the UI offers; anything else is snapped to these. */
        private val INTERVALS = intArrayOf(5, 15, 30, 60)
    }
}
