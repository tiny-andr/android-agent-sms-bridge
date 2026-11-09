package com.tinyandr.smsbridge.http

import android.content.Context
import com.tinyandr.smsbridge.util.Crypto
import java.security.KeyStore
import java.security.cert.X509Certificate
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext

/**
 * TLS material for the local API.
 *
 * The keystore is shipped inside the APK on purpose: every install serves the
 * same self-signed certificate, so a client can pin it by fingerprint and refuse
 * anything else. An attacker on the same Wi-Fi can no longer read tokens or SMS
 * bodies; to decrypt they would have to also extract this private key from the
 * APK, which is a much higher bar than a packet capture.
 */
object TlsServer {

    private const val P12_ASSET = "server.p12"
    private const val P12_PASSWORD = "smsbridge"
    private const val ALIAS = "smsbridge"

    @Volatile
    private var cachedContext: SSLContext? = null

    @Volatile
    private var cachedFingerprint: String? = null

    fun sslContext(ctx: Context): SSLContext {
        cachedContext?.let { return it }
        synchronized(this) {
            cachedContext?.let { return it }
            val ks = KeyStore.getInstance("PKCS12")
            ctx.assets.open(P12_ASSET).use { input ->
                ks.load(input, P12_PASSWORD.toCharArray())
            }
            val kmf = KeyManagerFactory.getInstance(KeyManagerFactory.getDefaultAlgorithm())
            kmf.init(ks, P12_PASSWORD.toCharArray())
            val sc = SSLContext.getInstance("TLS")
            sc.init(kmf.keyManagers, null, null)
            val cert = ks.getCertificate(ALIAS) as X509Certificate
            cachedFingerprint = Crypto.sha256Hex(cert.encoded)
            cachedContext = sc
            return sc
        }
    }

    /** SHA-256 of the DER certificate, lowercase hex, no separators. */
    fun fingerprint(ctx: Context): String {
        cachedFingerprint?.let { return it }
        sslContext(ctx)
        return cachedFingerprint ?: ""
    }

    /** Same fingerprint grouped for display, e.g. "A1 B2 C3 …". */
    fun prettyFingerprint(ctx: Context): String =
        fingerprint(ctx).chunked(2).joinToString(" ").uppercase()
}
