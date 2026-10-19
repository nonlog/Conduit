package com.conduit.sync

import java.net.InetAddress
import org.junit.Assert.*
import org.junit.Test

class LanDiscoveryPolicyTest {
    private fun ip(text: String) = InetAddress.getByName(text)
    private fun subnet(text: String, bits: Int = 24) = LanDiscoveryPolicy.Subnet(ip(text), bits)

    @Test fun hotspotAddressWinsOverEthernetAndIpv6() {
        val selected = LanDiscoveryPolicy.selectAddress(
            listOf(ip("2001:db8::1"), ip("198.51.100.1"), ip("192.0.2.1")),
            listOf(subnet("192.0.2.30")),
        )
        assertEquals(ip("192.0.2.1"), selected)
    }

    @Test fun neverSelectsLocalOrSubnetNetworkOrBroadcast() {
        assertNull(LanDiscoveryPolicy.selectAddress(
            listOf(ip("192.0.2.0"), ip("192.0.2.255"), ip("192.0.2.30")),
            listOf(subnet("192.0.2.30")),
        ))
    }

    @Test fun rejectsSyntheticMulticastLoopbackAndIpv6() {
        assertNull(LanDiscoveryPolicy.selectAddress(
            listOf(ip("198.18.0.1"), ip("198.19.255.254"), ip("224.0.0.251"),
                ip("127.0.0.1"), ip("0.0.0.0"), ip("::1"), ip("fe80::1234")),
            emptyList(),
        ))
    }

    @Test fun onlyConfiguredPeerPassesFilterUnlessPairingExplicitlyAllowsAny() {
        assertTrue(LanDiscoveryPolicy.acceptsPeer("paired-desktop", "paired-desktop"))
        assertFalse(LanDiscoveryPolicy.acceptsPeer("paired-desktop", "other-desktop"))
        assertFalse(LanDiscoveryPolicy.acceptsPeer("paired-desktop", null))
        assertTrue(LanDiscoveryPolicy.acceptsPeer(null, "new-desktop"))
    }

    @Test fun unsupportedOffLinkAddressesDoNotEndDiscovery() {
        assertNull(LanDiscoveryPolicy.selectAddress(
            listOf(ip("198.51.100.12")), listOf(subnet("192.0.2.30")),
        ))
        assertEquals(ip("192.0.2.12"), LanDiscoveryPolicy.selectAddress(
            listOf(ip("192.0.2.12")), listOf(subnet("192.0.2.30")),
        ))
    }

    @Test fun staleResolversAndDeadlinesCannotFinishNewBurst() {
        val epochs = DiscoveryEpoch()
        val old = epochs.begin()
        epochs.stop()
        val fresh = epochs.begin()
        assertFalse(epochs.isCurrent(old))
        assertFalse(epochs.finish(old))
        assertTrue(epochs.isCurrent(fresh))
        assertTrue(epochs.finish(fresh))
        assertFalse(epochs.finish(fresh))
        assertFalse(epochs.isCurrent(fresh))
    }

    @Test fun addressSelectionIsStableAcrossNsdOrdering() {
        val candidates = listOf(ip("192.0.2.20"), ip("192.0.2.10"), ip("192.0.2.20"))
        val networks = listOf(subnet("192.0.2.30"))
        assertEquals(ip("192.0.2.10"), LanDiscoveryPolicy.selectAddress(candidates, networks))
        assertEquals(ip("192.0.2.10"), LanDiscoveryPolicy.selectAddress(candidates.reversed(), networks))
    }
}
