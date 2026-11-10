@file:OptIn(androidx.compose.material3.ExperimentalMaterial3ExpressiveApi::class)

package com.conduit.sync

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Outline
import androidx.compose.ui.graphics.Path
import android.view.HapticFeedbackConstants
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.sin

internal object ExpressiveLayout {
    val pageInset = 16.dp
    val cardCorner = 28.dp
    val blockCorner = 20.dp
    val cardSpacing = 12.dp
    val dockHeight = 62.dp
}

/**
 * FlClash / prism_ui scalloped polygon shape (cookie12Sided, cookie9Sided, cookie6Sided, clover4Leaf)
 * that morphs continuously between a pure circle (progress = 0f) and an N-lobed shape (progress = 1f).
 */
internal class FlClashScallopShape(
    private val lobes: Int,
    private val depth: Float,
    private val progress: Float = 1f,
    private val phaseRad: Float = 0f,
) : Shape {
    override fun createOutline(size: Size, layoutDirection: LayoutDirection, density: Density): Outline {
        val path = Path()
        val cx = size.width / 2f
        val cy = size.height / 2f
        val maxR = min(cx, cy)
        val p = progress.coerceIn(0f, 1f)
        val amp = (depth * p).coerceIn(0f, 0.35f)
        val baseR = maxR / (1f + amp)
        val steps = 96
        for (i in 0 until steps) {
            val theta = (2.0 * PI * i / steps).toFloat()
            val r = baseR * (1f + amp * cos(lobes * (theta - phaseRad)))
            val x = cx + r * cos(theta)
            val y = cy + r * sin(theta)
            if (i == 0) path.moveTo(x, y) else path.lineTo(x, y)
        }
        path.close()
        return Outline.Generic(path)
    }
}

/** FlClash / prism_ui ElasticPress: spring scale on pointer down without changing layout bounds. */
@Composable
internal fun Modifier.elasticPress(pressed: Boolean, pressedScale: Float = 0.96f): Modifier {
    val animated = LocalConduitMotion.current
    val scale by animateFloatAsState(
        targetValue = if (pressed) pressedScale else 1f,
        animationSpec = if (animated) MaterialTheme.motionScheme.fastSpatialSpec() else snap(),
        label = "Elastic press",
    )
    return this.graphicsLayer {
        scaleX = scale
        scaleY = scale
    }
}

/** FlClash SurfaceLevel.card: borderless tonal card with generous rounded corners. */
@Composable
internal fun ExpressiveCard(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(ExpressiveLayout.cardCorner),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        shadowElevation = 0.dp,
    ) {
        // Surface is a Box, unlike the previous Material Card's ColumnScope.
        // Keep sequential settings rows vertical, including the OEM battery controls.
        Column { content() }
    }
}

/** FlClash DashboardBlock (SurfaceLevel.block): nested tonal block inside a card. */
@Composable
internal fun ExpressiveBlock(
    modifier: Modifier = Modifier,
    corner: Dp = ExpressiveLayout.blockCorner,
    color: Color = MaterialTheme.colorScheme.surfaceContainer,
    content: @Composable ColumnScope.() -> Unit,
) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(corner),
        color = color,
        shadowElevation = 0.dp,
    ) {
        Column(content = content)
    }
}

/** FlClash DashboardSwitchRow icon badge: morphs between Cookie6Sided (active) and Circle (idle). */
@Composable
internal fun FlClashShapeBadge(
    icon: Int,
    active: Boolean,
    modifier: Modifier = Modifier,
    size: Dp = 40.dp,
    iconSize: Dp = 20.dp,
) {
    val c = MaterialTheme.colorScheme
    val animated = LocalConduitMotion.current
    val progress by animateFloatAsState(
        targetValue = if (active) 1f else 0f,
        animationSpec = if (animated) MaterialTheme.motionScheme.defaultSpatialSpec() else snap(),
        label = "Badge morph",
    )
    val bg by animateColorAsState(
        targetValue = if (active) c.primaryContainer else c.surfaceContainerHighest,
        animationSpec = if (animated) tween(ConduitMotion.feedbackMillis) else snap(),
        label = "Badge background",
    )
    val tint by animateColorAsState(
        targetValue = if (active) c.primary else c.onSurfaceVariant,
        animationSpec = if (animated) tween(ConduitMotion.feedbackMillis) else snap(),
        label = "Badge tint",
    )
    Box(
        modifier = modifier
            .size(size)
            .graphicsLayer { rotationZ = progress * 30f }
            .clip(FlClashScallopShape(lobes = 6, depth = 0.11f, progress = progress))
            .background(bg),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            painter = painterResource(icon),
            contentDescription = null,
            modifier = Modifier.size(iconSize).graphicsLayer { rotationZ = -progress * 30f },
            tint = tint,
        )
    }
}

@Composable
internal fun ExpressiveSectionTitle(title: String, detail: String? = null) {
    Row(
        Modifier.fillMaxWidth().padding(start = 16.dp, end = 12.dp, top = 12.dp, bottom = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            title,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (!detail.isNullOrBlank()) {
            Text(
                detail,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** Single-weight page title and a compact tonal action, styled after FlClash's header bar. */
@Composable
internal fun ExpressiveTopBar(title: String, action: (() -> Unit)? = null, back: (() -> Unit)? = null) {
    Row(
        Modifier
            .fillMaxWidth()
            .statusBarsPadding()
            .heightIn(min = 72.dp)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (back != null) {
            IconButton(onClick = back) {
                Icon(painterResource(R.drawable.ic_arrow_back), "Back", modifier = Modifier.size(22.dp))
            }
        }
        Text(
            title,
            Modifier.weight(1f),
            style = MaterialTheme.typography.headlineLarge,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (action != null) {
            val interaction = remember { MutableInteractionSource() }
            val pressed by interaction.collectIsPressedAsState()
            FilledTonalIconButton(
                onClick = action,
                interactionSource = interaction,
                modifier = Modifier.size(44.dp).elasticPress(pressed, 0.92f),
                colors = IconButtonDefaults.filledTonalIconButtonColors(
                    containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
                    contentColor = MaterialTheme.colorScheme.onSurface,
                ),
            ) {
                Icon(painterResource(R.drawable.ic_add), "Pair desktop", Modifier.size(24.dp))
            }
        }
    }
}

/** FlClash / prism_ui floating capsule NavigationDock with spring lens indicator. */
@Composable
internal fun ExpressiveNavigationDock(
    selectedSettings: Boolean,
    onHome: () -> Unit,
    onSettings: () -> Unit,
    destination: String = if (selectedSettings) "Settings" else "Home",
    onTransfers: () -> Unit = {},
    onClipboard: () -> Unit = {},
) {
    val selectedIndex = listOf("Home", "Transfers", "Clipboard", "Settings").indexOf(destination).coerceAtLeast(0)
    val animated = LocalConduitMotion.current
    Box(
        Modifier
            .fillMaxWidth()
            .navigationBarsPadding()
            .padding(horizontal = 20.dp, vertical = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        Surface(
            modifier = Modifier.widthIn(max = 340.dp).fillMaxWidth(),
            shape = CircleShape,
            color = MaterialTheme.colorScheme.surfaceContainer,
            tonalElevation = 3.dp,
            shadowElevation = 8.dp,
        ) {
            BoxWithConstraints(Modifier.height(ExpressiveLayout.dockHeight).padding(4.dp)) {
                val cell = maxWidth / 4
                val lensPosition by animateFloatAsState(
                    targetValue = selectedIndex.toFloat(),
                    animationSpec = if (animated) MaterialTheme.motionScheme.defaultSpatialSpec() else snap(),
                    label = "Navigation lens position",
                )
                val offset = cell * lensPosition
                val travelFraction = abs(selectedIndex.toFloat() - lensPosition).coerceIn(0f, 1f)
                Box(
                    Modifier
                        .offset(x = offset)
                        .testTag("navigation-indicator")
                        .width(cell)
                        .fillMaxHeight(),
                ) {
                    Box(
                        Modifier
                            .fillMaxSize()
                            .graphicsLayer {
                                scaleX = 1f + 0.10f * travelFraction
                                scaleY = 1f - 0.04f * travelFraction
                            }
                            .clip(CircleShape)
                            .background(MaterialTheme.colorScheme.surfaceContainerHighest),
                    )
                }
                Row(Modifier.fillMaxSize().selectableGroup()) {
                    DockItem(
                        label = "Home",
                        icon = R.drawable.ic_dashboard,
                        selected = selectedIndex == 0,
                        coverage = (1f - abs(lensPosition - 0f)).coerceIn(0f, 1f),
                        onClick = onHome,
                        modifier = Modifier.weight(1f),
                    )
                    DockItem(
                        label = "Transfers",
                        icon = R.drawable.ic_transfer,
                        selected = selectedIndex == 1,
                        coverage = (1f - abs(lensPosition - 1f)).coerceIn(0f, 1f),
                        onClick = onTransfers,
                        modifier = Modifier.weight(1f),
                    )
                    DockItem(
                        label = "Clipboard",
                        icon = R.drawable.ic_file_outline,
                        selected = selectedIndex == 2,
                        coverage = (1f - abs(lensPosition - 2f)).coerceIn(0f, 1f),
                        onClick = onClipboard,
                        modifier = Modifier.weight(1f),
                    )
                    DockItem(
                        label = "Settings",
                        icon = R.drawable.ic_settings_outline,
                        selected = selectedIndex == 3,
                        coverage = (1f - abs(lensPosition - 3f)).coerceIn(0f, 1f),
                        onClick = onSettings,
                        modifier = Modifier.weight(1f),
                    )
                }
            }
        }
    }
}

@Composable
private fun DockItem(
    label: String,
    icon: Int,
    selected: Boolean,
    coverage: Float,
    onClick: () -> Unit,
    modifier: Modifier,
) {
    val view = LocalView.current
    val interactive = remember { MutableInteractionSource() }
    val pressed by interactive.collectIsPressedAsState()
    val animated = LocalConduitMotion.current
    val pressScale by animateFloatAsState(
        if (pressed) 0.90f else 1f,
        animationSpec = if (animated) MaterialTheme.motionScheme.fastSpatialSpec() else snap(),
        label = "Navigation press",
    )
    val color = lerp(
        MaterialTheme.colorScheme.onSurfaceVariant,
        MaterialTheme.colorScheme.primary,
        coverage,
    )
    val scale = (1f + 0.08f * coverage) * pressScale
    Column(
        modifier
            .fillMaxHeight()
            .selectable(
                selected = selected,
                onClick = {
                    view.performHapticFeedback(HapticFeedbackConstants.VIRTUAL_KEY)
                    onClick()
                },
                role = Role.Tab,
                interactionSource = interactive,
                indication = null,
            )
            .semantics(mergeDescendants = true) { contentDescription = label },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(
            painterResource(icon),
            null,
            Modifier.size(23.dp).graphicsLayer {
                scaleX = scale
                scaleY = scale
            },
            tint = color,
        )
        Spacer(Modifier.height(2.dp))
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = color,
            fontWeight = if (coverage >= 0.5f) FontWeight.SemiBold else FontWeight.Medium,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
