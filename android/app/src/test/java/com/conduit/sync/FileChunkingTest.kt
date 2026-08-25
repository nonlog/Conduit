package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * The chunk arithmetic, which is worth a test because both sides compute it independently
 * and the receiver *refuses* an offer whose count does not follow from the size. Get it
 * wrong by one and every transfer is rejected with nothing on this side to explain why —
 * mirrored by `a_dishonest_offer_is_refused_before_anything_is_created` in `file.rs`.
 */
class FileChunkingTest {

    private val chunk = 32 * 1024L

    @Test
    fun the_chunk_count_is_what_the_desktop_will_check_it_against() {
        // Exactly one chunk, and one byte either side of the boundary.
        assertEquals(1, Files.chunkCount(1))
        assertEquals(1, Files.chunkCount(chunk))
        assertEquals(2, Files.chunkCount(chunk + 1))
        assertEquals(2, Files.chunkCount(chunk * 2))
        assertEquals(3, Files.chunkCount(chunk * 2 + 1))

        // The ceiling, where a 32-bit intermediate would have overflowed.
        assertEquals(MAX_FILE / chunk, Files.chunkCount(MAX_FILE))
        assertEquals(16_384, Files.chunkCount(MAX_FILE))
    }

    /** `ceil(size / chunk)` for every size in a range, against the definition. */
    @Test
    fun it_is_a_ceiling_division_for_every_size_not_just_the_round_ones() {
        for (size in 1L..(chunk * 3)) {
            val expected = (size / chunk) + if (size % chunk == 0L) 0 else 1
            assertEquals("size $size", expected, Files.chunkCount(size))
        }
    }
}
