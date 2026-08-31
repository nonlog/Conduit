package com.conduit.sync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.os.Build
import android.util.Log
import android.widget.Toast
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
private const val RELAY_FAILOVER_DELAY_MS = 150L
private const val UNSTABLE_RELAY_SESSION_MS = 60_000L

internal fun retryCeilingMs(nowUptimeMs: Long, recoveryUntilUptimeMs: Long): Long =
    if (nowUptimeMs < recoveryUntilUptimeMs) RECOVERY_RETRY_MAX_MS else RETRY_MAX_MS

/** Sent by the UI. A disconnect has to be remembered, or START_STICKY undoes the user's tap. */
const val ACTION_CONNECT = "com.conduit.sync.CONNECT"
const val ACTION_DISCONNECT = "com.conduit.sync.DISCONNECT"

/** Sent by [ShareActivity]: URIs in the intent's ClipData, or text in EXTRA_TEXT. */
const val ACTION_SHARE = "com.conduit.sync.SHARE"

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
    private lateinit var relayQuality: RelayQualityStore
    private lateinit var localDeviceName: String
    private var relayEndpoints: List<RelayEndpoint> = emptyList()
    private val main = Handler(Looper.getMainLooper())
    private var foregroundVisible = false
    private val uploadProgressGate = TransferProgressGate()
    private val downloadProgressGate = TransferProgressGate()

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

    /** The desktop we are paired with, so the relay rendezvous survives a restart. */
    @Volatile private var knownPeer: String? = null

    /** Its name, so republishing the share-sheet shortcut happens on a rename and not per session. */
    @Volatile private var knownPeerName: String? = null

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
    private val retry = Runnable {
        if (destroyed || !Settings.linkWanted) return@Runnable
        Log.i(TAG, "retrying the link")
        redial()
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
            networkUp = true
            // A different network is a genuinely new chance, not a repeat of the attempt
            // that just failed, so it starts from the floor instead of serving out a
            // backoff earned on a network that no longer exists.
            cancelRetry()
            redial(net)
        }

        override fun onLost(net: Network) {
            // Suspend, not destroy: the socket is dead but [Link] and its sender thread
            // survive, so a reconnect reuses them instead of allocating a fresh set.
            Log.i(TAG, "network gone, suspending session")
            networkUp = false
            // Nothing to count down against; onAvailable is what resumes this.
            main.removeCallbacks(retry)
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
        knownPeerName = Identity.peerName(filesDir)
        LinkStatus.peerName = knownPeerName
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
            localDeviceName,
            object : Link.Events {
                override fun onState(state: LinkState, peer: String?) {
                    LinkStatus.state = state
                    LinkStatus.peer = peer
                    ClipboardAccessibilityService.setLinkActive(state == LinkState.Connected)
                    LinkTileService.refresh(this@SyncService)
                    when (state) {
                        // A completed handshake is the only proof the path works, so it is
                        // the only thing that earns a reset back to the short interval.
                        LinkState.Connected -> {
                            relayAttempt?.let { endpoint ->
                                relayAttemptConnected = true
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
                            main.post { showLinkedNotification() }
                        }
                        // Every exit from the reader thread lands here, whether the
                        // handshake completed or not. That is the point: a dial refused by
                        // a desktop that is not listening notified nothing before, which is
                        // exactly why a phone that missed one burst stayed dark until the
                        // next network event.
                        LinkState.Idle -> {
                            val endpoint = relayAttempt
                            val wasConnected = relayAttemptConnected
                            val connectedFor = if (wasConnected) {
                                SystemClock.elapsedRealtime() - relayConnectedAtMs
                            } else {
                                0L
                            }
                            main.post {
                                hideLinkNotification()
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
                                clearRelayPlan()
                                scheduleRetry()
                            }
                        }
                        LinkState.Discovering, LinkState.Retrying -> {}
                    }
                }

                override fun onText(text: String) = onRemoteText(text)

                override fun onImage(png: ByteArray, photo: Boolean, screenshot: Boolean) =
                    onRemoteImage(png, photo, screenshot)

                override fun onFileProgress(
                    name: String,
                    direction: FileTransferDirection,
                    transferred: Long,
                    total: Long,
                ) {
                    val gate = when (direction) {
                        FileTransferDirection.ToDesktop -> uploadProgressGate
                        FileTransferDirection.ToPhone -> downloadProgressGate
                    }
                    if (!gate.shouldPublish(transferred, total)) return
                    main.post {
                        FileTransfers.update(direction, name, transferred, total)
                        showTransferNotification(direction)
                    }
                }

                override fun onFileComplete(name: String, direction: FileTransferDirection) {
                    progressGate(direction).reset()
                    main.post {
                        FileTransfers.clear(direction)
                        hideTransferNotification(direction)
                        val message = when (direction) {
                            FileTransferDirection.ToDesktop -> "Sent $name"
                            FileTransferDirection.ToPhone -> "Received $name"
                        }
                        Toast.makeText(this@SyncService, message, Toast.LENGTH_LONG).show()
                    }
                }

                override fun onFileFailed(name: String, direction: FileTransferDirection) {
                    progressGate(direction).reset()
                    main.post {
                        FileTransfers.clear(direction)
                        hideTransferNotification(direction)
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

                override fun onPeer(deviceId: String) = rememberPeer(deviceId)

                override fun onPeerName(name: String) = rememberPeerName(name)

                override fun onSessionLost() {
                    // Deliberately does not dial: [onState] already scheduled a retry for
                    // this same teardown, and a second dial here would race it. The backoff
                    // was reset to its minimum when this session connected, so a link that
                    // was working comes back on the short interval anyway.
                    Log.i(TAG, "session lost")
                }
            },
            openIncomingFile = { offer -> Files.Incoming.begin(contentResolver, offer) },
        )
        discovery = Discovery(
            this,
            onFound = { address -> link.connect(address) },
            onEmpty = { dialRelay() },
        )
        photos = Photos(this, link).apply { start() }
        screenshots = Screenshots(this, link).apply { start() }
        activeLink = link

        clipboard.addPrimaryClipChangedListener(clipListener)
        // The default network, not a transport-filtered set: see [network].
        connectivity.registerDefaultNetworkCallback(network)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_DISCONNECT) {
            stopLink()
            return START_NOT_STICKY
        }
        // Android requires a service launched by startForegroundService() to enter foreground
        // quickly. This is only a short-lived connecting notification; the persistent one is
        // shown only after Noise is actually up, and is removed again on any disconnect.
        ensureStartupForeground()
        // A literal address on the intent skips discovery, which is the only way to drive
        // the link when the phone and the desktop are not on one subnet:
        //   adb reverse tcp:41112 tcp:41112
        //   adb shell am start-foreground-service -n com.conduit.sync/.SyncService \
        //       --es host 127.0.0.1
        val host = intent?.getStringExtra("host")
        when {
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
            // A null intent is the system restarting us under START_STICKY. Respecting the
            // remembered choice is the whole reason it is persisted.
            Settings.linkWanted -> redial()
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
        // Cleared next, so the notification relay stops handing frames to a link that
        // is being torn down.
        activeLink = null
        ClipboardAccessibilityService.setLinkActive(false)
        clipboard.removePrimaryClipChangedListener(clipListener)
        connectivity.unregisterNetworkCallback(network)
        screenshots.stop()
        photos.stop()
        discovery.stop()
        link.close()
        FileTransfers.clearAll()
        hideAllTransferNotifications()
        hideLinkNotification()
        LinkStatus.state = LinkState.Idle
        LinkStatus.peer = null
        LinkStatus.path = null
        clearRelayPlan()
        super.onDestroy()
    }

    override fun onBind(intent: Intent): IBinder? = null

    private fun progressGate(direction: FileTransferDirection): TransferProgressGate =
        when (direction) {
            FileTransferDirection.ToDesktop -> uploadProgressGate
            FileTransferDirection.ToPhone -> downloadProgressGate
        }

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
        if (!networkUp) {
            // No point counting down against a network that is gone; onAvailable redials.
            Log.i(TAG, "link down with no network, waiting for one")
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
        main.removeCallbacks(retry)
        retryMs = RETRY_MIN_MS
        recoveryUntilUptimeMs = 0L
        discovery.stop()
        clearRelayPlan()
        link.disconnect()
        FileTransfers.clearAll()
        hideAllTransferNotifications()
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
    private fun redial(net: Network? = null) {
        if (!Settings.linkWanted) {
            Log.i(TAG, "not dialling; the user turned the link off")
            return
        }
        val caps = connectivity.getNetworkCapabilities(net ?: connectivity.activeNetwork)
        val lan = caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true ||
            caps?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true
        if (lan) {
            Log.i(TAG, "network up on a LAN, bursting")
            clearRelayPlan()
            LinkStatus.path = "LAN"
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
        val peer = knownPeer ?: return
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
        LinkStatus.path = "Relay · ${endpoint.id.uppercase()}"
        Log.i(TAG, "trying relay ${endpoint.id} at ${endpoint.host}:${endpoint.port}")
        link.connectVia(
            InetSocketAddress.createUnresolved(endpoint.host, endpoint.port),
            peer,
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

    /** First handshake with a given desktop is the only one that writes. */
    private fun rememberPeer(deviceId: String) {
        if (deviceId == knownPeer) return
        knownPeer = deviceId
        runCatching { Identity.rememberPeer(filesDir, deviceId) }
            .onSuccess { Log.i(TAG, "paired with $deviceId, relay rendezvous stored") }
            .onFailure { Log.w(TAG, "could not store the peer id; relay stays unavailable", it) }
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
        for (index in 0 until clip.itemCount) {
            val uri = clip.getItemAt(index).uri ?: continue
            Log.i(TAG, "sharing $uri")
            link.sendFile(uri.toString()) { Files.open(this, uri) }
        }
    }

    /** Receives ClipData while [ClipboardChangeActivity] still owns foreground clipboard access. */
    private fun onAccessibilityClip(intent: Intent) {
        if (LinkStatus.state != LinkState.Connected) return
        sendLocalClip(intent.clipData, "accessibility")
    }

    private fun onLocalClip() = sendLocalClip(clipboard.primaryClip, "listener")

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
        if (foregroundVisible) return
        startForeground(
            LINK_NOTIFICATION_ID,
            linkNotification(linked = false),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
        )
        foregroundVisible = true
    }

    private fun showLinkedNotification() {
        if (LinkStatus.state != LinkState.Connected) return
        val notice = linkNotification(linked = true)
        if (foregroundVisible) {
            getSystemService(NotificationManager::class.java).notify(LINK_NOTIFICATION_ID, notice)
        } else {
            startForeground(
                LINK_NOTIFICATION_ID,
                notice,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            )
            foregroundVisible = true
        }
    }

    private fun refreshLinkedNotification() {
        if (LinkStatus.state == LinkState.Connected) showLinkedNotification()
    }

    private fun hideLinkNotification() {
        if (foregroundVisible) {
            stopForeground(STOP_FOREGROUND_REMOVE)
            foregroundVisible = false
        }
        // Defensive cancel for OEMs that leave the old foreground entry visible briefly.
        getSystemService(NotificationManager::class.java).cancel(LINK_NOTIFICATION_ID)
    }

    private fun linkNotification(linked: Boolean): Notification {
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            // LOW, so an always-present notification never makes a sound.
            NotificationChannel(LINK_CHANNEL, "Link", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shown while the desktop link is running"
            },
        )
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val peerName = knownPeerName ?: LinkStatus.peerName ?: "desktop"
        val content = when {
            !linked -> "Connecting to $peerName"
            else -> "Linked to $peerName"
        }
        return Notification.Builder(this, LINK_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_link)
            .setContentTitle("Conduit")
            .setContentText(content)
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .build()
    }

    private fun showTransferNotification(direction: FileTransferDirection) {
        val transfer = when (direction) {
            FileTransferDirection.ToDesktop -> FileTransfers.toDesktop
            FileTransferDirection.ToPhone -> FileTransfers.toPhone
        } ?: return
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                TRANSFER_CHANNEL,
                "File transfers",
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                description = "Upload and download progress between this phone and the linked desktop"
            },
        )
        val peer = knownPeerName ?: LinkStatus.peerName ?: "desktop"
        val receiving = direction == FileTransferDirection.ToPhone
        val notice = Notification.Builder(this, TRANSFER_CHANNEL)
            .setSmallIcon(if (receiving) R.drawable.ic_stat_download else R.drawable.ic_stat_upload)
            .setContentTitle(if (receiving) "Receiving from $peer" else "Sending to $peer")
            .setContentText(transfer.name)
            .setSubText("${formatBytes(transfer.transferred)} / ${formatBytes(transfer.total)}")
            .setProgress(100, transfer.percent, false)
            .setOnlyAlertOnce(true)
            .setOngoing(true)
            .build()
        manager.notify(transferNotificationId(direction), notice)
    }

    private fun hideTransferNotification(direction: FileTransferDirection) {
        getSystemService(NotificationManager::class.java).cancel(transferNotificationId(direction))
    }

    private fun hideAllTransferNotifications() {
        hideTransferNotification(FileTransferDirection.ToDesktop)
        hideTransferNotification(FileTransferDirection.ToPhone)
    }

    private fun transferNotificationId(direction: FileTransferDirection) = when (direction) {
        FileTransferDirection.ToDesktop -> UPLOAD_NOTIFICATION_ID
        FileTransferDirection.ToPhone -> DOWNLOAD_NOTIFICATION_ID
    }
}
