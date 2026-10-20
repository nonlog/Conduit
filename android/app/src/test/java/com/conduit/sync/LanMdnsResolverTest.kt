package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.InetAddress

class LanMdnsResolverTest {
    /** Captured real mDNS QU response from the desktop's physical Wi-Fi interface. */
    private val physicalAnswer = decode(
        "000084000000000100000000034c6f67056c6f63616c00000100010000003c0004c0a8013f",
    )
    /** The same on-link address, with a question and compressed answer name. */
    private val compressedAnswer = decode(
        "000084000001000100000000034c6f67056c6f63616c0000010001c00c00010001000000780004c0a8013f",
    )

    @Test fun onlyLocalHostnamesCanBeProbed() {
        assertEquals("log.local", LanMdnsResolver.hostname("Log.local."))
        assertEquals("log.local", LanMdnsResolver.nsdHostname("Log"))
        assertEquals("log.local", LanMdnsResolver.nsdHostname("Log.local."))
        assertEquals(null, LanMdnsResolver.nsdHostname("Log.example.com"))
        for (invalid in listOf("example.org", "http://log.local", "bad..local", "-bad.local", "a_b.local", "log.local/escape")) {
            assertEquals(null, LanMdnsResolver.hostname(invalid))
        }
    }

    @Test fun physicalQueryRequestsUnicastMdnsAResult() {
        val query = LanMdnsResolver.question("Log.local")
        assertEquals(1, query[5].toInt()) // one question
        assertEquals(0x80, query[query.lastIndex - 1].toInt() and 255)
        assertEquals(1, query.last().toInt())
    }

    @Test fun physicalWifiAnswerAndCompressedAnswerResolveTheSameHost() {
        for (reply in listOf(physicalAnswer, compressedAnswer)) {
            val addresses = LanMdnsResolver.addresses("log.local", reply)
            assertEquals(listOf("192.168.1.63"), addresses.map(InetAddress::getHostAddress))
            val chosen = LanDiscoveryPolicy.selectAddress(
                addresses, listOf(LanDiscoveryPolicy.Subnet(InetAddress.getByName("192.168.1.250"), 24)),
            )
            assertEquals("192.168.1.63", chosen?.hostAddress)
        }
    }

    @Test fun spoofedHostnameOrOffSubnetResponseIsNotAnOnLinkCandidate() {
        assertTrue(LanMdnsResolver.addresses("unrelated.local", physicalAnswer).isEmpty())
        val fake = physicalAnswer.clone().apply {
            this[lastIndex - 3] = 198.toByte(); this[lastIndex - 2] = 18
            this[lastIndex - 1] = 2; this[lastIndex] = 64
        }
        val candidates = LanMdnsResolver.addresses("log.local", fake)
        assertEquals(listOf("198.18.2.64"), candidates.map(InetAddress::getHostAddress))
        assertEquals(null, LanDiscoveryPolicy.selectAddress(
            candidates, listOf(LanDiscoveryPolicy.Subnet(InetAddress.getByName("192.168.1.250"), 24)),
        ))
    }

    @Test fun malformedResponsesAreRejectedWithoutFollowingPointersOrReadingPastBuffer() {
        assertTrue(LanMdnsResolver.addresses("log.local", physicalAnswer.copyOf(20)).isEmpty())
        assertTrue(LanMdnsResolver.addresses("log.local", physicalAnswer.copyOf(31)).isEmpty())
        val malformed = compressedAnswer.clone().apply {
            val pointer = indexOf(0xc0.toByte())
            this[pointer + 1] = pointer.toByte() // self-reference
        }
        assertTrue(LanMdnsResolver.addresses("log.local", malformed).isEmpty())
        val notAResponse = physicalAnswer.clone().apply { this[2] = 0; this[3] = 0 }
        assertTrue(LanMdnsResolver.addresses("log.local", notAResponse).isEmpty())
    }

    private fun decode(hex: String): ByteArray = hex.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
}
