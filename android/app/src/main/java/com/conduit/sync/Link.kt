package com.conduit.sync

import android.os.SystemClock
import android.util.Log
import com.conduit.sync.proto.ClipImageChunk
import com.conduit.sync.proto.ClipImageHeader
import com.conduit.sync.proto.ClipText
import com.conduit.sync.proto.Envelope
import com.conduit.sync.proto.FileChunk
import com.conduit.sync.proto.FileOffer
import com.conduit.sync.proto.FileResult
import com.conduit.sync.proto.Kind
import com.conduit.sync.proto.PairRequest
import java.net.InetSocketAddress
import java.net.InetAddress
import java.net.Socket
import java.net.UnknownHostException
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

private const val TAG = "conduit.link"

/** Long enough that a healthy peer's 60 s keepalive always beats it. */
private const val READ_DEADLINE_MS = 150_000

/**
 * The relay path's deadline, following the desktop's slower keepalive there. A ping a
 * minute is free on Wi-Fi and a radio wake a minute on cellular, so the desktop pings
 * every 240 s over the relay and this is the 2.5x that follows. The cost is that a
 * tunnel dying without a FIN goes unnoticed for up to ten minutes; the benefit is that
 * an idle phone on mobile data wakes its radio four times an hour instead of sixty.
 *
 * It doubles as the parking deadline: a phone that reaches the relay before the desktop
 * sits in exactly this blocked read until it is spliced.
 */
private const val RELAY_READ_DEADLINE_MS = 600_000

private const val CONNECT_TIMEOUT_MS = 5_000
private const val JOIN_TIMEOUT_MS = 2_000L

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
 * Replaces only Android/VPN benchmark-range fake DNS with the relay's known public fallback.
 * A normal public answer is returned unchanged, so the hostname remains authoritative whenever
 * the resolver is honest.
 */
internal fun relayTargetAddress(resolved: InetAddress, fallbackIp: String?): InetAddress {
    if (!isVpnFakeIp(resolved)) return resolved
    return fallbackIp?.let(InetAddress::getByName)
        ?: throw UnknownHostException(
            "relay resolved to VPN fake IP ${resolved.hostAddress}; no fallback configured",
        )
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
 * The one transport session this app ever has, and the two threads that own it.
 *
 * The whole design exists to make one invariant checkable: `opened == closed`. A
 * connection is a [Socket] inside a `use` block on a single reader thread, so every
 * exit — return, throw, or someone else closing the socket underneath it — runs the
 * same teardown. Nothing is reference-counted and nothing is destroyed from a
 * finalizer, which is how the native session leak this app replaces happened.
 *
 * Thread rules, which are the reason there are no locks here:
 *  - the reader thread is the only caller of [WireSession.recv]
 *  - the [sender] thread is the only caller of [WireSession.send], including the PONG
 *    the reader owes a PING, which it posts rather than writes
 *
 * Idle cost is two blocked syscalls: `recvfrom` on the reader and a queue wait on the
 * sender. No timers, no wakeups, no polling.
 */
class Link(
    private val privateKey: ByteArray,
    private val events: Events,
    private val openIncomingFile: (FileOffer) -> Files.Incoming? = { null },
) {

    interface Events {
        fun onState(state: LinkState, peer: String?)
        fun onText(text: String)

        /**
         * A complete image. [photo] is the backward-compatible non-clipboard marker;
         * [screenshot] distinguishes a screenshot from a camera photo on new peers.
         */
        fun onImage(png: ByteArray, photo: Boolean, screenshot: Boolean)

        /** Exact byte progress for one file transfer. */
        fun onFileProgress(
            name: String,
            direction: FileTransferDirection,
            transferred: Long,
            total: Long,
        ) {}

        /** The file reached its receiver and was fully published. */
        fun onFileComplete(name: String, direction: FileTransferDirection) {}

        /** A transfer that had started did not finish. */
        fun onFileFailed(name: String, direction: FileTransferDirection) {}

        /** A real bulk payload completed; used only for passive Relay quality learning. */
        fun onBulkTransfer(bytes: Long, elapsedMs: Long) {}

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

        /**
         * A session that was actually up has gone. A dial that never completed its
         * handshake deliberately does not call this: re-dialling on a refusal is how a
         * reconnect loop becomes a spin loop.
         */
        fun onSessionLost()
    }

    private val opened = AtomicLong()
    private val closed = AtomicLong()

    /**
     * Serializes connect, disconnect and every outbound frame. Created once per
     * service, not per connection, so reconnect churn cannot grow the thread count.
     *
     * ponytail: discard-oldest on a full queue. Notifications share it with clips, so
     * 64 rather than 16: a chat app catching up after a reconnect can post a burst no
     * human clipboard ever produces. Dropping the oldest is still right — a stale
     * notification mirror is worth less than a fresh one.
     */
    private val sender = ThreadPoolExecutor(
        1, 1, 0, TimeUnit.MILLISECONDS, ArrayBlockingQueue(64),
        { r -> Thread(r, "conduit-send") },
        ThreadPoolExecutor.DiscardOldestPolicy(),
    )

    /** Held only so another thread can close it; that close is what stops the reader. */
    @Volatile private var socket: Socket? = null
    @Volatile private var session: WireSession? = null
    private var reader: Thread? = null
    /**
     * A reader-thread PING that still needs a PONG from the single sender thread.
     *
     * Normally the queued sender task answers it immediately. A large file/image is different:
     * that task intentionally owns the sender executor for the whole transfer, so it services
     * this flag between chunks instead. That preserves the single Noise writer while preventing
     * a 512 MiB transfer from starving a heartbeat behind thousands of queued bytes.
     */
    @Volatile private var pongPending = false

    /**
     * The image being reassembled. Touched only by the reader thread, so it is neither
     * volatile nor locked, and it is cleared on teardown with the session.
     */
    private var incoming: Images.Assembly? = null
    private var incomingImageStartedMs = 0L

    /** The one desktop file being streamed into MediaStore Downloads on the reader thread. */
    private var incomingFile: Files.Incoming? = null
    private var incomingFileStartedMs = 0L

    /**
     * Reads [uri] on the sender thread and queues it as chunks.
     *
     * The read is a binder call into whichever app owns the provider, so it must not run
     * on the main thread — and the clipboard listener that notices a copied image runs
     * there. Handing over a lambda keeps that work on the one thread already dedicated
     * to outbound frames, and keeps this class free of Android's content APIs.
     */
    fun sendImage(
        what: String,
        photo: Boolean = false,
        screenshot: Boolean = false,
        load: () -> Images.Payload?,
    ) =
        sender.execute {
            val live = session
            if (live == null) {
                Log.d(TAG, "$what dropped, no session")
                return@execute
            }
            val payload = runCatching { load() }
                .onFailure { Log.w(TAG, "$what could not be read", it) }
                .getOrNull()
            if (payload == null || payload.bytes.isEmpty()) return@execute
            try {
                val started = SystemClock.elapsedRealtime()
                Images.send(live, payload, photo, screenshot) { sendPendingPong(live) }
                events.onBulkTransfer(
                    payload.bytes.size.toLong(),
                    (SystemClock.elapsedRealtime() - started).coerceAtLeast(1L),
                )
            } catch (e: Exception) {
                // Same rule as a failed text write: the socket is gone, so let the reader
                // notice and unwind rather than half-finishing the transfer.
                Log.w(TAG, "$what write failed", e)
                teardown()
            }
        }

    /**
     * Streams one file to the desktop on the sender thread.
     *
     * [open] is a lambda for the same reason [sendImage]'s [load] is, and more so: resolving
     * the file's size and opening its stream are both binder calls into whichever app owns
     * the URI, and the caller is a share intent arriving on the main thread. [what] only
     * names the transfer in the log until the real name is known.
     *
     * The stream is closed on every exit, including the throw a short read raises.
     */
    fun sendFile(what: String, open: () -> Files.Source?) = sender.execute {
        val live = session
        if (live == null) {
            Log.d(TAG, "$what dropped, no session")
            return@execute
        }
        val source = runCatching { open() }
            .onFailure { Log.w(TAG, "could not read $what", it) }
            .getOrNull() ?: return@execute
        try {
            val started = SystemClock.elapsedRealtime()
            source.stream.use { input ->
                Files.send(live, input, source.meta) { transferred, total ->
                    sendPendingPong(live)
                    events.onFileProgress(
                        source.meta.name,
                        FileTransferDirection.ToDesktop,
                        transferred,
                        total,
                    )
                }
            }
            events.onBulkTransfer(
                source.meta.size,
                (SystemClock.elapsedRealtime() - started).coerceAtLeast(1L),
            )
            events.onFileComplete(source.meta.name, FileTransferDirection.ToDesktop)
        } catch (e: Exception) {
            // Same rule as an image: the offer is already out and the desktop is committed
            // to a total, so there is nothing to salvage. Let the session unwind — the
            // desktop's transfer dies with it and deletes its own partial file.
            Log.w(TAG, "${source.meta.name} transfer failed", e)
            events.onFileFailed(source.meta.name, FileTransferDirection.ToDesktop)
            teardown()
        }
    }

    /** Dials on the reader thread. A no-op while a connection is already up. */
    fun connect(address: InetSocketAddress) = dial(address, null, null)
    /**
     * Parks at the relay under [rendezvous] and waits to be spliced onto the desktop.
     *
     * Same reader thread, same session, same teardown as a LAN dial — the relay only
     * changes which address is dialled and adds a 48-byte role-aware preamble, so nothing
     * about the `opened == closed` invariant depends on which path was taken.
     */
    fun connectVia(relay: InetSocketAddress, rendezvous: String, fallbackIp: String? = null) =
        dial(relay, rendezvous, fallbackIp)

    private fun dial(address: InetSocketAddress, rendezvous: String?, fallbackIp: String?) = sender.execute {
        if (reader?.isAlive == true) {
            Log.d(TAG, "already connected, ignoring dial to $address")
            return@execute
        }
        events.onState(LinkState.Discovering, null)
        reader = Thread({ pump(address, rendezvous, fallbackIp) }, "conduit-recv").apply { start() }
    }

    /**
     * Suspends the session without destroying this object. Called on a network change,
     * where the socket is dead but the app is not: keeping [Link] alive is what stops a
     * reconnect from allocating a new set of everything.
     */
    fun disconnect() = sender.execute { teardown() }

    fun sendText(text: String) {
        val clip = ClipText.newBuilder()
            .setText(text)
            .setMime("text/plain")
            .setTimestampMs(System.currentTimeMillis())
            .build()
        send(Kind.CLIP_TEXT, clip.toByteArray(), "clip")
    }

    /**
     * Queues one already-encoded frame. [what] only names the payload in the log, so a
     * dropped notification is distinguishable from a dropped clip.
     */
    fun send(kind: Kind, payload: ByteArray, what: String) = sender.execute {
        val live = session
        if (live == null) {
            Log.d(TAG, "$what dropped, no session")
            return@execute
        }
        try {
            live.send(kind, payload)
        } catch (e: Exception) {
            // A failed write means the socket is gone; let the reader notice and unwind.
            Log.w(TAG, "$what write failed", e)
            teardown()
        }
    }

    /** Final shutdown. After this the object is spent — the service is going away too. */
    fun close() {
        sender.execute { teardown() }
        sender.shutdown()
        sender.awaitTermination(JOIN_TIMEOUT_MS, TimeUnit.MILLISECONDS)
        Log.i(TAG, "link closed: opened=${opened.get()} closed=${closed.get()}")
    }

    /** Runs on the sender thread, so it never races [connect]. */
    private fun teardown() {
        // Closing under the blocked read is the only way to interrupt it.
        socket?.let { runCatching { it.close() } }
        reader?.join(JOIN_TIMEOUT_MS)
        reader = null
        pongPending = false
    }

    private fun pump(address: InetSocketAddress, rendezvous: String?, fallbackIp: String?) {
        val count = opened.incrementAndGet()
        Log.i(TAG, "session $count opened: opened=$count closed=${closed.get()}")
        var established = false
        try {
            Socket().use { sock ->
                socket = sock
                sock.tcpNoDelay = true
                sock.keepAlive = true
                // A deadline, not a poll: the kernel wakes nobody until it expires.
                sock.soTimeout =
                    if (rendezvous == null) READ_DEADLINE_MS else RELAY_READ_DEADLINE_MS
                // The relay arrives as a hostname, and resolving it blocks. This is the
                // one thread here that is allowed to, so it is resolved here rather than
                // on the connectivity callback that asked for the dial. Some Android VPNs
                // use 198.18/15 fake-IP DNS. That mapping can become a dead local TCP sink
                // during an underlying network handover, so only that unmistakable result
                // is replaced with the relay's pinned public fallback. The actual socket is
                // still an ordinary Socket, so Android/VPN routing remains in force.
                val target = if (address.isUnresolved) {
                    val host = address.hostString
                    val resolved = resolve(host)
                    val targetAddress = if (rendezvous != null) {
                        val fallback = relayTargetAddress(resolved, fallbackIp)
                        if (fallback != resolved) {
                        Log.i(
                            TAG,
                            "relay DNS $host -> fake ${resolved.hostAddress}; using ${fallback.hostAddress}",
                        )
                        }
                        fallback
                    } else {
                        resolved
                    }
                    InetSocketAddress(targetAddress, address.port)
                } else {
                    address
                }
                sock.connect(target, CONNECT_TIMEOUT_MS)
                if (rendezvous != null) {
                    // 48 bytes naming our initiator role and the rendezvous, then the relay
                    // is a pipe and never looks at this stream again. The transition relay
                    // still accepts old 47-byte clients; upgraded clients are explicit.
                    sock.getOutputStream().apply {
                        write(relayPreamble(rendezvous))
                        flush()
                    }
                    Log.i(TAG, "session $count parked at $target as ${rendezvous.take(12)}")
                }

                val live = WireSession.handshake(
                    sock.getInputStream(), sock.getOutputStream(), privateKey, initiator = true,
                )
                session = live
                established = true
                val peer = Identity.fingerprint(live.peerStatic)
                Log.i(TAG, "session $count up to $target, peer $peer")
                events.onPeer(Identity.deviceId(live.peerStatic))
                events.onState(LinkState.Connected, peer)

                while (true) dispatch(live.recv())
            }
        } catch (t: Throwable) {
            // Some OEM log pipelines drop Throwable stack continuations from logcat. Keep the
            // class/message in the first line so field diagnostics still say what failed.
            Log.w(TAG, "session $count ended: ${t.javaClass.name}: ${t.message}", t)
        } finally {
            session = null
            socket = null
            // A partial image dies with the session that was carrying it, so the next
            // one never inherits a half-filled buffer.
            incoming = null
            incomingImageStartedMs = 0L
            incomingFile?.let {
                events.onFileFailed(it.name, FileTransferDirection.ToPhone)
                it.close()
            }
            incomingFile = null
            incomingFileStartedMs = 0L
            events.onState(LinkState.Idle, null)
            Log.i(TAG, "session $count closed: opened=$count closed=${closed.incrementAndGet()}")
            if (established) events.onSessionLost()
        }
    }

    private fun resolve(host: String): InetAddress = try {
        InetAddress.getByName(host)
    } catch (t: Throwable) {
        Log.w(TAG, "relay DNS $host failed: ${t.javaClass.name}: ${t.message}")
        throw t
    }

    private fun dispatch(envelope: Envelope) {
        when (envelope.kind) {
            // Posted, not written: only the sender thread may touch the send counter.
            Kind.PING -> queuePong()
            Kind.PONG -> {}
            // Capped, because it is peer-supplied text on its way to a launcher shortcut
            // label and a notification. A desktop is not hostile, but a relay session is
            // reachable by anything that guesses a rendezvous.
            Kind.PAIR_REQUEST -> PairRequest.parseFrom(envelope.payload).deviceName
                .take(PEER_NAME_MAX)
                .takeIf { it.isNotBlank() }
                ?.let { events.onPeerName(it) }
            Kind.CLIP_TEXT -> events.onText(ClipText.parseFrom(envelope.payload).text)
            // Reassembly state lives on this thread and nowhere else, so it needs no
            // lock and cannot outlive the session that is filling it.
            Kind.CLIP_IMAGE_HEADER -> {
                incoming = runCatching {
                    Images.Assembly.begin(ClipImageHeader.parseFrom(envelope.payload))
                }.onFailure { Log.w(TAG, "refused an image header", it) }.getOrNull()
                incomingImageStartedMs = if (incoming != null) SystemClock.elapsedRealtime() else 0L
            }

            Kind.CLIP_IMAGE_CHUNK -> {
                val assembly = incoming
                if (assembly == null) {
                    Log.w(TAG, "image chunk with no header, dropped")
                    return
                }
                // A malformed transfer drops the image, never the session: the desktop's
                // clipboard must not be able to disconnect the phone.
                runCatching { assembly.push(ClipImageChunk.parseFrom(envelope.payload)) }
                    .onFailure {
                        Log.w(TAG, "image transfer dropped", it)
                        incoming = null
                        incomingImageStartedMs = 0L
                    }
                    .getOrNull()
                    ?.let { png ->
                        incoming = null
                        val started = incomingImageStartedMs
                        incomingImageStartedMs = 0L
                        if (started > 0L) {
                            events.onBulkTransfer(
                                png.size.toLong(),
                                (SystemClock.elapsedRealtime() - started).coerceAtLeast(1L),
                            )
                        }
                        Log.i(
                            TAG,
                            "image in: ${png.size} B, photo=${assembly.photo} " +
                                "screenshot=${assembly.screenshot}",
                        )
                        events.onImage(png, assembly.photo, assembly.screenshot)
                    }
            }

            Kind.FILE_OFFER -> {
                val offer = FileOffer.parseFrom(envelope.payload)
                incomingFile?.let {
                    events.onFileFailed(it.name, FileTransferDirection.ToPhone)
                    it.close()
                }
                incomingFile = runCatching { openIncomingFile(offer) }
                    .onSuccess { rx ->
                        if (rx != null) {
                            Log.i(
                                TAG,
                                "file in: ${rx.name}, ${offer.totalBytes} B, ${offer.chunkCount} chunks",
                            )
                            events.onFileProgress(
                                rx.name,
                                FileTransferDirection.ToPhone,
                                0L,
                                offer.totalBytes,
                            )
                            incomingFileStartedMs = SystemClock.elapsedRealtime()
                        } else {
                            queueFileResult(
                                offer.transferId.toByteArray(),
                                offer.name,
                                success = false,
                                error = "phone receiver is unavailable",
                            )
                        }
                    }
                    .onFailure {
                        Log.w(TAG, "refused a file offer", it)
                        queueFileResult(
                            offer.transferId.toByteArray(),
                            offer.name,
                            success = false,
                            error = it.message ?: it.javaClass.simpleName,
                        )
                    }
                    .getOrNull()
            }

            Kind.FILE_CHUNK -> {
                val chunk = FileChunk.parseFrom(envelope.payload)
                val rx = incomingFile
                if (rx == null) {
                    Log.w(TAG, "file chunk ${chunk.index} with no offer, dropped")
                    return
                }
                runCatching { rx.push(chunk) }
                    .onFailure {
                        Log.w(TAG, "file transfer dropped", it)
                        queueFileResult(
                            chunk.transferId.toByteArray(),
                            rx.name,
                            success = false,
                            error = it.message ?: it.javaClass.simpleName,
                        )
                        events.onFileFailed(rx.name, FileTransferDirection.ToPhone)
                        rx.close()
                        incomingFile = null
                        incomingFileStartedMs = 0L
                    }
                    .getOrNull()
                    ?.let { progress ->
                        events.onFileProgress(
                            rx.name,
                            FileTransferDirection.ToPhone,
                            progress.transferred,
                            progress.total,
                        )
                        if (progress.complete) {
                            val name = rx.name
                            val started = incomingFileStartedMs
                            rx.close()
                            incomingFile = null
                            incomingFileStartedMs = 0L
                            if (started > 0L) {
                                events.onBulkTransfer(
                                    progress.total,
                                    (SystemClock.elapsedRealtime() - started).coerceAtLeast(1L),
                                )
                            }
                            queueFileResult(
                                chunk.transferId.toByteArray(),
                                name,
                                success = true,
                                error = "",
                            )
                            events.onFileComplete(name, FileTransferDirection.ToPhone)
                        }
                    }
            }

            else -> Log.d(TAG, "unhandled kind ${envelope.kind}")
        }
    }

    private fun queuePong() {
        pongPending = true
        sender.execute {
            val live = session ?: return@execute
            runCatching { sendPendingPong(live) }
                .onFailure {
                    Log.w(TAG, "PONG write failed", it)
                    teardown()
                }
        }
    }

    /**
     * Receiver-side publication result. Queued onto the one sender executor so the reader never
     * touches the Noise write counter. It is intentionally one result per whole file, not a
     * per-chunk ACK, so it adds no stop-and-wait behaviour to the data path.
     */
    private fun queueFileResult(
        transferId: ByteArray,
        name: String,
        success: Boolean,
        error: String,
    ) {
        if (transferId.isEmpty()) return
        val result = FileResult.newBuilder()
            .setTransferId(com.google.protobuf.ByteString.copyFrom(transferId))
            .setSuccess(success)
            .setName(name.take(200))
            .setError(error.take(300))
            .build()
        send(Kind.FILE_RESULT, result.toByteArray(), "file result")
    }

    /** Runs only on the sender executor, either as its own task or between transfer chunks. */
    private fun sendPendingPong(live: WireSession) {
        if (!pongPending) return
        // Clear before writing: if another PING arrives during this send, the reader sets the
        // flag again and the next chunk boundary answers that newer probe instead of losing it.
        pongPending = false
        live.send(Kind.PONG)
    }
}
