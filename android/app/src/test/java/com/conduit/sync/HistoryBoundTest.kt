package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

/**
 * The history is the one structure in the app that grows with use, so the only thing worth
 * pinning is that it cannot grow without limit.
 *
 * Plain JVM test: [History.record] touches no Android API until [History.load] has given it a
 * directory, and without one it keeps the in-memory list and writes nothing — which is exactly
 * the half being checked here.
 */
class HistoryBoundTest {

    @Before
    fun empty() = History.clear()

    @Test
    fun a_long_clip_is_stored_truncated_not_whole() {
        History.record(Direction.Sent, "x".repeat(64_000))
        val stored = History.entries.single().preview
        assertEquals("the preview is capped, not the clipboard", 200, stored.length)
    }

    @Test
    fun the_list_stops_growing_and_drops_the_oldest() {
        // Well past the cap, and each entry distinguishable so ordering is checkable.
        repeat(150) { History.record(Direction.Sent, "clip $it") }
        assertEquals("bounded however long the app runs", 100, History.entries.size)
        // Newest first, so the UI needs no sorting and the trim takes the tail.
        assertEquals("clip 149", History.entries.first().preview)
        assertEquals("clip 50", History.entries.last().preview)
    }

    @Test
    fun worst_case_stays_small_enough_to_rewrite_on_every_clip() {
        repeat(150) { History.record(Direction.Received, "y".repeat(64_000)) }
        val bytes = History.entries.sumOf { it.preview.length }
        assertEquals(100, History.entries.size)
        // 100 entries x 200 chars. The arithmetic is the point: whatever the user copies,
        // the stored total has a ceiling in the tens of kilobytes.
        assertEquals(20_000, bytes)
        // Which is what makes the whole-file rewrite in History.save affordable on the
        // clipboard path, where the callers are the main thread and Link's reader thread.
        assertTrue("must stay small enough to write synchronously", bytes < 64 * 1024)
    }

    @Test
    fun direction_and_image_flag_survive_a_round_trip_in_memory() {
        History.record(Direction.Received, "Image, 34 kB", image = true)
        val entry = History.entries.single()
        assertEquals(Direction.Received, entry.direction)
        assertTrue("an image row must not offer tap-to-copy", entry.image)
    }
}
