package com.conduit.sync

import org.junit.Assert.*
import org.junit.Test

class TransferUiTest {
    @Test fun repeatedSharesOfTheSameUriAreIndependentRequestsAndNeverRedial() {
        val first = boundedShare(listOf("photo", "photo"))
        val second = boundedShare(listOf("photo"))
        assertEquals(listOf("photo"), first)
        assertEquals(first, second)
        assertEquals(16, boundedShare((1..40).toList()).size)
        for (connected in listOf(true, false)) {
            repeat(2) { assertFalse(serviceStartMayRedial(ACTION_SHARE, connected)) }
            assertFalse(serviceStartMayRedial(ACTION_CANCEL_TRANSFER, connected))
        }
        assertFalse(serviceStartMayRedial(null, true))
        assertTrue(serviceStartMayRedial(null, false))
    }

    @Test fun transferRowsUseIdsRatherThanNamesAndKeepRealCompletionSeparateFrom100Percent() {
        val a = FileTransfer(FileTransferDirection.ToDesktop, "same.jpg", 10, 10, "a", TransferPhase.WaitingConfirmation, canCancel = true)
        val b = a.copy(id = "b", transferred = 0, phase = TransferPhase.Queued)
        var rows = updateTransferHistory(emptyList(), a)
        rows = updateTransferHistory(rows, b)
        assertEquals(2, rows.size)
        assertEquals(100, rows[0].percent)
        assertFalse(rows[0].phase.terminal)
        assertEquals("Waiting for confirmation", rows[0].phase.label)
        rows = updateTransferHistory(rows, a.copy(phase = TransferPhase.Completed, canCancel = false))
        rows = updateTransferHistory(rows, a.copy(phase = TransferPhase.Failed))
        assertEquals(TransferPhase.Completed, rows[0].phase)
        assertEquals(TransferPhase.Queued, rows[1].phase)
    }

    @Test fun historyIsBoundedWithoutEvictingAnActiveFile() {
        val active = FileTransfer(FileTransferDirection.ToPhone, "active", 1, 2, "active")
        var rows = listOf(active)
        repeat(50) { index ->
            rows = updateTransferHistory(rows, active.copy(id = "done-$index", phase = TransferPhase.Completed))
        }
        assertEquals(33, rows.size)
        assertEquals(active, rows.first())
        assertEquals("done-49", rows.last().id)
    }
}
