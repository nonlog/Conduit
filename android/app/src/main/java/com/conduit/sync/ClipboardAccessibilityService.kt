package com.conduit.sync

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.AccessibilityServiceInfo
import android.os.SystemClock
import android.util.Log
import android.view.accessibility.AccessibilityEvent

private const val ACCESSIBILITY_TAG = "conduit.clip.accessibility"
private const val MIN_LAUNCH_INTERVAL_MS = 100L

/**
 * Stock-Android clipboard bridge adapted from Sefirah's accessibility path.
 *
 * This is the compatibility path for devices where the LSPosed hook is unavailable. It observes
 * UI copy events only while a Conduit link is connected. When the link is down, or when LSPosed is
 * active, eventTypes stay at zero even if Accessibility permission remains enabled. A likely copy
 * event opens one translucent activity long enough for Android 10+ to grant foreground clipboard
 * access; there is no polling.
 */
class ClipboardAccessibilityService : AccessibilityService() {
    companion object {
        private const val MONITORED_EVENTS =
            AccessibilityEvent.TYPE_VIEW_CLICKED or
                AccessibilityEvent.TYPE_VIEW_FOCUSED or
                AccessibilityEvent.TYPE_VIEW_LONG_CLICKED or
                AccessibilityEvent.TYPE_VIEW_SELECTED or
                AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED or
                AccessibilityEvent.TYPE_VIEW_TEXT_SELECTION_CHANGED or
                AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED or
                AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED or
                AccessibilityEvent.TYPE_NOTIFICATION_STATE_CHANGED

        @Volatile
        private var instance: ClipboardAccessibilityService? = null

        fun setLinkActive(active: Boolean) {
            val service = instance ?: return
            service.mainExecutor.execute {
                if (instance === service) service.configure(active)
            }
        }
    }

    private lateinit var detector: ClipboardDetection
    private var runForNextEventAlso = false
    private var lastLaunchAtMs = 0L

    override fun onServiceConnected() {
        super.onServiceConnected()
        instance = this
        detector = ClipboardDetection(ClipboardLocale.copyLabel(this))
        configure(LinkStatus.state == LinkState.Connected)
        Log.i(ACCESSIBILITY_TAG, "clipboard accessibility service connected")
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        if (
            event == null ||
            LinkStatus.state != LinkState.Connected ||
            ClipboardAccess.isLsposedActive()
        ) return
        val snapshot = ClipboardEventSnapshot.from(event) ?: return
        val detected = detector.isClipboardReadTrigger(snapshot)
        val followUp = runForNextEventAlso
        when {
            detected -> runForNextEventAlso = true
            followUp -> runForNextEventAlso = false
            else -> return
        }

        val now = SystemClock.elapsedRealtime()
        if (now - lastLaunchAtMs < MIN_LAUNCH_INTERVAL_MS) return
        lastLaunchAtMs = now
        ClipboardChangeActivity.launch(this)
    }

    override fun onInterrupt() = Unit

    override fun onDestroy() {
        if (instance === this) instance = null
        super.onDestroy()
    }

    private fun configure(active: Boolean) {
        val compatibilityActive = active && !ClipboardAccess.isLsposedActive()
        if (::detector.isInitialized && !compatibilityActive) detector.reset()
        if (!compatibilityActive) runForNextEventAlso = false
        serviceInfo = AccessibilityServiceInfo().apply {
            eventTypes = if (compatibilityActive) MONITORED_EVENTS else 0
            feedbackType = AccessibilityServiceInfo.FEEDBACK_GENERIC
            notificationTimeout = 120L
            flags = AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
        }
        Log.d(
            ACCESSIBILITY_TAG,
            "clipboard accessibility events active=$compatibilityActive " +
                "lsposed=${ClipboardAccess.isLsposedActive()}",
        )
    }
}
