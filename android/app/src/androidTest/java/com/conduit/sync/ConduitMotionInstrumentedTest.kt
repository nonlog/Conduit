package com.conduit.sync

import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.Text
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Virtual-clock assertions test intermediate movement, not just a final screenshot. CI emulator only. */
@RunWith(AndroidJUnit4::class)
class ConduitMotionInstrumentedTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun pageTransitionMovesThenSettlesAndPreservesTabState() {
        var page by mutableStateOf(0)
        compose.setContent {
            ConduitTheme(darkOverride = true, motionOverride = true) {
                Box(Modifier.size(320.dp, 400.dp)) {
                    MotionPageHost(page) { shown ->
                        var count by rememberSaveable { mutableStateOf(0) }
                        Column {
                            Text("Page $shown count $count")
                            Button(onClick = { count++ }) { Text("Increment $shown") }
                        }
                    }
                }
            }
        }
        compose.onNodeWithText("Increment 0").performClick()
        compose.mainClock.autoAdvance = false
        compose.runOnIdle { page = 3 }
        compose.mainClock.advanceTimeByFrame()
        compose.mainClock.advanceTimeBy(64)
        val middle = compose.onNodeWithTag("route-page-3").getUnclippedBoundsInRoot().left.value
        assertTrue("Page must visibly move during the transition: $middle", middle > 0.5f && middle < 60f)
        compose.mainClock.advanceTimeBy(600)
        assertEquals(0f, compose.onNodeWithTag("route-page-3").getUnclippedBoundsInRoot().left.value, 0.75f)
        compose.runOnIdle { page = 0 }
        compose.mainClock.advanceTimeBy(600)
        compose.onNodeWithText("Page 0 count 1").assertIsDisplayed()
        compose.onNodeWithText("Page 3 count 0").assertDoesNotExist()
    }

    @Test
    fun rapidNavigationAndChangingReducedMotionNeverLeaveStalePage() {
        var page by mutableStateOf(0)
        var motion by mutableStateOf(true)
        compose.setContent {
            ConduitTheme(motionOverride = motion) {
                Box(Modifier.size(320.dp, 400.dp)) { MotionPageHost(page) { Text("Destination $it") } }
            }
        }
        compose.mainClock.autoAdvance = false
        compose.runOnIdle { page = 3 }
        compose.mainClock.advanceTimeBy(48)
        compose.runOnIdle { page = 1 }
        compose.mainClock.advanceTimeBy(48)
        compose.runOnIdle { motion = false; page = 2 }
        compose.mainClock.advanceTimeBy(600)
        compose.onNodeWithText("Destination 2").assertIsDisplayed()
        compose.onNodeWithText("Destination 3").assertDoesNotExist()
        compose.onNodeWithText("Destination 1").assertDoesNotExist()
        compose.runOnIdle { page = 0 }
        compose.mainClock.advanceTimeByFrame()
        compose.mainClock.advanceTimeByFrame()
        assertEquals(0f, compose.onNodeWithTag("route-page-0").getUnclippedBoundsInRoot().left.value, 0.75f)
    }

    @Test
    fun dockIndicatorHasAnIntermediatePositionAndOneSelectedDestination() {
        var selected by mutableStateOf("Home")
        compose.setContent {
            ConduitTheme(motionOverride = true) {
                ExpressiveNavigationDock(selectedSettings = selected == "Settings",
                    onHome = { selected = "Home" }, onSettings = { selected = "Settings" }, destination = selected)
            }
        }
        val initial = compose.onNodeWithTag("navigation-indicator").getUnclippedBoundsInRoot().left.value
        compose.mainClock.autoAdvance = false
        compose.onNodeWithContentDescription("Settings").performClick()
        compose.mainClock.advanceTimeByFrame()
        compose.mainClock.advanceTimeBy(48)
        val middle = compose.onNodeWithTag("navigation-indicator").getUnclippedBoundsInRoot().left.value
        compose.mainClock.advanceTimeBy(1200)
        val end = compose.onNodeWithTag("navigation-indicator").getUnclippedBoundsInRoot().left.value
        assertTrue("Indicator must glide rather than jump: initial=$initial intermediate=$middle final=$end", middle > initial && middle < end)
        assertEquals("Settings", selected)
    }

    @Test
    fun expandableSectionCollapsesWithFiniteHeightTransition() {
        compose.setContent {
            ConduitTheme(motionOverride = true) {
                MotionSettingsSection("Test section") { Box(Modifier.height(160.dp)) { Text("Details") } }
            }
        }
        compose.mainClock.autoAdvance = false
        compose.onNodeWithTag("section-Test section").performClick()
        compose.mainClock.advanceTimeByFrame()
        compose.mainClock.advanceTimeBy(64)
        val bounds = compose.onNodeWithTag("section-content-Test section").getUnclippedBoundsInRoot()
        val height = (bounds.bottom - bounds.top).value
        assertTrue("Collapse must have an intermediate height", height > 1f && height < 159f)
        compose.mainClock.advanceTimeBy(600)
        compose.onNodeWithText("Details").assertDoesNotExist()
        compose.onNodeWithTag("section-Test section").performClick()
        compose.mainClock.advanceTimeBy(600)
        compose.onNodeWithText("Details").assertIsDisplayed()
    }

    @Test
    fun dialogDismissalFinishesBeforeItsModalHostIsDisposed() {
        var visible by mutableStateOf(false)
        compose.setContent {
            ConduitTheme(motionOverride = true) {
                MotionAlertDialog(visible, { visible = false }, title = { Text("Pair computer") },
                    text = { Text("Enter code") }, confirmButton = { Button(onClick = { visible = false }) { Text("Done") } })
            }
        }
        compose.runOnIdle { visible = true }
        compose.onNodeWithText("Done").assertIsDisplayed()
        compose.mainClock.autoAdvance = false
        compose.onNodeWithText("Done").performClick()
        compose.mainClock.advanceTimeByFrame()
        compose.mainClock.advanceTimeBy(48)
        compose.onNodeWithTag("dialog-motion-host").assertExists()
        compose.mainClock.advanceTimeBy(500)
        compose.onNodeWithTag("dialog-motion-host").assertDoesNotExist()
    }
}
