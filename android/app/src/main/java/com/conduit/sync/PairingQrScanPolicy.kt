package com.conduit.sync

/** Prevents two scanner tasks from competing for the same Activity result callback. */
internal class PairingQrScanGate {
    private var active = false

    @Synchronized
    fun tryBegin(): Boolean {
        if (active) return false
        active = true
        return true
    }

    @Synchronized
    fun finish(): Boolean {
        if (!active) return false
        active = false
        return true
    }

    @Synchronized
    fun cancel() {
        active = false
    }
}

internal fun isPairingQrResultDeliverable(
    activityDestroyed: Boolean,
    activityFinishing: Boolean,
    started: Boolean,
): Boolean = !activityDestroyed && !activityFinishing && started
