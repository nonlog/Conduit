package com.conduit.sync

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Replays `fixtures/noise_xx.txt` — a transcript produced by Rust `snow` — against the
 * hand-written [NoiseXX], in both roles, byte for byte.
 *
 * This is the check that makes rolling our own Noise defensible. Without it the Kotlin
 * side would only ever be tested against itself, which proves nothing: a consistent
 * misreading of the spec passes a self-test and fails against every real peer.
 *
 * Regenerate the fixture with `cargo test` after deleting it, never by hand.
 */
class NoiseInteropTest {
    private val fx: Map<String, String> = locate().readLines()
        .filter { it.isNotBlank() && !it.startsWith("#") }
        .associate { it.substringBefore('=') to it.substringAfter('=') }

    private fun raw(name: String) = unhex(fx.getValue(name))

    @Test
    fun initiatorMatchesTheReference() {
        val s = KeyPair.fromPrivate(raw("init_static"))
        assertEquals("X25519 public key", fx.getValue("init_static_pub"), hex(s.public))

        val hs = NoiseXX(initiator = true, s = s)
        hs.forceEphemeral(raw("init_ephemeral"))

        assertEquals("msg1: -> e", fx.getValue("msg1"), hex(hs.writeMessage()))
        assertEquals("msg2 carries no payload", 0, hs.readMessage(raw("msg2")).size)
        assertEquals(
            "responder's static, learned from msg2",
            fx.getValue("resp_static_pub"),
            hex(hs.remoteStatic!!),
        )
        assertEquals("msg3: -> s, se", fx.getValue("msg3"), hex(hs.writeMessage()))

        // Transport keys are split by role, so a swapped pair fails exactly here.
        val session = hs.intoTransport()
        assertEquals(fx.getValue("i2r"), hex(session.writeMessage(raw("i2r_plain"))))
        assertEquals(fx.getValue("r2i_plain"), hex(session.readMessage(raw("r2i"))))
    }

    @Test
    fun responderMatchesTheReference() {
        val s = KeyPair.fromPrivate(raw("resp_static"))
        assertEquals("X25519 public key", fx.getValue("resp_static_pub"), hex(s.public))

        val hs = NoiseXX(initiator = false, s = s)
        hs.forceEphemeral(raw("resp_ephemeral"))

        assertEquals("msg1 carries no payload", 0, hs.readMessage(raw("msg1")).size)
        assertEquals("msg2: <- e, ee, s, es", fx.getValue("msg2"), hex(hs.writeMessage()))
        assertEquals("msg3 carries no payload", 0, hs.readMessage(raw("msg3")).size)
        assertEquals(
            "initiator's static, learned from msg3",
            fx.getValue("init_static_pub"),
            hex(hs.remoteStatic!!),
        )

        val session = hs.intoTransport()
        assertEquals(fx.getValue("i2r_plain"), hex(session.readMessage(raw("i2r"))))
        assertEquals(fx.getValue("r2i"), hex(session.writeMessage(raw("r2i_plain"))))
    }

    /** Not Noise, but the same failure mode: disagree here and pairing is unverifiable. */
    @Test
    fun derivedNamesMatchTheReference() {
        val pub = raw("init_static_pub")
        assertEquals(fx.getValue("init_device_id"), Identity.deviceId(pub))
        assertEquals(fx.getValue("init_fingerprint"), Identity.fingerprint(pub))
    }

    private companion object {
        /** Walks up from the test's working directory, which Gradle does not promise. */
        fun locate(): File {
            var dir: File? = File("").absoluteFile
            while (dir != null) {
                val f = File(dir, "fixtures/noise_xx.txt")
                if (f.isFile) return f
                dir = dir.parentFile
            }
            error("fixtures/noise_xx.txt not found above ${File("").absolutePath}")
        }

        fun hex(bytes: ByteArray) =
            bytes.joinToString("") { (it.toInt() and 0xff).toString(16).padStart(2, '0') }

        fun unhex(s: String) =
            ByteArray(s.length / 2) { s.substring(it * 2, it * 2 + 2).toInt(16).toByte() }
    }
}
