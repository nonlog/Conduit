package com.conduit.sync

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingQrScanPolicyTest {
    @Test
    fun gate_allows_one_scan_and_reopens_after_completion() {
        val gate = PairingQrScanGate()

        assertTrue(gate.tryBegin())
        assertFalse(gate.tryBegin())
        assertTrue(gate.finish())
        assertFalse(gate.finish())
        assertTrue(gate.tryBegin())
    }

    @Test
    fun result_is_delivered_to_a_started_live_activity_without_waiting_for_resume() {
        assertTrue(isPairingQrResultDeliverable(false, false, true))
        assertFalse(isPairingQrResultDeliverable(true, false, true))
        assertFalse(isPairingQrResultDeliverable(false, true, true))
        assertFalse(isPairingQrResultDeliverable(false, false, false))
    }

    @Test
    fun real_offline_decoder_roundtrips_the_desktop_pairing_payload() {
        val payload = "conduit://pair?code=123456"
        val matrix = com.google.zxing.qrcode.QRCodeWriter().encode(payload,
            com.google.zxing.BarcodeFormat.QR_CODE, 256, 256)
        val pixels = IntArray(256 * 256) { index -> if (matrix[index % 256, index / 256]) -16777216 else -1 }
        val bitmap = com.google.zxing.BinaryBitmap(com.google.zxing.common.HybridBinarizer(
            com.google.zxing.RGBLuminanceSource(256, 256, pixels)))
        val result = com.google.zxing.qrcode.QRCodeReader().decode(bitmap)
        org.junit.Assert.assertEquals("123456", PairingCode.fromQrPayload(result.text))
        org.junit.Assert.assertNull(PairingCode.fromQrPayload("https://example.com/not-a-pairing-code"))
    }
}
