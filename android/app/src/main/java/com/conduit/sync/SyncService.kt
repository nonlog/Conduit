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
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import com.conduit.sync.proto.DeviceKind
import com.conduit.sync.proto.NotifAction
import com.conduit.sync.proto.PeerInfo
import com.conduit.sync.proto.PeerRoster
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

internal fun retryCeilingMs(
    nowUptimeMs: Long,
    recoveryUntilUptimeMs: Long,
): Long = if (nowUptimeMs < recoveryUntilUptimeMs) RECOVERY_RETRY_MAX_MS else RETRY_MAX_MS

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
internal fun completedPairingPath(
    viaRelay: Boolean,
    relayId: String?,
): String =
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
): Boolean =
    networkChanged && hasOpenSocket &&
        (!usingPhysicalLan || (!boundToNewDefault && !newDefaultIsVpn))

/** An established Relay socket using Wi-Fi/Ethernet can also be stranded when a
 * VPN remains the default while its underlying physical network disappears. */
internal fun shouldReconnectRelayForPhysicalLanLoss(
    lostKnownLan: Boolean,
    relaySocketActive: Boolean,
    relayNetworkClass: String,
): Boolean =
    lostKnownLan && relaySocketActive &&
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
internal fun isCurrentLinkGeneration(
    eventGeneration: Long,
    latestGeneration: Long,
): Boolean = eventGeneration == latestGeneration

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
): Boolean =
    !wasLinked && route == "LAN" && !relayAttemptActive &&
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
const val ACTION_REFRESH_WALLPAPER = "com.conduit.sync.REFRESH_WALLPAPER"
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
const val ACTION_SELECT_DEVICE = "com.conduit.sync.SELECT_DEVICE"
const val ACTION_FORGET_DEVICE = "com.conduit.sync.FORGET_DEVICE"

/** Per-computer pause/resume; [EXTRA_ENABLED] carries the new switch position. */
const val ACTION_SET_DEVICE_ENABLED = "com.conduit.sync.SET_DEVICE_ENABLED"
const val EXTRA_ENABLED = "enabled"
const val EXTRA_TRANSFER_ID = "transfer_id"
const val EXTRA_PEER_ID = "peer_id"

internal fun serviceStartMayRedial(
    action: String?,
    connected: Boolean,
): Boolean = !connected && action != ACTION_SHARE && action != ACTION_CANCEL_TRANSFER && action != ACTION_ACCESSIBILITY_CLIP

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

        @Volatile private var connectedSnapshot: List<Pair<String, Link>> = emptyList()

        /** Immutable snapshot: one authenticated transport per simultaneously linked computer. */
        fun connectedLinks(): List<Pair<String, Link>> = connectedSnapshot
    }

    private lateinit var phonePrivateKey: ByteArray
    private lateinit var companions: CompanionLinks
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

    private data class LanTarget(
        val address: InetSocketAddress,
        val network: Network?,
    )

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

    /** The paired desktop's own LAN "ip:port", announced on the authenticated session. */
    @Volatile private var relayLanHint: String? = null

    /** The desktop we are paired with, so the relay rendezvous survives a restart. */
    @Volatile private var knownPeer: String? = null

    /** The primary Link's name. Other computers have independent name/session state. */
    @Volatile private var knownPeerName: String? = null

    /** User-selected destination for explicit shares and files; other links remain connected. */
    private fun selectedLink(targetId: String? = null): Link? {
        val selected = targetId ?: PairedDesktops.selected(filesDir)
        if (selected == null) return connectedSnapshot.firstOrNull()?.second
        val candidate =
            if (selected == knownPeer) {
                link
            } else if (::companions.isInitialized) {
                companions.linkFor(selected)
            } else {
                null
            }
        // An explicit target must remain exact for Direct Share and revoke actions.
        if (targetId != null) return candidate
        if (connectedSnapshot.any { it.first == selected }) return candidate
        // The durable selection can be stale after another computer reconnects.
        // Prefer an already authenticated link and persist that active destination so
        // clipboard, files and links all agree with the status shown on screen.
        val fallback = connectedSnapshot.firstOrNull() ?: return candidate
        runCatching { PairedDesktops.choose(filesDir, fallback.first) }
        LinkStatus.selectedDeviceId = fallback.first
        return fallback.second
    }

    private fun publishPeerConnections() {
        if (!::link.isInitialized || destroyed) return
        val previouslyConnected = connectedSnapshot.isNotEmpty()
        val secondary =
            if (::companions.isInitialized) {
                companions.states().associateBy { it.id }
            } else {
                emptyMap()
            }
        val peers = PairedDesktops.list(filesDir)
        LinkStatus.devices =
            peers.map { peer ->
                val isPrimary = peer.id == knownPeer
                val extra = secondary[peer.id]
                val live = if (isPrimary) LinkStatus.state else extra?.status ?: LinkState.Idle
                ConnectedDevice(
                    id = peer.id,
                    name = peer.name,
                    // A paused computer reads as paused in the UI even while its stale socket is
                    // still closing, so one switch can never look half-off.
                    state = if (!Settings.linkWanted || !peer.enabled) LinkState.Paused else live,
                    path = if (isPrimary) LinkStatus.path.orEmpty() else extra?.path.orEmpty(),
                    enabled = peer.enabled,
                )
            }
        val connected = ArrayList<Pair<String, Link>>()
        if (LinkStatus.state == LinkState.Connected && knownPeer != null) {
            connected.add(knownPeer!! to link)
        }
        if (::companions.isInitialized) connected.addAll(companions.liveLinks())
        connectedSnapshot = connected.toList()
        publishRoster()
        // The primary link may have gone offline while a companion remains live.
        // Keep the legacy tile/notification reference useful without changing the
        // independent multi-link snapshot used by clipboard and shares.
        activeLink = connected.firstOrNull()?.second
        val selected = PairedDesktops.selected(filesDir)
        val activeSelection =
            selected?.takeIf { id -> connected.any { it.first == id } }
                ?: connected.firstOrNull()?.first
                ?: selected
        if (activeSelection != null && activeSelection != selected) {
            runCatching { PairedDesktops.choose(filesDir, activeSelection) }
        }
        LinkStatus.selectedDeviceId = activeSelection
        ClipboardAccessibilityService.setLinkActive(connected.isNotEmpty())
        setSyncObserversActive(connected.isNotEmpty())
        if (connected.isNotEmpty()) {
            showLinkedNotification()
        } else if (previouslyConnected && !pairingActive()) {
            showPairingOrHideOfflineNotification()
        }
        LinkTileService.refresh(this)
    }

    /**
     * Roster already delivered to each authenticated desktop, tagged with the exact session it
     * went to. Main thread only. Tagging by session rather than by peer is what lets a fast
     * reconnect receive the roster: a new [Link] or a new session generation is never mistaken
     * for one that was already told.
     */
    private val rosterSent = mutableMapOf<String, Pair<Pair<Link, Long>, PeerRoster>>()

    /**
     * Tells every linked desktop who else is trusted, online, paused, and on which route. Sent only
     * when the picture changes, and never to a session that already received that exact roster.
     */
    private fun publishRoster() {
        if (!::link.isInitialized) return
        val roster =
            PeerRoster
                .newBuilder()
                .addAllPeers(
                    LinkStatus.devices.map { device ->
                        PeerInfo
                            .newBuilder()
                            .setDeviceId(device.id)
                            .setDeviceName(device.name)
                            .setDeviceKind(DeviceKind.DEVICE_KIND_DESKTOP)
                            .setOnline(device.state == LinkState.Connected)
                            .setRoute(if (device.state == LinkState.Connected) device.path else "")
                            .setPaused(!Settings.linkWanted || !device.enabled)
                            .build()
                    },
                ).build()
        val live = connectedSnapshot
        rosterSent.keys.retainAll(live.map { it.first }.toSet())
        for ((id, session) in live) {
            val delivered = rosterSent[id]
            val target = session to session.sessionGeneration
            if (!session.hasOpenSocket) continue
            if (delivered != null && delivered.first == target && delivered.second == roster) continue
            if (session.sendRoster(roster)) {
                rosterSent[id] = target to roster
            }
        }
    }

    /** Explicit, user-opened LAN pairing window. Uptime excludes deep sleep and never wakes it. */
    @Volatile private var pairingUntilUptimeMs = 0L

    @Volatile private var pairingRendezvous: String? = null

    /** The six digits of the current window, so each pairing session can prove it knows them. */
    @Volatile private var pairingCode: String? = null
    private val pairingTimeout =
        Runnable {
            if (pairingUntilUptimeMs == 0L) return@Runnable
            pairingUntilUptimeMs = 0L
            pairingRendezvous = null
            LinkStatus.pairing = false
            Log.i(TAG, "pairing window expired")
            // The zero-work boundary does not depend on the radio: with no enabled computer there is
            // nothing to pair with or dial, so the service must go away even while offline.
            if (hasAnyEnabledPeer() && Settings.linkWanted) {
                // A different already linked PC must keep its foreground notification.
                showPairingOrHideOfflineNotification()
                if (networkUp) redial()
            } else if (!pauseIfNothingEnabled()) {
                stopSelf()
            }
        }

    /** Retry is scoped to the explicit pairing window only. A failed/no-result mDNS burst schedules
     * one later burst; nothing here exists during normal linked/offline operation. */
    private val pairingRetry =
        Runnable {
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
    private val offlineDemote =
        Runnable {
            offlineDemoteScheduled = false
            if (destroyed || !Settings.linkWanted || pairingActive() || connectedLinks().isNotEmpty()) {
                return@Runnable
            }
            Log.i(TAG, "transient reconnect grace expired; entering notification-free offline mode")
            hideLinkNotification()
        }
    private val detachedLinkNotificationCancel =
        Runnable {
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
    private val retry =
        Runnable {
            if (destroyed || !Settings.linkWanted || pairingActive()) return@Runnable
            Log.i(TAG, "retrying the link")
            redial()
        }

    /** Wallpaper updates are pushed by Android itself; this one delayed refresh is not polling. */
    private val wallpaperRefresh =
        Runnable {
            if (destroyed || connectedLinks().isEmpty()) return@Runnable
            connectedLinks().forEach { (_, session) ->
                // The event already invalidated the cache. Encode once, reuse for every desktop.
                session.sendWallpaperPreview(ByteArray(0)) { Wallpapers.preview(this) }
            }
        }
    private val wallpaperListener =
        WallpaperManager.OnColorsChangedListener { _, which ->
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
    private val network =
        object : ConnectivityManager.NetworkCallback() {
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
                    if (Settings.linkWanted &&
                        shouldReconnectForDefaultNetworkChange(
                            networkChanged = previousDefault != null && previousDefault != net,
                            hasOpenSocket = link.hasOpenSocket,
                            usingPhysicalLan = lan != null,
                            boundToNewDefault = lan == net,
                            newDefaultIsVpn = caps?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true,
                        )
                    ) {
                        Log.i(TAG, "default network changed $previousDefault -> $net; reconnecting on the new route")
                        relayLanRecheckDone = false
                        deferredLanHandoff = null
                        discovery.stop()
                        link.disconnect()
                    } else if (LinkStatus.state == LinkState.Connected) {
                        maybeRecheckLanAfterRelay(net)
                    } else if (pairingActive()) {
                        retryPairing(net)
                    } else {
                        redial(net)
                    }
                    if (::companions.isInitialized) {
                        companions.onDefaultChanged(
                            previousDefault != null && previousDefault != net,
                            caps?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true,
                            net,
                        )
                    }
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
                    if (::companions.isInitialized) {
                        companions.onDefaultChanged(
                            true,
                            defaultNetwork
                                ?.let { connectivity.getNetworkCapabilities(it) }
                                ?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true,
                            defaultNetwork,
                        )
                    }
                }
            }
        }

    /**
     * Watches physical LAN arrival independently of Android's default network. A VPN can remain
     * the default while Wi-Fi comes and goes underneath it, so the default callback alone cannot
     * trigger a Relay-to-LAN upgrade. This callback is event-driven; it performs no scans itself.
     */
    private val physicalLanNetworks =
        object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(net: Network) = physicalLanReady(net)

            override fun onLinkPropertiesChanged(
                net: Network,
                properties: LinkProperties,
            ) = physicalLanReady(net)

            override fun onCapabilitiesChanged(
                net: Network,
                capabilities: NetworkCapabilities,
            ) = physicalLanReady(net)

            override fun onLost(net: Network) {
                main.post {
                    if (destroyed) return@post
                    val wasKnownLan = readyLanAddresses.remove(net) != null
                    relayLanRecheckDone = false
                    if (deferredLanHandoff?.network == net) deferredLanHandoff = null
                    networkUp = connectivity.activeNetwork != null || physicalLanNetwork() != null
                    Log.i(TAG, "physical LAN lost: $net")
                    if (link.isBoundTo(net) ||
                        shouldReconnectRelayForPhysicalLanLoss(
                            lostKnownLan = wasKnownLan,
                            relaySocketActive = relayAttempt != null && link.hasOpenSocket && link.boundNetwork == null,
                            relayNetworkClass = relayNetworkClass,
                        )
                    ) {
                        Log.i(TAG, "network $net owned the last active route; reconnecting")
                        link.disconnect()
                    }
                    if (::companions.isInitialized) companions.onPhysicalLanLost(net)
                }
            }
        }

    private fun physicalLanReady(net: Network) {
        main.post {
            if (destroyed || !isPhysicalLanNetwork(net)) return@post
            // onAvailable can precede DHCP/address delivery on hotspots. Reconsider only a
            // genuinely new usable address set; repeated capability callbacks add no probes.
            val addresses =
                connectivity
                    .getLinkProperties(net)
                    ?.linkAddresses
                    .orEmpty()
                    .filter { it.address is java.net.Inet4Address && !it.address.isLoopbackAddress }
                    .map { "${it.address.hostAddress}/${it.prefixLength}" }
                    .sorted()
                    .joinToString(",")
            val previousAddresses = readyLanAddresses[net]
            if (addresses.isEmpty()) {
                if (previousAddresses != null) {
                    readyLanAddresses.remove(net)
                    if (shouldReconnectBoundLanAfterAddressChange(
                            previousAddresses,
                            addresses,
                            link.isBoundTo(net),
                        ) ||
                        shouldReconnectRelayForPhysicalLanLoss(
                            lostKnownLan = true,
                            relaySocketActive = relayAttempt != null && link.hasOpenSocket && link.boundNetwork == null,
                            relayNetworkClass = relayNetworkClass,
                        )
                    ) {
                        Log.i(TAG, "physical LAN lost its IPv4 address; reconnecting")
                        link.disconnect()
                    }
                    if (::companions.isInitialized) companions.onPhysicalLanLost(net)
                }
                return@post
            }
            if (previousAddresses == addresses) return@post
            readyLanAddresses[net] = addresses
            networkUp = true
            relayLanRecheckDone = false
            Log.i(TAG, "physical LAN addressing ready: $net")
            if (shouldReconnectBoundLanAfterAddressChange(
                    previousAddresses,
                    addresses,
                    link.isBoundTo(net),
                ) || (
                    previousAddresses != null &&
                        shouldReconnectRelayForPhysicalLanLoss(
                            lostKnownLan = true,
                            relaySocketActive = relayAttempt != null && link.hasOpenSocket && link.boundNetwork == null,
                            relayNetworkClass = relayNetworkClass,
                        )
                )
            ) {
                // An existing Relay TCP flow also owns the previous Wi-Fi source
                // address; DHCP replacing it without replacing Network invalidates
                // the socket even when the VPN/default Network identity is unchanged.
                Log.i(TAG, "LAN address changed $previousAddresses -> $addresses; reconnecting obsolete route")
                link.disconnect()
            } else if (LinkStatus.state == LinkState.Connected && relayAttemptConnected) {
                maybeRecheckLanAfterRelay(net)
            } else if (LinkStatus.state != LinkState.Connected) {
                if (pairingActive()) retryPairing(net) else redial(net)
            }
            if (::companions.isInitialized) companions.onPhysicalLanReady(net, previousAddresses != null)
        }
    }

    override fun onCreate() {
        super.onCreate()
        val identity = Identity.loadOrCreate(filesDir)
        phonePrivateKey = identity.private
        localDeviceId = Identity.deviceId(identity.public)
        LinkStatus.fingerprint = Identity.fingerprint(identity.public)
        Log.i(TAG, "identity $localDeviceId")

        clipboard = getSystemService(ClipboardManager::class.java)
        connectivity = getSystemService(ConnectivityManager::class.java)
        wallpaperManager = getSystemService(WallpaperManager::class.java)
        knownPeer = Identity.peer(filesDir)
        knownPeerName = Identity.peerName(filesDir)
        LinkStatus.pairedDeviceId = knownPeer
        LinkStatus.selectedDeviceId = PairedDesktops.selected(filesDir)
        LinkStatus.devices =
            Identity.peers(filesDir).map { peer ->
                ConnectedDevice(peer.id, peer.name, LinkState.Idle, "")
            }
        LinkStatus.peerName = knownPeerName
        LinkStatus.pairing = false
        // Refresh the shortcut once per service process so an app update can change its icon even
        // when the paired desktop name did not change.
        ShareTarget.publishAll(this, Identity.peers(filesDir))
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

        link =
            Link(
                identity.private,
                localDeviceId,
                localDeviceName,
                object : Link.Events {
                    override fun onState(
                        state: LinkState,
                        peer: String?,
                    ) {
                        val eventGeneration = link.sessionGeneration
                        val previousState = LinkStatus.state
                        LinkStatus.state = state
                        LinkStatus.peer = peer
                        val linked = state == LinkState.Connected
                        // The aggregate publish below owns the legacy activeLink reference too;
                        // updating it here would create a brief null window while a companion
                        // connection is still authenticated.
                        main.post {
                            if (!destroyed) publishPeerConnections()
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
                                val connectedFor =
                                    if (wasConnected) {
                                        SystemClock.elapsedRealtime() - relayConnectedAtMs
                                    } else {
                                        0L
                                    }
                                main.post {
                                    if (!isCurrentLinkGeneration(eventGeneration, link.sessionGeneration)) {
                                        Log.d(TAG, "ignoring delayed Idle from superseded session $eventGeneration")
                                        return@post
                                    }
                                    // The explicit unpair closes this transport on purpose. Its
                                    // old Idle event must not start an unwanted reconnect to the
                                    // peer whose trust is being removed.
                                    if (eventGeneration == revokingPrimaryGeneration) return@post
                                    // The user paused this computer on purpose; its deliberate close
                                    // must not start a recovery dial or a notification.
                                    if (!pairingActive() && primaryPaused()) {
                                        LinkStatus.state = LinkState.Paused
                                        LinkStatus.path = null
                                        clearRelayPlan()
                                        if (connectedLinks().isEmpty()) showPairingOrHideOfflineNotification()
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

                            LinkState.Discovering -> {
                                main.post {
                                    if (pairingActive()) {
                                        showPairingNotification()
                                    } else if (foregroundVisible) {
                                        // Keep the user-initiated connection attempt visible, but never
                                        // resurrect a notification for a background retry after we demoted.
                                        showConnectingNotification()
                                    }
                                }
                            }

                            LinkState.Waiting, LinkState.Retrying -> {
                                main.post {
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
                            }

                            LinkState.Pairing -> {
                                main.post { showPairingNotification() }
                            }

                            // A paused computer has nothing to announce, but a live companion still owns
                            // the notification and the foreground service.
                            LinkState.Paused -> {
                                main.post {
                                    if (connectedLinks().isEmpty()) hideLinkNotification()
                                }
                            }
                        }
                    }

                    override fun onText(text: String) = onRemoteText(text, knownPeer)

                    override fun onImage(
                        png: ByteArray,
                        photo: Boolean,
                        screenshot: Boolean,
                    ) = onRemoteImage(png, photo, screenshot, knownPeer)

                    override fun onTransfer(transfer: FileTransfer) {
                        main.post {
                            if (destroyed) return@post
                            FileTransfers.update(transfer)
                            if (FileTransfers.entries.any { it == transfer }) showTransferNotification(transfer)
                        }
                    }

                    override fun onSendRejected(reason: String) {
                        main.post {
                            if (!destroyed) {
                                android.widget.Toast
                                    .makeText(
                                        this@SyncService,
                                        reason,
                                        android.widget.Toast.LENGTH_SHORT,
                                    ).show()
                            }
                        }
                    }

                    override fun onTransfersIdle() {
                        main.post {
                            val target = deferredLanHandoff ?: return@post
                            deferredLanHandoff = null
                            if (!destroyed) beginLanHandoff(target)
                        }
                    }

                    override fun onBulkTransfer(
                        bytes: Long,
                        elapsedMs: Long,
                    ) {
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
                            val trusted = PairedDesktops.list(filesDir)
                            if (trusted.size >= PairedDesktops.MAX_PEERS &&
                                trusted.none { it.id == deviceId }
                            ) {
                                Log.w(TAG, "rejecting an additional desktop: trusted device registry full")
                                return false
                            }
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

                    override fun onPeerLanHint(hint: String) {
                        // Authenticated by the Noise handshake, so an unusable hint is only a
                        // failed dial; every callback replaces the previous address.
                        relayLanHint = hint.takeIf { it.isNotBlank() }
                    }

                    override fun onSessionLost() {
                        // Recovery policy lives in the preceding Idle state transition. Keeping this
                        // callback observational prevents a second dial from racing that one.
                        Log.i(TAG, "session lost")
                    }
                },
                openIncomingFile = { offer -> Files.Incoming.begin(contentResolver, offer) },
                pairingCodeProvider = { if (pairingActive()) pairingCode else null },
            )
        discovery =
            Discovery(
                this,
                onFound = { address ->
                    val target = LanTarget(address, discovery.resolvedNetwork)
                    main.post {
                        // A paused primary must not answer LAN discovery, except inside a pairing
                        // window — "Search the same LAN" has to work while this computer is paused.
                        if (destroyed || !Settings.linkWanted || (!pairingActive() && primaryPaused())) {
                            return@post
                        }
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
        photos = Photos(this) { connectedLinks().map { it.second } }
        screenshots = Screenshots(this) { connectedLinks().map { it.second } }
        activeLink = null
        companions =
            CompanionLinks(
                context = this,
                privateKey = phonePrivateKey,
                localId = localDeviceId,
                localName = localDeviceName,
                lanNetwork = { physicalLanNetwork() },
                relayEndpoints = { relayEndpoints },
                networkAvailable = { networkUp },
                onChange = { publishPeerConnections() },
                onIncomingText = { id, text -> onRemoteText(text, id) },
                onIncomingImage = { id, png, photo, screenshot ->
                    onRemoteImage(png, photo, screenshot, id)
                },
                onTransferEvent = { transfer ->
                    main.post {
                        if (!destroyed) {
                            FileTransfers.update(transfer)
                            if (FileTransfers.entries.any { it == transfer }) showTransferNotification(transfer)
                        }
                    }
                },
                onError = { reason ->
                    main.post {
                        if (!destroyed) {
                            android.widget.Toast
                                .makeText(
                                    this,
                                    reason,
                                    android.widget.Toast.LENGTH_SHORT,
                                ).show()
                        }
                    }
                },
                onAction = { action -> main.post { NotificationRelay.perform(action) } },
                onForgotten = { id -> forgetDevice(id, remote = true) },
                onWallpaper = { peerLink, hash ->
                    peerLink.sendWallpaperPreview(hash) { Wallpapers.preview(this) }
                },
            )
        companions.reconcile(knownPeer)

        // The default network drives overall online/offline state. A second system callback only
        // observes physical non-VPN LAN arrival so Relay can upgrade without polling.
        defaultNetwork = connectivity.activeNetwork
        networkUp = defaultNetwork != null || physicalLanNetwork() != null
        connectivity.registerDefaultNetworkCallback(network)
        connectivity.registerNetworkCallback(
            NetworkRequest
                .Builder()
                .addCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
                .build(),
            physicalLanNetworks,
        )
        if (Settings.linkWanted) companions.reconnectAll()
    }

    override fun onStartCommand(
        intent: Intent?,
        flags: Int,
        startId: Int,
    ): Int {
        when (intent?.action) {
            ACTION_REFRESH_WALLPAPER -> {
                Wallpapers.invalidate()
                main.removeCallbacks(wallpaperRefresh)
                if (connectedLinks().isNotEmpty()) main.post(wallpaperRefresh)
                // Never turn a concurrent reconnect off just because this refresh raced it.
                return if (Settings.linkWanted) START_STICKY else START_NOT_STICKY
            }

            ACTION_CANCEL_TRANSFER -> {
                intent.getStringExtra(EXTRA_TRANSFER_ID)?.let { id ->
                    link.cancelTransfer(id)
                    companions.cancelTransfer(id)
                }
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

            ACTION_SELECT_DEVICE -> {
                intent.getStringExtra(EXTRA_PEER_ID)?.let { id ->
                    runCatching { PairedDesktops.choose(filesDir, id) }
                        .onSuccess { publishPeerConnections() }
                        .onFailure { Log.w(TAG, "cannot select desktop", it) }
                }
                if (!Settings.linkWanted) {
                    hideLinkNotification()
                    stopSelf()
                    return START_NOT_STICKY
                }
                if (pauseIfNothingEnabled()) return START_NOT_STICKY
                return START_STICKY
            }

            ACTION_FORGET_DEVICE -> {
                intent.getStringExtra(EXTRA_PEER_ID)?.let { id ->
                    if (PairedDesktops.list(filesDir).any { it.id == id }) forgetDevice(id, remote = false)
                }
                return START_STICKY
            }

            ACTION_SET_DEVICE_ENABLED -> {
                intent.getStringExtra(EXTRA_PEER_ID)?.let { id ->
                    setDeviceEnabled(id, intent.getBooleanExtra(EXTRA_ENABLED, true))
                }
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
            intent?.action == ACTION_PAIR -> {
                beginPairing(intent?.getStringExtra(EXTRA_PAIRING_CODE))
            }

            intent?.action == ACTION_ACCESSIBILITY_CLIP -> {
                onAccessibilityClip(intent)
            }

            intent?.action == ACTION_SHARE -> {
                onShare(intent)
            }

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
                // The master switch outranks per-computer pauses: a phone whose computers were
                // all paused one by one must connect again on this tap.
                if (PairedDesktops.list(filesDir).let { it.isNotEmpty() && it.none { peer -> peer.enabled } }) {
                    PairedDesktops.enableAll(filesDir)
                    companions.reconcile(knownPeer)
                }
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
            Settings.linkWanted && serviceStartMayRedial(intent?.action, LinkStatus.state == LinkState.Connected) -> {
                redial()
            }

            Settings.linkWanted -> {
                Unit
            }

            else -> {
                Log.i(TAG, "restarted, but the user had turned the link off")
                hideLinkNotification()
                stopSelf(startId)
            }
        }
        if (Settings.linkWanted) companions.reconnectAll()
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
        connectedSnapshot = emptyList()
        companions.close()
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
        // A paused roster outlives the service: the screen must keep saying "Paused", not "Not linked".
        if (LinkStatus.state != LinkState.Paused) LinkStatus.state = LinkState.Idle
        LinkStatus.peer = null
        LinkStatus.path = null
        LinkStatus.pairing = false
        clearRelayPlan()
        super.onDestroy()
    }

    override fun onBind(intent: Intent): IBinder? = null

    /**
     * The zero-work boundary: with every trusted computer paused and no pairing window open there
     * is nothing to dial, so every socket, callback and notification goes away until a Connect tap.
     * Returns true when it did that and already stopped the service.
     */
    private fun pauseIfNothingEnabled(): Boolean {
        if (!Settings.linkWanted || pairingActive()) return false
        val peers = PairedDesktops.list(filesDir)
        if (peers.any { it.enabled }) return false
        Log.i(TAG, "no enabled computer and no pairing window; stopping all idle work")
        main.removeCallbacks(retry)
        main.removeCallbacks(pairingRetry)
        cancelOfflineDemotion()
        discovery.stop()
        clearRelayPlan()
        link.disconnect()
        if (::companions.isInitialized) {
            companions.stopSockets()
            companions.reconcile(knownPeer)
        }
        retryMs = RETRY_MIN_MS
        recoveryUntilUptimeMs = 0L
        LinkStatus.state = if (peers.isEmpty()) LinkState.Idle else LinkState.Paused
        LinkStatus.path = null
        publishPeerConnections()
        hideLinkNotification()
        LinkTileService.refresh(this)
        stopSelf()
        return true
    }

    /** True while at least one trusted computer is allowed to hold a transport. */
    private fun hasAnyEnabledPeer(): Boolean = PairedDesktops.list(filesDir).any { it.enabled }

    /** True when the main transport's computer is paused but the service is still wanted. */
    private fun primaryPaused(): Boolean =
        knownPeer?.let { id ->
            PairedDesktops.list(filesDir).firstOrNull { it.id == id }?.enabled == false
        } ?: false

    /**
     * Schedules the next attempt, doubling up to [RETRY_MAX_MS].
     *
     * Posted to the main thread because it is called from [Link]'s reader thread, which is
     * what keeps [retryMs] lock-free. A pending attempt is always dropped first, so however
     * many times this is called there is at most one outstanding retry — that single-slot
     * property is what stops a reconnect from becoming a spin loop.
     */
    private fun scheduleRetry() =
        main.post {
            if (destroyed) return@post
            main.removeCallbacks(retry)
            if (!Settings.linkWanted) {
                Log.i(TAG, "link down and the user wants it off, not retrying")
                return@post
            }
            if (pauseIfNothingEnabled()) return@post
            if (!pairingActive() && knownPeer == null) {
                Log.i(TAG, "no paired desktop and pairing is closed; not retrying")
                LinkStatus.state = LinkState.Idle
                LinkStatus.path = null
                return@post
            }
            if (!pairingActive() && primaryPaused()) {
                // Never stopSelf here: an enabled companion still needs this process and its
                // notification, and the paused primary must not schedule a dial either.
                Log.i(TAG, "primary computer is paused; leaving its transport down")
                LinkStatus.state = LinkState.Paused
                LinkStatus.path = null
                if (connectedLinks().isEmpty()) hideLinkNotification()
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
    private fun cancelRetry() =
        main.post {
            main.removeCallbacks(retry)
            retryMs = RETRY_MIN_MS
        }

    /**
     * The per-computer switch. Pausing one desktop closes only its own transport: the service and
     * every other enabled computer keep running, and pausing the last one stops all idle work.
     */
    private fun setDeviceEnabled(
        id: String,
        enabled: Boolean,
    ) {
        if (PairedDesktops.list(filesDir).none { it.id == id }) return
        Log.i(TAG, "computer ${id.take(8)} ${if (enabled) "resumed" else "paused"}")
        PairedDesktops.setEnabled(filesDir, id, enabled)
        if (enabled) {
            Settings.linkWanted = true
            // The UI starts this action as a foreground service, so own the notification now.
            ensureStartupForeground()
            cancelRetry()
        } else if (id == knownPeer) {
            main.removeCallbacks(retry)
            recoveryUntilUptimeMs = 0L
            discovery.stop()
            clearRelayPlan()
            LinkStatus.state = LinkState.Paused
            LinkStatus.path = null
            link.disconnect()
        }
        // Rebuilding the companion set is what drops a disabled peer's socket immediately.
        companions.reconcile(knownPeer)
        if (enabled) {
            if (id == knownPeer) redial() else companions.reconnectPeer(id)
        }
        publishPeerConnections()
        if (!enabled && !pauseIfNothingEnabled()) {
            if (!Settings.linkWanted) {
                // The master switch is already off, so this pause left no transport at all:
                // staying resident would only keep callbacks and sockets idling.
                stopSelf()
            } else if (connectedLinks().isEmpty()) {
                hideLinkNotification()
            }
        }
        LinkTileService.refresh(this)
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
        if (::companions.isInitialized) companions.stopSockets()
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

    private fun lanSubnets(net: Network): List<LanDiscoveryPolicy.Subnet> =
        connectivity
            .getLinkProperties(net)
            ?.linkAddresses
            .orEmpty()
            .map { LanDiscoveryPolicy.Subnet(it.address, it.prefixLength) }

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
        // The desktop's own address beats waiting for multicast. The burst above stays armed as the
        // one-shot fallback: a stale hint fails one dial and the same window can still answer.
        val hinted = LanDiscoveryPolicy.hintTarget(relayLanHint, lanSubnets(lan))
        if (hinted != null) {
            Log.i(TAG, "dialling the desktop's LAN hint at $hinted")
            beginLanHandoff(LanTarget(hinted, lan))
        }
    }

    private fun beginLanHandoff(target: LanTarget) {
        if (!Settings.linkWanted || !networkUp ||
            LinkStatus.state != LinkState.Connected || !relayAttemptConnected
        ) {
            return
        }
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
        if (pauseIfNothingEnabled()) return
        if (LinkStatus.state == LinkState.Connected && !pairingActive()) {
            maybeRecheckLanAfterRelay(net)
            return
        }
        if (pairingActive()) {
            retryPairing(net)
            return
        }
        if (knownPeer != null && primaryPaused()) {
            Log.i(TAG, "primary computer is paused; not dialling it")
            main.removeCallbacks(retry)
            discovery.stop()
            clearRelayPlan()
            LinkStatus.state = LinkState.Paused
            LinkStatus.path = null
            if (::companions.isInitialized) companions.reconcile(knownPeer)
            publishPeerConnections()
            return
        }
        if (knownPeer == null) {
            Log.i(TAG, "not dialling; no desktop is paired")
            if (PairedDesktops.list(filesDir).isEmpty()) {
                LinkStatus.state = LinkState.Idle
                LinkStatus.path = null
                hideLinkNotification()
                stopSelf()
            }
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
        relayQuality
            .candidates(context, relayEndpoints, System.currentTimeMillis())
            .forEach(relayCandidates::addLast)
        Log.i(TAG, "relay candidates for $context: ${relayCandidates.joinToString { it.id }}")
        dialNextRelay()
    }

    private fun dialNextRelay() {
        if (!Settings.linkWanted || !networkUp) return
        // A paused primary must not restart its rendezvous, not even from a queued failover.
        if (pairingRendezvous == null && primaryPaused()) return
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
        LinkStatus.path =
            if (pairingRendezvous != null) {
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
        fun classify(caps: NetworkCapabilities?): String? =
            when {
                caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true -> "wifi"
                caps?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true -> "ethernet"
                caps?.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) == true -> "cellular"
                else -> null
            }

        val activeCaps = connectivity.getNetworkCapabilities(connectivity.activeNetwork)
        classify(activeCaps)?.let { return it }
        if (activeCaps?.hasTransport(NetworkCapabilities.TRANSPORT_VPN) == true) {
            val physical =
                connectivity.allNetworks
                    .asSequence()
                    .mapNotNull { net -> connectivity.getNetworkCapabilities(net) }
                    .filter { caps -> !caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) }
                    .mapNotNull(::classify)
                    .firstOrNull()
            return physical?.let { "vpn-$it" } ?: "vpn"
        }
        return "other"
    }

    private fun pairingActive(): Boolean = pairingUntilUptimeMs > SystemClock.uptimeMillis()

    private fun beginPairing(rawCode: String?) {
        if (PairedDesktops.list(filesDir).size >= PairedDesktops.MAX_PEERS) {
            android.widget.Toast
                .makeText(
                    this,
                    "Maximum ${PairedDesktops.MAX_PEERS} computers paired; forget one to add another",
                    android.widget.Toast.LENGTH_LONG,
                ).show()
            return
        }
        val normalized = rawCode?.let(PairingCode::normalize).orEmpty()
        if (normalized.isNotEmpty() && !PairingCode.isValid(normalized)) {
            Log.w(TAG, "refusing malformed pairing code")
            return
        }
        Settings.linkWanted = true
        pairingUntilUptimeMs = SystemClock.uptimeMillis() + PAIRING_WINDOW_MS
        pairingRendezvous = normalized.takeIf(String::isNotEmpty)?.let(PairingCode::rendezvous)
        pairingCode = normalized.takeIf(String::isNotEmpty)
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
        relayQuality
            .candidates(context, relayEndpoints, System.currentTimeMillis())
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
        // Closing the window can be the moment the last enabled computer turns out to be gone;
        // that shutdown must not wait for a network event that may never arrive.
        if (pauseIfNothingEnabled()) return
        if (connectedLinks().isEmpty()) hideLinkNotification()
        if (resumeOldPeer && knownPeer != null && Settings.linkWanted && networkUp) {
            redial()
        }
    }

    private fun forgetPeer(remote: Boolean) {
        knownPeer?.let { forgetDevice(it, remote) }
    }

    /** No duplicate revocation tasks for the same Noise identity. Main-thread owned. */
    private val revokingPeers = mutableSetOf<String>()
    private var revokingPrimaryGeneration: Long? = null

    /** Unpair only the chosen authenticated computer; other connections and trust survive. */
    private fun forgetDevice(
        id: String,
        remote: Boolean,
    ) {
        if (PairedDesktops.list(filesDir).none { it.id == id }) return
        if (!remote) {
            // The old implementation removed the companion (and closed its writer)
            // immediately after enqueueing PAIR_RESPONSE(false). That often dropped
            // the revoke and left the desktop trusting this phone indefinitely.
            if (!revokingPeers.add(id)) return
            val target = selectedLink(id)
            if (target === link) revokingPrimaryGeneration = link.sessionGeneration
            if (target != null) {
                target.forgetPeer {
                    main.post {
                        revokingPeers.remove(id)
                        if (!destroyed) {
                            completeForgetDevice(id, remote = false)
                        } else {
                            PairedDesktops.remove(filesDir, id)
                        }
                    }
                }
                return
            }
            revokingPeers.remove(id)
        }
        completeForgetDevice(id, remote)
    }

    private fun completeForgetDevice(
        id: String,
        remote: Boolean,
    ) {
        if (PairedDesktops.list(filesDir).none { it.id == id }) return
        val primary = id == knownPeer
        Log.i(TAG, "forgetting one desktop: " + id.take(8))
        PairedDesktops.remove(filesDir, id)
        ShareTarget.publishAll(this, PairedDesktops.list(filesDir))
        if (!primary) {
            companions.reconcile(knownPeer)
            publishPeerConnections()
            if (!Settings.linkWanted) {
                hideLinkNotification()
                stopSelf()
            } else {
                pauseIfNothingEnabled()
            }
            return
        }
        pairingUntilUptimeMs = 0L
        pairingRendezvous = null
        LinkStatus.pairing = false
        main.removeCallbacks(pairingTimeout)
        main.removeCallbacks(pairingRetry)
        main.removeCallbacks(retry)
        discovery.stop()
        clearRelayPlan()
        knownPeer = Identity.peer(filesDir)
        knownPeerName = Identity.peerName(filesDir)
        LinkStatus.pairedDeviceId = knownPeer
        LinkStatus.peerName = knownPeerName
        LinkStatus.peer = null
        LinkStatus.state = LinkState.Idle
        LinkStatus.path = null
        companions.reconcile(knownPeer)
        if (remote) link.disconnect()
        publishPeerConnections()
        if (knownPeer == null) {
            // No remaining trusted desktop: preserve the explicit off behaviour.
            stopLink()
        } else {
            // The old owner closes after its unpair frame; its Idle callback and the
            // existing redial policy establish the promoted primary independently.
            main.postDelayed({ if (!destroyed && Settings.linkWanted) redial() }, 300L)
        }
        LinkTileService.refresh(this)
    }

    /** Persists a peer only after the desktop returned its authenticated pairing hello. */
    private fun rememberPeer(deviceId: String) {
        val completedPairing = pairingActive()
        val completedViaRelay = completedPairing && pairingRendezvous != null
        val completedRelayId = relayAttempt?.id
        val changed = deviceId != knownPeer
        if (changed) {
            // Authenticated does not mean durable: a full disk or failed write must
            // not displace the working primary in memory and leave the phone with
            // a phantom pairing that disappears on restart.
            runCatching { Identity.rememberPeer(filesDir, deviceId) }
                .onSuccess { Log.i(TAG, "paired with $deviceId, relay rendezvous stored") }
                .onFailure { cause ->
                    Log.e(TAG, "could not persist new desktop trust; rejecting session", cause)
                    main.post {
                        if (!destroyed) {
                            android.widget.Toast
                                .makeText(
                                    this,
                                    "Cannot save computer pairing; check available storage",
                                    android.widget.Toast.LENGTH_LONG,
                                ).show()
                        }
                    }
                    throw IllegalStateException("Pairing could not be saved", cause)
                }
        }
        knownPeer = deviceId
        LinkStatus.pairedDeviceId = deviceId
        if (completedPairing) {
            cancelPairing(resumeOldPeer = false)
            LinkStatus.path = completedPairingPath(completedViaRelay, completedRelayId)
        }
        // The previously trusted computer becomes an independent companion session.
        main.post {
            if (!destroyed && ::companions.isInitialized) {
                companions.reconcile(knownPeer)
                ShareTarget.publishAll(this, Identity.peers(filesDir))
                publishPeerConnections()
            }
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
            ShareTarget.publishAll(this, Identity.peers(filesDir))
            main.post { publishPeerConnections() }
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
        val requestedId = intent.getStringExtra(EXTRA_PEER_ID)
        val destination = selectedLink(requestedId)
        if (destination == null) {
            Log.w(TAG, "No selected trusted desktop is available for this share")
            return
        }
        if (requestedId != null && connectedSnapshot.none { it.first == requestedId }) {
            Log.w(TAG, "Selected desktop is trusted but offline; share not queued: ${requestedId.take(8)}")
            android.widget.Toast
                .makeText(this, "That desktop is offline", android.widget.Toast.LENGTH_SHORT)
                .show()
            return
        }
        intent.getStringExtra(Intent.EXTRA_TEXT)?.takeIf { it.isNotBlank() }?.let { shared ->
            sharedWebUrl(shared)?.let { url ->
                val title = intent.getStringExtra(Intent.EXTRA_TITLE)?.trim().orEmpty()
                Log.i(TAG, "sharing web page $url")
                destination.sendSharedUrl(url, title, localDeviceName)
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
            destination.sendText(text)
            return
        }
        val clip = intent.clipData
        if (clip == null || clip.itemCount == 0) {
            Log.w(TAG, "share carried neither text nor a URI")
            return
        }
        val uris =
            boundedShare(
                (0 until minOf(clip.itemCount, MAX_SHARE_FILES)).mapNotNull {
                    clip.getItemAt(it).uri
                },
            )
        if (clip.itemCount > MAX_SHARE_FILES) {
            android.widget.Toast
                .makeText(
                    this,
                    "Only the first $MAX_SHARE_FILES files were queued",
                    android.widget.Toast.LENGTH_SHORT,
                ).show()
        }
        for (uri in uris) {
            Log.i(TAG, "sharing $uri")
            destination.sendFile(uri.lastPathSegment?.take(200) ?: "Shared file") { Files.open(this, uri) }
        }
    }

    /** Receives ClipData while [ClipboardChangeActivity] still owns foreground clipboard access. */
    private fun onAccessibilityClip(intent: Intent) {
        if (connectedLinks().isEmpty()) return
        sendLocalClip(intent.clipData, "accessibility")
    }

    private fun onLocalClip() {
        // unregisterContentObserver/listener callbacks can already be queued when a session drops.
        // Never read the clipboard after the authenticated desktop is gone.
        if (connectedLinks().isEmpty()) return
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

    private fun sendLocalClip(
        clip: ClipData?,
        source: String,
    ) {
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
        connectedLinks().forEach { (_, destination) -> destination.sendText(text) }
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
        val payload =
            lazy(LazyThreadSafetyMode.SYNCHRONIZED) {
                Images.fromClipboard(this, clip)
            }
        connectedLinks().forEach { (_, destination) ->
            destination.sendImage("clip image") { payload.value }
        }
    }

    private fun onRemoteImage(
        png: ByteArray,
        photo: Boolean,
        screenshot: Boolean,
        origin: String?,
    ) {
        if (photo || screenshot) {
            // Capture images only ever travel phone -> desktop. The compatibility photo bit
            // is also set on screenshots, so either marker means "never touch clipboard".
            val kind = if (screenshot) "screenshot" else "photo"
            Log.i(TAG, "$kind in: ${png.size} B, dropped; the desktop does not send captures")
            return
        }
        Log.i(TAG, "clip image in: ${png.size} B")
        History.record(Direction.Received, "Image, ${png.size / 1024} kB", image = true)
        connectedLinks().forEach { (id, destination) ->
            if (id != origin) {
                destination.sendImage("clipboard image") {
                    Images.Payload(png, "image/png")
                }
            }
        }
        // The write itself is cheap, but it must happen where the clipboard expects to
        // be touched, and the listener it triggers arrives on the main thread too.
        main.post { Images.toClipboard(this, clipboard, png) }
    }

    private fun onRemoteText(
        text: String,
        origin: String?,
    ) {
        val normalised = text.replace("\r\n", "\n")
        if (normalised == lastText) return
        Log.i(TAG, "clip text in: ${normalised.length} chars")
        // Recorded before the write, because the listener fires on the main thread and
        // may observe the change before setPrimaryClip has returned here.
        lastText = normalised
        History.record(Direction.Received, normalised)
        // Relay a change arriving from one trusted computer to the other
        // simultaneously connected computers, but never echo it to its origin.
        connectedLinks().forEach { (id, destination) ->
            if (id != origin) destination.sendText(normalised)
        }
        main.post {
            clipboard.setPrimaryClip(ClipData.newPlainText("Conduit", normalised))
        }
    }

    private fun ensureStartupForeground() {
        if (connectedLinks().isNotEmpty()) {
            showLinkedNotification()
        } else {
            showConnectingNotification()
        }
    }

    private fun showConnectingNotification() {
        if (connectedLinks().isNotEmpty()) return
        val peer = knownPeerName ?: LinkStatus.peerName ?: "desktop"
        showLinkNotification("Connecting to $peer")
    }

    private fun showPairingNotification() {
        if (!pairingActive()) return
        cancelOfflineDemotion()
        showLinkNotification("Pairing · sync paused")
    }

    private fun showPairingOrHideOfflineNotification() {
        if (connectedLinks().isNotEmpty()) {
            showLinkedNotification()
        } else if (pairingActive()) {
            showPairingNotification()
        } else {
            // A foreground-service notification cannot be hidden while the service remains
            // foreground. Demote as soon as the link is no longer live; the passive Relay waiter
            // and bounded reconnect state machine may continue as an ordinary background service.
            hideLinkNotification()
        }
    }

    private fun showLinkedNotification() {
        val live = connectedLinks()
        if (live.isEmpty()) return
        cancelOfflineDemotion()
        val label =
            if (live.size > 1) {
                "Linked to ${live.size} computers"
            } else {
                PairedDesktops
                    .list(filesDir)
                    .firstOrNull { it.id == live.first().first }
                    ?.name
                    ?.ifBlank { "desktop" } ?: "desktop"
            }
        showLinkNotification(if (live.size > 1) label else "Linked to $label")
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
        if (connectedLinks().isNotEmpty()) showLinkedNotification()
    }

    private fun scheduleOfflineDemotion() {
        if (offlineDemoteScheduled || !foregroundVisible || pairingActive() ||
            connectedLinks().isNotEmpty()
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
        if (!destroyed &&
            shouldCancelDetachedLinkNotification(
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
        val open =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java),
                PendingIntent.FLAG_IMMUTABLE,
            )
        return Notification
            .Builder(this, LINK_CHANNEL)
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
            android.content.pm.PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                TRANSFER_CHANNEL,
                "File transfers",
                NotificationManager.IMPORTANCE_LOW,
            ),
        )
        val receiving = transfer.direction == FileTransferDirection.ToPhone
        val peer = knownPeerName ?: LinkStatus.peerName ?: "desktop"
        val openHome =
            PendingIntent.getActivity(
                this,
                0,
                Intent(this, MainActivity::class.java),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        val builder =
            Notification
                .Builder(this, TRANSFER_CHANNEL)
                .setSmallIcon(if (receiving) R.drawable.ic_stat_download else R.drawable.ic_stat_upload)
                .setContentTitle("${transfer.phase.label} · ${if (receiving) "from" else "to"} $peer")
                .setContentText(transfer.name)
                .setContentIntent(openHome)
                .setSubText(
                    transfer.error.ifBlank {
                        "${formatBytes(transfer.transferred)} / ${formatBytes(transfer.total)}"
                    },
                ).setOnlyAlertOnce(true)
                .setOngoing(!transfer.phase.terminal)
        if (!transfer.phase.terminal) {
            builder.setProgress(
                100,
                transfer.percent,
                transfer.phase == TransferPhase.Queued || transfer.phase == TransferPhase.WaitingConfirmation,
            )
        } else {
            builder.setAutoCancel(true).setTimeoutAfter(5_000L)
        }
        if (transfer.canCancel) {
            val cancel =
                PendingIntent.getService(
                    this,
                    0,
                    Intent(this, SyncService::class.java).apply {
                        action = ACTION_CANCEL_TRANSFER
                        data = android.net.Uri.parse("conduit://cancel/${transfer.id}")
                        putExtra(EXTRA_TRANSFER_ID, transfer.id)
                    },
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
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

    private fun transferNotificationId(direction: FileTransferDirection) =
        when (direction) {
            FileTransferDirection.ToDesktop -> UPLOAD_NOTIFICATION_ID
            FileTransferDirection.ToPhone -> DOWNLOAD_NOTIFICATION_ID
        }
}
