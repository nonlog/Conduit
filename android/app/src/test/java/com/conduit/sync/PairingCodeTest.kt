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
}
