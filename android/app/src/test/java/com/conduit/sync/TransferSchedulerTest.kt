package com.conduit.sync

import org.junit.Assert.*
import org.junit.Test

class TransferSchedulerTest {
    @Test fun nextFileWaitsForPublicationWithoutBlockingControlFramesOrSpinning() {
        val scheduler = TransferScheduler()
        val trace = mutableListOf<String>()
        var acknowledged = false
        val first: () -> Boolean = {
            if (acknowledged) false else {
                trace += "file-bytes-sent"
                scheduler.pauseBulk()
                true
            }
        }
        scheduler.bulk(first)
        scheduler.bulk { trace += "next-photo"; false }
        assertTrue(scheduler.step())
        repeat(3) { assertFalse(scheduler.step()) }
        repeat(40) { n -> scheduler.control { trace += "notification-$n" } }
        repeat(40) { assertTrue(scheduler.step()) }
        assertFalse(scheduler.step())
        assertFalse(trace.contains("next-photo"))
        scheduler.control(priority = true) {
            trace += "file-published"
            acknowledged = true
            scheduler.resumeBulk(first)
        }
        while (scheduler.step()) { }
        assertEquals("next-photo", trace.last())
        assertEquals(1, trace.count { it == "file-bytes-sent" })
        assertTrue(trace.indexOf("file-published") < trace.indexOf("next-photo"))
    }

    @Test fun cancellingAnotherQueuedJobDoesNotReleaseTheWaitingHead() {
        val scheduler = TransferScheduler()
        val head: () -> Boolean = { scheduler.pauseBulk(); true }
        val queued: () -> Boolean = { fail("cancelled item ran"); false }
        scheduler.bulk(head)
        scheduler.bulk(queued)
        scheduler.step()
        scheduler.resumeBulk(queued)
        assertTrue(scheduler.removeQueuedBulk(queued))
        assertFalse(scheduler.step())
    }

    @Test fun controlsAndCancellationRunBetweenBulkFramesWithoutInterleavingBulkJobs() {
        val scheduler = TransferScheduler()
        val trace = mutableListOf<String>()
        var chunks = 0
        var cancelled = false
        assertTrue(scheduler.bulk {
            if (cancelled) false else { trace += "mp3-${++chunks}"; true }
        })
        assertTrue(scheduler.bulk { trace += "photo"; false })
        scheduler.step()
        scheduler.control { trace += "clipboard" }
        scheduler.control(priority = true) { trace += "ack" }
        scheduler.control(priority = true) { trace += "pong" }
        scheduler.step()
        scheduler.step()
        scheduler.step()
        scheduler.control(priority = true) { cancelled = true; trace += "cancel" }
        scheduler.step()
        scheduler.step() // retires only the cancelled mp3
        scheduler.step()
        assertEquals(listOf("mp3-1", "ack", "pong", "clipboard", "cancel", "photo"), trace)
        assertFalse(scheduler.step())
    }

    @Test fun saturationRejectsTheNewTaskAndNeverDiscardsAcceptedWork() {
        val scheduler = TransferScheduler(capacity = 2, bulkCapacity = 1)
        val trace = mutableListOf<Int>()
        assertTrue(scheduler.control { trace += 1 })
        assertTrue(scheduler.control { trace += 2 })
        assertFalse(scheduler.control { trace += 3 })
        assertTrue(scheduler.bulk { trace += 4; false })
        assertFalse(scheduler.bulk { trace += 5; false })
        while (scheduler.step()) { /* deterministic drain, no running thread */ }
        assertEquals(listOf(1, 2, 4), trace)
    }

    @Test fun cleanupIsCoalescedAndCannotBeLostToAFullControlQueue() {
        var cleaned = 0
        val scheduler = TransferScheduler(capacity = 1, onMaintenance = { cleaned++ })
        scheduler.control { }
        assertFalse(scheduler.control { fail("must not run") })
        scheduler.requestMaintenance()
        scheduler.requestMaintenance()
        scheduler.step()
        assertEquals(1, cleaned)
        scheduler.close()
        assertFalse(scheduler.control { fail("closed") })
        scheduler.step()
        assertEquals(2, cleaned)
    }

    @Test fun notificationBurstsCannotStarveBulkIndefinitely() {
        val scheduler = TransferScheduler()
        var chunks = 0
        scheduler.bulk { chunks++; false }
        repeat(40) { scheduler.control { } }
        repeat(17) { scheduler.step() }
        assertEquals(1, chunks)
    }

    @Test fun cancellingAQueuedJobImmediatelyFreesItsAdmissionSlot() {
        val scheduler = TransferScheduler(bulkCapacity = 2)
        val cancelled: () -> Boolean = { fail("cancelled source must never be opened"); false }
        scheduler.bulk { true }
        scheduler.step()
        assertTrue(scheduler.bulk(cancelled))
        assertFalse(scheduler.bulk { false })
        assertTrue(scheduler.removeQueuedBulk(cancelled))
        assertTrue(scheduler.bulk { false })
    }
}
