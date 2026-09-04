package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class UiModelTest {
    @Test
    fun reconnectingAndDiscoveringRemainUserStoppable() {
        assertTrue(!isLinkRequestedState(LinkState.Idle))
        assertTrue(isLinkRequestedState(LinkState.Discovering))
        assertTrue(isLinkRequestedState(LinkState.Waiting))
        assertTrue(isLinkRequestedState(LinkState.Retrying))
        assertTrue(isLinkRequestedState(LinkState.Pairing))
        assertTrue(isLinkRequestedState(LinkState.Connected))
    }

    @Test
    fun historySearchMatchesTextAndDirectionCaseInsensitively() {
        val entries = listOf(
            HistoryEntry(Direction.Sent, "Alpha clipboard", 3L),
            HistoryEntry(Direction.Received, "Beta clipboard", 2L),
            HistoryEntry(Direction.Sent, "Gamma", 1L),
        )

        assertEquals(entries, filterHistory(entries, ""))
        assertEquals(listOf(entries[0]), filterHistory(entries, "ALPHA"))
        assertEquals(listOf(entries[1]), filterHistory(entries, "received"))
        assertTrue(filterHistory(entries, "missing").isEmpty())
    }

    @Test
    fun transferProgressIsBoundedAndHumanReadable() {
        val half = FileTransfer(FileTransferDirection.ToPhone, "x.bin", 512, 1024)
        assertEquals(0.5f, half.fraction, 0.0001f)
        assertEquals(50, half.percent)

        val over = FileTransfer(FileTransferDirection.ToDesktop, "x.bin", 2048, 1024)
        assertEquals(1f, over.fraction, 0.0001f)
        assertEquals(100, over.percent)
        assertEquals("1.0 KB", formatBytes(1024))
        assertEquals("1.0 MB", formatBytes(1024L * 1024L))
    }

    @Test
    fun transferProgressGateCapsIntermediateRefreshesButNeverDelaysEdges() {
        var now = 1_000L
        val gate = TransferProgressGate(minIntervalMs = 250L) { now }

        assertTrue(gate.shouldPublish(0, 1_000))
        now += 20
        assertTrue(!gate.shouldPublish(100, 1_000))
        now += 229
        assertTrue(!gate.shouldPublish(200, 1_000))
        now += 1
        assertTrue(gate.shouldPublish(300, 1_000))

        // Completion is immediate even if it follows the last repaint by one millisecond.
        now += 1
        assertTrue(gate.shouldPublish(1_000, 1_000))

        gate.reset()
        assertTrue(gate.shouldPublish(1, 1_000))
    }
}
