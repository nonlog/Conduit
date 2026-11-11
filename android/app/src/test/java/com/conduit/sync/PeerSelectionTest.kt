package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Test

class PeerSelectionTest {
    private val offline = ConnectedDevice("old", "IOTFRESH", LinkState.Retrying, "Relay")
    private val live = ConnectedDevice("live", "Log", LinkState.Connected, "LAN")

    @Test
    fun connectedPeerWinsOverStaleSelectedPeer() {
        assertEquals("live", resolveActiveDeviceId(listOf(offline, live), "old"))
    }

    @Test
    fun selectedConnectedPeerRemainsPreferred() {
        assertEquals("live", resolveActiveDeviceId(listOf(live, offline), "live"))
    }

    @Test
    fun offlineSelectionIsKeptWhenNothingIsConnected() {
        assertEquals("old", resolveActiveDeviceId(listOf(offline), "old"))
    }

    @Test
    fun companionConnectionKeepsClipboardGateOpenWhenPrimaryIsOffline() {
        assertEquals(true, hasConnectedPeer(LinkState.Retrying, listOf(offline, live)))
    }

    @Test
    fun pausedDesktopNeverBecomesTheActiveSendTarget() {
        val paused = ConnectedDevice("paused", "Home", LinkState.Paused, "", enabled = false)
        assertEquals("live", resolveActiveDeviceId(listOf(paused, live), "paused"))
        assertEquals("paused", resolveActiveDeviceId(listOf(paused), "paused"))
        assertEquals(false, hasConnectedPeer(LinkState.Paused, listOf(paused)))
    }
}
