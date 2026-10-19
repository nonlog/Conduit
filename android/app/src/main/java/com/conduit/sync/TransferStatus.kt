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
    val id: String = "",
    val phase: TransferPhase = if (direction == FileTransferDirection.ToPhone) TransferPhase.Receiving else TransferPhase.Sending,
    val error: String = "",
    val canCancel: Boolean = false,
) {
    val fraction: Float
        get() = if (total <= 0L) 0f else (transferred.toDouble() / total.toDouble())
            .coerceIn(0.0, 1.0)
            .toFloat()

    val percent: Int
        get() = (fraction * 100f).toInt().coerceIn(0, 100)
}

/** Keeps active jobs and at most 32 terminal rows. Terminal IDs cannot be revived by late work. */
internal fun updateTransferHistory(entries: List<FileTransfer>, value: FileTransfer): List<FileTransfer> {
    val old = entries.firstOrNull { it.id == value.id && it.direction == value.direction }
    if (old?.phase?.terminal == true) return entries
    val updated = if (old == null) entries + value else entries.map { if (it === old) value else it }
    val history = updated.filter { it.phase.terminal }.takeLast(32).toSet()
    return updated.filter { !it.phase.terminal || it in history }
}

object FileTransfers {
    var entries by mutableStateOf<List<FileTransfer>>(emptyList())
        private set

    val toDesktop get() = entries.firstOrNull { it.direction == FileTransferDirection.ToDesktop && !it.phase.terminal }
    val toPhone get() = entries.firstOrNull { it.direction == FileTransferDirection.ToPhone && !it.phase.terminal }

    @Synchronized fun update(value: FileTransfer) { entries = updateTransferHistory(entries, value) }

    @Synchronized fun failActive(reason: String) {
        entries = entries.map {
            if (it.phase.terminal) it else it.copy(phase = TransferPhase.Failed, error = reason, canCancel = false)
        }.takeLast(32)
    }

    @Synchronized fun clearAll() { entries = emptyList() }
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
