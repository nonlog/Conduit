package com.conduit.sync

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BootReceiverTest {
    @Test
    fun bootRestoreRequiresBothUserIntentAndAPairedPeer() {
        assertTrue(shouldStartLinkOnBoot(linkWanted = true, hasPairedPeer = true))
        assertFalse(shouldStartLinkOnBoot(linkWanted = false, hasPairedPeer = true))
        assertFalse(shouldStartLinkOnBoot(linkWanted = true, hasPairedPeer = false))
        assertFalse(shouldStartLinkOnBoot(linkWanted = false, hasPairedPeer = false))
    }
}
