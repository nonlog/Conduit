package com.conduit.sync

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.InetSocketAddress
import java.net.SocketException

class LanSocketFallbackTest {
    @Test fun vpnDeniedPhysicalBindMayRetryOnSystemRouteForPairedLanCandidate() {
        val lan = InetSocketAddress("192.168.50.42", 41112)
        assertTrue(shouldRetryLanThroughSystemRoute(
            lan, SocketException("Binding socket to network 112 failed: EPERM (Operation not permitted)"),
        ))
        assertTrue(shouldRetryLanThroughSystemRoute(
            lan, SocketException("binding failed").apply {
                initCause(IllegalStateException("EPERM (Operation not permitted)"))
            },
        ))
    }

    @Test fun arbitraryBindFailuresMustNotBeSilentlyRerouted() {
        val lan = InetSocketAddress("192.168.50.42", 41112)
        for (reason in listOf("ENETUNREACH", "EBADF", "EACCES", "EADDRNOTAVAIL")) {
            assertFalse(shouldRetryLanThroughSystemRoute(lan, SocketException("bind failed: $reason")))
        }
    }

    @Test fun untrustedOrNonLanTargetsNeverQualifyForThisFallback() {
        val permissionDenied = SocketException("bind failed: EPERM")
        for (target in listOf(
            InetSocketAddress("198.18.2.64", 41112),
            InetSocketAddress("127.0.0.1", 41112),
            InetSocketAddress("0.0.0.0", 41112),
            InetSocketAddress("224.0.0.251", 5353),
            InetSocketAddress.createUnresolved("remote.example.com", 41112),
        )) {
            assertFalse(shouldRetryLanThroughSystemRoute(target, permissionDenied))
        }
    }
}
