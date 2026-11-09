package com.tinyandr.smsbridge.ui

import com.tinyandr.smsbridge.util.applySystemBarPadding
import android.os.Bundle
import android.view.View
import androidx.appcompat.app.AppCompatActivity
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.databinding.ActivityAuditBinding
import com.tinyandr.smsbridge.databinding.ItemAuditBinding
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class AuditActivity : AppCompatActivity() {

    private lateinit var b: ActivityAuditBinding
    private val fmt = SimpleDateFormat("MM-dd HH:mm:ss", Locale.getDefault())

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        b = ActivityAuditBinding.inflate(layoutInflater)
        setContentView(b.root)
        applySystemBarPadding(b.root)
        b.btnClear.setOnClickListener {
            SmsBridge.audit.clear()
            render()
        }
    }

    override fun onResume() {
        super.onResume()
        render()
    }

    private fun render() {
        val entries = SmsBridge.audit.recent(200)
        b.auditList.removeAllViews()
        b.tvEmpty.visibility = if (entries.isEmpty()) View.VISIBLE else View.GONE
        entries.forEach { e ->
            val item = ItemAuditBinding.inflate(layoutInflater, b.auditList, false)
            item.tvLine1.text = "${e.clientName} · ${e.path}"
            val q = if (e.query.isBlank()) "" else " · ${e.query}"
            item.tvLine2.text = "${fmt.format(Date(e.ts))} · ${e.ip}$q"
            b.auditList.addView(item.root)
        }
    }
}
