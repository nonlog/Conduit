package com.conduit.sync

import android.util.Log
import com.conduit.sync.proto.ClipImageChunk
import com.conduit.sync.proto.ClipImageHeader
import com.conduit.sync.proto.ClipText
import com.conduit.sync.proto.Envelope
import com.conduit.sync.proto.Kind
import com.conduit.sync.proto.PairRequest
import java.net.InetSocketAddress
import java.net.Socket
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
 * The relay preamble's magic. `CDT1` then a 43-character rendezvous id is a fixed 47
 * bytes, so the relay needs no parser. Mirrored in `relay/src/main.rs` and `wire.rs`.
 */
private val RELAY_MAGIC = "CDT1".toByteArray()


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
class Link(private val privateKey: ByteArray, private val events: Events) {

    interface Events {
        fun onState(state: LinkState, peer: String?)
        fun onText(text: String)

        /**
         * A complete image. [photo] is the backward-compatible non-clipboard marker;
         * [screenshot] distinguishes a screenshot from a camera photo on new peers.
         */
        fun onImage(png: ByteArray, photo: Boolean, screenshot: Boolean)

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
     * The image being reassembled. Touched only by the reader thread, so it is neither
     * volatile nor locked, and it is cleared on teardown with the session.
     */
    private var incoming: Images.Assembly? = null

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
                Images.send(live, payload, photo, screenshot)
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
            source.stream.use { Files.send(live, it, source.meta) }
        } catch (e: Exception) {
            // Same rule as an image: the offer is already out and the desktop is committed
            // to a total, so there is nothing to salvage. Let the session unwind — the
            // desktop's transfer dies with it and deletes its own partial file.
            Log.w(TAG, "${source.meta.name} transfer failed", e)
            teardown()
        }
    }

    /** Dials on the reader thread. A no-op while a connection is already up. */
    fun connect(address: InetSocketAddress) = dial(address, null)
    /**
     * Parks at the relay under [rendezvous] and waits to be spliced onto the desktop.
     *
     * Same reader thread, same session, same teardown as a LAN dial — the relay only
     * changes which address is dialled and adds a 47-byte preamble, so nothing about the
     * `opened == closed` invariant depends on which path was taken.
     */
    fun connectVia(relay: InetSocketAddress, rendezvous: String) = dial(relay, rendezvous)

    private fun dial(address: InetSocketAddress, rendezvous: String?) = sender.execute {
        if (reader?.isAlive == true) {
            Log.d(TAG, "already connected, ignoring dial to $address")
            return@execute
        }
        events.onState(LinkState.Discovering, null)
        reader = Thread({ pump(address, rendezvous) }, "conduit-recv").apply { start() }
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
    }

    private fun pump(address: InetSocketAddress, rendezvous: String?) {
        val count = opened.incrementAndGet()
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
                // on the connectivity callback that asked for the dial.
                val target = if (address.isUnresolved) {
                    InetSocketAddress(address.hostName, address.port)
                } else {
                    address
                }
                sock.connect(target, CONNECT_TIMEOUT_MS)
                if (rendezvous != null) {
                    // 47 bytes naming the rendezvous, then the relay is a pipe and never
                    // looks at this stream again. Mirrored in `relay/src/main.rs`.
                    sock.getOutputStream().apply {
                        write(RELAY_MAGIC + rendezvous.toByteArray())
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
            Log.w(TAG, "session $count ended", t)
        } finally {
            session = null
            socket = null
            // A partial image dies with the session that was carrying it, so the next
            // one never inherits a half-filled buffer.
            incoming = null
            events.onState(LinkState.Idle, null)
            Log.i(TAG, "session $count closed: opened=$count closed=${closed.incrementAndGet()}")
            if (established) events.onSessionLost()
        }
    }

    private fun dispatch(envelope: Envelope) {
        when (envelope.kind) {
            // Posted, not written: only the sender thread may touch the send counter.
            Kind.PING -> sender.execute { runCatching { session?.send(Kind.PONG) } }
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
            Kind.CLIP_IMAGE_HEADER -> incoming = runCatching {
                Images.Assembly.begin(ClipImageHeader.parseFrom(envelope.payload))
            }.onFailure { Log.w(TAG, "refused an image header", it) }.getOrNull()

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
                    }
                    .getOrNull()
                    ?.let { png ->
                        incoming = null
                        Log.i(
                            TAG,
                            "image in: ${png.size} B, photo=${assembly.photo} " +
                                "screenshot=${assembly.screenshot}",
                        )
                        events.onImage(png, assembly.photo, assembly.screenshot)
                    }
            }

            else -> Log.d(TAG, "unhandled kind ${envelope.kind}")
        }
    }
}
