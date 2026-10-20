package com.conduit.sync

/** A passive capture observer can report no new item without implying a failed send. */
internal fun shouldReportMissingCapture(emptyIsNoop: Boolean, loadFailed: Boolean): Boolean =
    loadFailed || !emptyIsNoop
