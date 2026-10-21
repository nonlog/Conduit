package com.conduit.sync

import android.graphics.Bitmap
import androidx.compose.material3.MotionScheme
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertIsOff
import java.io.FileInputStream
import org.junit.Assert.assertSame
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
@OptIn(androidx.compose.material3.ExperimentalMaterial3ExpressiveApi::class)
@RunWith(AndroidJUnit4::class)
class ExpressiveLayoutInstrumentedTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun matchedSendButtonsRemainAlignedAtPhoneWidth() {
        var sendFiles = 0
        var sendPhoto = 0
        compose.setContent {
            ConduitTheme(darkOverride = true) {
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
        assertEquals(56f, (fb.bottom - fb.top).value, 0.75f)
        assertEquals((fb.bottom - fb.top).value, (pb.bottom - pb.top).value, 0.75f)
        assertEquals(fb.top.value, pb.top.value, 0.75f)
        assertEquals(fb.bottom.value, pb.bottom.value, 0.75f)
        assertEquals((fb.right - fb.left).value, (pb.right - pb.left).value, 0.75f)
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
            ConduitTheme(darkOverride = true) {
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
        assertEquals(56f, (fb.bottom - fb.top).value, 0.75f)
        assertEquals((fb.bottom - fb.top).value, (pb.bottom - pb.top).value, 0.75f)
        assertTrue("Photo action must stack below files", pb.top > fb.bottom)
        saveScreenshot("phone-narrow-276dp.png")
    }

    @Test
    fun suspendedNavigationRemainsInteractiveAndSelected() {
        var showSettings by mutableStateOf(false)
        compose.setContent {
            ConduitTheme(darkOverride = true) {
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
                ConduitTheme(darkOverride = true) {
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
        assertEquals(56f, (files.bottom - files.top).value, 0.75f)
        assertEquals(56f, (photo.bottom - photo.top).value, 0.75f)
        assertTrue("Large fonts must stack rather than clip action text", photo.top > files.bottom)
        saveScreenshot("phone-large-font-360dp.png")
    }

    @Test
    fun disconnectedStateDoesNotOfferFileActions() {
        compose.setContent {
            ConduitTheme(darkOverride = true) {
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

    @Test
    fun publicExpressiveThemeUsesNeutralReferenceSurfaceAndOfficialMotion() {
        compose.setContent {
            ConduitTheme(darkOverride = true) {
                assertSame(MotionScheme.expressive(), MaterialTheme.motionScheme)
                assertEquals(0xFF111414.toInt(), MaterialTheme.colorScheme.background.toArgb())
                assertEquals(0xFF1E2020.toInt(), MaterialTheme.colorScheme.surfaceContainerLow.toArgb())
                assertEquals(24f, MaterialTheme.typography.headlineLarge.fontSize.value, 0.01f)
            }
        }
    }

    @Test
    fun dashboardDarkShowsCompactActionsWithoutMarketingSubtitle() {
        compose.setContent { ConduitTheme(darkOverride = true) { DashboardFixture() } }
        compose.onNodeWithContentDescription("Send files").assertIsDisplayed()
        compose.onNodeWithText("Your devices, in sync").assertDoesNotExist()
        compose.onNodeWithText("Connection").assertIsDisplayed()
        saveScreenshot("dashboard-dark.png")
    }

    @Test
    fun settingsLightUsesGroupedRowsAndThemeControls() {
        compose.setContent { ConduitTheme(darkOverride = false) { DashboardFixture() } }
        compose.onNodeWithContentDescription("Settings").performClick()
        compose.onNodeWithText("Theme").assertIsDisplayed()
        compose.onNodeWithContentDescription("Wallpaper accent").assertIsDisplayed()
        compose.onNodeWithText("Liquid glass").assertDoesNotExist()
        saveScreenshot("settings-light.png")
    }

    @Test
    fun settingsDarkHasNoOversizedSectionHeadings() {
        compose.setContent { ConduitTheme(darkOverride = true) { DashboardFixture() } }
        compose.onNodeWithContentDescription("Settings").performClick()
        compose.onNodeWithText("Devices").assertIsDisplayed()
        saveScreenshot("settings-dark.png")
    }

    @Test
    fun wholeSwitchRowHasSingleAccessibleToggleAction() {
        var enabled by mutableStateOf(false)
        var calls = 0
        compose.setContent {
            ConduitTheme(darkOverride = true) {
                SwitchPreferenceRow(R.drawable.ic_palette, "Test preference", checked = enabled,
                    onCheckedChange = { enabled = it; calls++ })
            }
        }
        val row = compose.onNodeWithContentDescription("Test preference")
        row.assertIsOff().performClick().assertIsOn()
        compose.runOnIdle { assertEquals(1, calls) }
    }

    @Test
    fun fourDestinationsOpenRealTransferAndClipboardPages() {
        compose.setContent { ConduitTheme(darkOverride = true) { DashboardFixture() } }
        compose.onNodeWithContentDescription("Transfers").performClick()
        compose.onNodeWithText("No transfers yet").assertIsDisplayed()
        saveScreenshot("transfers-empty-dark.png")
        compose.onNodeWithContentDescription("Clipboard").performClick()
        compose.onNodeWithText("Search clipboard").assertIsDisplayed()
        saveScreenshot("clipboard-empty-dark.png")
        compose.onNodeWithContentDescription("Home").performClick()
        compose.onNodeWithContentDescription("Send files").assertIsDisplayed()
    }

    @Test
    fun groupedSurfaceNeverOverlaysSequentialSettingsRows() {
        compose.setContent {
            ConduitTheme(darkOverride = true) {
                ExpressiveCard {
                    SwitchPreferenceRow(R.drawable.ic_settings, "First preference", checked = false, onCheckedChange = {})
                    SettingsDivider()
                    SwitchPreferenceRow(R.drawable.ic_settings, "Second preference", checked = true, onCheckedChange = {})
                }
            }
        }
        val first = compose.onNodeWithContentDescription("First preference").assertIsDisplayed().getUnclippedBoundsInRoot()
        val second = compose.onNodeWithContentDescription("Second preference").assertIsDisplayed().getUnclippedBoundsInRoot()
        assertTrue("Rows in a grouped card must not overlap", second.top >= first.bottom)
        saveScreenshot("grouped-settings-rows.png")
    }

    @Test
    fun homeShowsTransferCountButKeepsTerminalHistoryOnTransfersPage() {
        val history = listOf(
            FileTransfer(FileTransferDirection.ToPhone, "qa-received.bin",
                2097152L, 2097152L, "qa-received", TransferPhase.Completed),
            FileTransfer(FileTransferDirection.ToDesktop, "qa-cancelled.bin",
                0L, 1048576L, "qa-cancelled", TransferPhase.Cancelled),
        )
        compose.setContent { ConduitTheme(darkOverride = false) { DashboardFixture(transfers = history) } }
        compose.onNodeWithContentDescription("Open transfer history").assertIsDisplayed()
        compose.onNodeWithText("File transfers", useUnmergedTree = true).assertExists()
        compose.onNodeWithText("2", useUnmergedTree = true).assertExists()
        compose.onNodeWithText("qa-received.bin").assertDoesNotExist()
        compose.onNodeWithText("qa-cancelled.bin").assertDoesNotExist()
        saveScreenshot("home-transfer-summary-light.png")
        compose.onNodeWithContentDescription("Open transfer history").performClick()
        compose.onNodeWithText("Recent transfers").assertIsDisplayed()
        compose.onNodeWithText("qa-received.bin").assertExists()
        compose.onNodeWithText("qa-cancelled.bin").assertExists()
    }

    @Test
    fun homeSummarizesActiveTransferAndPreservesCancellationInHistory() {
        val active = FileTransfer(FileTransferDirection.ToDesktop, "upload-in-progress.bin",
            262144L, 1048576L, "active-upload", TransferPhase.Sending, canCancel = true)
        var cancelledId: String? = null
        compose.setContent {
            ConduitTheme(darkOverride = true) {
                DashboardFixture(transfers = listOf(active),
                    onCancelTransfer = { cancelledId = it })
            }
        }
        compose.onNodeWithContentDescription("Open transfer history").assertIsDisplayed()
        compose.onNodeWithText("1 in progress · Tap to manage", useUnmergedTree = true).assertExists()
        compose.onNodeWithText("upload-in-progress.bin · 25%", useUnmergedTree = true).assertExists()
        // Home never renders a full-size history card or its cancel button.
        compose.onNodeWithContentDescription("Cancel upload-in-progress.bin").assertDoesNotExist()
        saveScreenshot("home-active-transfer-summary-dark.png")
        compose.onNodeWithContentDescription("Open transfer history").performClick()
        compose.onNodeWithContentDescription("Cancel upload-in-progress.bin").assertIsDisplayed().performClick()
        compose.runOnIdle { assertEquals("active-upload", cancelledId) }
    }

    @Test
    fun realOnePlusBatterySettingsRowsNeverOverlapAtLargeFontScale() {
        var optimizationsOpened = 0
        var backgroundSettingsOpened = 0
        compose.setContent {
            val base = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(base.density, 1.4f)) {
                ConduitTheme(darkOverride = false) {
                    Box(Modifier.width(320.dp)) {
                        BackgroundDeliverySettingsCard(
                            batteryOptimizationIgnored = true,
                            showOplusBatteryRow = true,
                            oplusAppFreezeSupported = false,
                            onRequestBatteryOptimization = { optimizationsOpened++ },
                            onOpenOplusFreezeSettings = { backgroundSettingsOpened++ },
                        )
                    }
                }
            }
        }
        val first = compose.onNodeWithText("Battery optimization", useUnmergedTree = true)
            .assertIsDisplayed().getUnclippedBoundsInRoot()
        val second = compose.onNodeWithText("Background battery settings", useUnmergedTree = true)
            .assertIsDisplayed().getUnclippedBoundsInRoot()
        assertTrue("OEM battery controls must occupy separate vertical rows", second.top >= first.bottom)
        saveScreenshot("oneplus-battery-settings-light-large-font.png")
        compose.onNodeWithText("Battery optimization").performClick()
        compose.onNodeWithText("Background battery settings").performClick()
        compose.runOnIdle {
            assertEquals(1, optimizationsOpened)
            assertEquals(1, backgroundSettingsOpened)
        }
    }

    @androidx.compose.runtime.Composable
    private fun DashboardFixture(
        transfers: List<FileTransfer> = emptyList(),
        onCancelTransfer: (String) -> Unit = {},
    ) {
        ConduitApp(
            appVersion = "test", peerName = "Desktop", pairedDeviceId = "ui-fixture", pairing = false,
            path = "LAN", state = LinkState.Connected, history = emptyList(), transfers = transfers,
            onCancelTransfer = onCancelTransfer, hideNotifications = false, onHideNotifications = {},
            clipboardMode = ClipboardSyncMode.Unavailable, clipboardAccessibilityEnabled = false,
            onOpenClipboardAccessibility = {}, batteryOptimizationIgnored = true,
            oplusAppFreezeSupported = false, onRequestBatteryOptimization = {}, onOpenOplusFreezeSettings = {},
            onConnect = {}, onDisconnect = {}, onPair = {}, onScanPairQr = {}, onCancelPair = {},
            onForget = {}, onSendFiles = {}, onClearHistory = {},
        )
    }

    private fun saveScreenshot(name: String) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val context = instrumentation.targetContext
        val folder = File(context.getExternalFilesDir(null), "ui-smoke").apply { mkdirs() }
        val png = File(folder, name)
        FileOutputStream(png).use {
            compose.onRoot().captureToImage().asAndroidBitmap().compress(Bitmap.CompressFormat.PNG, 100, it)
        }
        // Copy through the emulator shell while the test APK still exists. AGP uninstalls
        // it after tests and removes app-specific external storage, so pulling that later loses PNGs.
        fun shell(command: String): String = instrumentation.uiAutomation.executeShellCommand(command).use { fd ->
            FileInputStream(fd.fileDescriptor).use { it.readBytes().decodeToString().trim() }
        }
        // UiAutomation executes an argv, NOT a shell script: chaining with && merely
        // passes those words to mkdir. Separate commands ensure the PNG is really copied.
        val destination = "/sdcard/Download/conduit-ui-smoke/$name"
        assertTrue(shell("mkdir -p /sdcard/Download/conduit-ui-smoke").isBlank())
        assertTrue(shell("cp ${png.absolutePath} $destination").isBlank())
        val exportedBytes = shell("stat -c %s $destination").toLongOrNull()
        assertEquals("Screenshot must survive AGP test-app uninstall", png.length(), exportedBytes)

    }
}
