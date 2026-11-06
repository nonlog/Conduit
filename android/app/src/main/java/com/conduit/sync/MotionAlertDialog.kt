@file:OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
package com.conduit.sync

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.BasicAlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ProvideTextStyle
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

/** Keeps the native modal/focus contract until a finite dismiss transition has finished. */
@Composable
internal fun MotionAlertDialog(
    visible: Boolean,
    onDismissRequest: () -> Unit,
    title: @Composable () -> Unit,
    text: @Composable () -> Unit,
    confirmButton: @Composable () -> Unit,
    dismissButton: @Composable () -> Unit = {},
) {
    val state = remember { MutableTransitionState(false) }
    state.targetState = visible
    val motion = LocalConduitMotion.current
    if (state.currentState || state.targetState) {
        BasicAlertDialog(onDismissRequest = onDismissRequest) {
            AnimatedVisibility(
                visibleState = state,
                modifier = Modifier.testTag("dialog-motion-host"),
                enter = if (motion) fadeIn(tween(180)) + scaleIn(tween(220), initialScale = 0.94f) else EnterTransition.None,
                exit = if (motion) fadeOut(tween(140)) + scaleOut(tween(160), targetScale = 0.96f) else ExitTransition.None,
                label = "Dialog presence",
            ) {
                Surface(modifier = Modifier.testTag("motion-dialog").motionInteraction(visible),
                    shape = MaterialTheme.shapes.extraLarge,
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                    tonalElevation = 6.dp) {
                    Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
                        ProvideTextStyle(MaterialTheme.typography.headlineSmall) { title() }
                        Column(Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState())) {
                            ProvideTextStyle(MaterialTheme.typography.bodyMedium) { text() }
                        }
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                            dismissButton()
                            confirmButton()
                        }
                    }
                }
            }
        }
    }
}
