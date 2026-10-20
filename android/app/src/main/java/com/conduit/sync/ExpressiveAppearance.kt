package com.conduit.sync

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/**
 * FlClash-inspired design primitives, implemented independently in native Compose.
 *
 * Material expressive colors come from Android's wallpaper rather than a hard-coded palette.
 * Glass is reserved for foreground chrome; sheets and content remain opaque for text contrast.
 * There is no continuously refreshing shader, screen capture, or resident background renderer.
 */
internal object ExpressiveLayout {
    val pageInset = 20.dp
    val cardCorner = 30.dp
    val dockCorner = 38.dp
    val cardSpacing = 14.dp
}

/** An inexpensive frosted lens: layered translucent tones, subtle rim and elevation. */
@Composable
internal fun GlassPane(
    modifier: Modifier = Modifier,
    corner: androidx.compose.ui.unit.Dp = ExpressiveLayout.cardCorner,
    prominent: Boolean = false,
    content: @Composable () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    if (!Settings.liquidGlassEnabled) {
        Surface(
            modifier = modifier,
            shape = RoundedCornerShape(corner),
            color = if (isSystemInDarkTheme()) colors.surfaceContainerHigh else colors.surfaceBright,
            border = BorderStroke(1.dp, colors.outlineVariant.copy(alpha = 0.30f)),
        ) { content() }
        return
    }
    val dark = isSystemInDarkTheme()
    val background = if (dark) {
        colors.surfaceContainerHigh.copy(alpha = if (prominent) 0.94f else 0.88f)
    } else {
        colors.surfaceBright.copy(alpha = if (prominent) 0.93f else 0.84f)
    }
    val shape = RoundedCornerShape(corner)
    val tint = colors.primaryContainer.copy(alpha = if (prominent) 0.28f else 0.12f)
    val rim = Brush.verticalGradient(
        listOf(
            Color.White.copy(alpha = if (dark) 0.26f else 0.80f),
            colors.outlineVariant.copy(alpha = if (dark) 0.24f else 0.48f),
            colors.primary.copy(alpha = 0.11f),
        ),
    )
    Box(
        modifier = modifier
            .shadow(
                elevation = if (prominent) 12.dp else 9.dp,
                shape = shape,
                ambientColor = colors.onSurface.copy(alpha = 0.12f),
                spotColor = colors.onSurface.copy(alpha = 0.13f),
            )
            .clip(shape)
            .background(Brush.verticalGradient(listOf(tint, background, background)))
            .border(1.dp, rim, shape),
    ) {
        content()
    }
}

/** Material's tonal cards are intentionally quieter than the floating glass navigation. */
@Composable
internal fun ExpressiveCard(
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    Card(
        modifier = modifier,
        shape = RoundedCornerShape(26.dp),
        colors = CardDefaults.cardColors(
            containerColor = if (isSystemInDarkTheme()) colors.surfaceContainerHigh else colors.surfaceBright,
        ),
        border = BorderStroke(1.dp, colors.outlineVariant.copy(alpha = 0.32f)),
    ) { content() }
}

@Composable
internal fun ExpressiveSectionTitle(title: String, detail: String? = null) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(start = 4.dp, end = 4.dp, top = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleLarge,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurface,
        )
        if (!detail.isNullOrBlank()) {
            Spacer(Modifier.weight(1f))
            Text(
                text = detail,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
        }
    }
}

/**
 * The narrow, suspended dock follows FlClash's 62dp lens vocabulary rather than occupying an
 * opaque system-width navigation bar. Animate state transitions only when the user switches pages.
 */
@Composable
internal fun ExpressiveNavigationDock(
    selectedSettings: Boolean,
    onHome: () -> Unit,
    onSettings: () -> Unit,
) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .navigationBarsPadding()
            .padding(horizontal = ExpressiveLayout.pageInset, vertical = 10.dp),
        contentAlignment = Alignment.Center,
    ) {
        GlassPane(
            modifier = Modifier.widthIn(max = 360.dp).fillMaxWidth().height(76.dp),
            corner = ExpressiveLayout.dockCorner,
        ) {
            Row(
                modifier = Modifier.fillMaxWidth().height(76.dp).padding(5.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                ExpressiveDockDestination(
                    modifier = Modifier.weight(1f),
                    label = "Home",
                    icon = if (selectedSettings) R.drawable.ic_home_outline else R.drawable.ic_home,
                    selected = !selectedSettings,
                    onClick = onHome,
                )
                ExpressiveDockDestination(
                    modifier = Modifier.weight(1f),
                    label = "Settings",
                    icon = if (selectedSettings) R.drawable.ic_settings else R.drawable.ic_settings_outline,
                    selected = selectedSettings,
                    onClick = onSettings,
                )
            }
        }
    }
}

@Composable
private fun ExpressiveDockDestination(
    modifier: Modifier,
    label: String,
    icon: Int,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val background by animateColorAsState(
        targetValue = if (selected) colors.primaryContainer.copy(alpha = 0.88f)
        else Color.Transparent,
        label = "Dock selected lens",
    )
    val scale by animateFloatAsState(
        targetValue = if (selected) 1.0f else 0.95f,
        animationSpec = spring(dampingRatio = 0.72f, stiffness = 480f),
        label = "Dock spring",
    )
    val foreground = if (selected) colors.onPrimaryContainer else colors.onSurfaceVariant
    Column(
        modifier = modifier
            .fillMaxHeight()
            .clip(RoundedCornerShape(34.dp))
            .background(background)
            .clickable(role = Role.Tab, onClick = onClick)
            .semantics { this.selected = selected }
            .graphicsLayer { scaleX = scale; scaleY = scale },
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            painter = painterResource(icon),
            contentDescription = label,
            tint = foreground,
            modifier = Modifier.size(25.dp),
        )
        Spacer(Modifier.height(3.dp))
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Medium,
            color = foreground,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
