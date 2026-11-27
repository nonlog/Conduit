package com.conduit.sync

import android.util.Log
import java.io.File

/**
 * Optional JNI bridge to `libconduit_iroh_android.so` (Rust iroh QUIC WAN transport).
 *
 * When the native library is present in the APK, Conduit uses iroh (n0_dns/Pkarr address
 * lookup + UDP hole-punching + default/custom iroh relays) as its primary off-LAN transport
 * while keeping Noise XX + Protobuf unchanged on top of a local loopback TCP bridge.
 * If the native library is absent (e.g. local JVM unit tests), every method degrades safely.
 */
object IrohBridge {
    private const val TAG = "conduit.iroh"
    private val lock = Any()

    @Volatile
    private var loaded: Boolean = false

    @Volatile
    private var handle: Long = 0L

    @Volatile
    private var cachedEndpointId: String = ""

    init {
        loaded =
            runCatching {
                System.loadLibrary("conduit_iroh_android")
                true
            }.onFailure { err ->
                Log.w(TAG, "conduit_iroh_android native library unavailable: ${err.message}")
            }.getOrDefault(false)
    }

    val isAvailable: Boolean
        get() = loaded

    val endpointId: String
        get() = cachedEndpointId

    fun ensureStarted(
        filesDir: File,
        customRelays: List<String> = emptyList(),
        useDefaultRelays: Boolean = true,
    ): String {
        if (!loaded) return ""
        synchronized(lock) {
            if (handle != 0L) return cachedEndpointId
            val kp = runCatching { Identity.loadOrCreate(filesDir) }.getOrNull() ?: return ""
            val h =
                runCatching {
                    nativeStart(kp.private, customRelays.toTypedArray(), useDefaultRelays)
                }.onFailure { err ->
                    Log.w(TAG, "nativeStart failed", err)
                }.getOrDefault(0L)
            if (h != 0L) {
                handle = h
                cachedEndpointId =
                    runCatching { nativeEndpointId(h).orEmpty() }.getOrDefault("")
                Log.i(TAG, "iroh WAN endpoint ready: ${cachedEndpointId.take(12)}")
            }
            return cachedEndpointId
        }
    }

    /**
     * Dials [peerIrohIdHex] over iroh QUIC and returns a local loopback TCP port (`127.0.0.1:<port>`)
     * spliced to the QUIC bidirectional stream, or `-1` on failure.
     */
    fun dialLocalPort(
        peerIrohIdHex: String,
        timeoutMs: Int = 15_000,
    ): Int {
        if (!loaded || peerIrohIdHex.length != 64) return -1
        val h = handle
        if (h == 0L) return -1
        return runCatching { nativeDialLocalPort(h, peerIrohIdHex, timeoutMs) }
            .onFailure { err -> Log.w(TAG, "nativeDialLocalPort failed", err) }
            .getOrDefault(-1)
    }

    fun networkChanged() {
        val h = handle
        if (!loaded || h == 0L) return
        runCatching { nativeNetworkChanged(h) }
    }

    fun stop() {
        synchronized(lock) {
            val h = handle
            handle = 0L
            if (loaded && h != 0L) {
                runCatching { nativeStop(h) }
            }
        }
    }

    @JvmStatic
    private external fun nativeStart(
        noisePrivateKey: ByteArray,
        customRelays: Array<String>,
        useDefaultRelays: Boolean,
    ): Long

    @JvmStatic
    private external fun nativeEndpointId(handle: Long): String?

    @JvmStatic
    private external fun nativeDialLocalPort(
        handle: Long,
        peerIdHex: String,
        timeoutMs: Int,
    ): Int

    @JvmStatic
    private external fun nativeNetworkChanged(handle: Long)

    @JvmStatic
    private external fun nativeStop(handle: Long)
}
