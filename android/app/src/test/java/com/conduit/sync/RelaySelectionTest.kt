package com.conduit.sync

import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class RelaySelectionTest {
    private val tyo = RelayEndpoint("tyo", "tyo.example", 41113, "192.0.2.1")
    private val wa = RelayEndpoint("wa", "wa.example", 41113, "192.0.2.2")
    private val us = RelayEndpoint("us", "us.example", 41113, "192.0.2.3")

    @Test
    fun relayCatalogParserIsStrictAndKeepsFallbackOptional() {
        assertEquals(
            RelayEndpoint("wa", "wa.example", 41113, "192.0.2.2"),
            RelayCatalog.parse("wa|wa.example|41113|192.0.2.2"),
        )
        assertEquals(
            RelayEndpoint("us", "us.example", 41113, null),
            RelayCatalog.parse("us|us.example|41113"),
        )
        assertEquals(null, RelayCatalog.parse("bad|host|0"))
        assertEquals(null, RelayCatalog.parse("# comment"))
    }

    @Test
    fun selectorIsStickyButCoolsDownRepeatedDialFailuresWithoutAnyProbe() {
        val dir = Files.createTempDirectory("conduit-relay-quality").toFile()
        val store = RelayQualityStore(dir)
        val endpoints = listOf(tyo, wa, us)
        val now = 1_000_000L

        // With no observations, operator order is the deterministic tie-breaker.
        assertEquals(endpoints, store.candidates("vpn-cellular", endpoints, now))

        // A real successful session makes TYO sticky.
        store.connected("vpn-cellular", tyo, now)
        assertEquals(tyo, store.candidates("vpn-cellular", endpoints, now + 1).first())

        // Two real dial failures cool it down; there is no active retry/probe method in this store.
        store.dialFailed("vpn-cellular", tyo, now + 2)
        store.dialFailed("vpn-cellular", tyo, now + 3)
        val duringCooldown = store.candidates("vpn-cellular", endpoints, now + 4)
        assertTrue(tyo !in duringCooldown)
        assertEquals(wa, duringCooldown.first())

        // The outcome is persisted across process recreation.
        val reloaded = RelayQualityStore(dir)
        assertEquals(2, reloaded.snapshot("vpn-cellular", tyo).failureStreak)
        assertTrue(tyo !in reloaded.candidates("vpn-cellular", endpoints, now + 5))
    }

    @Test
    fun qualityHistoryIsSeparatedByNetworkClassAndLearnsOnlyFromRealPayloads() {
        val dir = Files.createTempDirectory("conduit-relay-context").toFile()
        val store = RelayQualityStore(dir)
        val now = 2_000_000L

        store.connected("wifi", wa, now)
        store.goodput("wifi", wa, bytes = 4L * 1024 * 1024, elapsedMs = 1_000)

        assertTrue(store.snapshot("wifi", wa).goodputBps > 4_000_000.0)
        assertEquals(0.0, store.snapshot("cellular", wa).goodputBps, 0.0)
        assertEquals(wa, store.candidates("wifi", listOf(tyo, wa), now + 1).first())
        assertEquals(tyo, store.candidates("cellular", listOf(tyo, wa), now + 1).first())
    }
}
