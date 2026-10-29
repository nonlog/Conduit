package com.conduit.sync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.app.WallpaperManager
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.os.Build
import android.util.Log
import com.conduit.sync.proto.NotifAction
import java.net.InetSocketAddress
import java.util.ArrayDeque

private const val TAG = "conduit.svc"
private const val LINK_CHANNEL = "link"
private const val TRANSFER_CHANNEL = "transfers"
private const val LINK_NOTIFICATION_ID = 1
private const val UPLOAD_NOTIFICATION_ID = 2
private const val DOWNLOAD_NOTIFICATION_ID = 3

/** Only used by the intent-driven path; mDNS carries the port otherwise. */
private const val PORT = 41112

/** Matches the desktop's ceiling; a longer clip is skipped, never truncated. */
private const val MAX_TEXT = 64_000

/**
 * First retry after a link goes down. Short, because the overwhelmingly common cause is a
 * desktop that is a second away from being ready — it just booted, or the daemon restarted.
 */
private const val RETRY_MIN_MS = 5_000L

/**
 * The backoff ceiling: twelve attempts an hour once a desktop has been down a while.
 *
 * Affordable because of what schedules it. [Handler.postDelayed] is measured on
 * `uptimeMillis`, which excludes deep sleep, and it acquires no wake lock — so a phone in
 * deep sleep does not advance toward the next attempt and nothing here can wake the device
 * or its radio. The retries land while the phone is awake anyway, which is what makes an
 * automatic reconnect compatible with an idle cost of zero. `AlarmManager` with a WAKEUP
 * type would have been the version of this feature that drains the battery.
 */
private const val RETRY_MAX_MS = 300_000L
/**
 * A link that was genuinely healthy seconds ago deserves faster repair than a desktop that has
 * simply been offline for hours. During a bounded recovery episode, cap exponential backoff at one
 * minute. If the path stays broken beyond the episode we automatically return to [RETRY_MAX_MS],
 * preserving the low-radio-wakeup steady state.
 */
private const val RECOVERY_RETRY_MAX_MS = 60_000L
private const val RECOVERY_WINDOW_MS = 10L * 60L * 1000L
/**
 * Keep an already-foreground link continuously foreground across a short reconnect/Relay
 * rendezvous. Dropping foreground for even a second here makes some OEM freezers classify the
 * process as background before Noise finishes, which can kill the brand-new session. If the peer
 * really is offline, this one-shot grace expires and the notification is removed as requested.
 */
private const val TRANSIENT_RECONNECT_FOREGROUND_GRACE_MS = 15_000L
/**
 * Some OEM notification services apply stopForeground asynchronously. A second, guarded cancel
 * after that state transition removes an otherwise stranded FGS notification without polling.
 */
private const val DETACHED_LINK_NOTIFICATION_CANCEL_DELAY_MS = 500L
private const val RELAY_FAILOVER_DELAY_MS = 150L
private const val UNSTABLE_RELAY_SESSION_MS = 60_000L
/** Bound, event-driven window for upgrading an already-linked Relay session to direct LAN. */
private const val RELAY_LAN_HANDOFF_DISCOVERY_MS = 30_000L

internal fun retryCeilingMs(nowUptimeMs: Long, recoveryUntilUptimeMs: Long): Long =
    if (nowUptimeMs < recoveryUntilUptimeMs) RECOVERY_RETRY_MAX_MS else RETRY_MAX_MS

internal fun shouldHoldForegroundForTransientReconnect(
    foregroundVisible: Boolean,
    pairing: Boolean,
    state: LinkState,
): Boolean =
    foregroundVisible &&
        !pairing &&
        (state == LinkState.Waiting || state == LinkState.Retrying)

internal fun shouldCancelDetachedLinkNotification(
    foregroundVisible: Boolean,
    pairing: Boolean,
    state: LinkState,
): Boolean = !foregroundVisible && !pairing && state != LinkState.Connected

/** Once pairing succeeds, the route chip describes the live transport instead of pairing state. */
internal fun completedPairingPath(viaRelay: Boolean, relayId: String?): String =
    if (!viaRelay) {
        "LAN"
    } else {
        relayId?.trim()?.takeIf(String::isNotEmpty)?.let { "Relay · ${it.uppercase()}" } ?: "Relay"
    }

/** One bounded Relay-to-LAN handoff window per Relay session / physical-LAN arrival. */
internal fun shouldRecheckLanAfterRelay(
    connectedViaRelay: Boolean,
    networkHasLan: Boolean,
    alreadyChecked: Boolean,
): Boolean = connectedViaRelay && networkHasLan && !alreadyChecked

/**
 * A new system default can strand a socket on the old route, even before Noise
 * completes (Relay park has no read deadline). Closing the old socket lets
 * the existing Idle handler immediately rediscover LAN or dial Relay. A direct LAN
 * socket is unaffected when a VPN becomes default above that same physical link.
 */
internal fun shouldReconnectForDefaultNetworkChange(
    networkChanged: Boolean,
    hasOpenSocket: Boolean,
    usingPhysicalLan: Boolean,
    boundToNewDefault: Boolean,
    newDefaultIsVpn: Boolean,
): Boolean = networkChanged && hasOpenSocket &&
    (!usingPhysicalLan || (!boundToNewDefault && !newDefaultIsVpn))

/** An established Relay socket using Wi-Fi/Ethernet can also be stranded when a
 * VPN remains the default while its underlying physical network disappears. */
internal fun shouldReconnectRelayForPhysicalLanLoss(
    lostKnownLan: Boolean,
    relaySocketActive: Boolean,
    relayNetworkClass: String,
): Boolean = lostKnownLan && relaySocketActive &&
    relayNetworkClass in setOf("wifi", "ethernet", "vpn-wifi", "vpn-ethernet")

/** Network handles may stay the same while DHCP assigns a new address. A socket
 * bound to that interface must be recreated rather than waiting for read timeout. */
internal fun shouldReconnectBoundLanAfterAddressChange(
    previousAddresses: String?,
    newAddresses: String,
    boundToNetwork: Boolean,
): Boolean = previousAddresses != null && previousAddresses != newAddresses && boundToNetwork

/** The previous reader may post Idle just before a replacement LAN dial begins.
 * Its delayed retry work must not affect the new socket. */
internal fun isCurrentLinkGeneration(eventGeneration: Long, latestGeneration: Long): Boolean =
    eventGeneration == latestGeneration

/**
 * A real mDNS advertisement is only a LAN candidate, not proof that the direct TCP/Noise
 * handshake works. If it fails, choose Relay immediately rather than rediscovering the same
 * unreachable candidate indefinitely; no new background polling is introduced.
 */
internal fun shouldFallbackToRelayAfterLanFailure(
    wasLinked: Boolean,
    route: String?,
    relayAttemptActive: Boolean,
    networkUp: Boolean,
    linkWanted: Boolean,
    hasPairedPeer: Boolean,
): Boolean = !wasLinked && route == "LAN" && !relayAttemptActive &&
    networkUp && linkWanted && hasPairedPeer

/**
 * A link that was genuinely established gets one immediate repair attempt before we demote the
 * service and enter notification-free offline backoff. This keeps a screen-off transient from
 * turning into a long outage while still removing the notification when the desktop is really gone.
 */
internal fun shouldRecoverEstablishedLinkImmediately(
    wasLinked: Boolean,
    networkUp: Boolean,
    linkWanted: Boolean,
    pairingActive: Boolean,
): Boolean = wasLinked && networkUp && linkWanted && !pairingActive

/** Sent by the UI. A disconnect has to be remembered, or START_STICKY undoes the user's tap. */
const val ACTION_CONNECT = "com.conduit.sync.CONNECT"
const val ACTION_BOOT_CONNECT = "com.conduit.sync.BOOT_CONNECT"
const val ACTION_DISCONNECT = "com.conduit.sync.DISCONNECT"
const val ACTION_PAIR = "com.conduit.sync.PAIR"
const val ACTION_CANCEL_PAIR = "com.conduit.sync.CANCEL_PAIR"
const val ACTION_FORGET = "com.conduit.sync.FORGET"
const val EXTRA_PAIRING_CODE = "pairing_code"

private const val PAIRING_WINDOW_MS = 120_000L
private const val PAIRING_RETRY_MS = 2_000L

/** Sent by [ShareActivity]: URIs in the intent's ClipData, or text in EXTRA_TEXT. */
const val ACTION_SHARE = "com.conduit.sync.SHARE"
const val ACTION_CANCEL_TRANSFER = "com.conduit.sync.CANCEL_TRANSFER"
const val EXTRA_TRANSFER_ID = "transfer_id"

internal fun serviceStartMayRedial(action: String?, connected: Boolean): Boolean =
    !connected && action != ACTION_SHARE && action != ACTION_CANCEL_TRANSFER && action != ACTION_ACCESSIBILITY_CLIP

/** A focused accessibility handoff carrying the exact ClipData Android just allowed us to read. */
const val ACTION_ACCESSIBILITY_CLIP = "com.conduit.sync.ACCESSIBILITY_CLIP"

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
    private lateinit var photos: Photos
    private lateinit var screenshots: Screenshots
    private lateinit var clipboard: ClipboardManager
    private lateinit var connectivity: ConnectivityManager
    private lateinit var wallpaperManager: WallpaperManager
    private lateinit var relayQuality: RelayQualityStore
    private lateinit var localDeviceId: String
    private lateinit var localDeviceName: String
    private var relayEndpoints: List<RelayEndpoint> = emptyList()
    private val main = Handler(Looper.getMainLooper())
    private var foregroundVisible = false
    /** Clipboard/capture/wallpaper observers are useful only while a Noise session is linked. */
    private var syncObserversActive = false
    private data class LanTarget(val address: InetSocketAddress, val network: Network?)
    private var deferredLanHandoff: LanTarget? = null
    private var defaultNetwork: Network? = null
    private val readyLanAddresses = mutableMapOf<Network, String>()

    /**
     * Relay selection state. Only [relayCandidates] is main-thread-owned. The attempt metadata is
     * volatile because Link callbacks arrive on its reader/sender threads. Nothing here has a
     * timer: a plan is created only when a real reconnect is already happening.
     */
    private val relayCandidates = ArrayDeque<RelayEndpoint>()
    @Volatile private var relayAttempt: RelayEndpoint? = null
    @Volatile private var relayAttemptConnected = false
    @Volatile private var relayConnectedAtMs = 0L
    @Volatile private var relayAttemptStartedAtMs = 0L
    @Volatile private var relayNetworkClass = "other"

    /**
     * Last text seen in either direction, LF-normalised. A change equal to this is our
     * own write coming back, and dropping it is the whole of ping-pong prevention.
     */
    @Volatile private var lastText = ""

    private val clipListener = ClipboardManager.OnPrimaryClipChangedListener { onLocalClip() }

    /** Set from the network callback, so a re-dial never has to ask the platform. */
    @Volatile private var networkUp = false

    /** Guards duplicate callbacks while one bounded Relay-to-LAN discovery window is active/done. */
    @Volatile private var relayLanRecheckDone = false

    /** The desktop we are paired with, so the relay rendezvous survives a restart. */
    @Volatile private var knownPeer: String? = null

    /** Its name, so republishing the share-sheet shortcut happens on a rename and not per session. */
    @Volatile private var knownPeerName: String? = null

    /** Explicit, user-opened LAN pairing window. Uptime excludes deep sleep and never wakes it. */
    @Volatile private var pairingUntilUptimeMs = 0L
    @Volatile private var pairingRendezvous: String? = null
    private val pairingTimeout = Runnable {
        if (pairingUntilUptimeMs == 0L) return@Runnable
        pairingUntilUptimeMs = 0L
        pairingRendezvous = null
        LinkStatus.pairing = false
        Log.i(TAG, "pairing window expired")
        if (knownPeer != null && Settings.linkWanted && networkUp) {
            hideLinkNotification()
            redial()
        } else if (knownPeer == null) {
            Settings.linkWanted = false
            LinkStatus.state = LinkState.Idle
            LinkStatus.path = null
            hideLinkNotification()
            stopSelf()
        }
    }

    /** Retry is scoped to the explicit pairing window only. A failed/no-result mDNS burst schedules
     * one later burst; nothing here exists during normal linked/offline operation. */
    private val pairingRetry = Runnable {
        if (!destroyed && pairingActive() && networkUp) retryPairing()
    }

    /** Set in [onDestroy] so a retry already in flight cannot touch a closed [Link]. */
    @Volatile private var destroyed = false

    /**
     * Retry state, touched only on the main thread — every mutation is posted there, so the
     * backoff needs no lock even though the events that drive it arrive on [Link]'s reader
     * thread.
     */
    private var retryMs = RETRY_MIN_MS
    /**
     * `uptimeMillis` matches Handler's delayed-callback clock and excludes deep sleep. Recovery
     * therefore cannot keep the phone or radio awake merely because wall-clock time is passing.
     */
    private var recoveryUntilUptimeMs = 0L
    private var offlineDemoteScheduled = false
    private val offlineDemote = Runnable {
        offlineDemoteScheduled = false
        if (destroyed || !Settings.linkWanted || pairingActive() || LinkStatus.state == LinkState.Connected) {
            return@Runnable
        }
        Log.i(TAG, "transient reconnect grace expired; entering notification-free offline mode")
        hideLinkNotification()
    }
    private val detachedLinkNotificationCancel = Runnable {
        if (!shouldCancelDetachedLinkNotification(
                foregroundVisible = foregroundVisible,
                pairing = pairingActive(),
                state = LinkStatus.state,
            )
        ) {
            return@Runnable
        }
        Log.d(TAG, "clearing detached link notification after foreground demotion")
        getSystemService(NotificationManager::class.java).cancel(LINK_NOTIFICATION_ID)
    }
    private val retry = Runnable {
        if (destroyed || !Settings.linkWanted || pairingActive()) return@Runnable
        Log.i(TAG, "retrying the link")
        redial()
    }

    /** Wallpaper updates are pushed by Android itself; this one delayed refresh is not polling. */
    private val wallpaperRefresh = Runnable {
        if (destroyed || LinkStatus.state != LinkState.Connected) return@Runnable
        link.sendWallpaperPreview(ByteArray(0)) { Wallpapers.preview(this, force = true) }
    }
    private val wallpaperListener = WallpaperManager.OnColorsChangedListener { _, which ->
        if ((which and WallpaperManager.FLAG_SYSTEM) != 0) {
            Wallpapers.invalidate()
            main.removeCallbacks(wallpaperRefresh)
            // OEM wallpaper services often update the colors callback a few milliseconds before the
            // rendered wallpaper file. Coalesce that one edge instead of polling for file stability.
            main.postDelayed(wallpaperRefresh, 400L)
        }
    }

    /**
     * The *default* network only. A Wi-Fi to cellular handover is then one
     * [onAvailable] for the network that replaced it, rather than a pair of events about
     * two networks that both look current — which is what a transport-filtered request
     * gives you, and why the single `networkUp` flag would have been wrong. Cellular is
     * included now that there is a relay for it to reach.
     */
    private val network = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(net: Network) {
            main.post {
                if (destroyed) return@post
                val previousDefault = defaultNetwork
                defaultNetwork = net
                networkUp = true
                cancelRetry()
                // onAvailable can repeat the current default. Only a new default
                // invalidates a Relay socket (including its indefinitely parked waiter).
                // An independent LAN socket survives a VPN becoming default above it.
                val lan = link.physicallyBoundNetwork
                val caps = connectivity.getNetworkCapabilities(net)
                if (Settings.linkWanted && shouldReconnectForDefaultNetworkChange(
                        networkChanged = previousDefault != null && previousDefault != net,
                        hasOpenSocket = link.hasOpenSocket,
                        usingPhysicalLan = lan != null,
                        boundToNewDefault = lan == net,
                        newDefaultIsVpn = caps?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true,
                    )) {
                    Log.i(TAG, "default network changed $previousDefault -> $net; reconnecting on the new route")
                    relayLanRecheckDone = false
                    deferredLanHandoff = null
                    discovery.stop()
                    link.disconnect()
                } else if (LinkStatus.state == LinkState.Connected) {
                    maybeRecheckLanAfterRelay(net)
                } else if (pairingActive()) retryPairing(net) else redial(net)
            }
        }

        override fun onLost(net: Network) {
            main.post {
                if (destroyed || defaultNetwork != net) return@post
                defaultNetwork = connectivity.activeNetwork
                networkUp = defaultNetwork != null || physicalLanNetwork() != null
                // The default can disappear before Android publishes its replacement.
                // A Relay socket is bound to the vanished system route even if some
                // alternate network is already up; otherwise it may look connected for
                // ten minutes. Keep an independently bound physical LAN stream intact.
                if (link.boundNetwork == null || link.isBoundTo(net) || !networkUp) {
                    Log.i(TAG, "default network lost: $net; closing obsolete session")
                    relayLanRecheckDone = false
                    main.removeCallbacks(retry)
                    link.disconnect()
                }
            }
        }
    }

    /**
     * Watches physical LAN arrival independently of Android's default network. A VPN can remain
     * the default while Wi-Fi comes and goes underneath it, so the default callback alone cannot
     * trigger a Relay-to-LAN upgrade. This callback is event-driven; it performs no scans itself.
     */
    private val physicalLanNetworks = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(net: Network) = physicalLanReady(net)

        override fun onLinkPropertiesChanged(net: Network, properties: LinkProperties) = physicalLanReady(net)

        override fun onCapabilitiesChanged(net: Network, capabilities: NetworkCapabilities) = physicalLanReady(net)

        override fun onLost(net: Network) {
            main.post {
                if (destroyed) return@post
                val wasKnownLan = readyLanAddresses.remove(net) != null
                relayLanRecheckDone = false
                if (deferredLanHandoff?.network == net) deferredLanHandoff = null
                networkUp = connectivity.activeNetwork != null || physicalLanNetwork() != null
                Log.i(TAG, "physical LAN lost: $net")
                if (link.isBoundTo(net) || shouldReconnectRelayForPhysicalLanLoss(
                        lostKnownLan = wasKnownLan,
                        relaySocketActive = relayAttempt != null && link.hasOpenSocket && link.boundNetwork == null,
                        relayNetworkClass = relayNetworkClass,
                    )) {
                    Log.i(TAG, "network $net owned the last active route; reconnecting")
                    link.disconnect()
                }
            }
        }
    }

    private fun physicalLanReady(net: Network) {
        main.post {
            if (destroyed || !isPhysicalLanNetwork(net)) return@post
            // onAvailable can precede DHCP/address delivery on hotspots. Reconsider only a
            // genuinely new usable address set; repeated capability callbacks add no probes.
            val addresses = connectivity.getLinkProperties(net)?.linkAddresses.orEmpty()
                .filter { it.address is java.net.Inet4Address && !it.address.isLoopbackAddress }
                .map { "${it.address.hostAddress}/${it.prefixLength}" }.sorted().joinToString(",")
            val previousAddresses = readyLanAddresses[net]
            if (addresses.isEmpty()) {
                if (previousAddresses != null) {
                    readyLanAddresses.remove(net)
                    if (shouldReconnectBoundLanAfterAddressChange(
                            previousAddresses, addresses, link.isBoundTo(net),
                        ) || shouldReconnectRelayForPhysicalLanLoss(
                            lostKnownLan = true,
                            relaySocketActive = relayAttempt != null && link.hasOpenSocket && link.boundNetwork == null,
                            relayNetworkClass = relayNetworkClass,
                        )) {
                        Log.i(TAG, "physical LAN lost its IPv4 address; reconnecting")
                        link.disconnect()
                    }
                }
                return@post
            }
            if (previousAddresses == addresses) return@post
            readyLanAddresses[net] = addresses
            networkUp = true
            relayLanRecheckDone = false
            Log.i(TAG, "physical LAN addressing ready: $net")
            if (shouldReconnectBoundLanAfterAddressChange(
                    previousAddresses, addresses, link.isBoundTo(net),
                )) {
                Log.i(TAG, "LAN address changed $previousAddresses -> $addresses; reconnecting")
                link.disconnect()
            } else if (LinkStatus.state == LinkState.Connected && relayAttemptConnected) {
                maybeRecheckLanAfterRelay(net)
            } else if (LinkStatus.state != LinkState.Connected) {
                if (pairingActive()) retryPairing(net) else redial(net)
            }
        }
    }

    override fun onCreate() {
        super.onCreate()
        val identity = Identity.loadOrCreate(filesDir)
        localDeviceId = Identity.deviceId(identity.public)
        LinkStatus.fingerprint = Identity.fingerprint(identity.public)
        Log.i(TAG, "identity $localDeviceId")

        clipboard = getSystemService(ClipboardManager::class.java)
        connectivity = getSystemService(ConnectivityManager::class.java)
        wallpaperManager = getSystemService(WallpaperManager::class.java)
        knownPeer = Identity.peer(filesDir)
        knownPeerName = Identity.peerName(filesDir)
        LinkStatus.pairedDeviceId = knownPeer
        LinkStatus.peerName = knownPeerName
        LinkStatus.pairing = false
        // Refresh the shortcut once per service process so an app update can change its icon even
        // when the paired desktop name did not change.
        knownPeerName?.let { ShareTarget.publish(this, it) }
        // Both stores, because this service can be the first component the system starts —
        // under START_STICKY it is started with no activity involved at all — and the
        // remembered disconnect it is about to read lives in one of them.
        Settings.load(this)
        History.load(this)
        relayEndpoints = RelayCatalog.load(filesDir)
        relayQuality = RelayQualityStore(filesDir)
        Log.i(TAG, "relay inventory: ${relayEndpoints.joinToString { it.id }}")
        localDeviceName = android.provider.Settings.Global
            .getString(contentResolver, "device_name")
            ?.trim()
            ?.takeIf(String::isNotEmpty)
            ?: Build.MODEL

        link = Link(
            identity.private,
            localDeviceId,
            localDeviceName,
            object : Link.Events {
                override fun onState(state: LinkState, peer: String?) {
                    val eventGeneration = link.sessionGeneration
                    val previousState = LinkStatus.state
                    LinkStatus.state = state
                    LinkStatus.peer = peer
                    val linked = state == LinkState.Connected
                    ClipboardAccessibilityService.setLinkActive(linked)
                    // NotificationListenerService is system-owned and cannot be unregistered cheaply,
                    // so make its hot path a single null check while the desktop is offline.
                    activeLink = if (linked) link else null
                    main.post {
                        if (!destroyed) setSyncObserversActive(linked)
                    }
                    LinkTileService.refresh(this@SyncService)
                    when (state) {
                        // A completed handshake is the only proof the path works, so it is
                        // the only thing that earns a reset back to the short interval.
                        LinkState.Connected -> {
                            relayAttempt?.let { endpoint ->
                                relayAttemptConnected = true
                                // A newly-established Relay session is a fresh opportunity to
                                // discover the same desktop directly on the physical LAN.
                                relayLanRecheckDone = false
                                relayConnectedAtMs = SystemClock.elapsedRealtime()
                                val sessionUpMs = (relayConnectedAtMs - relayAttemptStartedAtMs).coerceAtLeast(1L)
                                relayQuality.connected(
                                    relayNetworkClass,
                                    endpoint,
                                    System.currentTimeMillis(),
                                    sessionUpMs,
                                )
                            }
                            cancelRetry()
                            main.post {
                                cancelOfflineDemotion()
                                showLinkedNotification()
                                maybeRecheckLanAfterRelay()
                            }
                        }
                        // Every exit from the reader thread lands here, whether the
                        // handshake completed or not. That is the point: a dial refused by
                        // a desktop that is not listening notified nothing before, which is
                        // exactly why a phone that missed one burst stayed dark until the
                        // next network event.
                        LinkState.Idle -> {
                            val wasLinked = previousState == LinkState.Connected
                            val endpoint = relayAttempt
                            val wasConnected = relayAttemptConnected
                            val connectedFor = if (wasConnected) {
                                SystemClock.elapsedRealtime() - relayConnectedAtMs
                            } else {
                                0L
                            }
                            main.post {
                                if (!isCurrentLinkGeneration(eventGeneration, link.sessionGeneration)) {
                                    Log.d(TAG, "ignoring delayed Idle from superseded session $eventGeneration")
                                    return@post
                                }
                                if (pairingActive()) {
                                    showPairingNotification()
                                    if (pairingRendezvous != null && endpoint != null) {
                                        relayAttempt = null
                                        relayAttemptConnected = false
                                        if (relayCandidates.isNotEmpty()) {
                                            main.postDelayed({ dialNextRelay() }, RELAY_FAILOVER_DELAY_MS)
                                            return@post
                                        }
                                    }
                                    main.removeCallbacks(pairingRetry)
                                    main.postDelayed(pairingRetry, PAIRING_RETRY_MS)
                                    return@post
                                }
                                if (endpoint != null && networkUp && Settings.linkWanted) {
                                    if (!wasConnected) {
                                        relayQuality.dialFailed(
                                            relayNetworkClass,
                                            endpoint,
                                            System.currentTimeMillis(),
                                        )
                                        relayAttempt = null
                                        relayAttemptConnected = false
                                        if (relayCandidates.isNotEmpty()) {
                                            // Let the reader that reported Idle actually return before Link.dial
                                            // tests isAlive. This delay exists only inside an already-active
                                            // reconnect attempt; it never wakes an idle phone later.
                                            main.postDelayed({ dialNextRelay() }, RELAY_FAILOVER_DELAY_MS)
                                            return@post
                                        }
                                    } else {
                                        if (connectedFor in 1 until UNSTABLE_RELAY_SESSION_MS) {
                                            relayQuality.unstable(relayNetworkClass, endpoint)
                                        } else if (connectedFor >= UNSTABLE_RELAY_SESSION_MS) {
                                            // This was a proven-good path, not a handshake flap. For the next
                                            // few awake minutes recover promptly from a transient VPN/cellular
                                            // blackhole, then automatically fall back to the battery-saving
                                            // five-minute ceiling if the outage is genuinely long-lived.
                                            recoveryUntilUptimeMs =
                                                SystemClock.uptimeMillis() + RECOVERY_WINDOW_MS
                                            Log.i(
                                                TAG,
                                                "stable session lost; recovery retries capped at " +
                                                    "${RECOVERY_RETRY_MAX_MS / 1000}s",
                                            )
                                        }
                                    }
                                }
                                if (shouldFallbackToRelayAfterLanFailure(
                                        wasLinked = wasLinked,
                                        route = LinkStatus.path,
                                        relayAttemptActive = endpoint != null,
                                        networkUp = networkUp,
                                        linkWanted = Settings.linkWanted,
                                        hasPairedPeer = knownPeer != null,
                                    )
                                ) {
                                    // NSD can find an authentic on-link address whose socket is
                                    // forbidden by VPN policy. A second mDNS burst would find the
                                    // same unreachable desktop and starve a working Relay forever.
                                    Log.i(TAG, "direct LAN candidate failed; trying the paired Relay instead")
                                    clearRelayPlan()
                                    main.removeCallbacks(retry)
                                    dialRelay()
                                    return@post
                                }
                                if (shouldRecoverEstablishedLinkImmediately(
                                        wasLinked,
                                        networkUp,
                                        Settings.linkWanted,
                                        pairingActive(),
                                    )
                                ) {
                                    // Do not first demote and then wait five uptime seconds: in deep Doze that
                                    // Handler delay may not advance until the screen wakes. The reader teardown
                                    // itself is the event that gives us this one immediate, battery-free chance.
                                    clearRelayPlan()
                                    main.removeCallbacks(retry)
                                    retryMs = RETRY_MIN_MS
                                    showConnectingNotification()
                                    scheduleOfflineDemotion()
                                    Log.i(TAG, "established session lost; trying one immediate recovery")
                                    redial()
                                    return@post
                                }

                                // Either the immediate repair attempt failed, or this was never a proven
                                // session. This is the real Offline boundary: remove the link notification
                                // now and let the existing low-power backoff continue in the background.
                                showPairingOrHideOfflineNotification()
                                clearRelayPlan()
                                scheduleRetry()
                            }
                        }
                        LinkState.Discovering -> main.post {
                            if (pairingActive()) {
                                showPairingNotification()
                            } else if (foregroundVisible) {
                                // Keep the user-initiated connection attempt visible, but never
                                // resurrect a notification for a background retry after we demoted.
                                showConnectingNotification()
                            }
                        }
                        LinkState.Waiting, LinkState.Retrying -> main.post {
                            if (pairingActive()) {
                                showPairingNotification()
                            } else if (shouldHoldForegroundForTransientReconnect(
                                    foregroundVisible,
                                    pairing = false,
                                    state = state,
                                )
                            ) {
                                // Relay emits Waiting as soon as its socket is parked, before Noise has
                                // had a chance to complete. That is a transient rendezvous state, not proof
                                // that the desktop is offline. Keep the existing FGS continuous briefly.
                                showConnectingNotification()
                                scheduleOfflineDemotion()
                            } else {
                                hideLinkNotification()
                            }
                        }
                        LinkState.Pairing -> main.post { showPairingNotification() }
                    }
                }

                override fun onText(text: String) = onRemoteText(text)

                override fun onImage(png: ByteArray, photo: Boolean, screenshot: Boolean) =
                    onRemoteImage(png, photo, screenshot)

                override fun onTransfer(transfer: FileTransfer) {
                    main.post {
                        if (destroyed) return@post
                        FileTransfers.update(transfer)
                        if (FileTransfers.entries.any { it == transfer }) showTransferNotification(transfer)
                    }
                }

                override fun onSendRejected(reason: String) {
                    main.post {
                        if (!destroyed) android.widget.Toast.makeText(
                            this@SyncService, reason, android.widget.Toast.LENGTH_SHORT,
                        ).show()
                    }
                }

                override fun onTransfersIdle() {
                    main.post {
                        val target = deferredLanHandoff ?: return@post
                        deferredLanHandoff = null
                        if (!destroyed) beginLanHandoff(target)
                    }
                }

                override fun onBulkTransfer(bytes: Long, elapsedMs: Long) {
                    val endpoint = relayAttempt
                    if (endpoint != null && relayAttemptConnected) {
                        relayQuality.goodput(relayNetworkClass, endpoint, bytes, elapsedMs)
                    }
                }

                override fun onNotificationAction(action: NotifAction) {
                    // NotificationListenerService owns the PendingIntent capabilities. Resolve
                    // them there on the main thread only when a real Windows click arrives.
                    main.post { NotificationRelay.perform(action) }
                }

                override fun authorizePeer(
                    deviceId: String,
                    viaRelay: Boolean,
                    remotePairing: Boolean,
                ): Boolean {
                    if (pairingActive()) {
                        // Pairing/replacement is mutual. With a human pairing code, the temporary
                        // rendezvous deliberately uses Relay; without one, mDNS/LAN is used. The
                        // ordinary remembered-peer Relay cannot consume a Pair-new window because
                        // the service never dials that rendezvous while pairing is active.
                        val codeMode = pairingRendezvous != null
                        return remotePairing && if (codeMode) viaRelay else !viaRelay
                    }
                    return deviceId == knownPeer
                }

                override fun pairingRequested(): Boolean = pairingActive()

                override fun onPeerForgotten() {
                    main.post { forgetPeer(remote = true) }
                }

                override fun onPeer(deviceId: String) = rememberPeer(deviceId)

                override fun onPeerName(name: String) = rememberPeerName(name)

                override fun onWallpaperHash(hash: ByteArray) {
                    link.sendWallpaperPreview(hash) { Wallpapers.preview(this@SyncService) }
                }

                override fun onSessionLost() {
                    // Recovery policy lives in the preceding Idle state transition. Keeping this
                    // callback observational prevents a second dial from racing that one.
                    Log.i(TAG, "session lost")
                }
            },
            openIncomingFile = { offer -> Files.Incoming.begin(contentResolver, offer) },
        )
        discovery = Discovery(
            this,
            onFound = { address ->
                val target = LanTarget(address, discovery.resolvedNetwork)
                main.post {
                    if (destroyed || !Settings.linkWanted) return@post
                    if (target.network?.let(::isPhysicalLanNetwork) == false) return@post
                    if (LinkStatus.state == LinkState.Connected && relayAttemptConnected) {
                        beginLanHandoff(target)
                    } else if (LinkStatus.state != LinkState.Connected) {
                        LinkStatus.path = "LAN"
                        clearRelayPlan()
                        link.connect(target.address, target.network)
                    }
                }
            },
            onEmpty = {
                if (LinkStatus.state == LinkState.Connected && relayAttemptConnected) {
                    Log.i(TAG, "post-relay LAN recheck found nothing; keeping Relay")
                } else if (pairingActive()) {
                    LinkStatus.state = LinkState.Pairing
                    LinkStatus.path = "LAN · No desktop found"
                    showPairingNotification()
                    if (pairingRendezvous == null) {
                        main.removeCallbacks(pairingRetry)
                        main.postDelayed(pairingRetry, PAIRING_RETRY_MS)
                    }
                } else {
                    dialRelay()
                }
            },
            expectedPeerId = { if (pairingActive()) null else knownPeer },
        )
        // These observers are intentionally constructed now but registered only after Noise is up.
        // A parked Relay socket therefore does not make clipboard, MediaStore or wallpaper changes
        // wake Conduit while the desktop is offline.
        photos = Photos(this, link)
        screenshots = Screenshots(this, link)
        activeLink = null

        // The default network drives overall online/offline state. A second system callback only
        // observes physical non-VPN LAN arrival so Relay can upgrade without polling.
        defaultNetwork = connectivity.activeNetwork
        networkUp = defaultNetwork != null || physicalLanNetwork() != null
        connectivity.registerDefaultNetworkCallback(network)
        connectivity.registerNetworkCallback(
            NetworkRequest.Builder()
                .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                .build(),
            physicalLanNetworks,
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_CANCEL_TRANSFER -> {
                intent.getStringExtra(EXTRA_TRANSFER_ID)?.let(link::cancelTransfer)
                return START_STICKY
            }
            ACTION_DISCONNECT -> {
                stopLink()
                return START_NOT_STICKY
            }
            ACTION_FORGET -> {
                forgetPeer(remote = false)
                return START_NOT_STICKY
            }
            ACTION_CANCEL_PAIR -> {
                cancelPairing(resumeOldPeer = true)
                return START_STICKY
            }
        }
        // Explicit UI actions and BOOT_COMPLETED restoration arrive through startForegroundService()
        // and must enter foreground promptly. A START_STICKY restart has a null intent; keep that
        // restart background-only so an offline desktop never resurrects a notification by itself.
        if (intent != null && intent.action != ACTION_ACCESSIBILITY_CLIP) {
            ensureStartupForeground()
        }
        // A literal address on the intent skips discovery, which is the only way to drive
        // the link when the phone and the desktop are not on one subnet:
        //   adb reverse tcp:41112 tcp:41112
        //   adb shell am start-foreground-service -n com.conduit.sync/.SyncService \
        //       --es host 127.0.0.1
        val host = intent?.getStringExtra("host")
        when {
            intent?.action == ACTION_PAIR -> beginPairing(intent?.getStringExtra(EXTRA_PAIRING_CODE))
            intent?.action == ACTION_ACCESSIBILITY_CLIP -> onAccessibilityClip(intent)
            intent?.action == ACTION_SHARE -> onShare(intent)
            host != null -> {
                Settings.linkWanted = true
                cancelRetry()
                LinkStatus.path = "Direct"
                link.connect(InetSocketAddress(host, intent.getIntExtra("port", PORT)))
            }
            // An explicit tap always overrides a remembered disconnect and starts over from
            // the short interval, so the user never waits out a backoff they did not cause.
            intent?.action == ACTION_CONNECT -> {
                Settings.linkWanted = true
                cancelRetry()
                redial()
            }
            // Boot restore is not the same as a Connect tap: it must never override a remembered
            // Disconnect. BootReceiver already filters this, and the service checks again so a
            // concurrent state change cannot resurrect a link the user turned off.
            intent?.action == ACTION_BOOT_CONNECT && Settings.linkWanted -> {
                cancelRetry()
                Log.i(TAG, "boot completed; restoring companion link")
                redial()
            }
            // A null intent is the system restarting us under START_STICKY. Respecting the
            // remembered choice is the whole reason it is persisted.
            Settings.linkWanted && serviceStartMayRedial(intent?.action, LinkStatus.state == LinkState.Connected) -> redial()
            Settings.linkWanted -> Unit
            else -> {
                Log.i(TAG, "restarted, but the user had turned the link off")
                hideLinkNotification()
                stopSelf(startId)
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        // Set first, and the callbacks dropped on this thread rather than posted, so a
        // retry cannot fire against a [Link] that close() is about to spend.
        destroyed = true
        main.removeCallbacks(retry)
        cancelOfflineDemotion()
        main.removeCallbacks(detachedLinkNotificationCancel)
        main.removeCallbacks(pairingTimeout)
        main.removeCallbacks(pairingRetry)
        main.removeCallbacks(wallpaperRefresh)
        if (::wallpaperManager.isInitialized) wallpaperManager.removeOnColorsChangedListener(wallpaperListener)
        // Cleared next, so the notification relay stops handing frames to a link that
        // is being torn down.
        activeLink = null
        ClipboardAccessibilityService.setLinkActive(false)
        setSyncObserversActive(false)
        runCatching { connectivity.unregisterNetworkCallback(network) }
        runCatching { connectivity.unregisterNetworkCallback(physicalLanNetworks) }
        discovery.stop()
        link.close()
        hideAllTransferNotifications()
        FileTransfers.failActive("Link stopped")
        deferredLanHandoff = null
        hideLinkNotification()
        LinkStatus.state = LinkState.Idle
        LinkStatus.peer = null
        LinkStatus.path = null
        LinkStatus.pairing = false
        clearRelayPlan()
        super.onDestroy()
    }

    override fun onBind(intent: Intent): IBinder? = null


    /**
     * Schedules the next attempt, doubling up to [RETRY_MAX_MS].
     *
     * Posted to the main thread because it is called from [Link]'s reader thread, which is
     * what keeps [retryMs] lock-free. A pending attempt is always dropped first, so however
     * many times this is called there is at most one outstanding retry — that single-slot
     * property is what stops a reconnect from becoming a spin loop.
     */
    private fun scheduleRetry() = main.post {
        if (destroyed) return@post
        main.removeCallbacks(retry)
        if (!Settings.linkWanted) {
            Log.i(TAG, "link down and the user wants it off, not retrying")
            return@post
        }
        if (!pairingActive() && knownPeer == null) {
            Log.i(TAG, "no paired desktop and pairing is closed; not retrying")
            LinkStatus.state = LinkState.Idle
            LinkStatus.path = null
            hideLinkNotification()
            stopSelf()
            return@post
        }
        if (!networkUp) {
            // No point counting down against a network that is gone; onAvailable redials. Keep the
            // service background-only and notification-free while there is nothing to connect over.
            Log.i(TAG, "link down with no network, waiting for one")
            LinkStatus.state = LinkState.Waiting
            LinkStatus.path = null
            hideLinkNotification()
            return@post
        }
        if (pairingActive()) {
            LinkStatus.state = LinkState.Pairing
            showPairingNotification()
            return@post
        }
        LinkStatus.state = LinkState.Retrying
        hideLinkNotification()
        val ceiling = retryCeilingMs(SystemClock.uptimeMillis(), recoveryUntilUptimeMs)
        val delay = retryMs.coerceAtMost(ceiling)
        Log.i(
            TAG,
            "link down, retrying in ${delay / 1000}s (ceiling ${ceiling / 1000}s)",
        )
        main.postDelayed(retry, delay)
        retryMs = (delay * 2).coerceAtMost(ceiling)
    }

    /** Drops any pending attempt and returns the backoff to its floor. */
    private fun cancelRetry() = main.post {
        main.removeCallbacks(retry)
        retryMs = RETRY_MIN_MS
    }

    /** The user's tap. Remembered by [Settings], so a service restart does not undo it. */
    private fun stopLink() {
        Log.i(TAG, "user asked for a disconnect")
        Settings.linkWanted = false
        pairingUntilUptimeMs = 0L
        pairingRendezvous = null
        LinkStatus.pairing = false
        main.removeCallbacks(pairingTimeout)
        main.removeCallbacks(pairingRetry)
        main.removeCallbacks(retry)
        retryMs = RETRY_MIN_MS
        recoveryUntilUptimeMs = 0L
        discovery.stop()
        clearRelayPlan()
        link.disconnect()
        hideAllTransferNotifications()
        FileTransfers.failActive("Link stopped")
        deferredLanHandoff = null
        LinkStatus.state = LinkState.Idle
        LinkStatus.path = null
        hideLinkNotification()
        LinkTileService.refresh(this)
        stopSelf()
    }

    /**
     * One dial attempt, routed by what the current network can actually reach.
     *
     * On Wi-Fi or Ethernet the desktop is probably on this subnet, so an mDNS burst goes
     * first and [Discovery]'s empty-burst callback falls through to the relay — that is
     * the foreign-Wi-Fi case. On cellular mDNS cannot work at all, so it is skipped
     * rather than run and waited out: eight seconds of multicast on a mobile network is
     * eight seconds of radio for a guaranteed miss.
     */
    private fun isPhysicalLanNetwork(net: Network): Boolean {
        val caps = connectivity.getNetworkCapabilities(net) ?: return false
        if (caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) return false
        return caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) ||
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
    }

    /**
     * Returns the actual physical LAN Network, not merely "LAN exists". Android reports a VPN as
     * the default network (and may even expose WIFI|VPN on its capabilities), but NSD must be bound
     * to the underlying Wi-Fi/Ethernet Network or multicast discovery can disappear into the VPN.
     */
    private fun physicalLanNetwork(preferred: Network? = null): Network? {
        preferred?.takeIf(::isPhysicalLanNetwork)?.let { return it }
        val active = connectivity.activeNetwork
        if (active != null && active != preferred && isPhysicalLanNetwork(active)) return active
        return connectivity.allNetworks.asSequence().firstOrNull(::isPhysicalLanNetwork)
    }

    private fun networkHasLan(net: Network? = null): Boolean = physicalLanNetwork(net) != null

    /**
     * A Relay connection proves the desktop is online. If a physical LAN is present, open one
     * bounded NSD window on that exact Network. Thirty seconds is long enough to survive a missed
     * multicast/advertisement race without turning discovery into a periodic background activity.
     */
    private fun maybeRecheckLanAfterRelay(preferredLan: Network? = null) {
        val viaRelay = relayAttemptConnected && relayAttempt != null
        val lan = physicalLanNetwork(preferredLan)
        if (!shouldRecheckLanAfterRelay(viaRelay, lan != null, relayLanRecheckDone) || lan == null) return
        relayLanRecheckDone = true
        Log.i(TAG, "relay linked with physical LAN available; opening bounded direct-handoff discovery on $lan")
        discovery.burst(lan, RELAY_LAN_HANDOFF_DISCOVERY_MS)
    }

    private fun beginLanHandoff(target: LanTarget) {
        if (!Settings.linkWanted || !networkUp ||
            LinkStatus.state != LinkState.Connected || !relayAttemptConnected) return
        if (target.network?.let(::isPhysicalLanNetwork) == false) return
        if (link.hasActiveFileTransfers) {
            deferredLanHandoff = target
            Log.i(TAG, "deferring optional LAN handoff until file transfers finish")
            return
        }
        deferredLanHandoff = null
        if (!link.disconnectIfTransfersIdle()) {
            deferredLanHandoff = target
            return
        }
        Log.i(TAG, "LAN desktop appeared at ${target.address}; replacing idle Relay session")
        discovery.stop()
        LinkStatus.path = "LAN"
        clearRelayPlan()
        // The new dial owns a distinct generation; any delayed old Idle callback
        // cannot schedule a second retry or suppress fallback if this LAN dial fails.
        link.connect(target.address, target.network)
    }

    private fun redial(net: Network? = null) {
        if (!Settings.linkWanted) {
            Log.i(TAG, "not dialling; the user turned the link off")
            return
        }
        if (LinkStatus.state == LinkState.Connected && !pairingActive()) {
            maybeRecheckLanAfterRelay(net)
            return
        }
        if (pairingActive()) {
            retryPairing(net)
            return
        }
        if (knownPeer == null) {
            Log.i(TAG, "not dialling; no desktop is paired")
            LinkStatus.state = LinkState.Idle
            LinkStatus.path = null
            hideLinkNotification()
            stopSelf()
            return
        }
        val lan = physicalLanNetwork(net)
        if (lan != null) {
            Log.i(TAG, "network up on a LAN, bursting on $lan")
            clearRelayPlan()
            LinkStatus.path = "LAN"
            discovery.burst(lan)
        } else {
            Log.i(TAG, "network up off-LAN, going straight to the relay")
            dialRelay()
        }
    }

    /**
     * Parks at the relay under the desktop's device id.
     *
     * Unresolved on purpose: DNS blocks, and [Link]'s reader thread is the one thread
     * here allowed to block. Normal Relay reconnect still requires the remembered desktop id;
     * first/replacement pairing uses a separate two-minute code-derived Relay rendezvous.
     */
    private fun dialRelay() {
        val peer = knownPeer
        if (peer == null) {
            Log.i(TAG, "no paired desktop yet, so no normal relay rendezvous")
            LinkStatus.path = null
            // Nothing was dialled, so no reader thread will ever report Idle for this
            // attempt — and Idle is what schedules the next one. Without this the retry
            // chain ends here and an unpaired phone goes dark until the user taps.
            scheduleRetry()
            return
        }
        val context = currentNetworkClass()
        relayNetworkClass = context
        relayCandidates.clear()
        relayQuality.candidates(context, relayEndpoints, System.currentTimeMillis())
            .forEach(relayCandidates::addLast)
        Log.i(TAG, "relay candidates for $context: ${relayCandidates.joinToString { it.id }}")
        dialNextRelay()
    }

    private fun dialNextRelay() {
        if (!Settings.linkWanted || !networkUp) return
        val rendezvous = pairingRendezvous ?: knownPeer ?: return
        val endpoint = relayCandidates.pollFirst()
        if (endpoint == null) {
            clearRelayPlan()
            scheduleRetry()
            return
        }
        relayAttempt = endpoint
        relayAttemptConnected = false
        relayConnectedAtMs = 0L
        relayAttemptStartedAtMs = SystemClock.elapsedRealtime()
        LinkStatus.path = if (pairingRendezvous != null) {
            "Pairing · ${endpoint.id.uppercase()} Relay"
        } else {
            "Relay · ${endpoint.id.uppercase()}"
        }
        Log.i(TAG, "trying relay ${endpoint.id} at ${endpoint.host}:${endpoint.port}")
        link.connectVia(
            InetSocketAddress.createUnresolved(endpoint.host, endpoint.port),
            rendezvous,
            endpoint.fallbackIpv4,
        )
    }

    private fun clearRelayPlan() {
        relayCandidates.clear()
        relayAttempt = null
        relayAttemptConnected = false
        relayConnectedAtMs = 0L
        relayAttemptStartedAtMs = 0L
    }

    /**
     * Coarse context only; no extra permission and no periodic fingerprinting. If the default is a
     * VPN, inspect currently-present physical networks once at this natural reconnect event so
     * Wi-Fi and cellular histories do not contaminate each other behind the same VPN app.
     */
    private fun currentNetworkClass(): String {
        fun classify(caps: NetworkCapabilities?): String? = when {
            caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true -> "wifi"
            caps?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true -> "ethernet"
            caps?.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) == true -> "cellular"
            else -> null
        }

        val activeCaps = connectivity.getNetworkCapabilities(connectivity.activeNetwork)
        classify(activeCaps)?.let { return it }
        if (activeCaps?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true) {
            val physical = connectivity.allNetworks.asSequence()
                .mapNotNull { net -> connectivity.getNetworkCapabilities(net) }
                .filter { caps -> !caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) }
                .mapNotNull(::classify)
                .firstOrNull()
            return physical?.let { "vpn-$it" } ?: "vpn"
        }
        return "other"
    }

    private fun pairingActive(): Boolean =
        pairingUntilUptimeMs > SystemClock.uptimeMillis()

    private fun beginPairing(rawCode: String?) {
        val normalized = rawCode?.let(PairingCode::normalize).orEmpty()
        if (normalized.isNotEmpty() && !PairingCode.isValid(normalized)) {
            Log.w(TAG, "refusing malformed pairing code")
            return
        }
        Settings.linkWanted = true
        pairingUntilUptimeMs = SystemClock.uptimeMillis() + PAIRING_WINDOW_MS
        pairingRendezvous = normalized.takeIf(String::isNotEmpty)?.let(PairingCode::rendezvous)
        LinkStatus.pairing = true
        LinkStatus.state = LinkState.Pairing
        LinkStatus.path = if (pairingRendezvous != null) "Pairing · Relay" else "LAN · Pairing"
        main.removeCallbacks(pairingTimeout)
        main.removeCallbacks(pairingRetry)
        main.postDelayed(pairingTimeout, PAIRING_WINDOW_MS)
        main.removeCallbacks(retry)
        retryMs = RETRY_MIN_MS
        clearRelayPlan()
        discovery.stop()
        link.disconnect()
        Log.i(
            TAG,
            if (pairingRendezvous != null) {
                "pairing opened for ${PAIRING_WINDOW_MS / 1000}s with Relay code rendezvous"
            } else {
                "pairing opened for ${PAIRING_WINDOW_MS / 1000}s on LAN discovery"
            },
        )
        retryPairing()
    }

    private fun retryPairing(net: Network? = null) {
        if (!pairingActive() || !networkUp) return
        if (pairingRendezvous != null) dialPairingRelay() else searchPairingLan(net)
    }

    private fun dialPairingRelay() {
        if (!pairingActive() || pairingRendezvous == null || !networkUp) return
        val context = currentNetworkClass()
        relayNetworkClass = context
        relayCandidates.clear()
        relayQuality.candidates(context, relayEndpoints, System.currentTimeMillis())
            .forEach(relayCandidates::addLast)
        Log.i(TAG, "pairing relay candidates: ${relayCandidates.joinToString { it.id }}")
        dialNextRelay()
    }

    private fun searchPairingLan(net: Network? = null) {
        if (!pairingActive() || pairingRendezvous != null) return
        LinkStatus.state = LinkState.Pairing
        val lan = physicalLanNetwork(net)
        if (lan == null) {
            LinkStatus.path = "Same Wi-Fi required"
            showPairingNotification()
            return
        }
        LinkStatus.path = "LAN · Pairing"
        Log.i(TAG, "pairing window active; running one mDNS burst on $lan")
        discovery.burst(lan)
    }

    private fun cancelPairing(resumeOldPeer: Boolean) {
        // A trusted peer hello arrives on every normal reconnect. Pairing cleanup is allowed to
        // touch discovery/foreground state only when a real Pair-new window is actually open.
        if (!pairingActive()) return
        pairingUntilUptimeMs = 0L
        pairingRendezvous = null
        LinkStatus.pairing = false
        main.removeCallbacks(pairingTimeout)
        main.removeCallbacks(pairingRetry)
        discovery.stop()
        if (LinkStatus.state != LinkState.Connected) hideLinkNotification()
        if (resumeOldPeer && knownPeer != null && Settings.linkWanted && networkUp) {
            redial()
        }
    }

    private fun forgetPeer(remote: Boolean) {
        Log.i(TAG, if (remote) "desktop requested unpair" else "user forgot the paired desktop")
        pairingUntilUptimeMs = 0L
        pairingRendezvous = null
        LinkStatus.pairing = false
        main.removeCallbacks(pairingTimeout)
        main.removeCallbacks(pairingRetry)
        main.removeCallbacks(retry)
        discovery.stop()
        clearRelayPlan()
        Settings.linkWanted = false
        knownPeer = null
        knownPeerName = null
        Identity.forgetPeer(filesDir)
        ShareTarget.clear(this)
        LinkStatus.pairedDeviceId = null
        LinkStatus.peerName = null
        LinkStatus.peer = null
        LinkStatus.state = LinkState.Idle
        LinkStatus.path = null
        hideAllTransferNotifications()
        FileTransfers.failActive("Link stopped")
        deferredLanHandoff = null
        hideLinkNotification()
        if (!remote) link.forgetPeer()
        LinkTileService.refresh(this)
        stopSelf()
    }

    /** Persists a peer only after the desktop returned its authenticated pairing hello. */
    private fun rememberPeer(deviceId: String) {
        val completedPairing = pairingActive()
        val completedViaRelay = completedPairing && pairingRendezvous != null
        val completedRelayId = relayAttempt?.id
        val changed = deviceId != knownPeer
        knownPeer = deviceId
        LinkStatus.pairedDeviceId = deviceId
        if (changed) {
            runCatching { Identity.rememberPeer(filesDir, deviceId) }
                .onSuccess { Log.i(TAG, "paired with $deviceId, relay rendezvous stored") }
                .onFailure { Log.w(TAG, "could not store the peer id; relay stays unavailable", it) }
        }
        if (completedPairing) {
            cancelPairing(resumeOldPeer = false)
            LinkStatus.path = completedPairingPath(completedViaRelay, completedRelayId)
        }
    }

    /**
     * The desktop's name, which arrives once per session.
     *
     * Guarded on a change, because the shortcut is republished with it and the common case
     * is the same desktop with the same name reconnecting all day. A rename is rare and
     * cheap; doing this on every session would be a launcher IPC per reconnect.
     */
    private fun rememberPeerName(name: String) {
        LinkStatus.peerName = name
        if (name != knownPeerName) {
            knownPeerName = name
            Log.i(TAG, "the desktop calls itself '$name'")
            Identity.rememberPeerName(filesDir, name)
            ShareTarget.publish(this, name)
        }
        main.post { refreshLinkedNotification() }
        LinkTileService.refresh(this)
    }

    /**
     * A share from [ShareActivity]: files in the intent's ClipData, or text in EXTRA_TEXT.
     *
     * Nothing here touches the URIs. Resolving a size and opening a stream are both binder
     * calls into the app that owns the file, and this runs on the main thread — so each URI
     * becomes a lambda evaluated later on [Link]'s sender thread, which is also where the
     * grant this intent carried has to still be readable.
     */
    private fun onShare(intent: Intent) {
        intent.getStringExtra(Intent.EXTRA_TEXT)?.takeIf { it.isNotBlank() }?.let { shared ->
            sharedWebUrl(shared)?.let { url ->
                val title = intent.getStringExtra(Intent.EXTRA_TITLE)?.trim().orEmpty()
                Log.i(TAG, "sharing web page $url")
                link.sendSharedUrl(url, title, localDeviceName)
                return
            }
            val text = shared.replace("\r\n", "\n")
            if (text.length > MAX_TEXT) {
                Log.w(TAG, "shared text of ${text.length} chars is too large for one frame")
                return
            }
            // Recorded as our own write, so the copy the desktop makes does not come back.
            lastText = text
            History.record(Direction.Sent, text)
            link.sendText(text)
            return
        }
        val clip = intent.clipData
        if (clip == null || clip.itemCount == 0) {
            Log.w(TAG, "share carried neither text nor a URI")
            return
        }
        val uris = boundedShare((0 until minOf(clip.itemCount, MAX_SHARE_FILES)).mapNotNull {
            clip.getItemAt(it).uri
        })
        if (clip.itemCount > MAX_SHARE_FILES) {
            android.widget.Toast.makeText(this, "Only the first $MAX_SHARE_FILES files were queued",
                android.widget.Toast.LENGTH_SHORT).show()
        }
        for (uri in uris) {
            Log.i(TAG, "sharing $uri")
            link.sendFile(uri.lastPathSegment?.take(200) ?: "Shared file") { Files.open(this, uri) }
        }
    }

    /** Receives ClipData while [ClipboardChangeActivity] still owns foreground clipboard access. */
    private fun onAccessibilityClip(intent: Intent) {
        if (LinkStatus.state != LinkState.Connected) return
        sendLocalClip(intent.clipData, "accessibility")
    }

    private fun onLocalClip() {
        // unregisterContentObserver/listener callbacks can already be queued when a session drops.
        // Never read the clipboard after the authenticated desktop is gone.
        if (LinkStatus.state != LinkState.Connected) return
        sendLocalClip(clipboard.primaryClip, "listener")
    }

    private fun setSyncObserversActive(active: Boolean) {
        if (active == syncObserversActive) return
        syncObserversActive = active
        if (active) {
            clipboard.addPrimaryClipChangedListener(clipListener)
            photos.start()
            screenshots.start()
            wallpaperManager.addOnColorsChangedListener(wallpaperListener, main)
            Log.i(TAG, "linked: clipboard/capture/wallpaper observers enabled")
        } else {
            runCatching { clipboard.removePrimaryClipChangedListener(clipListener) }
            runCatching { photos.stop() }
            runCatching { screenshots.stop() }
            runCatching { wallpaperManager.removeOnColorsChangedListener(wallpaperListener) }
            main.removeCallbacks(wallpaperRefresh)
            Log.i(TAG, "offline: clipboard/capture/wallpaper observers suspended")
        }
    }

    private fun sendLocalClip(clip: ClipData?, source: String) {
        val item = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)
        if (item == null) {
            // On stock Android 10+ the background listener cannot read the clipboard. The
            // AccessibilityService focus handoff is the non-root path; LSPosed remains optional.
            Log.d(TAG, "clipboard unreadable from $source")
            return
        }
        val text = item.text?.toString()?.replace("\r\n", "\n")
        if (text == null) {
            onLocalImage(clip)
            return
        }
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
        History.record(Direction.Sent, text)
        link.sendText(text)
    }

    /**
     * A copied image, which on Android means a `content://` URI rather than bytes.
     *
     * The URI's authority is the echo check, and it is exact: our own write points at
     * [ImageProvider], so recognising it costs an equality test instead of reading the
     * file back on the main thread to discover we were the ones who wrote it.
     */
    private fun onLocalImage(clip: ClipData) {
        val uri = clip.getItemAt(0).uri ?: return
        if (ImageProvider.isOurs(uri)) {
            Log.d(TAG, "echo of our own image write, dropped")
            return
        }
        // Reading happens on the sender thread: opening the URI is a binder call into
        // whichever app owns it, and this listener is on the main thread.
        History.record(Direction.Sent, "Image", image = true)
        link.sendImage("clip image") { Images.fromClipboard(this, clip) }
    }

    private fun onRemoteImage(png: ByteArray, photo: Boolean, screenshot: Boolean) {
        if (photo || screenshot) {
            // Capture images only ever travel phone -> desktop. The compatibility photo bit
            // is also set on screenshots, so either marker means "never touch clipboard".
            val kind = if (screenshot) "screenshot" else "photo"
            Log.i(TAG, "$kind in: ${png.size} B, dropped; the desktop does not send captures")
            return
        }
        Log.i(TAG, "clip image in: ${png.size} B")
        History.record(Direction.Received, "Image, ${png.size / 1024} kB", image = true)
        // The write itself is cheap, but it must happen where the clipboard expects to
        // be touched, and the listener it triggers arrives on the main thread too.
        main.post { Images.toClipboard(this, clipboard, png) }
    }

    private fun onRemoteText(text: String) {
        val normalised = text.replace("\r\n", "\n")
        if (normalised == lastText) return
        Log.i(TAG, "clip text in: ${normalised.length} chars")
        // Recorded before the write, because the listener fires on the main thread and
        // may observe the change before setPrimaryClip has returned here.
        lastText = normalised
        History.record(Direction.Received, normalised)
        main.post {
            clipboard.setPrimaryClip(ClipData.newPlainText("Conduit", normalised))
        }
    }

    private fun ensureStartupForeground() {
        if (LinkStatus.state == LinkState.Connected) {
            showLinkedNotification()
        } else {
            showConnectingNotification()
        }
    }

    private fun showConnectingNotification() {
        if (LinkStatus.state == LinkState.Connected) return
        val peer = knownPeerName ?: LinkStatus.peerName ?: "desktop"
        showLinkNotification("Connecting to $peer")
    }

    private fun showPairingNotification() {
        if (!pairingActive()) return
        cancelOfflineDemotion()
        showLinkNotification("Pairing · sync paused")
    }

    private fun showPairingOrHideOfflineNotification() {
        if (pairingActive()) {
            showPairingNotification()
        } else {
            // A foreground-service notification cannot be hidden while the service remains
            // foreground. Demote as soon as the link is no longer live; the passive Relay waiter
            // and bounded reconnect state machine may continue as an ordinary background service.
            hideLinkNotification()
        }
    }

    private fun showLinkedNotification() {
        if (LinkStatus.state != LinkState.Connected) return
        cancelOfflineDemotion()
        val peer = knownPeerName ?: LinkStatus.peerName ?: "desktop"
        showLinkNotification("Linked to $peer")
    }

    private fun showLinkNotification(content: String) {
        main.removeCallbacks(detachedLinkNotificationCancel)
        val notice = linkNotification(content)
        if (foregroundVisible) {
            getSystemService(NotificationManager::class.java).notify(LINK_NOTIFICATION_ID, notice)
        } else {
            try {
                startForeground(
                    LINK_NOTIFICATION_ID,
                    notice,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE or
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                            ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING
                        } else {
                            0
                        },
                )
                foregroundVisible = true
            } catch (e: IllegalStateException) {
                // A sticky background restart can reconnect after Android has removed our right to
                // promote a foreground service. The session is still useful while the process
                // survives; never crash or bring back a stale offline notification just for this.
                Log.w(TAG, "could not promote background link to foreground", e)
                getSystemService(NotificationManager::class.java).cancel(LINK_NOTIFICATION_ID)
            }
        }
    }

    private fun refreshLinkedNotification() {
        if (LinkStatus.state == LinkState.Connected) showLinkedNotification()
    }

    private fun scheduleOfflineDemotion() {
        if (offlineDemoteScheduled || !foregroundVisible || pairingActive() ||
            LinkStatus.state == LinkState.Connected
        ) {
            return
        }
        offlineDemoteScheduled = true
        main.postDelayed(offlineDemote, TRANSIENT_RECONNECT_FOREGROUND_GRACE_MS)
    }

    private fun cancelOfflineDemotion() {
        if (!offlineDemoteScheduled) return
        offlineDemoteScheduled = false
        main.removeCallbacks(offlineDemote)
    }

    private fun hideLinkNotification() {
        cancelOfflineDemotion()
        main.removeCallbacks(detachedLinkNotificationCancel)
        val manager = getSystemService(NotificationManager::class.java)
        if (foregroundVisible) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            foregroundVisible = false
        }
        // Cancel immediately, then once more after the foreground-state binder transition. Some
        // OEM notification services ignore the first cancel while the old FGS flag is still live.
        manager.cancel(LINK_NOTIFICATION_ID)
        if (!destroyed && shouldCancelDetachedLinkNotification(
                foregroundVisible = foregroundVisible,
                pairing = pairingActive(),
                state = LinkStatus.state,
            )
        ) {
            main.postDelayed(
                detachedLinkNotificationCancel,
                DETACHED_LINK_NOTIFICATION_CANCEL_DELAY_MS,
            )
        }
    }

    private fun linkNotification(content: String): Notification {
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            // LOW, so an always-present notification never makes a sound.
            NotificationChannel(LINK_CHANNEL, "Link", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shown while Conduit is connecting, pairing, or linked"
            },
        )
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        return Notification.Builder(this, LINK_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_link)
            .setContentTitle("Conduit")
            .setContentText(content)
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .build()
    }

    private fun showTransferNotification(transfer: FileTransfer) {
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) !=
            android.content.pm.PackageManager.PERMISSION_GRANTED) return
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(
            TRANSFER_CHANNEL, "File transfers", NotificationManager.IMPORTANCE_LOW,
        ))
        val receiving = transfer.direction == FileTransferDirection.ToPhone
        val peer = knownPeerName ?: LinkStatus.peerName ?: "desktop"
        val openHome = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val builder = Notification.Builder(this, TRANSFER_CHANNEL)
            .setSmallIcon(if (receiving) R.drawable.ic_stat_download else R.drawable.ic_stat_upload)
            .setContentTitle("${transfer.phase.label} · ${if (receiving) "from" else "to"} $peer")
            .setContentText(transfer.name)
            .setContentIntent(openHome)
            .setSubText(transfer.error.ifBlank {
                "${formatBytes(transfer.transferred)} / ${formatBytes(transfer.total)}"
            })
            .setOnlyAlertOnce(true)
            .setOngoing(!transfer.phase.terminal)
        if (!transfer.phase.terminal) {
            builder.setProgress(100, transfer.percent,
                transfer.phase == TransferPhase.Queued || transfer.phase == TransferPhase.WaitingConfirmation)
        } else builder.setAutoCancel(true).setTimeoutAfter(5_000L)
        if (transfer.canCancel) {
            val cancel = PendingIntent.getService(
                this, 0, Intent(this, SyncService::class.java).apply {
                    action = ACTION_CANCEL_TRANSFER
                    data = android.net.Uri.parse("conduit://cancel/${transfer.id}")
                    putExtra(EXTRA_TRANSFER_ID, transfer.id)
                }, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            builder.addAction(Notification.Action.Builder(null, "Cancel", cancel).build())
        }
        manager.notify("file:${transfer.direction}:${transfer.id}", transferNotificationId(transfer.direction), builder.build())
    }

    private fun hideTransferNotification(direction: FileTransferDirection) {
        getSystemService(NotificationManager::class.java).cancel(transferNotificationId(direction))
    }

    private fun hideAllTransferNotifications() {
        val manager = getSystemService(NotificationManager::class.java)
        FileTransfers.entries.forEach {
            manager.cancel("file:${it.direction}:${it.id}", transferNotificationId(it.direction))
        }
        hideTransferNotification(FileTransferDirection.ToDesktop)
        hideTransferNotification(FileTransferDirection.ToPhone)
    }

    private fun transferNotificationId(direction: FileTransferDirection) = when (direction) {
        FileTransferDirection.ToDesktop -> UPLOAD_NOTIFICATION_ID
        FileTransferDirection.ToPhone -> DOWNLOAD_NOTIFICATION_ID
    }
}
