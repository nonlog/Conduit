package com.conduit.sync

import android.content.pm.PackageManager
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class PairingScannerInstrumentedTest {
    @Test
    fun scanner_is_local_nonexported_and_cancellation_does_not_create_a_pairing_code() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val contract = ScanContract()
        val intent = contract.createIntent(context, ScanOptions()
            .setDesiredBarcodeFormats(ScanOptions.QR_CODE)
            .setBarcodeImageEnabled(false)
            .setCaptureActivity(PairingScannerActivity::class.java))
        assertEquals(context.packageName, intent.component?.packageName)
        val activity = context.packageManager.getActivityInfo(requireNotNull(intent.component), 0)
        assertFalse(activity.exported)
        assertNull(contract.parseResult(android.app.Activity.RESULT_CANCELED, null).contents)
        // This was the release crash's exact initialization chain. It is no longer
        // packaged; scanning cannot accidentally restart an ML Kit background runtime.
        try {
            Class.forName("com.google.mlkit.vision.codescanner.GmsBarcodeScanning")
            fail("The Play-services scanner must not remain in the APK")
        } catch (_: ClassNotFoundException) { }
    }
}
