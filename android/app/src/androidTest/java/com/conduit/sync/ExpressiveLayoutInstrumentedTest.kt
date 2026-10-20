package com.conduit.sync

import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.assertDoesNotExist
import androidx.compose.ui.test.assertExists
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.io.FileOutputStream
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Runs on a clean GitHub Actions emulator, never on the user's linked device. */
@RunWith(AndroidJUnit4::class)
class ExpressiveLayoutInstrumentedTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun matchedSendButtonsRemainAlignedAtPhoneWidth() {
        var sendFiles = 0
        var sendPhoto = 0
        compose.setContent {
            MaterialTheme {
                Box(Modifier.width(360.dp)) {
                    ExpressiveDeviceCard(
                        state = LinkState.Connected,
                        peerName = "Desktop",
                        pairedDeviceId = "fixture-not-a-real-peer",
                        pairing = false,
                        path = "LAN",
                        onConnect = {},
                        onDisconnect = {},
                        onPair = {},
                        onCancelPair = {},
                        onSendFiles = { sendFiles++ },
                        onSendPhoto = { sendPhoto++ },
                    )
                }
            }
        }
        val files = compose.onNodeWithContentDescription("Send files")
        val photo = compose.onNodeWithContentDescription("Send photo")
        files.assertExists().assertIsDisplayed()
        photo.assertExists().assertIsDisplayed()
        val fb = files.getUnclippedBoundsInRoot()
        val pb = photo.getUnclippedBoundsInRoot()
        assertEquals(60f, fb.height.value, 0.75f)
        assertEquals(fb.height.value, pb.height.value, 0.75f)
        assertEquals(fb.top.value, pb.top.value, 0.75f)
        assertEquals(fb.bottom.value, pb.bottom.value, 0.75f)
        assertEquals(fb.width.value, pb.width.value, 0.75f)
        assertTrue("Standard phone must show actions side by side", pb.left > fb.right)
        files.performClick()
        photo.performClick()
        compose.runOnIdle {
            assertEquals(1, sendFiles)
            assertEquals(1, sendPhoto)
        }
        saveScreenshot("phone-connected-360dp.png")
    }

    @Test
    fun narrowWidthStacksButtonsWithoutClipping() {
        compose.setContent {
            MaterialTheme {
                Box(Modifier.width(276.dp)) {
                    ExpressiveDeviceCard(
                        state = LinkState.Connected,
                        peerName = "Desktop with a longer name",
                        pairedDeviceId = "fixture-not-a-real-peer",
                        pairing = false,
                        path = "Relay · TYO",
                        onConnect = {},
                        onDisconnect = {},
                        onPair = {},
                        onCancelPair = {},
                        onSendFiles = {},
                        onSendPhoto = {},
                    )
                }
            }
        }
        val fb = compose.onNodeWithContentDescription("Send files").assertIsDisplayed().getUnclippedBoundsInRoot()
        val pb = compose.onNodeWithContentDescription("Send photo").assertIsDisplayed().getUnclippedBoundsInRoot()
        assertEquals(60f, fb.height.value, 0.75f)
        assertEquals(fb.height.value, pb.height.value, 0.75f)
        assertTrue("Photo action must stack below files", pb.top > fb.bottom)
        saveScreenshot("phone-narrow-276dp.png")
    }

    @Test
    fun suspendedNavigationRemainsInteractiveAndSelected() {
        var showSettings by mutableStateOf(false)
        compose.setContent {
            MaterialTheme {
                ExpressiveNavigationDock(
                    selectedSettings = showSettings,
                    onHome = { showSettings = false },
                    onSettings = { showSettings = true },
                )
            }
        }
        compose.onNodeWithContentDescription("Settings").assertIsDisplayed().performClick()
        compose.runOnIdle { assertTrue(showSettings) }
        compose.onNodeWithContentDescription("Home").assertIsDisplayed().performClick()
        compose.runOnIdle { assertTrue(!showSettings) }
        saveScreenshot("floating-dock.png")
    }

    @Test
    fun largerSystemFontStacksActionsWithFullTouchTargets() {
        compose.setContent {
            val base = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(base.density, 1.5f)) {
                MaterialTheme {
                    Box(Modifier.width(360.dp)) {
                        ExpressiveDeviceCard(
                            state = LinkState.Connected,
                            peerName = "A very long desktop name for accessibility",
                            pairedDeviceId = "fixture-not-a-real-peer",
                            pairing = false,
                            path = "Relay",
                            onConnect = {},
                            onDisconnect = {},
                            onPair = {},
                            onCancelPair = {},
                            onSendFiles = {},
                            onSendPhoto = {},
                        )
                    }
                }
            }
        }
        val files = compose.onNodeWithContentDescription("Send files")
            .assertIsDisplayed().getUnclippedBoundsInRoot()
        val photo = compose.onNodeWithContentDescription("Send photo")
            .assertIsDisplayed().getUnclippedBoundsInRoot()
        assertEquals(60f, files.height.value, 0.75f)
        assertEquals(60f, photo.height.value, 0.75f)
        assertTrue("Large fonts must stack rather than clip action text", photo.top > files.bottom)
        saveScreenshot("phone-large-font-360dp.png")
    }

    @Test
    fun disconnectedStateDoesNotOfferFileActions() {
        compose.setContent {
            MaterialTheme {
                Box(Modifier.width(360.dp)) {
                    ExpressiveDeviceCard(
                        state = LinkState.Retrying,
                        peerName = "Desktop",
                        pairedDeviceId = "fixture-not-a-real-peer",
                        pairing = false,
                        path = "Relay",
                        onConnect = {},
                        onDisconnect = {},
                        onPair = {},
                        onCancelPair = {},
                        onSendFiles = {},
                        onSendPhoto = {},
                    )
                }
            }
        }
        compose.onNodeWithContentDescription("Send files").assertDoesNotExist()
        compose.onNodeWithContentDescription("Send photo").assertDoesNotExist()
        saveScreenshot("phone-reconnecting.png")
    }

    private fun saveScreenshot(name: String) {
        val appContext = InstrumentationRegistry.getInstrumentation().targetContext
        val folder = File(appContext.getExternalFilesDir(null), "ui-smoke")
        folder.mkdirs()
        FileOutputStream(File(folder, name)).use {
            compose.onRoot().captureToImage().asAndroidBitmap().compress(Bitmap.CompressFormat.PNG, 100, it)
        }
    }
}
