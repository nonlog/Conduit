package com.conduit.sync

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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

/** All transfer states remain independent: this composable never starts/stops a sender. */
@Composable
internal fun ExpressiveTransferCard(
    transfer: FileTransfer,
    peerName: String?,
    onCancel: (String) -> Unit,
) {
    val colors = MaterialTheme.colorScheme
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
            modifier = Modifier.fillMaxWidth().padding(18.dp),
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
                        .clip(RoundedCornerShape(16.dp))
                        .background(colors.secondaryContainer),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        painter = painterResource(icon),
                        contentDescription = label,
                        tint = colors.onSecondaryContainer,
                        modifier = Modifier.size(23.dp),
                    )
                }
                Column(Modifier.weight(1f)) {
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
                Text(
                    text = transfer.percent.toString() + "%",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    color = colors.primary,
                )
            }
            LinearProgressIndicator(
                progress = { transfer.fraction },
                modifier = Modifier.fillMaxWidth(),
                color = colors.primary,
                trackColor = colors.surfaceContainerHighest,
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Column(Modifier.weight(1f)) {
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
                    TextButton(
                        onClick = { onCancel(transfer.id) },
                        modifier = Modifier.heightIn(min = 48.dp).semantics {
                            contentDescription = "Cancel " + transfer.name
                        },
                        shape = RoundedCornerShape(20.dp),
                    ) { Text("Cancel") }
                }
            }
        }
    }
}
