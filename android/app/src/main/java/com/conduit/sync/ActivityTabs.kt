package com.conduit.sync

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp

/** These destinations expose real existing state, not dashboard placeholder metrics. */
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
