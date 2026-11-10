package com.conduit.sync

import androidx.compose.foundation.LocalIndication
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** The dashboard summarizes transfer activity; only the Transfers destination owns history rows. */
@Composable
internal fun HomeTransfersSummary(
    transfers: List<FileTransfer>,
    onOpenTransfers: () -> Unit,
) {
    val active = transfers.filterNot { it.phase.terminal }
    val highlighted = active.firstOrNull()
    val subtitle = when {
        active.isNotEmpty() -> "${active.size} in progress · Tap to manage"
        transfers.isEmpty() -> "No transfers yet"
        else -> "Recent files sent and received"
    }
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    ExpressiveCard(
        modifier = Modifier
            .fillMaxWidth()
            .elasticPress(pressed, 0.97f)
            .clickable(
                interactionSource = interaction,
                indication = LocalIndication.current,
                onClick = onOpenTransfers,
            )
            .semantics(mergeDescendants = true) { contentDescription = "Open transfer history" },
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 18.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Icon(
                    painterResource(R.drawable.ic_transfer),
                    null,
                    modifier = Modifier.size(19.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Text(
                    "File transfers",
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleMedium,
                )
                StatusPill(
                    transfers.size.toString(),
                    active = active.isNotEmpty(),
                    showDot = active.isNotEmpty(),
                )
                Icon(
                    painterResource(R.drawable.ic_chevron),
                    null,
                    modifier = Modifier.size(20.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            ExpressiveBlock(
                modifier = Modifier.fillMaxWidth(),
                corner = 18.dp,
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 14.dp, vertical = 12.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        subtitle,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    if (highlighted != null) {
                        Text(
                            "${highlighted.name} · ${highlighted.percent}%",
                            style = MaterialTheme.typography.bodySmall,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        LinearProgressIndicator(
                            progress = { highlighted.fraction },
                            modifier = Modifier.fillMaxWidth().height(6.dp).clip(CircleShape),
                            color = MaterialTheme.colorScheme.primary,
                            trackColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                        )
                    }
                }
            }
        }
    }
}

@Composable
internal fun TransfersTab(
    modifier: Modifier,
    transfers: List<FileTransfer>,
    peerName: String?,
    onCancel: (String) -> Unit,
) {
    val active = transfers.filter { !it.phase.terminal }
    val finished = transfers.filter { it.phase.terminal }
    LazyColumn(
        modifier.fillMaxSize(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (transfers.isEmpty()) {
            item {
                ExpressiveCard(Modifier.fillMaxWidth()) {
                    Row(
                        Modifier.padding(22.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        FlClashShapeBadge(
                            icon = R.drawable.ic_transfer,
                            active = false,
                            size = 48.dp,
                            iconSize = 24.dp,
                        )
                        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Text("No transfers yet", style = MaterialTheme.typography.titleMedium)
                            Text(
                                "Files and photos you send or receive appear here.",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        }
        if (active.isNotEmpty()) {
            item { ExpressiveSectionTitle("In progress", active.size.toString()) }
            items(active, key = { "${it.direction}:${it.id}" }) { ExpressiveTransferCard(it, peerName, onCancel) }
        }
        if (finished.isNotEmpty()) {
            item { ExpressiveSectionTitle("Recent transfers") }
            items(finished, key = { "${it.direction}:${it.id}" }) { ExpressiveTransferCard(it, peerName, onCancel) }
        }
    }
}

@Composable
internal fun ClipboardTab(modifier: Modifier, history: List<HistoryEntry>, onClear: () -> Unit) {
    var query by rememberSaveable { mutableStateOf("") }
    var confirmClear by rememberSaveable { mutableStateOf(false) }
    MotionAlertDialog(
        visible = confirmClear,
        onDismissRequest = { confirmClear = false },
        title = { Text("Clear clipboard history?") },
        text = { Text("This removes saved items from this phone. Your pairing and settings are kept.") },
        confirmButton = {
            TextButton(onClick = {
                confirmClear = false
                onClear()
            }) { Text("Clear") }
        },
        dismissButton = {
            TextButton(onClick = { confirmClear = false }) { Text("Cancel") }
        },
    )
    val filtered = filterHistory(history, query)
    LazyColumn(
        modifier.fillMaxSize(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            OutlinedTextField(
                value = query,
                onValueChange = { query = it },
                label = { Text("Search clipboard") },
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(24.dp),
                singleLine = true,
            )
        }
        if (history.isNotEmpty()) {
            item {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 4.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    StatusPill("${history.size} saved items", active = false)
                    TextButton(onClick = { confirmClear = true }) { Text("Clear history") }
                }
            }
        }
        if (filtered.isEmpty()) {
            item {
                ExpressiveCard(Modifier.fillMaxWidth()) {
                    Row(
                        Modifier.padding(22.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        FlClashShapeBadge(
                            icon = R.drawable.ic_file_outline,
                            active = false,
                            size = 46.dp,
                            iconSize = 22.dp,
                        )
                        Text(
                            if (history.isEmpty()) "No clipboard items yet" else "No matches",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        } else {
            items(filtered, key = { "${it.at}:${it.direction}:${it.preview.hashCode()}" }) { ClipRow(it) }
        }
    }
}
