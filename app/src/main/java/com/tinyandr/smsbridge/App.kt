package com.tinyandr.smsbridge

import android.app.Application
import com.tinyandr.smsbridge.data.AuditLog
import com.tinyandr.smsbridge.data.ClientStore
import com.tinyandr.smsbridge.data.EventStore
import com.tinyandr.smsbridge.data.PairCodeManager
import com.tinyandr.smsbridge.data.Settings
import com.tinyandr.smsbridge.data.SmsRepository

class App : Application() {
    override fun onCreate() {
        super.onCreate()
        SmsBridge.init(this)
    }
}

/** Tiny service locator for this app. */
object SmsBridge {
    private lateinit var app: Application

    fun init(a: Application) {
        app = a
    }

    val settings: Settings by lazy { Settings(app) }
    val clients: ClientStore by lazy { ClientStore(app) }
    val repo: SmsRepository by lazy { SmsRepository(app) }
    val events: EventStore by lazy { EventStore(app, repo) }
    val audit: AuditLog by lazy { AuditLog(app) }
    val pairCodes: PairCodeManager by lazy { PairCodeManager() }
}
