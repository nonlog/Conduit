package com.conduit.sync

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.provider.Settings as AndroidSettings
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts.RequestMultiplePermissions
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
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
import androidx.compose.material3.FilledIconToggleButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private const val TAG = "conduit.ui"

/** What the one screen has to say. */
enum class LinkState(val label: String) {
    Idle("Not linked"),
    Discovering("Looking for the desktop"),

    /** Relay is reachable and this socket is parked without polling until the desktop appears. */
    Waiting("Waiting for the desktop"),

    /** Down, with an attempt already scheduled. See `SyncService.scheduleRetry`. */
    Retrying("Reconnecting"),
    Connected("Linked"),
}

/** Discovery and retry are still an active user request and therefore must remain stoppable. */
internal fun isLinkRequestedState(state: LinkState): Boolean = state != LinkState.Idle

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
    private var clipboardMode by mutableStateOf(ClipboardSyncMode.Unavailable)
    private var clipboardAccessibilityEnabled by mutableStateOf(false)

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
        refreshClipboardAccessMode()
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
                    clipboardMode = clipboardMode,
                    clipboardAccessibilityEnabled = clipboardAccessibilityEnabled,
                    onOpenClipboardAccessibility = {
                        startActivity(Intent(AndroidSettings.ACTION_ACCESSIBILITY_SETTINGS))
                    },
                    onConnect = { send(ACTION_CONNECT) },
                    onDisconnect = { send(ACTION_DISCONNECT) },
                    onClearHistory = History::clear,
                )
            }
        }
    }

    override fun onResume() {
        super.onResume()
        refreshClipboardAccessMode()
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

    private fun refreshClipboardAccessMode() {
        clipboardAccessibilityEnabled = ClipboardAccess.isAccessibilityEnabled(this)
        clipboardMode = ClipboardAccess.mode(this)
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

private enum class MainTab(val title: String) {
    Home("Home"),
    Settings("Settings"),
}

@OptIn(ExperimentalMaterial3Api::class)
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
    clipboardMode: ClipboardSyncMode,
    clipboardAccessibilityEnabled: Boolean,
    onOpenClipboardAccessibility: () -> Unit,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onClearHistory: () -> Unit,
) {
    var page by rememberSaveable { mutableStateOf("main") }
    var tabName by rememberSaveable { mutableStateOf(MainTab.Home.name) }
    val tab = runCatching { MainTab.valueOf(tabName) }.getOrDefault(MainTab.Home)

    BackHandler(enabled = page == "history") { page = "main" }
    if (page == "history") {
        HistoryScreen(
            history = history,
            onBack = { page = "main" },
            onClear = onClearHistory,
        )
        return
    }

    Scaffold(
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(
                    titleContentColor = MaterialTheme.colorScheme.onSecondaryContainer,
                ),
                title = { Text(tab.title) },
            )
        },
        bottomBar = {
            NavigationBar {
                MainTab.entries.forEach { item ->
                    val selected = item == tab
                    NavigationBarItem(
                        selected = selected,
                        onClick = { tabName = item.name },
                        icon = {
                            Icon(
                                painter = painterResource(
                                    when (item) {
                                        MainTab.Home -> R.drawable.ic_home
                                        MainTab.Settings -> R.drawable.ic_settings
                                    },
                                ),
                                contentDescription = null,
                                modifier = Modifier.size(24.dp),
                            )
                        },
                        label = { Text(item.title) },
                    )
                }
            }
        },
    ) { insets ->
        when (tab) {
            MainTab.Home -> HomeTab(
                modifier = Modifier.padding(insets),
                peerName = peerName,
                path = path,
                state = state,
                history = history,
                toDesktop = toDesktop,
                toPhone = toPhone,
                onConnect = onConnect,
                onDisconnect = onDisconnect,
                onOpenHistory = { page = "history" },
            )
            MainTab.Settings -> SettingsTab(
                modifier = Modifier.padding(insets),
                historyCount = history.size,
                hideNotifications = hideNotifications,
                onHideNotifications = onHideNotifications,
                clipboardMode = clipboardMode,
                clipboardAccessibilityEnabled = clipboardAccessibilityEnabled,
                onOpenClipboardAccessibility = onOpenClipboardAccessibility,
                onOpenHistory = { page = "history" },
            )
        }
    }
}

@Composable
private fun HomeTab(
    modifier: Modifier,
    peerName: String?,
    path: String?,
    state: LinkState,
    history: List<HistoryEntry>,
    toDesktop: FileTransfer?,
    toPhone: FileTransfer?,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onOpenHistory: () -> Unit,
) {
    LazyColumn(
        modifier = modifier,
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        item {
            SefirahDeviceCard(
                state = state,
                peerName = peerName,
                path = path,
                onConnect = onConnect,
                onDisconnect = onDisconnect,
            )
        }
        item {
            DeviceActionsCard(history = history, onOpenHistory = onOpenHistory)
        }
        if (toDesktop != null || toPhone != null) {
            item {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Transfers", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                    toDesktop?.let { TransferCard(it, peerName) }
                    toPhone?.let { TransferCard(it, peerName) }
                }
            }
        }
    }
}

@Composable
private fun SettingsTab(
    modifier: Modifier,
    historyCount: Int,
    hideNotifications: Boolean,
    onHideNotifications: (Boolean) -> Unit,
    clipboardMode: ClipboardSyncMode,
    clipboardAccessibilityEnabled: Boolean,
    onOpenClipboardAccessibility: () -> Unit,
    onOpenHistory: () -> Unit,
) {
    LazyColumn(
        modifier = modifier,
        contentPadding = PaddingValues(vertical = 4.dp),
    ) {
        item {
            PreferenceRow(
                icon = R.drawable.ic_history,
                title = "Clipboard history",
                subtitle = if (historyCount == 0) "No saved items" else "$historyCount saved items",
                onClick = onOpenHistory,
            )
        }
        item {
            PreferenceRow(
                icon = R.drawable.ic_sync,
                title = "Clipboard sync",
                subtitle = when (clipboardMode) {
                    ClipboardSyncMode.Lsposed -> if (clipboardAccessibilityEnabled) {
                        "Mode: LSPosed (root) · Accessibility enabled as standby"
                    } else {
                        "Mode: LSPosed (root) · Accessibility not required"
                    }
                    ClipboardSyncMode.Accessibility ->
                        "Mode: Accessibility · Non-root compatibility"
                    ClipboardSyncMode.Unavailable ->
                        "Mode: unavailable · Enable Accessibility on non-root devices"
                },
                onClick = onOpenClipboardAccessibility,
            )
        }
        item {
            SwitchPreferenceRow(
                icon = R.drawable.ic_notifications,
                title = "Hide notification content",
                subtitle = "Show app names only on Windows",
                checked = hideNotifications,
                onCheckedChange = onHideNotifications,
            )
        }
        item { HorizontalDivider(modifier = Modifier.padding(vertical = 4.dp)) }
        item {
            PreferenceRow(
                icon = R.drawable.ic_brand_sync,
                title = "Conduit",
                subtitle = "Connected-device sync",
            )
        }
    }
}

@Composable
private fun SefirahDeviceCard(
    state: LinkState,
    peerName: String?,
    path: String?,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
) {
    // A user-requested link stays "on" while discovery/retry is in progress. Treating only the
    // fully connected state as active made the toggle call Connect again while the UI said
    // "Looking for the desktop", leaving no way to stop an unavailable desktop search.
    val linkRequested = isLinkRequestedState(state)
    Card(modifier = Modifier.fillMaxWidth(), shape = MaterialTheme.shapes.large, colors = CardDefaults.cardColors()) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(
                modifier = Modifier.size(56.dp),
                shape = CircleShape,
                color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.14f),
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(
                        painter = painterResource(R.drawable.ic_brand_sync),
                        contentDescription = null,
                        modifier = Modifier.size(36.dp),
                        tint = MaterialTheme.colorScheme.primary,
                    )
                }
            }
            Spacer(Modifier.size(16.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    peerName ?: "No computer",
                    style = MaterialTheme.typography.bodyLarge,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.primary,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(state.label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.primary)
                path?.takeIf(String::isNotBlank)?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            FilledIconToggleButton(
                checked = linkRequested,
                onCheckedChange = { if (linkRequested) onDisconnect() else onConnect() },
                colors = IconButtonDefaults.filledIconToggleButtonColors(
                    containerColor = MaterialTheme.colorScheme.surfaceVariant,
                    contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                    checkedContainerColor = MaterialTheme.colorScheme.primaryContainer,
                    checkedContentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                ),
            ) {
                Icon(
                    painter = painterResource(R.drawable.ic_sync),
                    contentDescription = if (linkRequested) "Disconnect" else "Connect",
                )
            }
        }
    }
}

@Composable
private fun DeviceActionsCard(history: List<HistoryEntry>, onOpenHistory: () -> Unit) {
    val latest = history.firstOrNull()
    Card(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onOpenHistory),
        shape = MaterialTheme.shapes.large,
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Surface(
                modifier = Modifier.size(48.dp),
                shape = CircleShape,
                color = MaterialTheme.colorScheme.primaryContainer,
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(
                        painter = painterResource(R.drawable.ic_history),
                        contentDescription = null,
                        modifier = Modifier.size(24.dp),
                        tint = MaterialTheme.colorScheme.onPrimaryContainer,
                    )
                }
            }
            Spacer(Modifier.size(14.dp))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Clipboard", style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
                    if (history.isNotEmpty()) {
                        Spacer(Modifier.size(8.dp))
                        Text(
                            history.size.toString(),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
                Text(
                    latest?.preview?.takeIf(String::isNotBlank) ?: "No clipboard history yet",
                    style = MaterialTheme.typography.bodyMedium,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                    color = if (latest == null) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
                )
                latest?.let {
                    Text(
                        it.ago().toString(),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            latest?.let { clip ->
                Spacer(Modifier.size(12.dp))
                Surface(
                    modifier = Modifier.size(36.dp),
                    shape = CircleShape,
                    color = MaterialTheme.colorScheme.surfaceVariant,
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        val sent = clip.direction == Direction.Sent
                        Icon(
                            painter = painterResource(if (sent) R.drawable.ic_stat_upload else R.drawable.ic_stat_download),
                            contentDescription = if (sent) "Sent to desktop" else "Received from desktop",
                            modifier = Modifier.size(20.dp),
                            tint = MaterialTheme.colorScheme.primary,
                        )
                    }
                }
            }
        }
    }
}
@Composable
private fun PreferenceRow(
    icon: Int,
    title: String,
    subtitle: String? = null,
    onClick: (() -> Unit)? = null,
    trailing: (@Composable () -> Unit)? = null,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 64.dp)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            painter = painterResource(icon),
            contentDescription = null,
            modifier = Modifier.padding(start = 16.dp, end = 8.dp).size(24.dp),
            tint = MaterialTheme.colorScheme.primary,
        )
        Column(
            modifier = Modifier.weight(1f).padding(vertical = 16.dp),
        ) {
            Text(
                title,
                modifier = Modifier.padding(horizontal = 16.dp),
                style = MaterialTheme.typography.titleLarge,
                fontSize = 16.sp,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            subtitle?.takeIf(String::isNotBlank)?.let {
                Text(
                    it,
                    modifier = Modifier.padding(horizontal = 16.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
            }
        }
        trailing?.let { Box(Modifier.padding(end = 16.dp), contentAlignment = Alignment.Center) { it() } }
    }
}

@Composable
private fun SwitchPreferenceRow(
    icon: Int,
    title: String,
    subtitle: String? = null,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    PreferenceRow(
        icon = icon,
        title = title,
        subtitle = subtitle,
        onClick = { onCheckedChange(!checked) },
        trailing = {
            Switch(
                checked = checked,
                onCheckedChange = null,
                modifier = Modifier.semantics { contentDescription = title },
            )
        },
    )
}

@Composable
private fun TransferCard(transfer: FileTransfer, peerName: String?) {
    val peer = peerName ?: "desktop"
    val direction = when (transfer.direction) {
        FileTransferDirection.ToDesktop -> "To $peer"
        FileTransferDirection.ToPhone -> "From $peer"
    }
    Card(modifier = Modifier.fillMaxWidth(), colors = CardDefaults.cardColors()) {
        Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(direction, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                Spacer(Modifier.weight(1f))
                Text("${transfer.percent}%", style = MaterialTheme.typography.titleSmall)
            }
            Text(transfer.name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            LinearProgressIndicator(progress = { transfer.fraction }, modifier = Modifier.fillMaxWidth())
            Text("${formatBytes(transfer.transferred)} of ${formatBytes(transfer.total)}", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
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
    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.large,
        colors = CardDefaults.cardColors(),
    ) {
        Column {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onOpenHistory)
                    .padding(horizontal = 16.dp, vertical = 14.dp)
                    .heightIn(min = 48.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        "Clipboard history",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Medium,
                    )
                    Text(
                        if (historyCount == 0) "No saved items" else "$historyCount saved items",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Text(
                    "Open",
                    style = MaterialTheme.typography.labelLarge,
                    color = MaterialTheme.colorScheme.primary,
                )
            }

            HorizontalDivider(modifier = Modifier.padding(horizontal = 16.dp))

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 14.dp)
                    .heightIn(min = 48.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(modifier = Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    Text(
                        "Hide notification content",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Medium,
                    )
                    Text(
                        "Mirror app names only",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
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
