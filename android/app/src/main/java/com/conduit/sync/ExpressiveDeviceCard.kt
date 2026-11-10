@file:OptIn(androidx.compose.material3.ExperimentalMaterial3ExpressiveApi::class)

package com.conduit.sync

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.Crossfade
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** FlClash CurrentOutboundCard + OutboundMode connected action group; no transport ownership in UI. */
@Composable
internal fun ExpressiveDeviceCard(
    state: LinkState,
    peerName: String?,
    pairedDeviceId: String?,
    pairing: Boolean,
    path: String?,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onPair: () -> Unit,
    onCancelPair: () -> Unit,
    onSendFiles: () -> Unit,
    onSendPhoto: () -> Unit,
) {
    val c = MaterialTheme.colorScheme
    val motion = LocalConduitMotion.current
    val paired = pairedDeviceId != null
    val linked = state == LinkState.Connected
    val requested = isLinkRequestedState(state)
    val stateLabel = when {
        pairing -> "Pairing"
        !paired -> "Not paired"
        linked -> "Linked"
        requested -> "Connecting"
        else -> "Not connected"
    }
    val heroProgress by animateFloatAsState(
        targetValue = if (linked) 1f else 0f,
        animationSpec = if (motion) MaterialTheme.motionScheme.slowSpatialSpec() else snap(),
        label = "Hero cookie progress",
    )
    val toggleProgress by animateFloatAsState(
        targetValue = if (linked || requested || pairing) 1f else 0f,
        animationSpec = if (motion) MaterialTheme.motionScheme.defaultSpatialSpec() else snap(),
        label = "Start toggle morph",
    )
    Column(verticalArrangement = Arrangement.spacedBy(ExpressiveLayout.cardSpacing)) {
        ExpressiveCard(Modifier.fillMaxWidth()) {
            Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(ExpressiveLayout.cardCorner))) {
                // FlClash _BackgroundDeco: 12-sided cookie watermark at top-end that rotates & swells when linked
                Box(
                    Modifier
                        .align(Alignment.TopEnd)
                        .offset(x = 56.dp, y = (-62).dp)
                        .size(216.dp)
                        .graphicsLayer {
                            rotationZ = heroProgress * 30f
                            val s = 1f + 0.12f * heroProgress
                            scaleX = s
                            scaleY = s
                        }
                        .clip(FlClashScallopShape(lobes = 12, depth = 0.075f, progress = 1f))
                        .background(c.primary.copy(alpha = 0.04f + 0.06f * heroProgress)),
                )
                Column(
                    Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 18.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(9.dp),
                    ) {
                        Icon(
                            painterResource(R.drawable.ic_connection_status),
                            null,
                            Modifier.size(19.dp),
                            tint = c.onSurfaceVariant,
                        )
                        Text(
                            "Connection",
                            style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.weight(1f),
                        )
                        StatusPill(stateLabel, active = linked, showDot = true)
                    }
                    Row(
                        Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(14.dp),
                    ) {
                        Column(
                            Modifier.weight(1f),
                            verticalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(12.dp),
                            ) {
                                // FlClash clover4Leaf badge for the active peer
                                Box(
                                    Modifier
                                        .size(46.dp)
                                        .clip(FlClashScallopShape(lobes = 4, depth = 0.14f, progress = 1f))
                                        .background(if (linked) c.primaryContainer else c.surfaceContainerHighest),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Icon(
                                        painterResource(R.drawable.ic_desktop),
                                        null,
                                        Modifier.size(22.dp),
                                        tint = if (linked) c.primary else c.onSurfaceVariant,
                                    )
                                }
                                Text(
                                    peerName ?: if (paired) "Your desktop" else "Add a desktop",
                                    style = MaterialTheme.typography.headlineMedium,
                                    fontWeight = FontWeight.SemiBold,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.weight(1f),
                                )
                            }
                            ExpressiveBlock(
                                modifier = Modifier.fillMaxWidth(),
                                corner = 16.dp,
                            ) {
                                Row(
                                    modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 9.dp),
                                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    if (paired && !path.isNullOrBlank()) {
                                        StatusPill(path, active = false)
                                    }
                                    Text(
                                        if (linked) {
                                            "Connected"
                                        } else if (pairing) {
                                            "Waiting for approval"
                                        } else if (paired) {
                                            "Ready to reconnect"
                                        } else {
                                            "Pair to get started"
                                        },
                                        style = MaterialTheme.typography.bodySmall,
                                        color = c.onSurfaceVariant,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                        modifier = Modifier.weight(1f),
                                    )
                                }
                            }
                        }
                        val description = when {
                            pairing -> "Cancel pairing"
                            !paired -> "Pair desktop"
                            requested -> "Disconnect"
                            else -> "Connect"
                        }
                        val toggleInteraction = remember { MutableInteractionSource() }
                        val togglePressed by toggleInteraction.collectIsPressedAsState()
                        val toggleContainer by animateColorAsState(
                            targetValue = if (linked) c.primary else if (requested || pairing) c.primaryContainer else c.primary,
                            animationSpec = if (motion) tween(200) else snap(),
                            label = "Toggle container",
                        )
                        val toggleContent by animateColorAsState(
                            targetValue = if (linked) c.onPrimary else if (requested || pairing) c.primary else c.onPrimary,
                            animationSpec = if (motion) tween(200) else snap(),
                            label = "Toggle content",
                        )
                        // FlClash _StartToggle: morphs between Circle and Cookie9Sided while rotating 40deg
                        FilledIconButton(
                            onClick = {
                                when {
                                    pairing -> onCancelPair()
                                    !paired -> onPair()
                                    requested -> onDisconnect()
                                    else -> onConnect()
                                }
                            },
                            interactionSource = toggleInteraction,
                            shapes = IconButtonDefaults.shapes(
                                shape = FlClashScallopShape(lobes = 9, depth = 0.095f, progress = toggleProgress),
                                pressedShape = FlClashScallopShape(lobes = 9, depth = 0.05f, progress = toggleProgress),
                            ),
                            modifier = Modifier
                                .size(64.dp)
                                .elasticPress(togglePressed, 0.92f)
                                .graphicsLayer { rotationZ = toggleProgress * 40f }
                                .semantics { contentDescription = description },
                            colors = IconButtonDefaults.filledIconButtonColors(
                                containerColor = toggleContainer,
                                contentColor = toggleContent,
                            ),
                        ) {
                            Box(Modifier.graphicsLayer { rotationZ = -toggleProgress * 40f }) {
                                Crossfade(
                                    targetState = when {
                                        pairing -> R.drawable.ic_close
                                        !paired -> R.drawable.ic_add
                                        requested -> R.drawable.ic_pause
                                        else -> R.drawable.ic_play
                                    },
                                    animationSpec = if (motion) tween(ConduitMotion.feedbackMillis) else snap(),
                                    label = "Connect control",
                                ) { glyph ->
                                    Icon(
                                        painterResource(glyph),
                                        contentDescription = null,
                                        modifier = Modifier.size(26.dp),
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
        MotionReveal(visible = linked) {
            ExpressiveCard(Modifier.fillMaxWidth()) {
                Column(
                    Modifier.padding(horizontal = 20.dp, vertical = 18.dp),
                    verticalArrangement = Arrangement.spacedBy(14.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(9.dp),
                    ) {
                        Icon(
                            painterResource(R.drawable.ic_stat_upload),
                            null,
                            Modifier.size(19.dp),
                            tint = c.onSurfaceVariant,
                        )
                        Text(
                            "Send",
                            style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.weight(1f),
                        )
                        StatusPill("Encrypted", active = false)
                    }
                    BoxWithConstraints(Modifier.fillMaxWidth()) {
                        if (maxWidth < 280.dp || LocalDensity.current.fontScale >= 1.35f) {
                            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                SendAction(
                                    "Send files",
                                    R.drawable.ic_file_outline,
                                    onSendFiles,
                                    ConnectedSlot.Top,
                                    Modifier.fillMaxWidth(),
                                )
                                SendAction(
                                    "Send photo",
                                    R.drawable.ic_photo,
                                    onSendPhoto,
                                    ConnectedSlot.Bottom,
                                    Modifier.fillMaxWidth(),
                                )
                            }
                        } else {
                            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                                SendAction(
                                    "Send files",
                                    R.drawable.ic_file_outline,
                                    onSendFiles,
                                    ConnectedSlot.Leading,
                                    Modifier.weight(1f),
                                )
                                SendAction(
                                    "Send photo",
                                    R.drawable.ic_photo,
                                    onSendPhoto,
                                    ConnectedSlot.Trailing,
                                    Modifier.weight(1f),
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

private enum class ConnectedSlot { Leading, Trailing, Top, Bottom }

@Composable
internal fun StatusPill(text: String, active: Boolean = false, showDot: Boolean = false) {
    val c = MaterialTheme.colorScheme
    val motion = LocalConduitMotion.current
    val color by animateColorAsState(
        if (active) c.primaryContainer else c.surfaceContainerHighest,
        animationSpec = if (motion) tween(200) else snap(),
        label = "Connection status color",
    )
    val contentColor by animateColorAsState(
        if (active) c.primary else c.onSurfaceVariant,
        animationSpec = if (motion) tween(200) else snap(),
        label = "Connection status ink",
    )
    Surface(shape = CircleShape, color = color) {
        Row(
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            if (showDot) {
                Box(
                    Modifier
                        .size(6.dp)
                        .clip(CircleShape)
                        .background(contentColor),
                )
            }
            AnimatedContent(
                targetState = text,
                label = "Connection status",
                transitionSpec = {
                    fadeIn(tween(if (motion) 180 else 0)) togetherWith fadeOut(tween(if (motion) 120 else 0))
                },
            ) { label ->
                Text(
                    label,
                    style = MaterialTheme.typography.labelMedium,
                    color = contentColor,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun SendAction(
    label: String,
    icon: Int,
    onClick: () -> Unit,
    slot: ConnectedSlot,
    modifier: Modifier,
) {
    val c = MaterialTheme.colorScheme
    val motion = LocalConduitMotion.current
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val outer = 26.dp
    val inner by animateDpAsState(
        targetValue = if (pressed) outer else 12.dp,
        animationSpec = if (motion) MaterialTheme.motionScheme.fastSpatialSpec() else snap(),
        label = "Connected button inner corner",
    )
    val shape = when (slot) {
        ConnectedSlot.Leading -> RoundedCornerShape(
            topStart = outer,
            bottomStart = outer,
            topEnd = inner,
            bottomEnd = inner,
        )
        ConnectedSlot.Trailing -> RoundedCornerShape(
            topStart = inner,
            bottomStart = inner,
            topEnd = outer,
            bottomEnd = outer,
        )
        ConnectedSlot.Top -> RoundedCornerShape(
            topStart = outer,
            topEnd = outer,
            bottomStart = inner,
            bottomEnd = inner,
        )
        ConnectedSlot.Bottom -> RoundedCornerShape(
            topStart = inner,
            topEnd = inner,
            bottomStart = outer,
            bottomEnd = outer,
        )
    }
    FilledTonalButton(
        onClick = onClick,
        interactionSource = interaction,
        modifier = modifier
            .height(56.dp)
            .elasticPress(pressed, 0.96f)
            .semantics { contentDescription = label },
        shapes = ButtonDefaults.shapes(shape = shape, pressedShape = RoundedCornerShape(outer)),
        colors = ButtonDefaults.filledTonalButtonColors(
            containerColor = c.surfaceContainer,
            contentColor = c.onSurface,
        ),
        contentPadding = PaddingValues(horizontal = 12.dp),
    ) {
        Icon(painterResource(icon), null, Modifier.size(20.dp), tint = c.primary)
        Spacer(Modifier.width(8.dp))
        Text(
            label,
            style = MaterialTheme.typography.labelLarge,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
