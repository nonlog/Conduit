package com.conduit.sync

import android.os.SystemClock
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

private const val PROGRESS_MIN_INTERVAL_MS = 250L

enum class FileTransferDirection {
    ToDesktop,
    ToPhone,
}

data class FileTransfer(
    val direction: FileTransferDirection,
    val name: String,
    val transferred: Long,
    val total: Long,
) {
    val fraction: Float
        get() = if (total <= 0L) 0f else (transferred.toDouble() / total.toDouble())
            .coerceIn(0.0, 1.0)
            .toFloat()

    val percent: Int
        get() = (fraction * 100f).toInt().coerceIn(0, 100)
}

/**
 * Two independent slots because TCP/Noise can carry one file in each direction at once.
 * Compose observes these fields directly; updates are synchronized because one direction is
 * driven by the sender executor and the other by the reader thread.
 */
object FileTransfers {
    var toDesktop by mutableStateOf<FileTransfer?>(null)
        private set

    var toPhone by mutableStateOf<FileTransfer?>(null)
        private set

    @Synchronized
    fun update(direction: FileTransferDirection, name: String, transferred: Long, total: Long) {
        val value = FileTransfer(
            direction = direction,
            name = name,
            transferred = transferred.coerceIn(0L, total.coerceAtLeast(0L)),
            total = total.coerceAtLeast(0L),
        )
        when (direction) {
            FileTransferDirection.ToDesktop -> toDesktop = value
            FileTransferDirection.ToPhone -> toPhone = value
        }
    }

    @Synchronized
    fun clear(direction: FileTransferDirection) {
        when (direction) {
            FileTransferDirection.ToDesktop -> toDesktop = null
            FileTransferDirection.ToPhone -> toPhone = null
        }
    }

    @Synchronized
    fun clearAll() {
        toDesktop = null
        toPhone = null
    }
}

/**
 * Keeps transfer UI/SystemUI updates bounded without changing the wire cadence.
 *
 * File chunks can arrive much faster than a human-readable progress indicator needs to repaint.
 * Publishing every 32 KiB chunk turns a large transfer into hundreds or thousands of main-thread
 * posts and notification-manager IPCs. Intermediate updates are therefore capped at 4 Hz; start
 * and final progress are always published immediately. One gate is used per transfer direction.
 */
class TransferProgressGate(
    private val minIntervalMs: Long = PROGRESS_MIN_INTERVAL_MS,
    private val clockMs: () -> Long = SystemClock::elapsedRealtime,
) {
    private var lastPublishedAt = Long.MIN_VALUE

    @Synchronized
    fun shouldPublish(transferred: Long, total: Long): Boolean {
        if (transferred <= 0L) {
            lastPublishedAt = clockMs()
            return true
        }
        if (total > 0L && transferred >= total) {
            lastPublishedAt = clockMs()
            return true
        }
        val now = clockMs()
        if (lastPublishedAt == Long.MIN_VALUE || now - lastPublishedAt >= minIntervalMs) {
            lastPublishedAt = now
            return true
        }
        return false
    }

    @Synchronized
    fun reset() {
        lastPublishedAt = Long.MIN_VALUE
    }
}

internal fun formatBytes(bytes: Long): String {
    if (bytes < 1024L) return "$bytes B"
    val units = arrayOf("KB", "MB", "GB")
    var value = bytes.toDouble()
    var unit = -1
    while (value >= 1024.0 && unit < units.lastIndex) {
        value /= 1024.0
        unit++
    }
    val digits = if (value >= 100.0) 0 else 1
    return "%.${digits}f %s".format(value, units[unit])
}
