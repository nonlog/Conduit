package com.conduit.sync

import android.net.Network
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import com.conduit.sync.proto.ClipImageChunk
import com.conduit.sync.proto.ClipImageHeader
import com.conduit.sync.proto.ClipText
import com.conduit.sync.proto.Envelope
import com.conduit.sync.proto.FileCancel
import com.conduit.sync.proto.FileChunk
import com.conduit.sync.proto.FileOffer
import com.conduit.sync.proto.FileResult
import com.conduit.sync.proto.Kind
import com.conduit.sync.proto.NotifAction
import com.conduit.sync.proto.PairRequest
import com.conduit.sync.proto.PairResponse
import com.conduit.sync.proto.SharedUrl
import com.conduit.sync.proto.WallpaperPreview
import java.net.InetSocketAddress
import java.net.InetAddress
import java.net.Inet4Address
import java.net.Socket
import java.net.SocketException
import java.util.concurrent.ConcurrentHashMap
import com.google.protobuf.ByteString
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicInteger

private const val TAG = "conduit.link"

/** Long enough that a healthy peer's 60 s keepalive always beats it. */
private const val READ_DEADLINE_MS = 150_000

/**
 * Once Noise is up, the relay path follows the desktop's slower keepalive. The desktop
 * pings every 240 s over the relay and this is the 2.5x read deadline that detects a
 * genuinely dead established tunnel without minute-by-minute radio wakes.
 */
private const val RELAY_READ_DEADLINE_MS = 600_000

/**
 * Before the desktop arrives, an explicit-role relay waiter has no userspace deadline.
 * The relay is deliberately designed to park a waiter for hours and uses kernel TCP
 * keepalive to reap dead sockets and refresh NAT. A ten-minute Android read timeout here
 * used to tear down a healthy parked socket and start the whole discovery/relay cycle
 * again even though nothing had changed. Socket.setSoTimeout(0) means block indefinitely;
 * network changes and an explicit disconnect still close the socket immediately.
 */
private const val RELAY_PARK_READ_DEADLINE_MS = 0

internal fun initialReadDeadlineMs(rendezvous: String?): Int =
    if (rendezvous == null) READ_DEADLINE_MS else RELAY_PARK_READ_DEADLINE_MS

private const val CONNECT_TIMEOUT_MS = 5_000

/** A hostname, not an essay. Long enough for any real machine name. */
private const val PEER_NAME_MAX = 64

/**
 * The role-aware relay preamble's magic. The role marker is deliberately outside base64url,
 * so a transition relay can distinguish this 48-byte form from the deployed 47-byte legacy
 * form without consuming any Noise byte. Mirrored in `relay/src/main.rs` and `wire.rs`.
 */
private val RELAY_MAGIC = "CDT1".toByteArray()
private const val RELAY_INITIATOR = '>'.code.toByte()

internal fun relayPreamble(rendezvous: String): ByteArray {
    require(rendezvous.length == ID_LEN && rendezvous.all(::relayIdChar)) {
        "relay rendezvous id must be $ID_LEN base64url characters"
    }
    return RELAY_MAGIC + byteArrayOf(RELAY_INITIATOR) + rendezvous.toByteArray(Charsets.US_ASCII)
}

private fun relayIdChar(c: Char): Boolean =
    c in 'A'..'Z' || c in 'a'..'z' || c in '0'..'9' || c == '-' || c == '_'

/**
 * Replaces Android/VPN benchmark-range fake DNS only when the user explicitly configured a
 * fallback for that relay. Without one, preserve the synthetic address: fake-IP VPNs route that
 * address back to the requested hostname, so rejecting it would reject every production Relay.
 */
internal fun relayTargetAddress(resolved: InetAddress, fallbackIp: String?): InetAddress {
    if (!isVpnFakeIp(resolved)) return resolved
    return fallbackIp?.let(InetAddress::getByName) ?: resolved
}

/** 198.18.0.0/15 is the benchmark range commonly used by Android fake-IP VPN DNS. */
internal fun isVpnFakeIp(address: InetAddress): Boolean {
    val bytes = address.address
    if (bytes.size != 4) return false
    val first = bytes[0].toInt() and 0xff
    val second = bytes[1].toInt() and 0xff
    return first == 198 && (second == 18 || second == 19)
}

/**
 * VPNs that disallow bypass reject binding an app socket to physical Wi-Fi with EPERM.
 * Only a resolved, previously selected LAN IPv4 candidate may retry without an explicit
 * network binding. This does not bypass the VPN: Android's ordinary routing and VPN policy
 * still decide whether the unbound connection is permitted to reach the desktop.
 */
internal fun isNetworkBindingDenied(error: SocketException): Boolean =
    generateSequence<Throwable>(error) { it.cause }
        .any { cause -> cause.message?.contains("EPERM") == true }

internal fun shouldRetryLanThroughSystemRoute(address: InetSocketAddress, error: SocketException): Boolean {
    val ip = address.address
    return ip is Inet4Address && !ip.isAnyLocalAddress && !ip.isLoopbackAddress &&
        !ip.isMulticastAddress && !isVpnFakeIp(ip) && isNetworkBindingDenied(error)
}

/**
 * Each connection owns its reader and receive state. One blocked scheduler thread owns all
 * Noise application writes, taking one bulk frame per turn. Controls can run between chunks.
 * Closing a socket interrupts both reads and writes without waiting for that scheduler.
 * Deadlines exist only while a file is active or awaiting publication, never on an idle link.
 */
class Link(
    private val privateKey: ByteArray,
    private val localDeviceId: String,
    private val localDeviceName: String,
    private val events: Events,
    private val openIncomingFile: (FileOffer) -> Files.Incoming? = { null },
) {

    interface Events {
        fun onTransfer(transfer: FileTransfer) {}
        fun onSendRejected(reason: String) {}
        fun onTransfersIdle() {}

        fun onState(state: LinkState, peer: String?)
        fun onText(text: String)

        /**
         * A complete image. [photo] is the backward-compatible non-clipboard marker;
         * [screenshot] distinguishes a screenshot from a camera photo on new peers.
         */
        fun onImage(png: ByteArray, photo: Boolean, screenshot: Boolean)

        /** A real bulk payload completed; used only for passive Relay quality learning. */
        fun onBulkTransfer(bytes: Long, elapsedMs: Long) {}

        /** A Windows toast action, already authenticated by the live Noise peer. */
        fun onNotificationAction(action: NotifAction) {}

        /**
         * Called after Noise XX exposes the peer static key but before any application data is
         * accepted. Return true only for the stored desktop or during an explicit LAN pairing
         * window. Pairing never learns a new peer through Relay.
         */
        fun authorizePeer(deviceId: String, viaRelay: Boolean, remotePairing: Boolean): Boolean

        /** True only while the user has explicitly opened this phone's LAN pairing window. */
        fun pairingRequested(): Boolean

        /** The authenticated desktop asked both sides to forget this relationship. */
        fun onPeerForgotten()

        /**
         * The peer's stable id, on every completed handshake. The service persists it
         * because it doubles as the relay rendezvous, and the relay is unusable until
         * one direct session has said what it is.
         */
        fun onPeer(deviceId: String)

        /**
         * The desktop's own name for itself, announced once per session.
         *
         * Separate from [onPeer] because the id is derived from a key and the name is
         * whatever the desktop is called — the phone shows the name and rendezvous with
         * the id, and only one of the two is worth reading out loud.
         */
        fun onPeerName(name: String)

        /** The desktop's cached phone-wallpaper hash. Empty means it has no preview yet. */
        fun onWallpaperHash(hash: ByteArray) {}

        /**
         * A session that was actually up has gone. A dial that never completed its
         * handshake deliberately does not call this: re-dialling on a refusal is how a
         * reconnect loop becomes a spin loop.
         */
        fun onSessionLost()
    }

    private val opened = AtomicLong()
    private val closed = AtomicLong()
    private val connectionLock = Any()
    private fun cleanUploads() {
        uploads.values.filter { !usable(it.owner) }.forEach {
            finish(it, TransferPhase.Failed, "Connection lost")
        }
    }
    private val sender = TransferScheduler(onMaintenance = ::cleanUploads, onTaskFailure = {
        Log.w(TAG, "transfer scheduler task failed", it)
        current?.let(::stop)
    })
    private val senderThread = Thread({ sender.run() }, "conduit-send").apply { start() }
    private val deadlines = Handler(Looper.getMainLooper())
    @Volatile private var current: Connection? = null
    @Volatile private var spent = false

    // Every callback/task captures its owner; stale work can only close its own socket.
    private class Connection(val generation: Long, val network: Network?, val socket: Socket = Socket()) {
        @Volatile var live: WireSession? = null
        @Volatile var authenticated = false
        @Volatile var transferProtocol = false
        val receiveLock = Any()
        var image: Images.Assembly? = null
        var imageStarted = 0L
        @Volatile var incoming: Files.Incoming? = null
        val receiveGate = TransferProgressGate()
        var incomingStarted = 0L
        var incomingUpdated = 0L
        var receiveTimeout: Runnable? = null
        val receivedIds = java.util.ArrayDeque<String>()
        val pendingResults = AtomicInteger()
    }

    private class Upload(val owner: Connection, val id: String, val what: String) {
        val receipt = TransferReceipt(id)
        var source: Files.Outgoing? = null
        var offered = false
        var name = what
        var total = 0L
        var written = 0L
        var started = 0L
        var timeout: Runnable? = null
        var publishedPhase: TransferPhase? = null
        val gate = TransferProgressGate()
        var work: (() -> Boolean)? = null
        var sourceFailure: String? = null
    }
    private val uploads = ConcurrentHashMap<String, Upload>()
    private val uploadLock = Any()

    val hasActiveFileTransfers: Boolean
        get() = uploads.isNotEmpty() || current?.let { it.incoming != null || it.pendingResults.get() > 0 } == true

    fun isBoundTo(network: Network): Boolean =
        current?.let { it.network == network && !it.socket.isClosed } == true

    val boundNetwork: Network? get() = current?.network

    private fun usable(owner: Connection): Boolean =
        current === owner && !owner.socket.isClosed && owner.live != null

    private fun report(upload: Upload) {
        val phase = upload.receipt.phase
        if (upload.publishedPhase == phase && !upload.gate.shouldPublish(upload.written, upload.total)) return
        upload.publishedPhase = phase
        events.onTransfer(FileTransfer(
            FileTransferDirection.ToDesktop, upload.name, upload.written, upload.total,
            upload.id, phase, upload.receipt.error,
            !phase.terminal && phase != TransferPhase.Cancelling && (!upload.offered || upload.owner.transferProtocol),
        ))
    }

    private fun finish(upload: Upload, phase: TransferPhase, reason: String = "") {
        if (upload.receipt.finish(phase, reason)) retire(upload)
    }

    private fun retire(upload: Upload) {
        upload.work?.let { sender.removeQueuedBulk(it); sender.resumeBulk(it) }
        upload.work = null
        upload.timeout?.let(deadlines::removeCallbacks)
        upload.timeout = null
        runCatching { upload.source?.close() }
        upload.source = null
        uploads.remove(upload.id, upload)
        report(upload)
        if (!hasActiveFileTransfers) events.onTransfersIdle()
    }

    private fun control(
        owner: Connection,
        priority: Boolean = false,
        critical: Boolean = priority,
        after: () -> Unit = {},
        task: (WireSession) -> Unit,
    ): Boolean {
        val accepted = sender.control(priority) {
            try {
                if (usable(owner)) task(requireNotNull(owner.live))
            } catch (e: OversizedMessageException) {
                Log.w(TAG, "local control payload refused before encryption", e)
                events.onSendRejected("Content is too large to synchronize in one message")
            } catch (e: Exception) {
                Log.w(TAG, "session ${owner.generation} control failed", e)
                stop(owner)
            } finally { after() }
        }
        if (!accepted) {
            events.onSendRejected("Send queue is full; try again")
            // Losing an ACK/cancel would leave the peers with incompatible state.
            if (critical) stop(owner)
            after()
        }
        return accepted
    }

    private fun wire(owner: Connection, kind: Kind, payload: ByteArray = byteArrayOf()) {
        check(usable(owner)) { "Transfer session ended" }
        requireNotNull(owner.live).send(kind, payload)
    }

    fun sendImage(
        what: String,
        photo: Boolean = false,
        screenshot: Boolean = false,
        emptyIsNoop: Boolean = false,
        load: () -> Images.Payload?,
    ) {
        val owner = current?.takeIf { it.authenticated && usable(it) }
        if (owner == null) {
            if (!emptyIsNoop) events.onSendRejected("$what: desktop is offline")
            return
        }
        var outgoing: Images.Outgoing? = null
        var size = 0L
        var started = 0L
        val accepted = sender.bulk {
            if (!usable(owner)) return@bulk false
            if (outgoing == null) {
                val result = runCatching { load() }
                val payload = result.getOrNull()
                if (payload == null) {
                    // MediaStore emits changes for unrelated/deleted/already-sent rows too.
                    // A capture observer returning "nothing new" is not a failed user send.
                    if (shouldReportMissingCapture(emptyIsNoop, result.isFailure)) {
                        events.onSendRejected("$what could not be read")
                    }
                    return@bulk false
                }
                if (payload.bytes.size !in 1..MAX_IMAGE) {
                    events.onSendRejected("$what is empty or too large")
                    return@bulk false
                }
                if (!usable(owner)) return@bulk false
                size = payload.bytes.size.toLong()
                started = SystemClock.elapsedRealtime()
                outgoing = Images.Outgoing(payload, photo, screenshot)
            }
            try {
                val done = requireNotNull(outgoing).step { kind, bytes -> wire(owner, kind, bytes) }
                if (done) events.onBulkTransfer(size, (SystemClock.elapsedRealtime() - started).coerceAtLeast(1L))
                !done
            } catch (e: Exception) {
                Log.w(TAG, "$what write failed", e)
                stop(owner)
                false
            }
        }
        if (!accepted) events.onSendRejected("$what: transfer queue is full; try again")
    }

    fun sendWallpaperPreview(desktopHash: ByteArray, load: () -> Wallpapers.Preview?) {
        val owner = current?.takeIf { it.authenticated } ?: return
        control(owner) { live ->
            val preview = runCatching { load() }.getOrNull() ?: return@control
            if (!usable(owner) || desktopHash.contentEquals(preview.sha256)) return@control
            val message = WallpaperPreview.newBuilder()
                .setJpeg(ByteString.copyFrom(preview.jpeg)).setSha256(ByteString.copyFrom(preview.sha256)).build()
            live.send(Kind.WALLPAPER_PREVIEW, message.toByteArray())
        }
    }

    // The ID exists before admission; every rejection is a visible terminal row.
    fun sendFile(what: String, open: () -> Files.Source?): String = synchronized(connectionLock) {
        val id = newTransferId()
        val owner = current?.takeIf { it.authenticated && usable(it) }
        if (owner == null) {
            events.onTransfer(FileTransfer(FileTransferDirection.ToDesktop, what, 0, 0, id,
                TransferPhase.Failed, "Desktop is offline", false))
            return@synchronized id
        }
        val upload = Upload(owner, id, what)
        val admitted = synchronized(uploadLock) {
            if (uploads.size >= 16) false else { uploads[id] = upload; true }
        }
        report(upload)
        val work = { uploadStep(upload, open) }
        upload.work = work
        if (!admitted || !sender.bulk(work)) {
            finish(upload, TransferPhase.Failed, "Transfer queue is full; try again")
        }
        id
    }

    private fun uploadStep(upload: Upload, open: () -> Files.Source?): Boolean {
        val owner = upload.owner
        if (!usable(owner)) { finish(upload, TransferPhase.Failed, "Connection lost"); return false }
        if (upload.receipt.phase.terminal) return false
        if (upload.receipt.phase == TransferPhase.WaitingConfirmation || upload.receipt.phase == TransferPhase.Cancelling) {
            sender.pauseBulk()
            return true
        }
        if (upload.source == null) {
            try {
                val source = requireNotNull(open()) { "Source is unavailable, empty, unknown-size or over 512 MB" }
                upload.name = source.meta.name.take(200)
                upload.total = source.meta.size
                upload.source = try {
                    Files.Outgoing(ByteString.copyFrom(transferIdBytes(upload.id)), source.stream, source.meta)
                } catch (e: Exception) { runCatching { source.stream.close() }; throw e }
                if (!usable(owner)) { finish(upload, TransferPhase.Failed, "Connection lost"); return false }
                upload.started = SystemClock.elapsedRealtime()
                upload.receipt.sending()
            } catch (e: Exception) {
                finish(upload, TransferPhase.Failed, e.message ?: "Could not read source")
                return false
            }
        }
        val source = requireNotNull(upload.source)
        if (!upload.offered) {
            try {
                wire(owner, Kind.FILE_OFFER, source.offer.toByteArray())
                upload.offered = true
                report(upload)
            } catch (e: Exception) {
                finish(upload, TransferPhase.Failed, "Connection lost")
                stop(owner)
                return false
            }
            return true
        }
        // Source errors are recoverable without taking clipboard or other files down.
        val chunk = try { source.next() } catch (e: Exception) {
            val reason = e.message ?: "Could not read source"
            cancelWire(upload, reason)
            if (owner.transferProtocol && usable(owner)) {
                upload.sourceFailure = reason
                upload.receipt.requestCancel(reason)
                report(upload)
                awaitReceipt(upload)
                return true
            }
            finish(upload, TransferPhase.Failed, reason)
            return false
        }
        try {
            wire(owner, Kind.FILE_CHUNK, chunk.toByteArray())
            upload.written = source.sent
            if (upload.written == upload.total) {
                upload.receipt.sent(owner.transferProtocol)
                runCatching { source.close() }
                upload.source = null
                events.onBulkTransfer(upload.total, (SystemClock.elapsedRealtime() - upload.started).coerceAtLeast(1L))
                report(upload)
                if (owner.transferProtocol) {
                    awaitReceipt(upload)
                } else retire(upload)
                return owner.transferProtocol
            }
            report(upload)
            return true
        } catch (e: Exception) {
            finish(upload, TransferPhase.Failed, "Connection lost")
            stop(owner)
            return false
        }
    }

    private fun awaitReceipt(upload: Upload) {
        sender.pauseBulk()
        if (upload.timeout != null) return
        val timeout = Runnable {
            control(upload.owner, priority = true) {
                if (!upload.receipt.phase.terminal) {
                    cancelWire(upload, "Receiver confirmation timed out")
                    finish(upload, TransferPhase.Failed,
                        upload.sourceFailure ?: "Receiver confirmation timed out; delivery is unconfirmed")
                }
            }
        }
        upload.timeout = timeout
        deadlines.postDelayed(timeout, TRANSFER_TIMEOUT_MS)
    }

    private fun cancelWire(upload: Upload, reason: String) {
        if (!upload.offered || !usable(upload.owner)) return
        if (!upload.owner.transferProtocol) {
            // shortcut: legacy peers cannot discard one partial file; source failure closes the session.
            stop(upload.owner)
            return
        }
        val cancel = FileCancel.newBuilder().setTransferId(ByteString.copyFrom(transferIdBytes(upload.id)))
            .setReason(reason.take(300)).build()
        try { wire(upload.owner, Kind.FILE_CANCEL, cancel.toByteArray()) }
        catch (e: Exception) { stop(upload.owner) }
    }

    fun cancelTransfer(id: String) {
        val owner = current ?: return
        control(owner, priority = true, critical = false) {
            val upload = uploads[id]?.takeIf { it.owner === owner }
            if (upload != null) {
                if (upload.offered && !owner.transferProtocol) {
                    events.onSendRejected("This desktop cannot cancel an active transfer")
                } else {
                    cancelWire(upload, "Cancelled on phone")
                    if (!upload.offered) {
                        finish(upload, TransferPhase.Cancelled)
                    } else if (usable(owner)) {
                        upload.receipt.requestCancel("Waiting for the desktop to stop this transfer")
                        report(upload)
                        awaitReceipt(upload)
                    }
                }
            } else synchronized(owner.receiveLock) {
                val rx = owner.incoming
                if (rx != null && transferIdString(rx.id) == id && owner.transferProtocol) {
                    val cancel = FileCancel.newBuilder().setTransferId(ByteString.copyFrom(rx.id))
                        .setReason("Cancelled on phone").build()
                    wire(owner, Kind.FILE_CANCEL, cancel.toByteArray())
                    endIncoming(owner, TransferPhase.Cancelled, "", result = true)
                }
            }
        }
    }

    fun connect(address: InetSocketAddress, network: Network? = null) = dial(address, null, null, network)
    fun connectVia(relay: InetSocketAddress, rendezvous: String, fallbackIp: String? = null) =
        dial(relay, rendezvous, fallbackIp, null)

    private fun dial(address: InetSocketAddress, rendezvous: String?, fallbackIp: String?, network: Network?) {
        synchronized(connectionLock) {
            if (spent || current?.socket?.isClosed == false) return
            val owner = Connection(opened.incrementAndGet(), network)
            current = owner
            events.onState(LinkState.Discovering, null)
            Thread({ pump(owner, address, rendezvous, fallbackIp, network) }, "conduit-recv").start()
        }
    }

    // Closing here, never queued behind bulk or a blocked socket write.
    fun disconnect() { current?.let(::stop) }

    /** Atomic optional handoff: an arriving offer cannot race the active-transfer check. */
    fun disconnectIfTransfersIdle(): Boolean = synchronized(connectionLock) {
        val owner = current ?: return@synchronized true
        synchronized(owner.receiveLock) {
            synchronized(uploadLock) {
                if (hasActiveFileTransfers) false else { stop(owner); true }
            }
        }
    }

    private fun stop(owner: Connection) { runCatching { owner.socket.close() } }

    fun sendText(text: String) {
        val clip = ClipText.newBuilder().setText(text).setMime("text/plain")
            .setTimestampMs(System.currentTimeMillis()).build()
        send(Kind.CLIP_TEXT, clip.toByteArray(), "clip")
    }

    fun sendSharedUrl(url: String, title: String, sourceDevice: String) {
        val shared = SharedUrl.newBuilder().setUrl(url).setTitle(title.take(512))
            .setTimestampMs(System.currentTimeMillis()).setSourceDevice(sourceDevice.take(64)).build()
        send(Kind.SHARED_URL, shared.toByteArray(), "shared url")
    }

    fun send(kind: Kind, payload: ByteArray, what: String) = send(kind, what) { payload }
    fun send(kind: Kind, what: String, build: () -> ByteArray) {
        val owner = current?.takeIf { it.authenticated && usable(it) }
        if (owner == null) { events.onSendRejected("$what: desktop is offline"); return }
        control(owner) { live ->
            val payload = runCatching { build() }.getOrElse {
                events.onSendRejected("$what could not be prepared")
                return@control
            }
            if (usable(owner)) live.send(kind, payload)
        }
    }

    fun forgetPeer() {
        val owner = current ?: return
        control(owner, priority = true) { live ->
            try { live.send(Kind.PAIR_RESPONSE, PairResponse.newBuilder().setAccept(false).build().toByteArray()) }
            finally { stop(owner) }
        }
    }

    fun close() {
        synchronized(connectionLock) { spent = true; current?.let(::stop) }
        sender.close()
    }

    private fun pump(owner: Connection, address: InetSocketAddress, rendezvous: String?, fallbackIp: String?, network: Network?) {
        var established = false
        try {
            owner.socket.use { sock ->
                val target = if (address.isUnresolved) {
                    val resolved = InetAddress.getByName(address.hostString)
                    InetSocketAddress(if (rendezvous != null) relayTargetAddress(resolved, fallbackIp) else resolved, address.port)
                } else address
                if (rendezvous == null && network != null) {
                    try {
                        network.bindSocket(sock)
                    } catch (denied: SocketException) {
                        if (!shouldRetryLanThroughSystemRoute(target, denied)) throw denied
                        // The VPN disallows bypassing it to choose the physical Wi-Fi. Keep this
                        // exact LAN target, but let Android/VPN route an ordinary app socket.
                        // There is no fwmark/source-IP bypass or change to device VPN settings.
                        Log.i(TAG, "VPN denied physical LAN socket binding; trying system-routed LAN")
                    }
                }
                sock.tcpNoDelay = true
                sock.keepAlive = true
                sock.soTimeout = initialReadDeadlineMs(rendezvous)
                sock.connect(target, CONNECT_TIMEOUT_MS)
                if (rendezvous != null) {
                    sock.getOutputStream().apply { write(relayPreamble(rendezvous)); flush() }
                    if (current === owner) events.onState(LinkState.Waiting, null)
                }
                val live = WireSession.handshake(sock.getInputStream(), sock.getOutputStream(), privateKey, initiator = true)
                owner.live = live
                val remoteId = Identity.deviceId(live.peerStatic)
                val viaRelay = rendezvous != null
                if (viaRelay) sock.soTimeout = RELAY_READ_DEADLINE_MS
                val hello = PairRequest.newBuilder().setDeviceId(localDeviceId)
                    .setDeviceName(localDeviceName.take(PEER_NAME_MAX))
                    .setPairing(events.pairingRequested()).setTransferProtocol(1).build()
                control(owner, priority = true) { it.send(Kind.PAIR_REQUEST, hello.toByteArray()) }
                val first = live.recv()
                if (first.kind != Kind.PAIR_REQUEST) throw SecurityException("Desktop did not send pairing hello")
                val desktopHello = PairRequest.parseFrom(first.payload)
                if (desktopHello.deviceId.isNotEmpty() && desktopHello.deviceId != remoteId) {
                    throw SecurityException("Desktop hello does not match its Noise identity")
                }
                if (!events.authorizePeer(remoteId, viaRelay, desktopHello.pairing)) {
                    throw SecurityException("Desktop is not paired; open pairing on both devices")
                }
                synchronized(connectionLock) {
                    if (!usable(owner)) return
                    owner.transferProtocol = desktopHello.transferProtocol >= 1
                    owner.authenticated = true
                    events.onPeer(remoteId)
                    desktopHello.deviceName.take(PEER_NAME_MAX).takeIf { it.isNotBlank() }?.let(events::onPeerName)
                    established = true
                    events.onState(LinkState.Connected, Identity.fingerprint(live.peerStatic))
                    events.onWallpaperHash(desktopHello.wallpaperHash.toByteArray())
                }
                while (usable(owner)) {
                    val envelope = live.recv()
                    if (usable(owner)) {
                        try { dispatch(owner, envelope) }
                        catch (malformed: com.google.protobuf.InvalidProtocolBufferException) {
                            // The encrypted frame is already consumed. Invalid inner data must not
                            // break an unrelated transfer or desynchronise the Noise stream.
                            Log.w(TAG, "malformed ${envelope.kind} payload dropped", malformed)
                        }
                    }
                }
            }
        } catch (t: Exception) {
            Log.w(TAG, "session ${owner.generation} ended: ${t.javaClass.name}: ${t.message}", t)
        } finally {
            stop(owner)
            synchronized(owner.receiveLock) {
                if (owner.incoming != null) endIncoming(owner, TransferPhase.Failed, "Connection lost")
                owner.image = null
            }
            sender.requestMaintenance()
            owner.live = null
            synchronized(connectionLock) {
                if (current === owner) {
                    current = null
                    events.onState(LinkState.Idle, null)
                    if (established && !spent) events.onSessionLost()
                }
            }
            Log.i(TAG, "session ${owner.generation} closed: opened=${opened.get()} closed=${closed.incrementAndGet()}")
        }
    }

    private fun dispatch(owner: Connection, envelope: Envelope) {
        when (envelope.kind) {
            Kind.PING -> control(owner, priority = true) { it.send(Kind.PONG) }
            Kind.PONG -> {}
            Kind.PAIR_REQUEST -> {
                val hello = PairRequest.parseFrom(envelope.payload)
                hello.deviceName.take(PEER_NAME_MAX).takeIf { it.isNotBlank() }?.let(events::onPeerName)
                events.onWallpaperHash(hello.wallpaperHash.toByteArray())
            }
            Kind.PAIR_RESPONSE -> {
                if (!PairResponse.parseFrom(envelope.payload).accept) {
                    events.onPeerForgotten()
                    throw SecurityException("Desktop removed this phone")
                }
            }
            Kind.CLIP_TEXT -> events.onText(ClipText.parseFrom(envelope.payload).text)
            Kind.NOTIF_ACTION -> events.onNotificationAction(NotifAction.parseFrom(envelope.payload))
            Kind.CLIP_IMAGE_HEADER -> {
                if (owner.image == null) {
                    owner.image = runCatching { Images.Assembly.begin(ClipImageHeader.parseFrom(envelope.payload)) }.getOrNull()
                    owner.imageStarted = SystemClock.elapsedRealtime()
                }
            }
            Kind.CLIP_IMAGE_CHUNK -> {
                val chunk = ClipImageChunk.parseFrom(envelope.payload)
                val assembly = owner.image ?: return
                if (!assembly.matches(chunk)) return
                try {
                    assembly.push(chunk)?.let { bytes ->
                        owner.image = null
                        events.onBulkTransfer(bytes.size.toLong(), (SystemClock.elapsedRealtime() - owner.imageStarted).coerceAtLeast(1L))
                        events.onImage(bytes, assembly.photo, assembly.screenshot)
                    }
                } catch (e: Exception) { owner.image = null }
            }
            Kind.FILE_OFFER -> receiveOffer(owner, FileOffer.parseFrom(envelope.payload))
            Kind.FILE_CHUNK -> receiveChunk(owner, FileChunk.parseFrom(envelope.payload))
            Kind.FILE_RESULT -> {
                val result = FileResult.parseFrom(envelope.payload)
                if (!owner.transferProtocol) return
                control(owner, priority = true) {
                    val id = transferIdString(result.transferId.toByteArray())
                    val upload = uploads[id]?.takeIf { it.owner === owner } ?: return@control
                    if (upload.sourceFailure != null) {
                        if (!result.success) finish(upload, TransferPhase.Failed, requireNotNull(upload.sourceFailure))
                    } else if (upload.receipt.result(id, result.success, result.cancelled, result.error)) retire(upload)
                }
            }
            Kind.FILE_CANCEL -> {
                if (!owner.transferProtocol) return
                val cancel = FileCancel.parseFrom(envelope.payload)
                val id = transferIdString(cancel.transferId.toByteArray())
                control(owner, priority = true) {
                    uploads[id]?.takeIf { it.owner === owner }?.let {
                        if (it.offered) {
                            it.receipt.requestCancel(cancel.reason)
                            report(it)
                            awaitReceipt(it)
                        } else finish(it, TransferPhase.Cancelled, cancel.reason)
                    }
                }
                synchronized(owner.receiveLock) {
                    if (owner.incoming?.id?.contentEquals(cancel.transferId.toByteArray()) == true) {
                        endIncoming(owner, TransferPhase.Cancelled, cancel.reason, result = true)
                    }
                }
            }
            else -> Log.d(TAG, "unhandled kind ${envelope.kind}")
        }
    }

    private fun receiveOffer(owner: Connection, offer: FileOffer) = synchronized(owner.receiveLock) {
        if (!usable(owner)) return@synchronized
        val id = transferIdString(offer.transferId.toByteArray())
        if (owner.incoming?.id?.contentEquals(offer.transferId.toByteArray()) == true) return@synchronized
        if (owner.incoming != null || id in owner.receivedIds || uploads.containsKey(id)) {
            queueFileResult(owner, offer.transferId.toByteArray(), offer.name, false, "Receiver busy or stale transfer id")
            return@synchronized
        }
        try {
            val rx = requireNotNull(openIncomingFile(offer)) { "Phone receiver is unavailable" }
            owner.incoming = rx
            owner.incomingStarted = SystemClock.elapsedRealtime()
            owner.incomingUpdated = owner.incomingStarted
            owner.receiveGate.reset()
            reportIncoming(owner, rx, 0, TransferPhase.Receiving)
            armReceiveTimeout(owner)
        } catch (e: Exception) {
            queueFileResult(owner, offer.transferId.toByteArray(), offer.name, false, e.message ?: "Could not receive")
        }
    }

    private fun receiveChunk(owner: Connection, chunk: FileChunk) = synchronized(owner.receiveLock) {
        if (!usable(owner)) return@synchronized
        val rx = owner.incoming ?: return@synchronized
        if (!rx.id.contentEquals(chunk.transferId.toByteArray())) return@synchronized
        try {
            val progress = rx.push(chunk)
            owner.incomingUpdated = SystemClock.elapsedRealtime()
            reportIncoming(owner, rx, progress.transferred, TransferPhase.Receiving)
            if (progress.complete) {
                events.onBulkTransfer(progress.total, (SystemClock.elapsedRealtime() - owner.incomingStarted).coerceAtLeast(1L))
                endIncoming(owner, TransferPhase.Completed, "", result = true)
            }
        } catch (e: Exception) {
            endIncoming(owner, TransferPhase.Failed, e.message ?: "Could not publish file", result = true)
        }
    }

    private fun reportIncoming(owner: Connection, rx: Files.Incoming, bytes: Long, phase: TransferPhase, error: String = "") {
        if (phase == TransferPhase.Receiving && !owner.receiveGate.shouldPublish(bytes, rx.total)) return
        events.onTransfer(FileTransfer(FileTransferDirection.ToPhone, rx.name, bytes, rx.total,
            transferIdString(rx.id), phase, error.take(300), !phase.terminal && owner.transferProtocol))
    }

    private fun endIncoming(owner: Connection, phase: TransferPhase, error: String, result: Boolean = false) {
        val rx = owner.incoming ?: return
        owner.receiveTimeout?.let(deadlines::removeCallbacks)
        owner.receiveTimeout = null
        owner.incoming = null
        rx.close()
        owner.receivedIds.addLast(transferIdString(rx.id))
        while (owner.receivedIds.size > 32) owner.receivedIds.removeFirst()
        reportIncoming(owner, rx, if (phase == TransferPhase.Completed) rx.total else rx.transferred, phase, error)
        if (result) queueFileResult(owner, rx.id, rx.name, phase == TransferPhase.Completed, error, phase == TransferPhase.Cancelled)
        if (current === owner && !hasActiveFileTransfers) events.onTransfersIdle()
    }

    private fun armReceiveTimeout(owner: Connection) {
        val expected = owner.incoming ?: return
        val timeout = object : Runnable {
            override fun run() {
                control(owner, priority = true) {
                    synchronized(owner.receiveLock) {
                        if (owner.incoming === expected) {
                            val remaining = TRANSFER_TIMEOUT_MS - (SystemClock.elapsedRealtime() - owner.incomingUpdated)
                            if (remaining > 0) deadlines.postDelayed(this, remaining)
                            else endIncoming(owner, TransferPhase.Failed, "Receive timed out", result = true)
                        }
                    }
                }
            }
        }
        owner.receiveTimeout = timeout
        deadlines.postDelayed(timeout, TRANSFER_TIMEOUT_MS)
    }

    private fun queueFileResult(owner: Connection, id: ByteArray, name: String, success: Boolean, error: String, cancelled: Boolean = false) {
        if (id.size !in 1..64) return
        val result = FileResult.newBuilder().setTransferId(ByteString.copyFrom(id))
            .setSuccess(success).setName(name.take(200)).setError(error.take(300)).setCancelled(cancelled).build()
        owner.pendingResults.incrementAndGet()
        control(owner, priority = true, after = {
            owner.pendingResults.decrementAndGet()
            if (usable(owner) && !hasActiveFileTransfers) events.onTransfersIdle()
        }) { it.send(Kind.FILE_RESULT, result.toByteArray()) }
    }

    companion object { private const val TRANSFER_TIMEOUT_MS = 120_000L }
}
