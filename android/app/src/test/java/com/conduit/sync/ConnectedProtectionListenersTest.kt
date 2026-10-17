package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ConnectedProtectionListenersTest {
    @Test
    fun only_the_target_uid_can_hold_a_lease_and_the_last_listener_releases_it_once() {
        val events = mutableListOf<String>()
        val leases = ConnectedProtectionListeners<String>(
            { events.add("acquire:$it") }, { events.add("release:$it") },
        )
        assertFalse(leases.add("spoofed", 10062, 10061) { error("untrusted listener") })
        assertTrue(leases.add("first", 10061, 10061) { events.add("unlink:first") })
        assertFalse(leases.add("first", 10061, 10061) { error("duplicate listener") })
        assertTrue(leases.add("second", 10061, 10061) { events.add("unlink:second") })
        leases.remove("first", 10062)
        assertEquals(listOf("acquire:10061"), events)
        leases.remove("first", 10061)
        assertEquals(listOf("acquire:10061", "unlink:first"), events)
        // Binder death and explicit removal call the same method, including a repeated death.
        leases.remove("second", 10061)
        leases.remove("second", 10061)
        assertEquals(listOf("acquire:10061", "unlink:first", "unlink:second", "release:10061"), events)
        assertTrue(leases.add("reconnected", 10061, 10061) {})
        assertEquals("acquire:10061", events.last())
        // A late death from the previous install must not cancel the new install's UID.
        assertTrue(leases.add("new-install", 10063, 10063) {})
        leases.remove("reconnected", 10061)
        assertEquals("release:10061", events.last())
        leases.remove("new-install", 10063)
        assertEquals("release:10063", events.last())

        var rejected = true
        val unavailable = ConnectedProtectionListeners<String>(
            { if (rejected) error("OEM rejected request") }, { error("ungranted lease") },
        )
        assertTrue(runCatching { unavailable.add("retry", 10061, 10061) {} }.isFailure)
        rejected = false
        assertTrue(unavailable.add("retry", 10061, 10061) {})
    }
}
