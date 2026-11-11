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

    @Test fun onLinkDesktopHintBecomesOneDirectDial() {
        val target = LanDiscoveryPolicy.hintTarget("192.0.2.5:45999", listOf(subnet("192.0.2.30")))
        assertEquals(ip("192.0.2.5"), target?.address)
        assertEquals(45999, target?.port)
    }

    @Test fun hintOffThePhysicalSubnetIsRefused() {
        // Tailscale/CGNAT and a routed foreign subnet are not the Wi-Fi the hint came in on.
        assertNull(LanDiscoveryPolicy.hintTarget("100.101.102.103:45999", listOf(subnet("192.0.2.30"))))
        assertNull(LanDiscoveryPolicy.hintTarget("198.51.100.5:45999", listOf(subnet("192.0.2.30"))))
    }

    @Test fun hintWithoutAProvableSubnetIsRefused() {
        assertNull(LanDiscoveryPolicy.hintTarget("192.0.2.5:45999", emptyList()))
        assertNull(LanDiscoveryPolicy.hintTarget("192.0.2.5:45999", listOf(subnet("2001:db8::1"))))
    }

    @Test fun malformedHintsAreRefused() {
        val networks = listOf(subnet("192.0.2.30"))
        assertNull(LanDiscoveryPolicy.hintTarget(null, networks))
        assertNull(LanDiscoveryPolicy.hintTarget("", networks))
        assertNull(LanDiscoveryPolicy.hintTarget("192.0.2.5", networks))
        assertNull(LanDiscoveryPolicy.hintTarget("192.0.2.5:", networks))
        assertNull(LanDiscoveryPolicy.hintTarget("192.0.2.5:45999:1", networks))
        assertNull(LanDiscoveryPolicy.hintTarget("192.0.2.300:45999", networks))
        assertNull(LanDiscoveryPolicy.hintTarget("desktop.local:45999", networks))
    }

    @Test fun hintPortMustBeDialable() {
        val networks = listOf(subnet("192.0.2.30"))
        assertNull(LanDiscoveryPolicy.hintTarget("192.0.2.5:0", networks))
        assertNull(LanDiscoveryPolicy.hintTarget("192.0.2.5:65536", networks))
        assertEquals(65535, LanDiscoveryPolicy.hintTarget("192.0.2.5:65535", networks)?.port)
    }

    @Test fun ipv6AndSyntheticHintsAreRefused() {
        assertNull(LanDiscoveryPolicy.hintTarget("[fd00::1]:45999", listOf(subnet("192.0.2.30"))))
        assertNull(LanDiscoveryPolicy.hintTarget("fd00::1:45999", listOf(subnet("192.0.2.30"))))
        assertNull(LanDiscoveryPolicy.hintTarget("198.18.0.1:45999", listOf(subnet("198.18.0.30"))))
    }
}
