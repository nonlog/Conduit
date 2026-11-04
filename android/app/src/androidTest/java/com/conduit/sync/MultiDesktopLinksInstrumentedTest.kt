package com.conduit.sync

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.conduit.sync.proto.ClipText
import com.conduit.sync.proto.Kind
import com.conduit.sync.proto.PairRequest
import com.conduit.sync.proto.PairResponse
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CountDownLatch
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Actual Noise transport acceptance, using two independently keyed desktop servers on
 * loopback. Exercises concurrently active Link instances, not just trust-store metadata.
 * Runs in a CI Android emulator and does not touch any user's device/identity.
 */
@RunWith(AndroidJUnit4::class)
class MultiDesktopLinksInstrumentedTest {
    private class TestDesktop(val name: String) : AutoCloseable {
        val keys = KeyPair.generate()
        val id = Identity.deviceId(keys.public)
        private val server = ServerSocket(0, 1, InetAddress.getLoopbackAddress())
        private val client = AtomicReference<Socket?>()
        private val failed = AtomicReference<Throwable?>()
        private val ready = CountDownLatch(1)
        val received = LinkedBlockingQueue<String>()
        val revoked = CountDownLatch(1)
        val address = InetSocketAddress(InetAddress.getLoopbackAddress(), server.localPort)
        private val worker = Thread({
            try {
                val socket = server.accept()
                client.set(socket)
                socket.use {
                    val session = WireSession.handshake(
                        socket.getInputStream(), socket.getOutputStream(), keys.private,
                        initiator = false,
                    )
                    session.send(Kind.PAIR_REQUEST, PairRequest.newBuilder()
                        .setDeviceId(id).setDeviceName(name).setTransferProtocol(1).build().toByteArray())
                    val phoneHello = session.recv()
                    check(phoneHello.kind == Kind.PAIR_REQUEST) { "phone did not send pairing hello" }
                    ready.countDown()
                    while (!socket.isClosed) {
                        val message = session.recv()
                        when (message.kind) {
                            Kind.CLIP_TEXT -> received.put(ClipText.parseFrom(message.payload).text)
                            Kind.PAIR_RESPONSE -> {
                                if (!PairResponse.parseFrom(message.payload).accept) revoked.countDown()
                            }
                            Kind.PING -> session.send(Kind.PONG)
                            else -> Unit
                        }
                    }
                }
            } catch (error: Exception) {
                if (ready.count == 0L && client.get()?.isClosed == true) return@Thread
                failed.set(error)
            } finally {
                ready.countDown()
            }
        }, "conduit-test-$name").apply { isDaemon = true; start() }

        fun awaitHandshake() {
            assertTrue("$name never completed Noise handshake", ready.await(12, TimeUnit.SECONDS))
            failed.get()?.let { throw AssertionError("$name: protocol server failed", it) }
        }

        override fun close() {
            runCatching { client.getAndSet(null)?.close() }
            runCatching { server.close() }
            worker.join(1500)
        }
    }

    private fun events(desktop: TestDesktop, connected: CountDownLatch): Link.Events =
        object : Link.Events {
            override fun onState(state: LinkState, peer: String?) {
                if (state == LinkState.Connected) connected.countDown()
            }
            override fun onText(text: String) = Unit
            override fun onImage(png: ByteArray, photo: Boolean, screenshot: Boolean) = Unit
            override fun authorizePeer(deviceId: String, viaRelay: Boolean, remotePairing: Boolean) =
                deviceId == desktop.id && !viaRelay
            override fun pairingRequested() = false
            override fun onPeerForgotten() = Unit
            override fun onPeer(deviceId: String) = Unit
            override fun onPeerName(name: String) = Unit
            override fun onSessionLost() = Unit
        }

    @Test
    fun twoDesktopsStayAuthenticatedAndTransferTargetsRemainIndependent() {
        val phone = KeyPair.generate()
        val phoneId = Identity.deviceId(phone.public)
        TestDesktop("Personal PC").use { home ->
            TestDesktop("Work PC").use { work ->
                val aUp = CountDownLatch(1)
                val bUp = CountDownLatch(1)
                val a = Link(phone.private, phoneId, "Test phone", events(home, aUp))
                val b = Link(phone.private, phoneId, "Test phone", events(work, bUp))
                try {
                    a.connect(home.address)
                    b.connect(work.address)
                    home.awaitHandshake()
                    work.awaitHandshake()
                    assertTrue("Personal PC did not authenticate", aUp.await(8, TimeUnit.SECONDS))
                    assertTrue("Work PC did not authenticate", bUp.await(8, TimeUnit.SECONDS))

                    a.sendText("home only")
                    assertEquals("home only", home.received.poll(6, TimeUnit.SECONDS))
                    assertNull("A-target share went to work PC", work.received.poll(500, TimeUnit.MILLISECONDS))

                    b.sendText("work only")
                    assertEquals("work only", work.received.poll(6, TimeUnit.SECONDS))
                    assertNull("B-target share went to personal PC", home.received.poll(500, TimeUnit.MILLISECONDS))

                    val revokeFinished = CountDownLatch(1)
                    a.forgetPeer { revokeFinished.countDown() }
                    assertTrue("Revocation did not reach personal PC", home.revoked.await(6, TimeUnit.SECONDS))
                    assertTrue("Revocation completion callback was skipped", revokeFinished.await(6, TimeUnit.SECONDS))
                    assertEquals("Work PC received somebody else's revocation", 1L, work.revoked.count)

                    // Revoking the personal PC must not cancel the work PC's writer.
                    b.sendText("still work")
                    assertEquals("still work", work.received.poll(6, TimeUnit.SECONDS))
                    assertTrue("Closing one link closed the other", b.hasOpenSocket)
                } finally {
                    a.close()
                    b.close()
                }
            }
        }
    }
}
