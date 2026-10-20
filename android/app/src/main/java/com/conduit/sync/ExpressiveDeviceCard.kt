package com.conduit.sync

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.FilledIconToggleButton
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/**
 * FlClash-inspired expressive device overview. This remains presentation-only:
 * connection, pairing, file selection and cancellation continue through the existing callbacks.
 */
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
    val colors = MaterialTheme.colorScheme
    val paired = pairedDeviceId != null
    val connected = state == LinkState.Connected
    val requested = isLinkRequestedState(state)
    val stateColor = when {
        connected -> colors.tertiary
        pairing || state == LinkState.Discovering || state == LinkState.Retrying -> colors.primary
        else -> colors.outline
    }

    GlassPane(
        modifier = Modifier.fillMaxWidth(),
        corner = 32.dp,
        prominent = connected,
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(13.dp),
            ) {
                Surface(
                    shape = RoundedCornerShape(23.dp),
                    color = colors.primaryContainer,
                    modifier = Modifier.size(64.dp),
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            painter = painterResource(R.drawable.ic_desktop),
                            contentDescription = "Paired desktop",
                            tint = colors.onPrimaryContainer,
                            modifier = Modifier.size(31.dp),
                        )
                    }
                }
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Text(
                        text = when {
                            pairing -> "Pairing a desktop"
                            paired -> peerName ?: "Your computer"
                            else -> "Add your computer"
                        },
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(7.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Box(
                            Modifier.size(8.dp).clip(CircleShape).background(stateColor),
                        )
                        Text(
                            text = when {
                                pairing -> "Pairing in progress"
                                !paired -> "Ready to pair"
                                else -> state.label
                            },
                            style = MaterialTheme.typography.bodyMedium,
                            fontWeight = FontWeight.Medium,
                            color = colors.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                if (!paired || pairing) {
                    FilledTonalButton(
                        onClick = if (pairing) onCancelPair else onPair,
                        modifier = Modifier.height(52.dp),
                        shape = CircleShape,
                        contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 14.dp),
                    ) { Text(if (pairing) "Cancel" else "Pair") }
                } else {
                    FilledIconToggleButton(
                        checked = requested,
                        onCheckedChange = { if (requested) onDisconnect() else onConnect() },
                        colors = IconButtonDefaults.filledIconToggleButtonColors(
                            containerColor = colors.surfaceContainerHighest,
                            contentColor = colors.onSurfaceVariant,
                            checkedContainerColor = colors.primary,
                            checkedContentColor = colors.onPrimary,
                        ),
                        modifier = Modifier.size(54.dp),
                    ) {
                        Icon(
                            painter = painterResource(R.drawable.ic_sync),
                            contentDescription = if (requested) "Disconnect" else "Connect",
                            modifier = Modifier.size(27.dp),
                        )
                    }
                }
            }

            path?.takeIf(String::isNotBlank)?.let { route ->
                Surface(
                    color = colors.secondaryContainer.copy(alpha = 0.78f),
                    shape = CircleShape,
                ) {
                    Text(
                        text = route,
                        modifier = Modifier.padding(horizontal = 13.dp, vertical = 7.dp),
                        style = MaterialTheme.typography.labelMedium,
                        color = colors.onSecondaryContainer,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            if (!paired || pairing) {
                Text(
                    text = if (pairing) {
                        "Keep pairing open on Windows until both devices recognize each other."
                    } else {
                        "Pair once to share files, photos and clipboard items over LAN or Relay."
                    },
                    style = MaterialTheme.typography.bodyMedium,
                    color = colors.onSurfaceVariant,
                )
            }

            if (connected) {
                HorizontalDivider(color = colors.outlineVariant.copy(alpha = 0.55f))
                BoxWithConstraints(Modifier.fillMaxWidth()) {
                    if (maxWidth < 310.dp) {
                        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                            ExpressiveSendAction(
                                label = "Send files",
                                icon = R.drawable.ic_stat_upload,
                                onClick = onSendFiles,
                                modifier = Modifier.fillMaxWidth(),
                                primary = true,
                            )
                            ExpressiveSendAction(
                                label = "Send photo",
                                icon = R.drawable.ic_photo,
                                onClick = onSendPhoto,
                                modifier = Modifier.fillMaxWidth(),
                                primary = false,
                            )
                        }
                    } else {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            ExpressiveSendAction(
                                label = "Send files",
                                icon = R.drawable.ic_stat_upload,
                                onClick = onSendFiles,
                                modifier = Modifier.weight(1f),
                                primary = true,
                            )
                            ExpressiveSendAction(
                                label = "Send photo",
                                icon = R.drawable.ic_photo,
                                onClick = onSendPhoto,
                                modifier = Modifier.weight(1f),
                                primary = false,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ExpressiveSendAction(
    label: String,
    icon: Int,
    onClick: () -> Unit,
    modifier: Modifier,
    primary: Boolean,
) {
    val colors = MaterialTheme.colorScheme
    FilledTonalButton(
        onClick = onClick,
        modifier = modifier
            .height(60.dp)
            .semantics { contentDescription = label },
        shape = RoundedCornerShape(21.dp),
        colors = ButtonDefaults.filledTonalButtonColors(
            containerColor = if (primary) colors.primaryContainer else colors.tertiaryContainer,
            contentColor = if (primary) colors.onPrimaryContainer else colors.onTertiaryContainer,
        ),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 9.dp),
    ) {
        Icon(painterResource(icon), contentDescription = null, modifier = Modifier.size(20.dp))
        Spacer(Modifier.width(7.dp))
        Text(label, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
