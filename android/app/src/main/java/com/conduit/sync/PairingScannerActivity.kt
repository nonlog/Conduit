package com.conduit.sync

import com.journeyapps.barcodescanner.CaptureActivity

/** CaptureActivity releases the camera in onPause; no scanner ever runs in SyncService. */
class PairingScannerActivity : CaptureActivity()
