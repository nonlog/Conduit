package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingCodeTest {
    @Test
    fun normalizesHumanFormattingAndDerivesStableRendezvous() {
        assertEquals("AB12CD34EF", PairingCode.normalize("ab12-cd34 ef"))
        assertEquals("AB12C-D34EF", PairingCode.display("ab12cd34ef"))
        assertTrue(PairingCode.isValid("AB12C-D34EF"))
        assertFalse(PairingCode.isValid("ABC"))
        assertEquals(
            PairingCode.rendezvous("AB12C-D34EF"),
            PairingCode.rendezvous("ab12cd34ef"),
        )
        assertEquals(
            "zTn_59YR4I0UaVAJgmJdGDFoOByFrXZpgzlCssnMVHM",
            PairingCode.rendezvous("AB12C-D34EF"),
        )
        assertEquals(43, PairingCode.rendezvous("AB12C-D34EF").length)
    }
}
