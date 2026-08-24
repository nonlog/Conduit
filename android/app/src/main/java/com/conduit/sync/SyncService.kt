package com.conduit.sync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.util.Log
import java.net.InetSocketAddress

private const val TAG = "conduit.svc"
private const val CHANNEL = "link"
private const val NOTIFICATION_ID = 1

/** Only used by the intent-driven path; mDNS carries the port otherwise. */
private const val PORT = 41112

/** Ours, in Tokyo. Must match `RELAY` in the daemon's `main.rs`. */
private const val RELAY_HOST = "tyo.414222.xyz"
private const val RELAY_PORT = 41113

/** Matches the desktop's ceiling; a longer clip is skipped, never truncated. */
private const val MAX_TEXT = 64_000

/**
 * The long-running half of the app.
 *
 * `connectedDevice`, not `dataSync`: Android 15 caps a `dataSync` foreground service at
 * six hours a day, which for a companion link means it dies every afternoon.
 *
 * Everything here is edge-triggered. The clipboard arrives through
 * `OnPrimaryClipChangedListener`, the network through a [ConnectivityManager] callback,
 * the peer's address through an mDNS burst, and the peer's frames through [Link]'s
 * reader thread. There is no timer in this file and nothing to poll.
 */
class SyncService : Service() {

    companion object {
        /**
         * The live link, borrowed by [NotificationRelay]. The system binds and unbinds
         * that service on its own schedule, so it cannot own a transport; same process,
         * so this is a plain reference rather than IPC. Null means nothing is connected
         * and a notification is dropped, which is the correct outcome.
         */
        @Volatile
        var activeLink: Link? = null
            private set
    }

    private lateinit var link: Link
    private lateinit var discovery: Discovery
    private lateinit var clipboard: ClipboardManager
    private lateinit var connectivity: ConnectivityManager
    private val main = Handler(Looper.getMainLooper())

    /**
     * Last text seen in either direction, LF-normalised. A change equal to this is our
     * own write coming back, and dropping it is the whole of ping-pong prevention.
     */
    @Volatile private var lastText = ""

    private val clipListener = ClipboardManager.OnPrimaryClipChangedListener { onLocalClip() }

    /** Set from the network callback, so a re-dial never has to ask the platform. */
    @Volatile private var networkUp = false

    /** The desktop we are paired with, so the relay rendezvous survives a restart. */
    @Volatile private var knownPeer: String? = null

    /**
     * The *default* network only. A Wi-Fi to cellular handover is then one
     * [onAvailable] for the network that replaced it, rather than a pair of events about
     * two networks that both look current — which is what a transport-filtered request
     * gives you, and why the single `networkUp` flag would have been wrong. Cellular is
     * included now that there is a relay for it to reach.
     */
    private val network = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(net: Network) {
            networkUp = true
            redial(net)
        }

        override fun onLost(net: Network) {
            // Suspend, not destroy: the socket is dead but [Link] and its sender thread
            // survive, so a reconnect reuses them instead of allocating a fresh set.
            Log.i(TAG, "network gone, suspending session")
            networkUp = false
            link.disconnect()
        }
    }

    override fun onCreate() {
        super.onCreate()
        val identity = Identity.loadOrCreate(filesDir)
        LinkStatus.fingerprint = Identity.fingerprint(identity.public)
        Log.i(TAG, "identity ${Identity.deviceId(identity.public)}")

        clipboard = getSystemService(ClipboardManager::class.java)
        connectivity = getSystemService(ConnectivityManager::class.java)
        knownPeer = Identity.peer(filesDir)

        link = Link(
            identity.private,
            object : Link.Events {
                override fun onState(state: LinkState, peer: String?) {
                    LinkStatus.state = state
                    LinkStatus.peer = peer
                }

                override fun onText(text: String) = onRemoteText(text)

                override fun onPeer(deviceId: String) = rememberPeer(deviceId)

                override fun onSessionLost() {
                    // Edge-triggered reconnect: one attempt per lost session, and only
                    // while a network is present. A desktop that stays down costs one
                    // 8-second burst, not a retry timer.
                    if (networkUp) redial()
                }
            },
        )
        discovery = Discovery(
            this,
            onFound = { address -> link.connect(address) },
            onEmpty = { dialRelay() },
        )
        activeLink = link

        clipboard.addPrimaryClipChangedListener(clipListener)
        // The default network, not a transport-filtered set: see [network].
        connectivity.registerDefaultNetworkCallback(network)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForeground(
            NOTIFICATION_ID,
            notification(),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
        )
        // A literal address on the intent skips discovery, which is the only way to drive
        // the link when the phone and the desktop are not on one subnet:
        //   adb reverse tcp:41112 tcp:41112
        //   adb shell am start-foreground-service -n com.conduit.sync/.SyncService \
        //       --es host 127.0.0.1
        val host = intent?.getStringExtra("host")
        if (host != null) {
            link.connect(InetSocketAddress(host, intent.getIntExtra("port", PORT)))
        } else {
            // The network callback only fires on a change, so the first dial is ours.
            redial()
        }
        return START_STICKY
    }

    override fun onDestroy() {
        // Cleared first, so the notification relay stops handing frames to a link that
        // is being torn down.
        activeLink = null
        clipboard.removePrimaryClipChangedListener(clipListener)
        connectivity.unregisterNetworkCallback(network)
        discovery.stop()
        link.close()
        LinkStatus.state = LinkState.Idle
        LinkStatus.peer = null
        super.onDestroy()
    }

    override fun onBind(intent: Intent): IBinder? = null

    /**
     * One dial attempt, routed by what the current network can actually reach.
     *
     * On Wi-Fi or Ethernet the desktop is probably on this subnet, so an mDNS burst goes
     * first and [Discovery]'s empty-burst callback falls through to the relay — that is
     * the foreign-Wi-Fi case. On cellular mDNS cannot work at all, so it is skipped
     * rather than run and waited out: eight seconds of multicast on a mobile network is
     * eight seconds of radio for a guaranteed miss.
     */
    private fun redial(net: Network? = null) {
        val caps = connectivity.getNetworkCapabilities(net ?: connectivity.activeNetwork)
        val lan = caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true ||
            caps?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true
        if (lan) {
            Log.i(TAG, "network up on a LAN, bursting")
            discovery.burst()
        } else {
            Log.i(TAG, "network up off-LAN, going straight to the relay")
            dialRelay()
        }
    }

    /**
     * Parks at the relay under the desktop's device id.
     *
     * Unresolved on purpose: DNS blocks, and [Link]'s reader thread is the one thread
     * here allowed to block. Without a remembered peer there is no rendezvous to present,
     * which is why pairing has to happen once on a LAN.
     */
    private fun dialRelay() {
        val peer = knownPeer
        if (peer == null) {
            Log.i(TAG, "no paired desktop yet, so no relay rendezvous; pair on a LAN first")
            return
        }
        link.connectVia(InetSocketAddress.createUnresolved(RELAY_HOST, RELAY_PORT), peer)
    }

    /** First handshake with a given desktop is the only one that writes. */
    private fun rememberPeer(deviceId: String) {
        if (deviceId == knownPeer) return
        knownPeer = deviceId
        runCatching { Identity.rememberPeer(filesDir, deviceId) }
            .onSuccess { Log.i(TAG, "paired with $deviceId, relay rendezvous stored") }
            .onFailure { Log.w(TAG, "could not store the peer id; relay stays unavailable", it) }
    }

    private fun onLocalClip() {
        val item = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)
        if (item == null) {
            // Expected on stock Android 10+: a background app is not allowed to read the
            // clipboard. The LSPosed hook on ClipboardService is what lifts this.
            Log.d(TAG, "clipboard unreadable from the background")
            return
        }
        val text = item.text?.toString()?.replace("\r\n", "\n") ?: return
        if (text.isEmpty()) return
        if (text == lastText) {
            Log.d(TAG, "echo of our own write, dropped")
            return
        }
        if (text.length > MAX_TEXT) {
            Log.w(TAG, "clip of ${text.length} chars is too large for one frame, skipped")
            return
        }
        lastText = text
        link.sendText(text)
    }

    private fun onRemoteText(text: String) {
        val normalised = text.replace("\r\n", "\n")
        if (normalised == lastText) return
        Log.i(TAG, "clip text in: ${normalised.length} chars")
        // Recorded before the write, because the listener fires on the main thread and
        // may observe the change before setPrimaryClip has returned here.
        lastText = normalised
        main.post {
            clipboard.setPrimaryClip(ClipData.newPlainText("conduit", normalised))
        }
    }

    private fun notification(): Notification {
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            // LOW, so an always-present notification never makes a sound.
            NotificationChannel(CHANNEL, "Link", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shown while the desktop link is running"
            },
        )
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_link)
            .setContentTitle("conduit")
            .setContentText("Clipboard linked to the desktop")
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }
}
