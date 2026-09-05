package com.conduit.sync

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
