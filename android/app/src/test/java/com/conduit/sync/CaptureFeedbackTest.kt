package com.conduit.sync

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CaptureFeedbackTest {
    @Test fun passiveObserverWithNoNewPhotoDoesNotNotifyFailure() {
        assertFalse(shouldReportMissingCapture(emptyIsNoop = true, loadFailed = false))
    }
    @Test fun actualLoadFailureIsStillVisible() {
        assertTrue(shouldReportMissingCapture(emptyIsNoop = true, loadFailed = true))
    }
    @Test fun explicitRequestDoesNotSilentlyDiscardMissingContent() {
        assertTrue(shouldReportMissingCapture(emptyIsNoop = false, loadFailed = false))
    }
}
