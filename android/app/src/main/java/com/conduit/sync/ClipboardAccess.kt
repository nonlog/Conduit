package com.conduit.sync

import android.content.ComponentName
import android.content.Context
import android.provider.Settings as AndroidSettings

internal enum class ClipboardSyncMode(val label: String) {
    Lsposed("LSPosed"),
    Accessibility("Accessibility"),
    Unavailable("Unavailable"),
}

internal fun selectClipboardSyncMode(
    lsposedActive: Boolean,
    accessibilityEnabled: Boolean,
): ClipboardSyncMode = when {
    lsposedActive -> ClipboardSyncMode.Lsposed
    accessibilityEnabled -> ClipboardSyncMode.Accessibility
    else -> ClipboardSyncMode.Unavailable
}

/**
 * Runtime clipboard-access capability. LSPosed replaces only [isLsposedActive] in Conduit's own
 * process; ordinary/non-root installs keep the default false and use Accessibility as fallback.
 */
internal object ClipboardAccess {
    @JvmStatic
    fun isLsposedActive(): Boolean = false

    fun isAccessibilityEnabled(context: Context): Boolean {
        val expected = ComponentName(context, ClipboardAccessibilityService::class.java)
        val enabled = AndroidSettings.Secure.getString(
            context.contentResolver,
            AndroidSettings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
        ).orEmpty()
        return enabled
            .split(':')
            .mapNotNull(ComponentName::unflattenFromString)
            .any { it == expected }
    }

    fun mode(context: Context): ClipboardSyncMode = selectClipboardSyncMode(
        lsposedActive = isLsposedActive(),
        accessibilityEnabled = isAccessibilityEnabled(context),
    )
}
