package com.conduit.sync

import org.junit.Assert.assertEquals
import org.junit.Test

class ClipboardAccessTest {
    @Test
    fun lsposed_is_preferred_when_both_paths_are_available() {
        assertEquals(
            ClipboardSyncMode.Lsposed,
            selectClipboardSyncMode(lsposedActive = true, accessibilityEnabled = true),
        )
    }

    @Test
    fun accessibility_is_the_non_root_fallback() {
        assertEquals(
            ClipboardSyncMode.Accessibility,
            selectClipboardSyncMode(lsposedActive = false, accessibilityEnabled = true),
        )
    }

    @Test
    fun unavailable_requires_enabling_a_compatibility_path() {
        assertEquals(
            ClipboardSyncMode.Unavailable,
            selectClipboardSyncMode(lsposedActive = false, accessibilityEnabled = false),
        )
    }
}
