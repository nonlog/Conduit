package com.conduit.sync

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts.RequestMultiplePermissions
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
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
                HomeScreen(
                    fingerprint = LinkStatus.fingerprint,
                    peerName = LinkStatus.peerName,
                    peerFingerprint = LinkStatus.peer,
                    path = LinkStatus.path,
                    state = LinkStatus.state,
                    history = History.entries,
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
        startForegroundService(Intent(this, SyncService::class.java).setAction(action))
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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HomeScreen(
    fingerprint: String,
    peerName: String?,
    peerFingerprint: String?,
    path: String?,
    state: LinkState,
    history: List<HistoryEntry>,
    hideNotifications: Boolean,
    onHideNotifications: (Boolean) -> Unit,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onClearHistory: () -> Unit,
) {
    Scaffold(topBar = { TopAppBar(title = { Text("Conduit") }) }) { insets ->
        // One lazy list for the whole screen, so the history scrolls under the cards
        // instead of the cards needing their own scroll container.
        LazyColumn(
            modifier = Modifier.padding(insets),
            contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            item { StatusCard(state, peerName, peerFingerprint, path, onConnect, onDisconnect) }
            item { IdentityCard(fingerprint) }
            item { SettingsCard(hideNotifications, onHideNotifications) }
            item { HistoryHeader(history.isNotEmpty(), onClearHistory) }
            if (history.isEmpty()) {
                item {
                    Text(
                        "Nothing synced yet.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            items(history, key = { "${it.at}:${it.direction}" }) { ClipRow(it) }
        }
    }
}

@Composable
private fun StatusCard(
    state: LinkState,
    peerName: String?,
    peerFingerprint: String?,
    path: String?,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = if (state == LinkState.Connected) {
                MaterialTheme.colorScheme.secondaryContainer
            } else {
                MaterialTheme.colorScheme.surfaceVariant
            },
        ),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Dot(state)
                Spacer(Modifier.size(8.dp))
                // The label carries the state; the dot only repeats it, so colour is
                // never the only signal.
                Text(state.label, style = MaterialTheme.typography.titleMedium)
                Spacer(Modifier.weight(1f))
                path?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            // The desktop's name, which is the only line on this card a human recognises at
            // a glance. Shown even while disconnected, because it is remembered — "Not
            // linked / LOG" says considerably more than "Not linked" alone.
            peerName?.let {
                Text(it, style = MaterialTheme.typography.titleLarge)
            }
            peerFingerprint?.let {
                Text(
                    it,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                if (state == LinkState.Connected) {
                    // Outlined, because disconnecting is not the action to draw the eye.
                    OutlinedButton(onClick = onDisconnect, modifier = Modifier.heightIn(min = 48.dp)) {
                        Text("Disconnect")
                    }
                } else {
                    Button(onClick = onConnect, modifier = Modifier.heightIn(min = 48.dp)) {
                        Text(if (state == LinkState.Retrying) "Connect now" else "Connect")
                    }
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
private fun HistoryHeader(any: Boolean, onClear: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text("Clipboard history", style = MaterialTheme.typography.titleSmall)
        if (any) {
            TextButton(onClick = onClear, modifier = Modifier.heightIn(min = 48.dp)) {
                Text("Clear")
            }
        }
    }
}

@Composable
private fun ClipRow(entry: HistoryEntry) {
    val clipboard = LocalClipboardManager.current
    val arrow = if (entry.direction == Direction.Sent) "↑" else "↓"
    Card(
        modifier = Modifier.fillMaxWidth(),
        // Tapping puts it back on the clipboard. Only the stored preview, which for a long
        // clip is a prefix — the full text was deliberately never kept, so the history
        // cannot grow without bound.
        onClick = { if (!entry.image) clipboard.setText(AnnotatedString(entry.preview)) },
    ) {
        Column(
            modifier = Modifier.padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    // The arrow is a glyph, so the direction has to be said out loud for
                    // anything that is not reading it with eyes.
                    .semantics { contentDescription = entry.direction.name },
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    "$arrow ${entry.direction.name}",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.primary,
                )
                Text(
                    entry.ago().toString(),
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Text(
                entry.preview,
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 3,
            )
        }
    }
    HorizontalDivider(color = Color.Transparent)
}

@Composable
private fun SettingsCard(hideNotifications: Boolean, onHideNotifications: (Boolean) -> Unit) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text("Settings", style = MaterialTheme.typography.titleSmall)
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 48.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text("Hide notification content", style = MaterialTheme.typography.bodyLarge)
                    Text(
                        // Says which redaction this is, because Android has one of its own
                        // that looks the same on the desktop and is fixed somewhere else.
                        "Mirror only the app name, never the message. Off by default — " +
                            "Android's own \"Sensitive notification content\" is a separate " +
                            "restriction this switch cannot lift.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Spacer(Modifier.size(12.dp))
                Switch(checked = hideNotifications, onCheckedChange = onHideNotifications)
            }
        }
    }
}

@Composable
private fun IdentityCard(fingerprint: String) {
    val clipboard = LocalClipboardManager.current
    // Tapping copies, which is how you get the fingerprint into the desktop's pairing
    // field. It is also the only way to originate a clip from inside this app while it
    // holds focus, which makes it the one trigger that tests the outbound path without
    // the LSPosed hook.
    Card(
        modifier = Modifier.fillMaxWidth(),
        onClick = { clipboard.setText(AnnotatedString(fingerprint)) },
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text("This phone", style = MaterialTheme.typography.titleSmall)
            // Compared by eye against the desktop during pairing, so monospace.
            Text(
                fingerprint,
                style = MaterialTheme.typography.bodyMedium,
                fontFamily = FontFamily.Monospace,
            )
        }
    }
}
