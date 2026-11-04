package com.conduit.sync

import android.content.Context
import android.net.Network
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.conduit.sync.proto.NotifAction
import java.net.InetSocketAddress

private const val EXTRA_TAG = "conduit.peers"

/**
 * Additional encrypted desktop sessions. The original Link is retained as the main transport for
 * upgrade compatibility, while each other trusted desktop owns an independent Link, writer queue,
 * Noise handshake, discovery window, Relay rendezvous and bounded reconnect backoff.
 *
 * No socket or callback from one computer can replace the socket of another. A phone may be
 * connected to a desktop via LAN and another via Relay at the same time.
 */
internal class CompanionLinks(
    private val context: Context,
    private val privateKey: ByteArray,
    private val localId: String,
    private val localName: String,
    private val lanNetwork: () -> Network?,
    private val relayEndpoints: () -> List<RelayEndpoint>,
    private val networkAvailable: () -> Boolean,
    private val onChange: () -> Unit,
    private val onIncomingText: (String, String) -> Unit,
    private val onIncomingImage: (String, ByteArray, Boolean, Boolean) -> Unit,
    private val onTransferEvent: (FileTransfer) -> Unit,
    private val onError: (String) -> Unit,
    private val onAction: (NotifAction) -> Unit,
    private val onForgotten: (String) -> Unit,
    private val onWallpaper: (Link, ByteArray) -> Unit,
) {
    private val main = Handler(Looper.getMainLooper())
    private val peers = LinkedHashMap<String, Peer>()
    private var closed = false

    data class State(val id: String, val status: LinkState, val path: String, val linked: Boolean)

    fun states(): List<State> = peers.values.map {
        State(it.id, it.state, it.path, it.state == LinkState.Connected)
    }

    fun linkFor(id: String): Link? = peers[id]?.link
    fun liveLinks(): List<Pair<String, Link>> = peers.values
        .filter { it.state == LinkState.Connected }
        .map { it.id to it.link }

    /** Always called on the Android main thread after the trust registry changes. */
    fun reconcile(primaryId: String?) {
        if (closed) return
        val allowed = PairedDesktops.list(context.filesDir).map { it.id }
            .filter { it != primaryId }.take(PairedDesktops.MAX_PEERS - 1).toSet()
        for ((id, session) in peers.toMap()) {
            if (id !in allowed) {
                peers.remove(id)
                session.close()
            }
        }
        for (id in allowed) {
            if (!peers.containsKey(id)) {
                peers[id] = Peer(id)
                if (Settings.linkWanted && networkAvailable()) peers[id]?.redial()
            }
        }
        onChange()
    }

    fun reconnectAll() {
        if (closed || !Settings.linkWanted || !networkAvailable()) return
        peers.values.forEach { peer ->
            if (!peer.link.hasOpenSocket) peer.redial()
        }
    }

    fun onDefaultChanged(changed: Boolean, isVpn: Boolean, newNetwork: Network?) {
        if (closed || !Settings.linkWanted) return
        for (peer in peers.values) {
            if (shouldReconnectForDefaultNetworkChange(
                    networkChanged = changed,
                    hasOpenSocket = peer.link.hasOpenSocket,
                    usingPhysicalLan = peer.link.physicallyBoundNetwork != null,
                    boundToNewDefault = peer.link.physicallyBoundNetwork == newNetwork,
                    newDefaultIsVpn = isVpn,
                )) {
                peer.link.disconnect()
            } else if (!peer.link.hasOpenSocket && networkAvailable()) {
                peer.redial()
            }
        }
    }

    fun onPhysicalLanReady(network: Network, addressChanged: Boolean) {
        if (closed || !Settings.linkWanted) return
        for (peer in peers.values) {
            if (addressChanged && peer.link.isBoundTo(network)) {
                peer.link.disconnect()
            } else if (peer.state == LinkState.Connected && peer.path == "Relay") {
                peer.discover(network)
            } else if (peer.state != LinkState.Connected && !peer.link.hasOpenSocket) {
                peer.redial(network)
            }
        }
    }

    fun onPhysicalLanLost(network: Network) {
        if (closed) return
        for (peer in peers.values) {
            if (peer.link.isBoundTo(network) || peer.path == "Relay" &&
                peer.link.hasOpenSocket && peer.networkClass in
                    setOf("wifi", "ethernet", "vpn-wifi", "vpn-ethernet")
            ) peer.link.disconnect()
        }
    }

    fun stopSockets() {
        peers.values.forEach { it.link.disconnect() }
    }

    fun cancelTransfer(id: String) {
        peers.values.forEach { it.link.cancelTransfer(id) }
    }

    fun close() {
        closed = true
        peers.values.forEach { it.close() }
        peers.clear()
        onChange()
    }

    private inner class Peer(val id: String) {
        var state: LinkState = LinkState.Idle
        var path = ""
        var networkClass = "other"
        var retryMs = 5_000L
        var pendingHandoff: Pair<InetSocketAddress, Network?>? = null
        private val candidates = ArrayDeque<RelayEndpoint>()
        private val retry = Runnable { redial() }

        val link: Link by lazy { Link(
            privateKey, localId, localName,
            object : Link.Events {
                override fun authorizePeer(
                    deviceId: String, viaRelay: Boolean, remotePairing: Boolean,
                ): Boolean = deviceId == id

                override fun pairingRequested(): Boolean = false

                override fun onPeer(deviceId: String) {
                    // The exact stored public-key hash was checked by authorizePeer.
                    // A secondary session must never change the main transport pointer.
                    if (deviceId != id) Log.w(EXTRA_TAG, "unexpected peer identity")
                }

                override fun onPeerName(name: String) {
                    PairedDesktops.rename(context.filesDir, id, name)
                    main.post { if (!closed && peers[id] === this@Peer) onChange() }
                }

                override fun onPeerForgotten() {
                    main.post {
                        if (!closed && peers[id] === this@Peer) onForgotten(id)
                    }
                }

                override fun onState(next: LinkState, fingerprint: String?) {
                    val generation = link.sessionGeneration
                    main.post {
                        if (closed || peers[id] !== this@Peer ||
                            generation != link.sessionGeneration) return@post
                        state = next
                        if (next == LinkState.Connected) {
                            retryMs = 5_000L
                            main.removeCallbacks(retry)
                            if (path == "Relay") lanNetwork()?.let(::discover)
                        } else if (next == LinkState.Idle && Settings.linkWanted && networkAvailable()) {
                            // A lost LAN path falls through to Relay immediately; a failed
                            // relay candidate tries the next endpoint without a discovery loop.
                            if (path == "LAN") tryRelay()
                            else if (candidates.isNotEmpty()) dialNextRelay()
                            else scheduleRetry()
                        }
                        onChange()
                    }
                }

                override fun onText(text: String) = onIncomingText(id, text)
                override fun onImage(png: ByteArray, photo: Boolean, screenshot: Boolean) =
                    onIncomingImage(id, png, photo, screenshot)
                override fun onTransfer(transfer: FileTransfer) = onTransferEvent(transfer)
                override fun onSendRejected(reason: String) = onError(reason)
                override fun onTransfersIdle() {
                    main.post {
                        pendingHandoff?.let { target ->
                            pendingHandoff = null
                            handoff(target.first, target.second)
                        }
                    }
                }
                override fun onNotificationAction(action: NotifAction) = onAction(action)
                override fun onWallpaperHash(hash: ByteArray) = onWallpaper(link, hash)
                override fun onSessionLost() = Unit
            },
            openIncomingFile = { offer -> Files.Incoming.begin(context.contentResolver, offer) },
        ) }

        val discovery: Discovery by lazy { Discovery(context,
            onFound = { address ->
                val network = discovery.resolvedNetwork
                main.post {
                    if (closed || peers[id] !== this@Peer) return@post
                    if (state == LinkState.Connected) {
                        if (path == "Relay") handoff(address, network)
                    } else {
                        path = "LAN"
                        candidates.clear()
                        link.disconnect()
                        link.connect(address, network)
                    }
                    onChange()
                }
            },
            onEmpty = {
                main.post {
                    if (closed || peers[id] !== this@Peer) return@post
                    if (state != LinkState.Connected && !link.hasOpenSocket) tryRelay()
                }
            },
            expectedPeerId = { id },
        ) }

        fun discover(network: Network) {
            if (!closed && Settings.linkWanted) discovery.burst(network)
        }

        fun redial(preferredLan: Network? = null) {
            if (closed || peers[id] !== this || !Settings.linkWanted || !networkAvailable()) return
            if (link.hasOpenSocket) return
            main.removeCallbacks(retry)
            val lan = preferredLan ?: lanNetwork()
            if (lan == null) tryRelay() else discover(lan)
        }

        private fun tryRelay() {
            if (closed || !Settings.linkWanted || !networkAvailable()) return
            networkClass = if (lanNetwork() != null) "wifi" else "cellular"
            candidates.clear()
            candidates.addAll(relayEndpoints().take(8))
            dialNextRelay()
        }

        private fun dialNextRelay() {
            val endpoint = candidates.removeFirstOrNull()
            if (endpoint == null) {
                scheduleRetry()
                return
            }
            path = "Relay"
            link.connectVia(
                InetSocketAddress.createUnresolved(endpoint.host, endpoint.port),
                id, endpoint.fallbackIpv4,
            )
        }

        private fun handoff(address: InetSocketAddress, network: Network?) {
            if (link.hasActiveFileTransfers) {
                pendingHandoff = address to network
                return
            }
            if (!link.disconnectIfTransfersIdle()) {
                pendingHandoff = address to network
                return
            }
            path = "LAN"
            candidates.clear()
            link.connect(address, network)
        }

        private fun scheduleRetry() {
            if (closed || !Settings.linkWanted || !networkAvailable()) return
            main.removeCallbacks(retry)
            main.postDelayed(retry, retryMs)
            retryMs = (retryMs * 2).coerceAtMost(300_000L)
        }

        fun close() {
            main.removeCallbacks(retry)
            discovery.stop()
            link.close()
        }
    }
}
