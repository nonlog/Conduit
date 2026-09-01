package com.conduit.sync

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ClipboardDetectionTest {
    @Test
    fun localized_copy_button_is_detected() {
        val detector = ClipboardDetection("复制")
        assertTrue(
            detector.isClipboardReadTrigger(
                ClipboardEventSnapshot(
                    kind = ClipboardEventKind.ViewClicked,
                    text = listOf("复制"),
                ),
            ),
        )
    }

    @Test
    fun selection_collapse_after_selection_is_detected() {
        val detector = ClipboardDetection("Copy")
        val selected = ClipboardEventSnapshot(
            kind = ClipboardEventKind.ViewTextSelectionChanged,
            packageName = "app",
            className = "Editor",
            text = listOf("hello"),
            fromIndex = 0,
            toIndex = 5,
        )
        val collapsed = selected.copy(fromIndex = 5, toIndex = 5)
        assertFalse(detector.isClipboardReadTrigger(selected))
        assertTrue(detector.isClipboardReadTrigger(collapsed))
    }

    @Test
    fun ordinary_ui_events_do_not_request_clipboard_access() {
        val detector = ClipboardDetection("Copy")
        assertFalse(
            detector.isClipboardReadTrigger(
                ClipboardEventSnapshot(
                    kind = ClipboardEventKind.ViewClicked,
                    text = listOf("Open settings"),
                ),
            ),
        )
    }
}
