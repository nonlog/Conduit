@file:OptIn(androidx.compose.material3.ExperimentalMaterial3ExpressiveApi::class)

package com.conduit.sync

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** Dashboard structure drawn from the user's reference; no transport ownership in the UI. */
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
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        ExpressiveCard(Modifier.fillMaxWidth()) {
            Box(Modifier.fillMaxWidth().clip(RoundedCornerShape(28.dp))) {
                // Real Material Expressive shape API. Static decoration, no background renderer.
                Box(Modifier.align(Alignment.TopEnd).offset(x = 72.dp, y = (-26).dp)
                    .size(204.dp).background(c.surfaceContainer, MaterialShapes.Cookie9Sided.toShape()))
                Column(Modifier.fillMaxWidth().padding(20.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(9.dp)) {
                        Icon(painterResource(R.drawable.ic_connection_status), null, Modifier.size(20.dp), tint = c.onSurfaceVariant)
                        Text("Connection", style = MaterialTheme.typography.titleMedium)
                        StatusPill(stateLabel, active = linked)
                    }
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(9.dp)) {
                            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                                Box(Modifier.size(44.dp).background(c.surfaceContainerHighest, MaterialShapes.Clover4Leaf.toShape()),
                                    contentAlignment = Alignment.Center) {
                                    Icon(painterResource(R.drawable.ic_desktop), null, Modifier.size(23.dp), tint = c.onSurfaceVariant)
                                }
                                Text(peerName ?: if (paired) "Your desktop" else "Add a desktop",
                                    style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.SemiBold,
                                    maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                            }
                            Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                                if (paired && !path.isNullOrBlank()) StatusPill(path, active = false)
                                Text(if (linked) "Connected" else if (pairing) "Waiting for approval" else if (paired) "Ready to reconnect" else "Pair to get started",
                                    style = MaterialTheme.typography.bodySmall, color = c.onSurfaceVariant,
                                    maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                            }
                        }
                        val description = when {
                            pairing -> "Cancel pairing"
                            !paired -> "Pair desktop"
                            requested -> "Disconnect"
                            else -> "Connect"
                        }
                        FilledIconButton(
                            onClick = { when { pairing -> onCancelPair(); !paired -> onPair(); requested -> onDisconnect(); else -> onConnect() } },
                            shapes = IconButtonDefaults.shapes(shape = CircleShape, pressedShape = RoundedCornerShape(26.dp)),
                            modifier = Modifier.size(82.dp).semantics { contentDescription = description },
                            colors = IconButtonDefaults.filledIconButtonColors(containerColor = c.primary, contentColor = c.onPrimary),
                        ) {
                            Icon(painterResource(when { pairing -> R.drawable.ic_close; !paired -> R.drawable.ic_add; requested -> R.drawable.ic_pause; else -> R.drawable.ic_play }),
                                contentDescription = null, modifier = Modifier.size(32.dp))
                        }
                    }
                }
            }
        }
        if (linked) {
            ExpressiveCard(Modifier.fillMaxWidth()) {
                Column(Modifier.padding(18.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(9.dp)) {
                        Icon(painterResource(R.drawable.ic_stat_upload), null, Modifier.size(20.dp), tint = c.onSurfaceVariant)
                        Text("Send", style = MaterialTheme.typography.titleMedium)
                    }
                    BoxWithConstraints(Modifier.fillMaxWidth()) {
                        if (maxWidth < 270.dp || LocalDensity.current.fontScale >= 1.35f) {
                            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                SendAction("Send files", R.drawable.ic_file_outline, onSendFiles, Modifier.fillMaxWidth())
                                SendAction("Send photo", R.drawable.ic_photo, onSendPhoto, Modifier.fillMaxWidth())
                            }
                        } else {
                            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                                SendAction("Send files", R.drawable.ic_file_outline, onSendFiles, Modifier.weight(1f))
                                SendAction("Send photo", R.drawable.ic_photo, onSendPhoto, Modifier.weight(1f))
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
internal fun StatusPill(text: String, active: Boolean = false) {
    val c = MaterialTheme.colorScheme
    Surface(shape = CircleShape, color = if (active) c.primaryContainer else c.surfaceContainer) {
        Text(text, Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
            style = MaterialTheme.typography.labelMedium,
            color = if (active) c.primary else c.onSurfaceVariant,
            maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
private fun SendAction(label: String, icon: Int, onClick: () -> Unit, modifier: Modifier) {
    val c = MaterialTheme.colorScheme
    FilledTonalButton(
        onClick = onClick,
        modifier = modifier.height(56.dp).semantics { contentDescription = label },
        shapes = ButtonDefaults.shapes(shape = RoundedCornerShape(24.dp), pressedShape = RoundedCornerShape(16.dp)),
        colors = ButtonDefaults.filledTonalButtonColors(containerColor = c.surfaceContainer, contentColor = c.onSurface),
        contentPadding = PaddingValues(horizontal = 10.dp),
    ) {
        Icon(painterResource(icon), null, Modifier.size(21.dp), tint = c.primary)
        Spacer(Modifier.width(8.dp))
        Text(label, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
