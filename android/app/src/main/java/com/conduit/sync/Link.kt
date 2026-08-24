package com.conduit.sync

import android.util.Log
import com.conduit.sync.proto.ClipText
import com.conduit.sync.proto.Envelope
import com.conduit.sync.proto.Kind
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

private const val TAG = "conduit.link"

/** Long enough that a healthy peer's 60 s keepalive always beats it. */
private const val READ_DEADLINE_MS = 150_000
private const val CONNECT_TIMEOUT_MS = 5_000
private const val JOIN_TIMEOUT_MS = 2_000L

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

    /** Dials on the reader thread. A no-op while a connection is already up. */
    fun connect(address: InetSocketAddress) = sender.execute {
        if (reader?.isAlive == true) {
            Log.d(TAG, "already connected, ignoring dial to $address")
            return@execute
        }
        events.onState(LinkState.Discovering, null)
        reader = Thread({ pump(address) }, "conduit-recv").apply { start() }
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

    private fun pump(address: InetSocketAddress) {
        val count = opened.incrementAndGet()
        var established = false
        try {
            Socket().use { sock ->
                socket = sock
                sock.tcpNoDelay = true
                sock.keepAlive = true
                // A deadline, not a poll: the kernel wakes nobody until it expires.
                sock.soTimeout = READ_DEADLINE_MS
                sock.connect(address, CONNECT_TIMEOUT_MS)

                val live = WireSession.handshake(
                    sock.getInputStream(), sock.getOutputStream(), privateKey, initiator = true,
                )
                session = live
                established = true
                val peer = Identity.fingerprint(live.peerStatic)
                Log.i(TAG, "session $count up to $address, peer $peer")
                events.onState(LinkState.Connected, peer)

                while (true) dispatch(live.recv())
            }
        } catch (t: Throwable) {
            Log.w(TAG, "session $count ended", t)
        } finally {
            session = null
            socket = null
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
            Kind.CLIP_TEXT -> events.onText(ClipText.parseFrom(envelope.payload).text)
            else -> Log.d(TAG, "unhandled kind ${envelope.kind}")
        }
    }
}
