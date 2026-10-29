package com.conduit.sync

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NetworkHandoffRecoveryTest {
    @Test
    fun lanToCellularRecreatesOldBoundSocketImmediately() {
        assertTrue(shouldReconnectForDefaultNetworkChange(
            networkChanged = true, hasOpenSocket = true, usingPhysicalLan = true,
            boundToNewDefault = false, newDefaultIsVpn = false,
        ))
    }

    @Test
    fun cellularToWifiRecreatesRelayWithoutWaitingForPongTimeout() {
        assertTrue(shouldReconnectForDefaultNetworkChange(
            networkChanged = true, hasOpenSocket = true, usingPhysicalLan = false,
            boundToNewDefault = false, newDefaultIsVpn = false,
        ))
    }

    @Test
    fun infinitelyParkedRelayWaiterMustAlsoFollowNewDefault() {
        // A parked Relay TCP connection has no read timeout and is not yet
        // Connected. Socket ownership, not the UI's Connected state, matters.
        assertTrue(shouldReconnectForDefaultNetworkChange(
            networkChanged = true, hasOpenSocket = true, usingPhysicalLan = false,
            boundToNewDefault = false, newDefaultIsVpn = true,
        ))
    }

    @Test
    fun startupDuplicatesAndVpnOverlayDoNotDestroyLiveLanTransfers() {
        assertFalse(shouldReconnectForDefaultNetworkChange(
            networkChanged = false, hasOpenSocket = true, usingPhysicalLan = true,
            boundToNewDefault = false, newDefaultIsVpn = false,
        ))
        assertFalse(shouldReconnectForDefaultNetworkChange(
            networkChanged = true, hasOpenSocket = false, usingPhysicalLan = false,
            boundToNewDefault = false, newDefaultIsVpn = false,
        ))
        assertFalse(shouldReconnectForDefaultNetworkChange(
            networkChanged = true, hasOpenSocket = true, usingPhysicalLan = true,
            boundToNewDefault = false, newDefaultIsVpn = true,
        ))
        assertFalse(shouldReconnectForDefaultNetworkChange(
            networkChanged = true, hasOpenSocket = true, usingPhysicalLan = true,
            boundToNewDefault = true, newDefaultIsVpn = false,
        ))
    }

    @Test
    fun wifiLossUnderSameVpnDefaultRecreatesOnlyWifiBackedRelay() {
        for (networkClass in listOf("wifi", "ethernet", "vpn-wifi", "vpn-ethernet")) {
            assertTrue(shouldReconnectRelayForPhysicalLanLoss(
                lostKnownLan = true, relaySocketActive = true, relayNetworkClass = networkClass,
            ))
        }
        for (networkClass in listOf("cellular", "vpn-cellular", "other")) {
            assertFalse(shouldReconnectRelayForPhysicalLanLoss(
                lostKnownLan = true, relaySocketActive = true, relayNetworkClass = networkClass,
            ))
        }
        assertFalse(shouldReconnectRelayForPhysicalLanLoss(
            lostKnownLan = false, relaySocketActive = true, relayNetworkClass = "vpn-wifi",
        ))
        assertFalse(shouldReconnectRelayForPhysicalLanLoss(
            lostKnownLan = true, relaySocketActive = false, relayNetworkClass = "vpn-wifi",
        ))
    }

    @Test
    fun sameWifiHandleNewDhcpAddressAndIpLossInvalidateBoundLan() {
        assertTrue(shouldReconnectBoundLanAfterAddressChange("192.168.1.2/24", "192.168.20.3/24", true))
        assertTrue(shouldReconnectBoundLanAfterAddressChange("192.168.1.2/24", "", true))
        assertFalse(shouldReconnectBoundLanAfterAddressChange("192.168.1.2/24", "192.168.1.2/24", true))
        assertFalse(shouldReconnectBoundLanAfterAddressChange(null, "192.168.20.3/24", true))
        assertFalse(shouldReconnectBoundLanAfterAddressChange("192.168.1.2/24", "192.168.20.3/24", false))
    }

    @Test
    fun relayToLanHandoffCannotSuppressFailureOfReplacementSession() {
        assertFalse(isCurrentLinkGeneration(eventGeneration = 7L, latestGeneration = 8L))
        assertTrue(isCurrentLinkGeneration(eventGeneration = 8L, latestGeneration = 8L))
        // A failed direct candidate after a Relay handoff still has a usable
        // remembered peer and must fall back to Relay without a user tap.
        assertTrue(shouldFallbackToRelayAfterLanFailure(
            wasLinked = false, route = "LAN", relayAttemptActive = false,
            networkUp = true, linkWanted = true, hasPairedPeer = true,
        ))
    }
}
