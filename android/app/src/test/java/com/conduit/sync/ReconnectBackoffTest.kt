package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Test

class ReconnectBackoffTest {
    @Test
    fun parkedRelayDoesNotDropAnExistingForegroundRecovery() {
        assertEquals(
            true,
            shouldHoldForegroundForTransientReconnect(
                foregroundVisible = true,
                pairing = false,
                state = LinkState.Waiting,
            ),
        )
        assertEquals(
            true,
            shouldHoldForegroundForTransientReconnect(
                foregroundVisible = true,
                pairing = false,
                state = LinkState.Retrying,
            ),
        )
        assertEquals(
            false,
            shouldHoldForegroundForTransientReconnect(
                foregroundVisible = false,
                pairing = false,
                state = LinkState.Waiting,
            ),
        )
        assertEquals(
            false,
            shouldHoldForegroundForTransientReconnect(
                foregroundVisible = true,
                pairing = true,
                state = LinkState.Waiting,
            ),
        )
        assertEquals(
            false,
            shouldHoldForegroundForTransientReconnect(
                foregroundVisible = true,
                pairing = false,
                state = LinkState.Connected,
            ),
        )
    }

    @Test
    fun detachedNotificationCleanupCannotDeleteALiveOrPairingNotification() {
        assertEquals(true, shouldCancelDetachedLinkNotification(false, false, LinkState.Retrying))
        assertEquals(true, shouldCancelDetachedLinkNotification(false, false, LinkState.Idle))
        assertEquals(false, shouldCancelDetachedLinkNotification(true, false, LinkState.Retrying))
        assertEquals(false, shouldCancelDetachedLinkNotification(false, true, LinkState.Retrying))
        assertEquals(false, shouldCancelDetachedLinkNotification(false, false, LinkState.Connected))
    }

    @Test
    fun onlyProvenLiveSessionGetsImmediateForegroundRecovery() {
        assertEquals(true, shouldRecoverEstablishedLinkImmediately(true, true, true, false))
        assertEquals(false, shouldRecoverEstablishedLinkImmediately(false, true, true, false))
        assertEquals(false, shouldRecoverEstablishedLinkImmediately(true, false, true, false))
        assertEquals(false, shouldRecoverEstablishedLinkImmediately(true, true, false, false))
        assertEquals(false, shouldRecoverEstablishedLinkImmediately(true, true, true, true))
    }

    @Test
    fun relayLanHandoffWindowRequiresRelayLanAndNoActiveWindow() {
        assertEquals(true, shouldRecheckLanAfterRelay(true, true, false))
        assertEquals(false, shouldRecheckLanAfterRelay(true, true, true))
        assertEquals(false, shouldRecheckLanAfterRelay(true, false, false))
        assertEquals(false, shouldRecheckLanAfterRelay(false, true, false))
    }

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
