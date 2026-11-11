package com.conduit.sync

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingCodeTest {
    @Test
    fun normalizesSixDigitsAndDerivesStableRendezvous() {
        assertEquals("123456", PairingCode.normalize("12 34-56"))
        assertEquals("123456", PairingCode.display("12-3456"))
        assertTrue(PairingCode.isValid("123456"))
        assertFalse(PairingCode.isValid("123 456"))
        assertFalse(PairingCode.isValid("12345"))
        assertFalse(PairingCode.isValid("12AB56"))
        assertEquals(
            PairingCode.rendezvous("123-456"),
            PairingCode.rendezvous("123456"),
        )
        assertEquals(
            "3sLbGZON6YWYSIrLdCIGl7TWmbRLGLVRBqCwooefYBY",
            PairingCode.rendezvous("123456"),
        )
        assertEquals(43, PairingCode.rendezvous("123456").length)
    }

    @Test
    fun pairingProofBindsTheCodeAndBothNoiseIdentities() {
        val phone = "phone-id"
        val desktop = "desk-id"
        assertEquals(
            """7b777be2bb68164f3af6294241c51cb2b0d96704ab2a3f3451e614a3501fb7de""",
            PairingCode.proof("123456", phone, desktop).joinToString("") {
                (it.toInt() and 0xff).toString(16).padStart(2, '0')
            },
        )
        // Formatting in the code the user reads must not change the digest.
        assertArrayEquals(
            PairingCode.proof("123456", phone, desktop),
            PairingCode.proof("12 34-56", phone, desktop),
        )
        // Swapping the two identities is a different pairing, not the same one.
        assertFalse(
            PairingCode.proof("123456", phone, desktop)
                .contentEquals(PairingCode.proof("123456", desktop, phone)),
        )
        assertEquals(32, PairingCode.proof("123456", phone, desktop).size)
    }

    @Test
    fun parsesOnlyConduitPairingQrPayloads() {
        assertEquals("123456", PairingCode.fromQrPayload("conduit://pair?code=123456"))
        assertEquals("123456", PairingCode.fromQrPayload("CONDUIT://PAIR?foo=1&code=123456"))
        assertEquals("123456", PairingCode.fromQrPayload("conduit://pair?code=12%2034-56"))
        assertEquals(null, PairingCode.fromQrPayload("https://example.com/?code=123456"))
        assertEquals(null, PairingCode.fromQrPayload("conduit://other?code=123456"))
        assertEquals(null, PairingCode.fromQrPayload("conduit://pair?code=12345"))
        assertEquals(null, PairingCode.fromQrPayload("conduit://pair"))
    }
}
