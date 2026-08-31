package com.conduit.sync

import android.view.accessibility.AccessibilityEvent

internal enum class ClipboardEventKind {
    ViewClicked,
    ViewFocused,
    ViewLongClicked,
    ViewSelected,
    ViewTextChanged,
    ViewTextSelectionChanged,
    WindowContentChanged,
    WindowStateChanged,
    NotificationStateChanged;

    companion object {
        fun from(type: Int): ClipboardEventKind? = when (type) {
            AccessibilityEvent.TYPE_VIEW_CLICKED -> ViewClicked
            AccessibilityEvent.TYPE_VIEW_FOCUSED -> ViewFocused
            AccessibilityEvent.TYPE_VIEW_LONG_CLICKED -> ViewLongClicked
            AccessibilityEvent.TYPE_VIEW_SELECTED -> ViewSelected
            AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED -> ViewTextChanged
            AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED -> ViewTextSelectionChanged
            AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED -> WindowContentChanged
            AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED -> WindowStateChanged
            AccessibilityEvent.TYPE_NOTIFICATION_STATE_CHANGED -> NotificationStateChanged
            else -> null
        }
    }
}

internal data class ClipboardEventSnapshot(
    val kind: ClipboardEventKind,
    val packageName: String = "",
    val className: String = "",
    val text: List<String> = emptyList(),
    val contentDescription: String = "",
    val fromIndex: Int = -1,
    val toIndex: Int = -1,
) {
    companion object {
        fun from(event: AccessibilityEvent): ClipboardEventSnapshot? {
            val kind = ClipboardEventKind.from(event.eventType) ?: return null
            return ClipboardEventSnapshot(
                kind = kind,
                packageName = event.packageName?.toString().orEmpty(),
                className = event.className?.toString().orEmpty(),
                text = event.text.map { it?.toString().orEmpty() },
                contentDescription = event.contentDescription?.toString().orEmpty(),
                fromIndex = event.fromIndex,
                toIndex = event.toIndex,
            )
        }
    }
}

/** Pure, bounded event recognizer based on Sefirah/XClipper's copy heuristics. */
internal class ClipboardDetection(copyLabel: String) {
    private val actionLabels = setOf(copyLabel, "Copy", "Cut", "复制", "複製", "剪切")
        .map { it.lowercase() }
        .filter(String::isNotBlank)
        .toSet()
    private var selectedText: ClipboardEventSnapshot? = null
    private var windowCopyArmed = false

    fun reset() {
        selectedText = null
        windowCopyArmed = false
    }

    fun isClipboardReadTrigger(event: ClipboardEventSnapshot): Boolean {
        val copyAction = containsCopyAction(event)
        when (event.kind) {
            ClipboardEventKind.ViewClicked, ClipboardEventKind.ViewLongClicked -> {
                if (copyAction) return true
            }
            ClipboardEventKind.ViewTextSelectionChanged -> {
                val previous = selectedText
                if (event.fromIndex >= 0 && event.toIndex >= 0 && event.fromIndex != event.toIndex) {
                    selectedText = event
                } else if (
                    event.fromIndex >= 0 &&
                    event.fromIndex == event.toIndex &&
                    previous != null &&
                    previous.fromIndex != previous.toIndex &&
                    sameTextSource(previous, event)
                ) {
                    selectedText = null
                    return true
                }
            }
            ClipboardEventKind.WindowStateChanged -> windowCopyArmed = copyAction
            ClipboardEventKind.WindowContentChanged -> {
                if (windowCopyArmed) {
                    windowCopyArmed = false
                    return true
                }
            }
            ClipboardEventKind.NotificationStateChanged -> {
                if (containsCopiedToast(event)) return true
            }
            else -> Unit
        }
        return false
    }

    private fun containsCopyAction(event: ClipboardEventSnapshot): Boolean =
        (event.text + event.contentDescription)
            .asSequence()
            .map(String::trim)
            .filter { it.isNotEmpty() && it.length <= 48 }
            .map(String::lowercase)
            .any { token -> actionLabels.any(token::contains) }

    private fun containsCopiedToast(event: ClipboardEventSnapshot): Boolean {
        val copied = listOf("copied", "clipboard", "已复制", "已複製", "剪贴板", "剪貼簿")
        return event.text
            .asSequence()
            .map(String::lowercase)
            .any { token -> token.length <= 160 && copied.any(token::contains) }
    }

    private fun sameTextSource(a: ClipboardEventSnapshot, b: ClipboardEventSnapshot): Boolean =
        a.packageName == b.packageName &&
            a.className == b.className &&
            a.text.firstOrNull() == b.text.firstOrNull()
}
