package com.tinyandr.smsbridge.ui

import com.tinyandr.smsbridge.util.applySystemBarPadding
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.LayoutInflater
import android.view.View
import androidx.appcompat.app.AppCompatActivity
import com.tinyandr.smsbridge.R
import com.tinyandr.smsbridge.SmsBridge
import com.tinyandr.smsbridge.data.PairedClient
import com.tinyandr.smsbridge.databinding.ActivityPairBinding
import com.tinyandr.smsbridge.databinding.ItemClientBinding
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

class PairActivity : AppCompatActivity() {

    private lateinit var b: ActivityPairBinding
    private val handler = Handler(Looper.getMainLooper())
    private val fmt = SimpleDateFormat("MM-dd HH:mm", Locale.getDefault())

    private val ticker = object : Runnable {
        override fun run() {
            updateCode()
            handler.postDelayed(this, 1000)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        b = ActivityPairBinding.inflate(layoutInflater)
        setContentView(b.root)
        applySystemBarPadding(b.root)

        b.btnGenerate.setOnClickListener {
            SmsBridge.pairCodes.generate()
            updateCode()
        }
        SmsBridge.pairCodes.generate()
    }

    override fun onResume() {
        super.onResume()
        updateCode()
        renderClients()
        handler.post(ticker)
    }

    override fun onPause() {
        super.onPause()
        handler.removeCallbacks(ticker)
    }

    private fun updateCode() {
        val cur = SmsBridge.pairCodes.current()
        if (cur == null) {
            b.tvPairCode.text = "------"
            b.tvCodeExpiry.text = "已过期，点下方按钮生成新码"
        } else {
            val code = cur.first
            b.tvPairCode.text = code.chunked(3).joinToString(" ")
            val left = ((cur.second - System.currentTimeMillis()) / 1000).coerceAtLeast(0)
            b.tvCodeExpiry.text = "剩余 ${left} 秒 · 一次性"
        }
    }

    private fun renderClients() {
        val clients = SmsBridge.clients.list().sortedByDescending { it.createdAt }
        b.clientList.removeAllViews()
        b.tvNoClients.visibility = if (clients.isEmpty()) View.VISIBLE else View.GONE
        clients.forEach { c ->
            val item = ItemClientBinding.inflate(LayoutInflater.from(this), b.clientList, false)
            bindClient(item, c)
            b.clientList.addView(item.root)
        }
    }

    private fun bindClient(item: ItemClientBinding, c: PairedClient) {
        item.tvName.text = c.name
        item.tvMeta.text = "ID ${c.id.take(8)} · ${getString(R.string.pair_create_time)} ${fmt.format(Date(c.createdAt))}"
        val lastActive = if (c.lastActiveAt == 0L) "从未" else fmt.format(Date(c.lastActiveAt))
        item.tvStats.text =
            "${getString(R.string.pair_last_active)} $lastActive · ${getString(R.string.pair_requests)} ${c.requestCount} · ${c.lastIp}"
        if (c.revoked) {
            item.tvName.text = "${c.name}（${getString(R.string.pair_revoked)}）"
            item.btnRevoke.isEnabled = false
            item.btnRevoke.alpha = 0.4f
        }
        item.btnRevoke.setOnClickListener {
            SmsBridge.clients.revoke(c.id)
            renderClients()
        }
        item.btnRemove.setOnClickListener {
            SmsBridge.clients.remove(c.id)
            renderClients()
        }
    }
}
