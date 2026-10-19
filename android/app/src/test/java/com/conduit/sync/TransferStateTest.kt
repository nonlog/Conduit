package com.conduit.sync

import com.conduit.sync.proto.FileCancel
import com.conduit.sync.proto.FileResult
import com.conduit.sync.proto.PairRequest
import com.google.protobuf.ByteString
import org.junit.Assert.*
import org.junit.Test

class TransferStateTest {
    @Test fun cancellationWaitsForReceiverAndDoesNotFalselyOverridePublication() {
        val sending = TransferReceipt("cancel-mid-file")
        sending.sending()
        assertTrue(sending.requestCancel())
        assertEquals(TransferPhase.Cancelling, sending.phase)
        assertFalse(sending.phase.terminal)
        assertFalse(sending.result(sending.id, true, false, ""))
        assertTrue(sending.result(sending.id, false, true, "Cancelled"))
        assertEquals(TransferPhase.Cancelled, sending.phase)

        val completed = TransferReceipt("cancel-after-last-byte")
        completed.sending()
        completed.sent(true)
        completed.requestCancel()
        assertTrue(completed.result(completed.id, true, false, ""))
        assertEquals(TransferPhase.Completed, completed.phase)
    }

    @Test fun unpredictableIdsAreAllocatedIndependentlyEvenForIdenticalShares() {
        val ids = List(100) { newTransferId() }
        assertEquals(100, ids.toSet().size)
        ids.forEach { assertEquals(16, transferIdBytes(it).size); assertEquals(it, transferIdString(transferIdBytes(it))) }
    }

    @Test fun bytesSentRequiresMatchingReceiverConfirmation() {
        val receipt = TransferReceipt("current")
        assertTrue(receipt.sending())
        assertFalse(receipt.result("current", true, false, "")) // premature success
        assertTrue(receipt.sent(confirmable = true))
        assertEquals(TransferPhase.WaitingConfirmation, receipt.phase)
        assertFalse(receipt.result("stale", true, false, ""))
        assertTrue(receipt.result("current", true, false, ""))
        assertEquals(TransferPhase.Completed, receipt.phase)
        assertFalse(receipt.result("current", false, false, "late failure"))
    }

    @Test fun earlyNegativeResultStopsSendingAndCannotBeRevived() {
        val receipt = TransferReceipt("current")
        receipt.sending()
        assertTrue(receipt.result("current", false, false, "Downloads is full"))
        assertEquals(TransferPhase.Failed, receipt.phase)
        assertEquals("Downloads is full", receipt.error)
        assertFalse(receipt.sent(true))
        assertFalse(receipt.result("current", true, false, ""))
    }

    @Test fun cancelDisconnectAndTimeoutNeverConfirmCompletion() {
        for (phase in listOf(TransferPhase.Cancelled, TransferPhase.Failed)) {
            val receipt = TransferReceipt("a")
            receipt.sending()
            receipt.sent(true)
            assertTrue(receipt.finish(phase, "cancel, disconnect or timeout"))
            assertFalse(receipt.result("a", true, false, ""))
            assertEquals(phase, receipt.phase)
        }
        val receipt = TransferReceipt("a")
        receipt.sending()
        assertTrue(receipt.result("a", false, true, "Remote cancel"))
        assertEquals(TransferPhase.Cancelled, receipt.phase)
    }

    @Test fun legacyPeerIsExplicitlyUnconfirmedAndQueuedJobsCanBeCancelled() {
        val legacy = TransferReceipt("a")
        legacy.sending()
        legacy.sent(false)
        assertEquals(TransferPhase.Unconfirmed, legacy.phase)
        assertFalse(legacy.result("a", true, false, ""))
        val queued = TransferReceipt("b")
        assertTrue(queued.finish(TransferPhase.Cancelled))
        assertFalse(queued.sending())
    }

    @Test fun capabilityAndCancellationUseTheExactSharedSchema() {
        assertEquals(1, PairRequest.parseFrom(PairRequest.newBuilder().setTransferProtocol(1).build().toByteArray()).transferProtocol)
        val id = ByteString.copyFrom(ByteArray(16) { 4 })
        val cancel = FileCancel.parseFrom(FileCancel.newBuilder().setTransferId(id).setReason("user").build().toByteArray())
        assertEquals(id, cancel.transferId)
        val result = FileResult.parseFrom(FileResult.newBuilder().setTransferId(id).setCancelled(true).build().toByteArray())
        assertTrue(result.cancelled)
        assertFalse(result.success)
    }
}
