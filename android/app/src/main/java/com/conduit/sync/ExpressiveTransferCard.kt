@file:OptIn(androidx.compose.material3.ExperimentalMaterial3ExpressiveApi::class)

package com.conduit.sync

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** All transfer states remain independent: this composable never starts/stops a sender. */
@Composable
internal fun ExpressiveTransferCard(
    transfer: FileTransfer,
    peerName: String?,
    onCancel: (String) -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val motion = LocalConduitMotion.current
    val active = !transfer.phase.terminal
    val displayedProgress by animateFloatAsState(
        transfer.fraction,
        animationSpec = if (motion) tween(180) else snap(),
        label = "Transfer progress",
    )
    val badgeMorph by animateFloatAsState(
        targetValue = if (active) 1f else 0f,
        animationSpec = if (motion) MaterialTheme.motionScheme.defaultSpatialSpec() else snap(),
        label = "Transfer badge morph",
    )
    val peer = peerName ?: "desktop"
    val label = when (transfer.direction) {
        FileTransferDirection.ToDesktop -> "To " + peer
        FileTransferDirection.ToPhone -> "From " + peer
    }
    val icon = if (transfer.direction == FileTransferDirection.ToDesktop) {
        R.drawable.ic_stat_upload
    } else {
        R.drawable.ic_stat_download
    }
    ExpressiveCard(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 18.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Box(
                    modifier = Modifier
                        .size(46.dp)
                        .graphicsLayer { rotationZ = badgeMorph * 30f }
                        .clip(FlClashScallopShape(lobes = 6, depth = 0.11f, progress = badgeMorph))
                        .background(if (active) colors.primaryContainer else colors.surfaceContainerHighest),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        painter = painterResource(icon),
                        contentDescription = label,
                        tint = if (active) colors.primary else colors.onSurfaceVariant,
                        modifier = Modifier.size(22.dp).graphicsLayer { rotationZ = -badgeMorph * 30f },
                    )
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        text = transfer.name,
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        text = label,
                        style = MaterialTheme.typography.labelMedium,
                        color = colors.onSurfaceVariant,
                    )
                }
                StatusPill(
                    text = transfer.percent.toString() + "%",
                    active = active,
                )
            }
            ExpressiveBlock(
                modifier = Modifier.fillMaxWidth(),
                corner = 18.dp,
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth().padding(14.dp),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    LinearProgressIndicator(
                        progress = { displayedProgress },
                        modifier = Modifier.fillMaxWidth().height(6.dp).clip(CircleShape),
                        color = colors.primary,
                        trackColor = colors.surfaceContainerHighest,
                    )
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                    ) {
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            Text(
                                text = transfer.phase.label,
                                style = MaterialTheme.typography.labelLarge,
                                color = colors.onSurface,
                            )
                            Text(
                                text = formatBytes(transfer.transferred) + " of " + formatBytes(transfer.total),
                                style = MaterialTheme.typography.bodySmall,
                                color = colors.onSurfaceVariant,
                            )
                            if (transfer.error.isNotBlank()) {
                                Text(
                                    text = transfer.error,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = colors.error,
                                )
                            }
                        }
                        if (transfer.canCancel) {
                            FilledTonalButton(
                                onClick = { onCancel(transfer.id) },
                                modifier = Modifier.heightIn(min = 48.dp).semantics {
                                    contentDescription = "Cancel " + transfer.name
                                },
                                shape = RoundedCornerShape(18.dp),
                                colors = ButtonDefaults.filledTonalButtonColors(
                                    containerColor = colors.surfaceContainerHighest,
                                    contentColor = colors.onSurface,
                                ),
                            ) {
                                Text("Cancel")
                            }
                        }
                    }
                }
            }
        }
    }
}
