package com.conduit.sync

import android.database.ContentObserver
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics

/** Finite, input-driven motion. No frame loop or running offscreen animation is owned here. */
internal val LocalConduitMotion = staticCompositionLocalOf { true }

internal object ConduitMotion {
    const val pageMillis = 280
    const val revealMillis = 240
    const val feedbackMillis = 160
}

@Composable
internal fun rememberSystemMotionEnabled(): Boolean {
    val resolver = LocalContext.current.contentResolver
    fun enabled() = Settings.Global.getFloat(resolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) > 0f
    var allowed by remember(resolver) { mutableStateOf(enabled()) }
    DisposableEffect(resolver) {
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) { allowed = enabled() }
        }
        resolver.registerContentObserver(Settings.Global.getUriFor(Settings.Global.ANIMATOR_DURATION_SCALE), false, observer)
        onDispose { resolver.unregisterContentObserver(observer) }
    }
    return allowed
}

/** Preserve each tab's saveable state, slide directly between chosen tabs, never intermediate pages. */
@Composable
internal fun MotionPageHost(index: Int, modifier: Modifier = Modifier, content: @Composable (Int) -> Unit) {
    val saved = rememberSaveableStateHolder()
    val enabled = LocalConduitMotion.current
    AnimatedContent(
        targetState = index,
        modifier = modifier.fillMaxSize().clipToBounds(),
        transitionSpec = {
            if (!enabled) {
                EnterTransition.None togetherWith ExitTransition.None
            } else {
            val direction = if (targetState > initialState) 1 else -1
            ((slideInHorizontally(tween(ConduitMotion.pageMillis, easing = FastOutSlowInEasing)) { it * direction / 6 } +
                fadeIn(tween(200))) togetherWith
                (slideOutHorizontally(tween(ConduitMotion.pageMillis, easing = FastOutSlowInEasing)) { -it * direction / 8 } +
                    fadeOut(tween(160)))).using(SizeTransform(clip = false))
            }
        },
        label = "Tab transition",
    ) { shown ->
        // An exiting screen must not steal taps or appear twice to accessibility services.
        val active = shown == index
        Box(Modifier.fillMaxSize().testTag("route-page-$shown").motionInteraction(active)) {
            saved.SaveableStateProvider(shown) { content(shown) }
        }
    }
}

internal fun Modifier.motionInteraction(active: Boolean): Modifier = if (active) this else {
    clearAndSetSemantics { }.pointerInput(Unit) {
        awaitPointerEventScope {
            while (true) {
                // Suspends waiting for an actual input event; it is not a timer or polling loop.
                awaitPointerEvent(PointerEventPass.Initial).changes.forEach { it.consume() }
            }
        }
    }
}

@Composable
internal fun MotionReveal(visible: Boolean, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    if (!LocalConduitMotion.current) {
        if (visible) Box(modifier) { content() }
        return
    }
    // Start settled: opening an already connected app must not replay an entrance animation.
    val state = remember { MutableTransitionState(visible) }
    state.targetState = visible
    AnimatedVisibility(
        visibleState = state,
        modifier = modifier,
        enter = expandVertically(tween(ConduitMotion.revealMillis, easing = FastOutSlowInEasing)) + fadeIn(tween(180)),
        exit = shrinkVertically(tween(200, easing = FastOutSlowInEasing)) + fadeOut(tween(140)),
    ) {
        Box(Modifier.motionInteraction(visible)) { content() }
    }
}
