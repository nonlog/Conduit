package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class UiModelTest {
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
}
