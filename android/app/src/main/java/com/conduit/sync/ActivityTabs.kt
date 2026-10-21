package com.conduit.sync

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
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
    ExpressiveCard(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onOpenTransfers)
            .semantics(mergeDescendants = true) { contentDescription = "Open transfer history" },
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = androidx.compose.ui.Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Icon(painterResource(R.drawable.ic_transfer), null,
                    modifier = Modifier.size(22.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                Text("File transfers", modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleMedium)
                StatusPill(transfers.size.toString(), active = active.isNotEmpty())
                Icon(painterResource(R.drawable.ic_chevron), null,
                    modifier = Modifier.size(20.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Text(subtitle, style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (highlighted != null) {
                Text(
                    "${highlighted.name} · ${highlighted.percent}%",
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 1,
                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                )
                LinearProgressIndicator(
                    progress = { highlighted.fraction },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        }
    }
}

@Composable
internal fun TransfersTab(
    modifier: Modifier, transfers: List<FileTransfer>, peerName: String?, onCancel: (String) -> Unit,
) {
    val active = transfers.filter { !it.phase.terminal }
    val finished = transfers.filter { it.phase.terminal }
    LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp)) {
        if (transfers.isEmpty()) {
            item {
                ExpressiveCard(Modifier.fillMaxWidth()) {
                    Column(Modifier.padding(24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Icon(painterResource(R.drawable.ic_transfer), null, Modifier.size(28.dp), tint = MaterialTheme.colorScheme.primary)
                        Text("No transfers yet", style = MaterialTheme.typography.titleMedium)
                        Text("Files and photos you send or receive appear here.", style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant)
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
    if (confirmClear) {
        AlertDialog(onDismissRequest = { confirmClear = false }, title = { Text("Clear clipboard history?") },
            text = { Text("This removes saved items from this phone. Your pairing and settings are kept.") },
            confirmButton = { TextButton(onClick = { confirmClear = false; onClear() }) { Text("Clear") } },
            dismissButton = { TextButton(onClick = { confirmClear = false }) { Text("Cancel") } })
    }
    val filtered = filterHistory(history, query)
    LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp)) {
        item {
            OutlinedTextField(value = query, onValueChange = { query = it }, label = { Text("Search clipboard") },
                modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(24.dp), singleLine = true)
        }
        if (history.isNotEmpty()) {
            item {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                    Text("${history.size} saved items", style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(top = 16.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    TextButton(onClick = { confirmClear = true }) { Text("Clear history") }
                }
            }
        }
        if (filtered.isEmpty()) {
            item {
                ExpressiveCard(Modifier.fillMaxWidth()) {
                    Text(if (history.isEmpty()) "No clipboard items yet" else "No matches",
                        modifier = Modifier.padding(24.dp), style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        } else {
            items(filtered, key = { "${it.at}:${it.direction}:${it.preview.hashCode()}" }) { ClipRow(it) }
        }
    }
}
