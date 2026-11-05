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
}
