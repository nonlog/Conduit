package com.conduit.sync

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts.RequestMultiplePermissions
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

private const val TAG = "conduit.ui"

/** What the one screen has to say. */
enum class LinkState(val label: String) {
    Idle("Not linked"),
    Discovering("Looking for the desktop"),

    /** Down, with an attempt already scheduled. See `SyncService.scheduleRetry`. */
    Retrying("Reconnecting"),
    Connected("Linked"),
}

/**
 * The screen's whole state. Snapshot state rather than a flow because writes come from
 * [Link]'s threads and Compose already handles that; a repository layer for four
 * fields would be scaffolding.
 */
object LinkStatus {
    var state by mutableStateOf(LinkState.Idle)

    /**
     * The peer's fingerprint, not a device name — it is what [Link] hands to `onState`,
     * and it is the value compared by eye against the desktop when pairing.
     */
    var peer by mutableStateOf<String?>(null)

    /**
     * The desktop's own name for itself, which is the one thing here a human recognises.
     * Survives a restart, so it is on screen before the first session of the day connects.
     */
    var peerName by mutableStateOf<String?>(null)

    /** "LAN", "Relay" or "Direct": which route the current attempt is taking. */
    var path by mutableStateOf<String?>(null)
    var fingerprint by mutableStateOf("-- : -- : -- : -- : -- : -- : -- : --")
}

class MainActivity : ComponentActivity() {
    /**
     * Registered before `onCreate` runs, which is the contract: the result callback has
     * to survive the activity being recreated behind the system's permission dialog.
     */
    private val ask = registerForActivityResult(RequestMultiplePermissions()) { granted ->
        granted.filterValues { !it }.keys.forEach {
            // Not fatal, and not worth nagging over. Photo mirroring simply stays off
            // until it is granted in Settings, and the service says so in its log.
            Log.i(TAG, "$it denied")
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Shown before anything is running, so the desktop can be paired against it.
        LinkStatus.fingerprint = Identity.fingerprint(Identity.loadOrCreate(filesDir).public)
        // Same reason as History below: the activity can be the first component to run, and
        // an empty desktop name until the service happens to start reads as "never paired".
        if (LinkStatus.peerName == null) LinkStatus.peerName = Identity.peerName(filesDir)
        // The activity can be the first component to run, so it loads history too rather
        // than showing an empty list until the service happens to start.
        History.load(this)
        Settings.load(this)
        request()
        // A host on the launch intent pins the address and links straight away. It has to
        // go through the activity: Android 12+ refuses a foreground service started from
        // the background, so `am start-foreground-service` cannot drive the service itself.
        intent.getStringExtra("host")?.let(::startLink)
        setContent {
            ConduitTheme {
                ConduitApp(
                    peerName = LinkStatus.peerName,
                    path = LinkStatus.path,
                    state = LinkStatus.state,
                    history = History.entries,
                    toDesktop = FileTransfers.toDesktop,
                    toPhone = FileTransfers.toPhone,
                    hideNotifications = Settings.hideNotificationContent,
                    onHideNotifications = { Settings.hideNotificationContent = it },
                    onConnect = { send(ACTION_CONNECT) },
                    onDisconnect = { send(ACTION_DISCONNECT) },
                    onClearHistory = History::clear,
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        // singleTop means a second launch reuses this instance, so onCreate does not run
        // and the new extras would be dropped. setIntent first, because startLink reads
        // the port off the activity's current intent.
        setIntent(intent)
        intent.getStringExtra("host")?.let(::startLink)
    }

    /**
     * The two runtime permissions the manifest declares.
     *
     * Only ever asked for what is actually missing, and only the dangerous ones: reading
     * photos, and — on Android 13+ — posting the foreground service's own notification,
     * without which the service runs but shows nothing. Everything else Conduit needs is
     * install-time or a trip to Settings the user has to make anyway.
     */
    private fun request() {
        val wanted = buildList {
            add(Photos.READ)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                add(Manifest.permission.POST_NOTIFICATIONS)
            }
        }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty()) ask.launch(wanted.toTypedArray())
    }

    /** A null host means discover; anything else is a literal address. */
    private fun startLink(host: String?) {
        val service = Intent(this, SyncService::class.java)
        if (host != null) {
            service.putExtra("host", host)
            // Left unset when absent, so the service's own default applies.
            intent.getIntExtra("port", 0).takeIf { it > 0 }?.let { service.putExtra("port", it) }
        }
        startForegroundService(service)
    }

    /** Connect and disconnect are the same call with a different action. */
    private fun send(action: String) {
        val intent = Intent(this, SyncService::class.java).setAction(action)
        if (action == ACTION_DISCONNECT) startService(intent) else startForegroundService(intent)
    }
}

@Composable
private fun ConduitTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val scheme = when {
        // Material You wallpaper colours where the platform has them.
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.S ->
            if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
        dark -> darkColorScheme()
        else -> lightColorScheme()
    }
    MaterialTheme(colorScheme = scheme, content = content)
}

@Composable
private fun ConduitApp(
    peerName: String?,
    path: String?,
    state: LinkState,
    history: List<HistoryEntry>,
    toDesktop: FileTransfer?,
    toPhone: FileTransfer?,
    hideNotifications: Boolean,
    onHideNotifications: (Boolean) -> Unit,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onClearHistory: () -> Unit,
) {
    var page by rememberSaveable { mutableStateOf("home") }
    BackHandler(enabled = page == "history") { page = "home" }
    if (page == "history") {
        HistoryScreen(
            history = history,
            onBack = { page = "home" },
            onClear = onClearHistory,
        )
    } else {
        HomeScreen(
            peerName = peerName,
            path = path,
            state = state,
            historyCount = history.size,
            toDesktop = toDesktop,
            toPhone = toPhone,
            hideNotifications = hideNotifications,
            onHideNotifications = onHideNotifications,
            onConnect = onConnect,
            onDisconnect = onDisconnect,
            onOpenHistory = { page = "history" },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HomeScreen(
    peerName: String?,
    path: String?,
    state: LinkState,
    historyCount: Int,
    toDesktop: FileTransfer?,
    toPhone: FileTransfer?,
    hideNotifications: Boolean,
    onHideNotifications: (Boolean) -> Unit,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onOpenHistory: () -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(title = { Text("Conduit", fontWeight = FontWeight.SemiBold) })
        },
    ) { insets ->
        LazyColumn(
            modifier = Modifier.padding(insets),
            contentPadding = PaddingValues(start = 16.dp, top = 8.dp, end = 16.dp, bottom = 32.dp),
            verticalArrangement = Arrangement.spacedBy(24.dp),
        ) {
            item {
                ConnectionPanel(
                    state = state,
                    peerName = peerName,
                    path = path,
                    onConnect = onConnect,
                    onDisconnect = onDisconnect,
                )
            }
            if (toDesktop != null || toPhone != null) {
                item {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        SectionTitle("Transfers")
                        toDesktop?.let { TransferCard(it, peerName) }
                        toPhone?.let { TransferCard(it, peerName) }
                    }
                }
            }
            item {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    SectionTitle("Settings")
                    SettingsGroup(
                        historyCount = historyCount,
                        hideNotifications = hideNotifications,
                        onHideNotifications = onHideNotifications,
                        onOpenHistory = onOpenHistory,
                    )
                }
            }
        }
    }
}

@Composable
private fun SectionTitle(title: String) {
    Text(
        title,
        style = MaterialTheme.typography.titleMedium,
        fontWeight = FontWeight.SemiBold,
    )
}

@Composable
private fun ConnectionPanel(
    state: LinkState,
    peerName: String?,
    path: String?,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
) {
    val linked = state == LinkState.Connected
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 14.dp)
                .heightIn(min = 56.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Dot(state)
            Spacer(Modifier.size(12.dp))
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    text = peerName ?: "No computer",
                    style = MaterialTheme.typography.titleLarge,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    text = path?.takeIf(String::isNotBlank)?.let { "${state.label} · $it" } ?: state.label,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Spacer(Modifier.size(12.dp))
            if (linked) {
                TextButton(onClick = onDisconnect, modifier = Modifier.heightIn(min = 44.dp)) {
                    Text("Disconnect")
                }
            } else {
                Button(onClick = onConnect, modifier = Modifier.heightIn(min = 44.dp)) {
                    Text("Connect")
                }
            }
        }
    }
}

/** Decoration only — [LinkState.label] is what actually says the state. */
@Composable
private fun Dot(state: LinkState) {
    val colour = when (state) {
        LinkState.Connected -> Color(0xFF2FBF71)
        LinkState.Discovering, LinkState.Retrying -> Color(0xFFE0A02F)
        LinkState.Idle -> MaterialTheme.colorScheme.outline
    }
    Box(Modifier.size(10.dp).background(colour, CircleShape))
}

@Composable
private fun TransferCard(transfer: FileTransfer, peerName: String?) {
    val peer = peerName ?: "desktop"
    val direction = when (transfer.direction) {
        FileTransferDirection.ToDesktop -> "To $peer"
        FileTransferDirection.ToPhone -> "From $peer"
    }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.tertiaryContainer,
        ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    direction,
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.onTertiaryContainer,
                )
                Spacer(Modifier.weight(1f))
                Text(
                    "${transfer.percent}%",
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.onTertiaryContainer,
                )
            }
            Text(
                transfer.name,
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            LinearProgressIndicator(
                progress = { transfer.fraction },
                modifier = Modifier.fillMaxWidth(),
            )
            Text(
                "${formatBytes(transfer.transferred)} of ${formatBytes(transfer.total)}",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onTertiaryContainer,
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HistoryScreen(
    history: List<HistoryEntry>,
    onBack: () -> Unit,
    onClear: () -> Unit,
) {
    var query by rememberSaveable { mutableStateOf("") }
    val filtered = filterHistory(history, query)
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Clipboard history") },
                navigationIcon = {
                    TextButton(onClick = onBack, modifier = Modifier.heightIn(min = 48.dp)) {
                        Text("Back")
                    }
                },
                actions = {
                    if (history.isNotEmpty()) {
                        TextButton(onClick = onClear, modifier = Modifier.heightIn(min = 48.dp)) {
                            Text("Clear")
                        }
                    }
                },
            )
        },
    ) { insets ->
        LazyColumn(
            modifier = Modifier.padding(insets),
            contentPadding = PaddingValues(start = 16.dp, top = 8.dp, end = 16.dp, bottom = 28.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            item {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Search") },
                )
            }
            if (history.isEmpty()) {
                item {
                    Text(
                        "No clipboard history",
                        modifier = Modifier.padding(vertical = 12.dp),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            } else if (filtered.isEmpty()) {
                item {
                    Text(
                        "No matches",
                        modifier = Modifier.padding(vertical = 12.dp),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            } else {
                items(filtered, key = { "${it.at}:${it.direction}:${it.preview.hashCode()}" }) { ClipRow(it) }
            }
        }
    }
}

internal fun filterHistory(entries: List<HistoryEntry>, query: String): List<HistoryEntry> {
    val needle = query.trim()
    if (needle.isEmpty()) return entries
    return entries.filter { entry ->
        entry.preview.contains(needle, ignoreCase = true) ||
            entry.direction.name.contains(needle, ignoreCase = true)
    }
}

@Composable
private fun ClipRow(entry: HistoryEntry) {
    val clipboard = LocalClipboardManager.current
    val directionLabel = if (entry.direction == Direction.Sent) "Sent" else "Received"
    Card(
        modifier = Modifier.fillMaxWidth(),
        onClick = { if (!entry.image) clipboard.setText(AnnotatedString(entry.preview)) },
    ) {
        Column(
            modifier = Modifier.padding(14.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Surface(
                    modifier = Modifier.semantics { contentDescription = entry.direction.name },
                    shape = CircleShape,
                    color = MaterialTheme.colorScheme.secondaryContainer,
                ) {
                    Text(
                        directionLabel,
                        modifier = Modifier.padding(horizontal = 10.dp, vertical = 5.dp),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSecondaryContainer,
                    )
                }
                Text(
                    entry.ago().toString(),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Text(
                entry.preview,
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 4,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun SettingsGroup(
    historyCount: Int,
    hideNotifications: Boolean,
    onHideNotifications: (Boolean) -> Unit,
    onOpenHistory: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Card(
            modifier = Modifier.fillMaxWidth(),
            onClick = onOpenHistory,
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 12.dp)
                    .heightIn(min = 48.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "Clipboard history",
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Medium,
                )
                Text(
                    if (historyCount == 0) "Empty" else historyCount.toString(),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        Card(
            modifier = Modifier.fillMaxWidth(),
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 12.dp)
                    .heightIn(min = 48.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "Hide notification content",
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Medium,
                )
                Spacer(Modifier.size(12.dp))
                Switch(
                    checked = hideNotifications,
                    onCheckedChange = onHideNotifications,
                    modifier = Modifier.semantics {
                        contentDescription = "Hide notification content on Windows"
                    },
                )
            }
        }
    }
}
