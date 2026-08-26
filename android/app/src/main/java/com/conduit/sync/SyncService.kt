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
/**
 * Only used when a VPN resolves [RELAY_HOST] into the fake-IP benchmark range 198.18/15.
 * Normal DNS keeps using the hostname, so a future relay move needs this updated only for
 * fake-IP VPN users rather than turning the endpoint into a permanently pinned address.
 */
private const val RELAY_FALLBACK_IPV4 = "138.3.214.175"

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

/** Sent by the UI. A disconnect has to be remembered, or START_STICKY undoes the user's tap. */
const val ACTION_CONNECT = "com.conduit.sync.CONNECT"
const val ACTION_DISCONNECT = "com.conduit.sync.DISCONNECT"

/** Sent by [ShareActivity]: URIs in the intent's ClipData, or text in EXTRA_TEXT. */
const val ACTION_SHARE = "com.conduit.sync.SHARE"

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
        // Both stores, because this service can be the first component the system starts —
        // under START_STICKY it is started with no activity involved at all — and the
        // remembered disconnect it is about to read lives in one of them.
        Settings.load(this)
        History.load(this)

        link = Link(
            identity.private,
            object : Link.Events {
                override fun onState(state: LinkState, peer: String?) {
                    LinkStatus.state = state
                    LinkStatus.peer = peer
                    when (state) {
                        // A completed handshake is the only proof the path works, so it is
                        // the only thing that earns a reset back to the short interval.
                        LinkState.Connected -> cancelRetry()
                        // Every exit from the reader thread lands here, whether the
                        // handshake completed or not. That is the point: a dial refused by
                        // a desktop that is not listening notified nothing before, which is
                        // exactly why a phone that missed one burst stayed dark until the
                        // next network event.
                        LinkState.Idle -> scheduleRetry()
                        LinkState.Discovering, LinkState.Retrying -> {}
                    }
                }

                override fun onText(text: String) = onRemoteText(text)

                override fun onImage(png: ByteArray, photo: Boolean, screenshot: Boolean) =
                    onRemoteImage(png, photo, screenshot)

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
        when {
            intent?.action == ACTION_DISCONNECT -> stopLink()
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
            else -> Log.i(TAG, "restarted, but the user had turned the link off")
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
        clipboard.removePrimaryClipChangedListener(clipListener)
        connectivity.unregisterNetworkCallback(network)
        screenshots.stop()
        photos.stop()
        discovery.stop()
        link.close()
        LinkStatus.state = LinkState.Idle
        LinkStatus.peer = null
        LinkStatus.path = null
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
        if (!networkUp) {
            // No point counting down against a network that is gone; onAvailable redials.
            Log.i(TAG, "link down with no network, waiting for one")
            return@post
        }
        LinkStatus.state = LinkState.Retrying
        Log.i(TAG, "link down, retrying in ${retryMs / 1000}s")
        main.postDelayed(retry, retryMs)
        retryMs = (retryMs * 2).coerceAtMost(RETRY_MAX_MS)
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
        discovery.stop()
        link.disconnect()
        LinkStatus.state = LinkState.Idle
        LinkStatus.path = null
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
        LinkStatus.path = "Relay"
        link.connectVia(
            InetSocketAddress.createUnresolved(RELAY_HOST, RELAY_PORT),
            peer,
            RELAY_FALLBACK_IPV4,
        )
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
        if (name == knownPeerName) return
        knownPeerName = name
        Log.i(TAG, "the desktop calls itself '$name'")
        Identity.rememberPeerName(filesDir, name)
        ShareTarget.publish(this, name)
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
            link.sendFile(uri.toString()) {
                // On the sender thread: History is safe from any thread and the name is only
                // known once the provider has been asked.
                Files.open(this, uri)?.also {
                    History.record(Direction.Sent, "File: ${it.meta.name}")
                }
            }
        }
    }

    private fun onLocalClip() {        val clip = clipboard.primaryClip
        val item = clip?.takeIf { it.itemCount > 0 }?.getItemAt(0)
        if (item == null) {
            // Expected on stock Android 10+: a background app is not allowed to read the
            // clipboard. The LSPosed hook on ClipboardService is what lifts this.
            Log.d(TAG, "clipboard unreadable from the background")
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
            .setContentTitle("Conduit")
            .setContentText("Clipboard linked to the desktop")
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }
}
