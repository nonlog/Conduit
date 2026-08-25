package com.conduit.sync

import com.conduit.sync.proto.ClipText
import com.conduit.sync.proto.Kind
import java.io.DataOutputStream
import java.io.PipedInputStream
import java.io.PipedOutputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

/**
 * Framing and codegen, checked against itself. Cross-language agreement is
 * [NoiseInteropTest]'s job; this only proves the loop closes.
 */
class WireSessionTest {
    @Test
    fun relayPreambleCarriesTheInitiatorRoleWithoutAmbiguity() {
        val id = Identity.deviceId(ByteArray(32) { 7 })
        val preamble = relayPreamble(id)
        assertEquals(48, preamble.size)
        assertArrayEquals("CDT1".toByteArray(), preamble.copyOfRange(0, 4))
        assertEquals('>'.code.toByte(), preamble[4])
        assertArrayEquals(id.toByteArray(), preamble.copyOfRange(5, preamble.size))
        assertThrows(IllegalArgumentException::class.java) { relayPreamble("short") }
    }

    @Test
    fun handshakeThenFramesRoundTrip() {
        val initiator = KeyPair.generate()
        val responder = KeyPair.generate()

        // Two pipes make a socket without a socket. 64 KiB so one frame never deadlocks.
        val toResponder = PipedOutputStream()
        val responderIn = PipedInputStream(toResponder, 1 shl 16)
        val toInitiator = PipedOutputStream()
        val initiatorIn = PipedInputStream(toInitiator, 1 shl 16)

        // XX interleaves, so the two roles cannot share a thread.
        var far: WireSession? = null
        var failure: Throwable? = null
        val thread = Thread {
            try {
                val session = WireSession.handshake(
                    responderIn, toInitiator, responder.private, initiator = false,
                )
                val envelope = session.recv()
                assertEquals(Kind.CLIP_TEXT, envelope.kind)
                assertEquals(1L, envelope.messageId)
                assertEquals("hello", ClipText.parseFrom(envelope.payload).text)
                session.send(Kind.PONG)
                far = session
            } catch (t: Throwable) {
                failure = t
            }
        }
        thread.start()

        val near = WireSession.handshake(
            initiatorIn, toResponder, initiator.private, initiator = true,
        )
        near.send(
            Kind.CLIP_TEXT,
            ClipText.newBuilder().setText("hello").setMime("text/plain").build().toByteArray(),
        )
        assertEquals(Kind.PONG, near.recv().kind)

        thread.join(5_000)
        failure?.let { throw it }

        // Each side learned the other's static key, which is what pairing will pin.
        assertArrayEquals(responder.public, near.peerStatic)
        assertArrayEquals(initiator.public, far!!.peerStatic)

        // A length outside 1..MAX_FRAME is refused before it can allocate anything.
        DataOutputStream(toInitiator).apply {
            writeInt(0)
            flush()
        }
        assertThrows(IllegalArgumentException::class.java) { near.recv() }
    }
}
