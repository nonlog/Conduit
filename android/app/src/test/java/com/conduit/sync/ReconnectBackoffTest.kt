package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Test

class ReconnectBackoffTest {
    @Test
    fun provenGoodSessionRecoveryUsesShortCeilingOnlyInsideItsWindow() {
        val now = 1_000_000L
        val recoveryUntil = now + 10 * 60 * 1000L

        assertEquals(60_000L, retryCeilingMs(now, recoveryUntil))
        assertEquals(60_000L, retryCeilingMs(recoveryUntil - 1, recoveryUntil))
        assertEquals(300_000L, retryCeilingMs(recoveryUntil, recoveryUntil))
        assertEquals(300_000L, retryCeilingMs(recoveryUntil + 1, recoveryUntil))
    }
}
