package com.tinyandr.smsbridge.ui

import com.tinyandr.smsbridge.util.applySystemBarPadding
import android.os.Bundle
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import com.tinyandr.smsbridge.R
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.databinding.ActivitySettingsBinding
import com.tinyandr.smsbridge.service.BridgeService

class SettingsActivity : AppCompatActivity() {

    private lateinit var b: ActivitySettingsBinding

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        b = ActivitySettingsBinding.inflate(layoutInflater)
        setContentView(b.root)
        applySystemBarPadding(b.root)

        fill()
        b.btnSave.setOnClickListener { save() }
    }

    private fun fill() {
        val s = SmsBridge.settings
        b.etPort.setText(s.port.toString())
        b.etMaxDays.setText(s.maxDays.toString())
        b.etMaxLimit.setText(s.maxLimit.toString())
        b.etRateLimit.setText(s.rateLimitPerMin.toString())
        b.etBlocked.setText(s.blockedAddresses)
        b.swOtp.isChecked = s.otpModeAllowed
        b.swHeartbeat.isChecked = s.heartbeatEnabled
        b.swHoldLocks.isChecked = s.holdLocks
    }

    private fun save() {
        val s = SmsBridge.settings
        val oldPort = s.port

        s.port = b.etPort.text?.toString()?.toIntOrNull()?.coerceIn(1024, 65535) ?: oldPort
        s.maxDays = b.etMaxDays.text?.toString()?.toIntOrNull()?.coerceIn(0, 3650) ?: s.maxDays
        s.maxLimit = b.etMaxLimit.text?.toString()?.toIntOrNull()?.coerceIn(1, 2000) ?: s.maxLimit
        s.rateLimitPerMin =
            b.etRateLimit.text?.toString()?.toIntOrNull()?.coerceIn(0, 10_000) ?: s.rateLimitPerMin
        s.blockedAddresses = b.etBlocked.text?.toString()?.trim() ?: ""
        s.otpModeAllowed = b.swOtp.isChecked
        s.heartbeatEnabled = b.swHeartbeat.isChecked
        s.holdLocks = b.swHoldLocks.isChecked

        if (BridgeService.running) {
            // Re-issue START: onStartCommand re-applies heartbeat/locks/port without
            // tearing the service down (a stop+start from the background is illegal).
            BridgeService.start(this)
        }
        Toast.makeText(this, getString(R.string.settings_saved), Toast.LENGTH_SHORT).show()
        finish()
    }
}
