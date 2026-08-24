package com.conduit.sync

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.net.InetSocketAddress

private const val TAG = "conduit.nsd"
private const val SERVICE_TYPE = "_conduit._tcp."

/** Long enough for a resolve on a busy Wi-Fi, short enough to never be "always on". */
private const val BURST_MS = 8_000L

/**
 * mDNS discovery, in bursts.
 *
 * Continuous discovery is what makes companion apps expensive: the radio is kept
 * interested and every multicast on the subnet wakes the app. A burst runs only when
 * something changed — service start, a new network, or the user asking — and stops
 * itself either on the first resolve or on the deadline, whichever comes first.
 *
 * One pending message on the main looper is the entire timing cost, and only while a
 * burst is in flight.
 */
class Discovery(context: Context, private val onFound: (InetSocketAddress) -> Unit) {
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val handler = Handler(Looper.getMainLooper())
    private val deadline = Runnable {
        Log.d(TAG, "burst found nothing")
        stop()
    }

    /** Guarded by `this`; the platform's callbacks arrive on its own threads. */
    private var listener: NsdManager.DiscoveryListener? = null

    fun burst() {
        synchronized(this) {
            if (listener != null) {
                Log.d(TAG, "burst already running")
                return
            }
            val fresh = Listener()
            listener = fresh
            handler.postDelayed(deadline, BURST_MS)
            nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, fresh)
        }
    }

    fun stop() {
        synchronized(this) {
            handler.removeCallbacks(deadline)
            val current = listener ?: return
            listener = null
            // Throws if the platform already stopped it out from under us, which is a
            // normal race with onStartDiscoveryFailed.
            runCatching { nsd.stopServiceDiscovery(current) }
        }
    }

    private inner class Listener : NsdManager.DiscoveryListener {
        override fun onServiceFound(info: NsdServiceInfo) {
            Log.d(TAG, "found ${info.serviceName}")
            @Suppress("DEPRECATION") // registerServiceInfoCallback is API 34; minSdk is 29.
            nsd.resolveService(info, Resolver())
        }

        override fun onServiceLost(info: NsdServiceInfo) {
            // The link notices a dead peer by itself; an mDNS goodbye adds nothing.
            Log.d(TAG, "lost ${info.serviceName}")
        }

        override fun onDiscoveryStarted(type: String) {
            Log.d(TAG, "burst started")
        }

        override fun onDiscoveryStopped(type: String) {
            Log.d(TAG, "burst stopped")
        }

        override fun onStartDiscoveryFailed(type: String, error: Int) {
            Log.w(TAG, "discovery would not start: $error")
            stop()
        }

        override fun onStopDiscoveryFailed(type: String, error: Int) {
            Log.w(TAG, "discovery would not stop: $error")
        }
    }

    private inner class Resolver : NsdManager.ResolveListener {
        override fun onServiceResolved(resolved: NsdServiceInfo) {
            @Suppress("DEPRECATION") // getHostAddresses is API 34; minSdk is 29.
            val host = resolved.host
            if (host == null) {
                Log.w(TAG, "${resolved.serviceName} resolved without an address")
                return
            }
            Log.i(TAG, "resolved ${resolved.serviceName} at $host:${resolved.port}")
            // One peer in M0, so the first answer ends the burst.
            stop()
            onFound(InetSocketAddress(host, resolved.port))
        }

        override fun onResolveFailed(failed: NsdServiceInfo, error: Int) {
            // The burst deadline still applies, so another advert can win.
            Log.w(TAG, "resolve of ${failed.serviceName} failed: $error")
        }
    }
}
