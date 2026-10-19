package com.conduit.sync

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import java.net.InetSocketAddress
import java.util.ArrayDeque

private const val TAG = "conduit.nsd"
private const val SERVICE_TYPE = "_conduit._tcp."
private const val BURST_MS = 8_000L
private const val MAX_RESOLVES_PER_BURST = 16

/** Event-triggered, bounded mDNS discovery. Every callback belongs to exactly one burst. */
class Discovery(
    context: Context,
    private val onFound: (InetSocketAddress) -> Unit,
    private val onEmpty: () -> Unit = {},
    private val expectedPeerId: () -> String? = { null },
) {
    private val nsd = context.getSystemService(NsdManager::class.java)
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val executor = context.mainExecutor
    private val handler = Handler(Looper.getMainLooper())
    private val epochs = DiscoveryEpoch()
    private var listener: Listener? = null

    /** Capture this in onFound before posting work; direct sockets use the same physical network. */
    var resolvedNetwork: Network? = null
        private set

    fun burst(network: Network? = null, durationMs: Long = BURST_MS) = onMain {
        val current = listener
        if (current != null && current.network == network) return@onMain
        stopInternal()
        resolvedNetwork = null
        val fresh = Listener(epochs.begin(), network)
        listener = fresh
        handler.postDelayed(fresh.deadline, durationMs.coerceIn(1L, 30_000L))
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU && network != null) {
                nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, network, executor, fresh)
            } else {
                @Suppress("DEPRECATION")
                nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, fresh)
            }
        }.onFailure {
            Log.w(TAG, "could not start LAN discovery", it)
            finishEmpty(fresh)
        }
    }

    fun stop() = onMain { stopInternal() }

    private fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == handler.looper) block() else handler.post { block() }
    }

    private fun isCurrent(candidate: Listener): Boolean =
        listener === candidate && epochs.isCurrent(candidate.epoch)

    private fun stopInternal() {
        epochs.stop()
        val current = listener ?: return
        listener = null
        handler.removeCallbacks(current.deadline)
        current.pending.clear()
        runCatching { nsd.stopServiceDiscovery(current) }
    }

    private fun finishEmpty(current: Listener) {
        if (!isCurrent(current) || !epochs.finish(current.epoch)) return
        listener = null
        handler.removeCallbacks(current.deadline)
        current.pending.clear()
        runCatching { nsd.stopServiceDiscovery(current) }
        Log.d(TAG, "bounded LAN discovery found no usable paired IPv4 address")
        onEmpty()
    }

    private fun acceptResolved(current: Listener, service: NsdServiceInfo) {
        if (!isCurrent(current)) return
        current.resolving = false
        val advertised = service.attributes["id"]?.toString(Charsets.UTF_8)
        val expected = expectedPeerId()
        val serviceNetwork = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            service.network ?: current.network
        } else {
            current.network
        }
        // A late resolver must not route a new burst onto a previous Wi-Fi interface.
        if (current.network != null && serviceNetwork != current.network) {
            resolveNext(current)
            return
        }
        if (!LanDiscoveryPolicy.acceptsPeer(expected, advertised) || service.port !in 1..65535) {
            Log.d(TAG, "ignoring an unpaired/invalid LAN service")
            resolveNext(current)
            return
        }
        val addresses = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            service.hostAddresses
        } else {
            @Suppress("DEPRECATION")
            listOfNotNull(service.host)
        }
        val subnets = serviceNetwork?.let(connectivity::getLinkProperties)?.linkAddresses.orEmpty()
            .map { LanDiscoveryPolicy.Subnet(it.address, it.prefixLength) }
        val address = LanDiscoveryPolicy.selectAddress(addresses, subnets)
        if (address == null) {
            // Some VPN clients replace .local A records with 198.18/15 fake IPs even when NSD
            // discovered the TXT record over the physical Wi-Fi. Windows advertised its real
            // 192.168.x.x mDNS address, but Android's resolver never returned that candidate.
            // Query the already-verified SRV hostname once through the same physical Network.
            val hostname = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.VANILLA_ICE_CREAM) {
                LanMdnsResolver.hostname(service.hostname)
            } else null
            val fallback = hostname ?: LanMdnsResolver.hostname("${service.serviceName}.local")
            if (serviceNetwork != null && fallback != null) {
                current.resolving = true
                Log.d(TAG, "NSD has no on-link IPv4 (${addresses.size} candidates); querying physical mDNS A")
                Thread({
                    val candidates = runCatching { LanMdnsResolver.query(serviceNetwork, fallback) }
                        .onFailure { Log.d(TAG, "physical mDNS query failed", it) }
                        .getOrDefault(emptyList())
                    onMain {
                        if (!isCurrent(current)) return@onMain
                        current.resolving = false
                        val selected = LanDiscoveryPolicy.selectAddress(candidates, subnets)
                        if (selected == null) {
                            Log.d(TAG, "physical mDNS yielded no usable paired IPv4")
                            resolveNext(current)
                        } else {
                            acceptAddress(current, selected, serviceNetwork, service.port)
                        }
                    }
                }, "conduit-lan-mdns").apply { isDaemon = true }.start()
            } else {
                Log.d(TAG, "service has no usable address or resolvable hostname on LAN")
                resolveNext(current)
            }
            return
        }
        acceptAddress(current, address, serviceNetwork, service.port)
    }

    private fun acceptAddress(current: Listener, address: java.net.InetAddress, network: Network?, port: Int) {
        if (!isCurrent(current)) return
        if (!epochs.finish(current.epoch)) return
        listener = null
        handler.removeCallbacks(current.deadline)
        current.pending.clear()
        runCatching { nsd.stopServiceDiscovery(current) }
        resolvedNetwork = network
        Log.i(TAG, "resolved paired LAN service on $network at $address:$port")
        // TXT is only a prefilter. Link still authenticates the stored Noise identity.
        onFound(InetSocketAddress(address, port))
    }

    private fun resolveNext(current: Listener) {
        if (!isCurrent(current) || current.resolving) return
        val info = current.pending.pollFirst() ?: return
        current.resolving = true
        val resolver = object : NsdManager.ResolveListener {
            override fun onServiceResolved(resolved: NsdServiceInfo) = onMain {
                acceptResolved(current, resolved)
            }
            override fun onResolveFailed(failed: NsdServiceInfo, error: Int) = onMain {
                if (!isCurrent(current)) return@onMain
                current.resolving = false
                Log.d(TAG, "LAN resolve failed: $error")
                resolveNext(current)
            }
        }
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                @Suppress("DEPRECATION")
                nsd.resolveService(info, executor, resolver)
            } else {
                @Suppress("DEPRECATION")
                nsd.resolveService(info, resolver)
            }
        }.onFailure {
            current.resolving = false
            Log.w(TAG, "LAN resolver unavailable", it)
            resolveNext(current)
        }
    }

    private inner class Listener(val epoch: Long, val network: Network?) : NsdManager.DiscoveryListener {
        val pending = ArrayDeque<NsdServiceInfo>()
        val seen = mutableSetOf<String>()
        var resolving = false
        val deadline = Runnable { finishEmpty(this) }

        override fun onServiceFound(info: NsdServiceInfo) = onMain {
            if (!isCurrent(this)) return@onMain
            val type = info.serviceType.orEmpty().trimEnd('.').lowercase()
            if (type != "_conduit._tcp" && type != "_conduit._tcp.local") return@onMain
            if (seen.size >= MAX_RESOLVES_PER_BURST || !seen.add(info.serviceName)) return@onMain
            pending.addLast(info)
            resolveNext(this)
        }
        override fun onServiceLost(info: NsdServiceInfo) = Unit
        override fun onDiscoveryStarted(type: String) = Unit
        override fun onDiscoveryStopped(type: String) = onMain { finishEmpty(this) }
        override fun onStartDiscoveryFailed(type: String, error: Int) = onMain {
            if (!isCurrent(this)) return@onMain
            Log.w(TAG, "LAN discovery failed: $error")
            finishEmpty(this)
        }
        override fun onStopDiscoveryFailed(type: String, error: Int) = Unit
    }
}
