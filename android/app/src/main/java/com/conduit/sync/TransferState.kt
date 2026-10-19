package com.conduit.sync

import java.security.SecureRandom

internal fun newTransferId(): String = ByteArray(16).also { transferRandom.nextBytes(it) }
    .joinToString("") { "%02x".format(it.toInt() and 255) }
private val transferRandom = SecureRandom()

internal fun transferIdBytes(id: String): ByteArray = id.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
internal fun transferIdString(bytes: ByteArray): String = bytes.joinToString("") { "%02x".format(it.toInt() and 255) }

enum class TransferPhase(val label: String, val terminal: Boolean = false) {
    Queued("Queued"), Sending("Sending"), Receiving("Receiving"),
    WaitingConfirmation("Waiting for confirmation"), Cancelling("Cancelling"), Completed("Completed", true),
    Failed("Failed", true), Cancelled("Cancelled", true),
    Unconfirmed("Sent · receiver confirmation unavailable", true),
}

/** Session-owned result matching; neither stale nor duplicate results can finish another job. */
internal class TransferReceipt(val id: String) {
    var phase = TransferPhase.Queued
        private set
    var error = ""
        private set
    private var allBytesSent = false

    @Synchronized fun sending(): Boolean {
        if (phase != TransferPhase.Queued) return false
        phase = TransferPhase.Sending
        return true
    }

    @Synchronized fun sent(confirmable: Boolean): Boolean {
        if (phase != TransferPhase.Sending) return false
        allBytesSent = true
        phase = if (confirmable) TransferPhase.WaitingConfirmation else TransferPhase.Unconfirmed
        return true
    }

    @Synchronized fun result(resultId: String, success: Boolean, cancelled: Boolean, reason: String): Boolean {
        if (resultId != id || phase.terminal || phase == TransferPhase.Queued) return false
        // A rejection can arrive immediately after the offer. A success requires all bytes sent.
        if (success && !cancelled && !allBytesSent) return false
        phase = if (cancelled) TransferPhase.Cancelled else if (success) TransferPhase.Completed else TransferPhase.Failed
        error = reason.take(300)
        return true
    }

    @Synchronized fun requestCancel(reason: String = ""): Boolean {
        if (phase.terminal || phase == TransferPhase.Queued) return false
        phase = TransferPhase.Cancelling
        error = reason.take(300)
        return true
    }

    @Synchronized fun finish(phase: TransferPhase, reason: String = ""): Boolean {
        require(phase.terminal)
        if (this.phase.terminal) return false
        this.phase = phase
        error = reason.take(300)
        return true
    }
}
