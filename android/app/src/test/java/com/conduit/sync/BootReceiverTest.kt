package com.conduit.sync

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BootReceiverTest {
    @Test
    fun bootRestoreRequiresBothUserIntentAndAnEnabledPeer() {
        assertTrue(shouldStartLinkOnBoot(linkWanted = true, hasEnabledPeer = true))
        assertFalse(shouldStartLinkOnBoot(linkWanted = false, hasEnabledPeer = true))
        assertFalse(shouldStartLinkOnBoot(linkWanted = true, hasEnabledPeer = false))
        assertFalse(shouldStartLinkOnBoot(linkWanted = false, hasEnabledPeer = false))
    }
}
