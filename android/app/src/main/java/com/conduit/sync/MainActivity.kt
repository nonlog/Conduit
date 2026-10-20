package com.conduit.sync

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings as AndroidSettings
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts.GetContent
import androidx.activity.result.contract.ActivityResultContracts.GetMultipleContents
import androidx.activity.result.contract.ActivityResultContracts.RequestMultiplePermissions
import androidx.compose.foundation.background
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconToggleButton
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Shapes
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.google.mlkit.vision.codescanner.GmsBarcodeScannerOptions
import com.google.mlkit.vision.codescanner.GmsBarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode

private const val TAG = "conduit.ui"
private const val OPPO_APP_FREEZE_FEATURE = "oplus.software.pms_app_frozen"
private const val OPPO_APP_DISABLE_FEATURE = "oppo.appdisable.support"

/** What the one screen has to say. */
enum class LinkState(val label: String) {
    Idle("Not linked"),
    Discovering("Looking for the desktop"),

    /** Relay is reachable and this socket is parked without polling until the desktop appears. */
    Waiting("Offline · auto reconnect on"),

    /** Down, with an attempt already scheduled. See `SyncService.scheduleRetry`. */
    Retrying("Reconnecting"),
    Pairing("Pairing on LAN"),
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

    /** Persisted desktop id, separate from the fingerprint of the currently live session. */
    var pairedDeviceId by mutableStateOf<String?>(null)

    /** True only during the explicit two-minute same-LAN pairing window. */
    var pairing by mutableStateOf(false)

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
    private var batteryOptimizationIgnored by mutableStateOf(true)

    /** The public Settings page is only usable on Oplus builds that expose the freezer feature. */
    private val oplusAppFreezeSupported: Boolean by lazy(LazyThreadSafetyMode.NONE) {
        packageManager.hasSystemFeature(OPPO_APP_FREEZE_FEATURE) ||
            packageManager.hasSystemFeature(OPPO_APP_DISABLE_FEATURE)
    }

    private val pairingScanner by lazy {
        val options = GmsBarcodeScannerOptions.Builder()
            .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
            .enableAutoZoom()
            .build()
        GmsBarcodeScanning.getClient(this, options)
    }

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
        // pairing state must be accurate before the background service happens to start.
        if (LinkStatus.pairedDeviceId == null) LinkStatus.pairedDeviceId = Identity.peer(filesDir)
        if (LinkStatus.peerName == null) LinkStatus.peerName = Identity.peerName(filesDir)
        // The activity can be the first component to run, so it loads history too rather
        // than showing an empty list until the service happens to start.
        History.load(this)
        Settings.load(this)
        refreshClipboardAccessMode()
        refreshBatteryOptimization()
        request()
        val appVersion = appVersionName()
        // A host on the launch intent pins the address and links straight away. It has to
        // go through the activity: Android 12+ refuses a foreground service started from
        // the background, so `am start-foreground-service` cannot drive the service itself.
        intent.getStringExtra("host")?.let(::startLink)
        handlePairIntent(intent)
        setContent {
            ConduitTheme {
                ConduitApp(
                    appVersion = appVersion,
                    peerName = LinkStatus.peerName,
                    pairedDeviceId = LinkStatus.pairedDeviceId,
                    pairing = LinkStatus.pairing,
                    path = LinkStatus.path,
                    state = LinkStatus.state,
                    history = History.entries,
                    transfers = FileTransfers.entries,
                    onCancelTransfer = { id ->
                        startService(Intent(this, SyncService::class.java)
                            .setAction(ACTION_CANCEL_TRANSFER).putExtra(EXTRA_TRANSFER_ID, id))
                    },
                    hideNotifications = Settings.hideNotificationContent,
                    onHideNotifications = { Settings.hideNotificationContent = it },
                    clipboardMode = clipboardMode,
                    clipboardAccessibilityEnabled = clipboardAccessibilityEnabled,
                    onOpenClipboardAccessibility = {
                        startActivity(Intent(AndroidSettings.ACTION_ACCESSIBILITY_SETTINGS))
                    },
                    batteryOptimizationIgnored = batteryOptimizationIgnored,
                    oplusAppFreezeSupported = oplusAppFreezeSupported,
                    onRequestBatteryOptimization = ::requestBatteryOptimization,
                    onOpenOplusFreezeSettings = ::openOplusFreezeSettings,
                    onConnect = { send(ACTION_CONNECT) },
                    onDisconnect = { send(ACTION_DISCONNECT) },
                    onPair = ::sendPair,
                    onScanPairQr = ::scanPairingQr,
                    onCancelPair = { send(ACTION_CANCEL_PAIR) },
                    onForget = { send(ACTION_FORGET) },
                    onSendFiles = ::sendFiles,
                    onClearHistory = History::clear,
                )
            }
        }
    }

    private fun sendFiles(uris: List<android.net.Uri>) {
        val unique = uris.distinct()
        if (unique.isEmpty()) return

        // The picker can stay open while the desktop disappears. Re-check at the moment we
        // actually hand work to the service instead of trusting the state that made the button
        // visible before the picker launched.
        if (LinkStatus.state != LinkState.Connected) {
            val peer = LinkStatus.peerName ?: "the desktop"
            Log.w(TAG, "picked files while ${LinkStatus.state}; connecting instead of sending")
            send(ACTION_CONNECT)
            android.widget.Toast.makeText(
                this,
                "Not linked to $peer yet - connecting, try again in a moment",
                android.widget.Toast.LENGTH_SHORT,
            ).show()
            return
        }

        val sending = boundedShare(unique)
        if (unique.size > sending.size) {
            Log.w(TAG, "picker returned ${unique.size} files, sending the first $MAX_SHARE_FILES")
            android.widget.Toast.makeText(
                this,
                "Queuing ${sending.size} of ${unique.size} files",
                android.widget.Toast.LENGTH_SHORT,
            ).show()
        }

        val clip = android.content.ClipData.newRawUri("conduit", sending.first())
        sending.drop(1).forEach { clip.addItem(android.content.ClipData.Item(it)) }
        runCatching {
            startForegroundService(
                Intent(this, SyncService::class.java).apply {
                    action = ACTION_SHARE
                    clipData = clip
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                },
            )
        }.onFailure {
            Log.w(TAG, "cannot send ${sending.size} files", it)
            android.widget.Toast.makeText(this, "Could not queue the selected files; check read access",
                android.widget.Toast.LENGTH_SHORT).show()
        }
    }

    @Suppress("DEPRECATION")
    private fun appVersionName(): String =
        packageManager.getPackageInfo(packageName, 0).versionName ?: "unknown"

    override fun onResume() {
        super.onResume()
        refreshClipboardAccessMode()
        refreshBatteryOptimization()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        // singleTop means a second launch reuses this instance, so onCreate does not run
        // and the new extras would be dropped. setIntent first, because startLink reads
        // the port off the activity's current intent.
        setIntent(intent)
        intent.getStringExtra("host")?.let(::startLink)
        handlePairIntent(intent)
    }

    /** QR payload from the Windows pairing dialog. Invalid/foreign links never start the service. */
    private fun handlePairIntent(intent: Intent): Boolean {
        if (intent.action != Intent.ACTION_VIEW) return false
        val uri = intent.data ?: return false
        val code = PairingCode.fromQrPayload(uri.toString())
        if (code == null) {
            android.widget.Toast.makeText(this, "Invalid Conduit pairing code", android.widget.Toast.LENGTH_SHORT).show()
            return true
        }
        sendPair(code)
        return true
    }

    /**
     * Opens the Google Play services QR scanner only when the user asks for it. Conduit itself
     * never receives camera frames and therefore needs no CAMERA permission or background camera
     * component. The scanner module is downloaded on first use if the device does not have it yet.
     */
    private fun scanPairingQr(onCode: (String) -> Unit) {
        pairingScanner.startScan()
            .addOnSuccessListener { barcode ->
                val code = PairingCode.fromQrPayload(barcode.rawValue.orEmpty())
                if (code == null) {
                    android.widget.Toast.makeText(
                        this,
                        "That is not a Conduit pairing QR code",
                        android.widget.Toast.LENGTH_SHORT,
                    ).show()
                    return@addOnSuccessListener
                }
                onCode(code)
            }
            .addOnCanceledListener {
                Log.d(TAG, "pairing QR scan cancelled")
            }
            .addOnFailureListener { error ->
                Log.w(TAG, "pairing QR scanner failed", error)
                android.widget.Toast.makeText(
                    this,
                    "Could not open the QR scanner",
                    android.widget.Toast.LENGTH_SHORT,
                ).show()
            }
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
        when (action) {
            ACTION_DISCONNECT, ACTION_CANCEL_PAIR, ACTION_FORGET -> startService(intent)
            else -> startForegroundService(intent)
        }
    }

    private fun sendPair(code: String?) {
        val intent = Intent(this, SyncService::class.java).setAction(ACTION_PAIR)
        code?.let { intent.putExtra(EXTRA_PAIRING_CODE, PairingCode.normalize(it)) }
        startForegroundService(intent)
    }

    private fun refreshClipboardAccessMode() {
        clipboardAccessibilityEnabled = ClipboardAccess.isAccessibilityEnabled(this)
        clipboardMode = ClipboardAccess.mode(this)
    }

    private fun refreshBatteryOptimization() {
        batteryOptimizationIgnored = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            getSystemService(PowerManager::class.java).isIgnoringBatteryOptimizations(packageName)
        } else {
            true
        }
    }

    private fun requestBatteryOptimization() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.M || batteryOptimizationIgnored) return
        val request = Intent(AndroidSettings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
            .setData(Uri.parse("package:$packageName"))
        runCatching { startActivity(request) }
            .onFailure {
                Log.w(TAG, "battery optimization request unavailable", it)
                startActivity(Intent(AndroidSettings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS))
            }
    }

    /** Opens the OEM freezer when that feature exists, otherwise the usable per-app controls. */
    private fun openOplusFreezeSettings() {
        // Some export builds keep this legacy Settings activity in the manifest but make it
        // finish immediately because the app-freezer feature is disabled for that model/region.
        // Open the standard page instead of appearing to do nothing. It contains this phone's
        // "Allow background activity" switch.
        if (!oplusAppFreezeSupported) {
            openBackgroundDeliverySettings()
            return
        }

        val oplus = Intent("oplus.intent.action.settings.APP_FORZEN_OPLUS_SETTINGS")
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP)
        runCatching { startActivity(oplus) }
            .onFailure {
                Log.i(TAG, "Oplus app-freeze settings unavailable", it)
                openBackgroundDeliverySettings()
            }
    }

    private fun openBackgroundDeliverySettings() {
        val battery = Intent("android.settings.VIEW_ADVANCED_POWER_USAGE_DETAIL")
            .setData(Uri.parse("package:$packageName"))
            .putExtra("request_ignore_background_restriction", true)
        runCatching { startActivity(battery) }
            .onFailure {
                Log.w(TAG, "per-app battery settings unavailable", it)
                startActivity(
                    Intent(AndroidSettings.ACTION_APPLICATION_DETAILS_SETTINGS)
                        .setData(Uri.parse("package:$packageName")),
                )
            }
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
    MaterialTheme(
        colorScheme = scheme,
        shapes = Shapes(
            extraSmall = RoundedCornerShape(10.dp),
            small = RoundedCornerShape(14.dp),
            medium = RoundedCornerShape(20.dp),
            large = RoundedCornerShape(28.dp),
            extraLarge = RoundedCornerShape(36.dp),
        ),
        content = content,
    )
}

private enum class MainTab(val title: String) {
    Home("Home"),
    Settings("Settings"),
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ConduitApp(
    appVersion: String,
    peerName: String?,
    pairedDeviceId: String?,
    pairing: Boolean,
    path: String?,
    state: LinkState,
    history: List<HistoryEntry>,
    transfers: List<FileTransfer>,
    onCancelTransfer: (String) -> Unit,
    hideNotifications: Boolean,
    onHideNotifications: (Boolean) -> Unit,
    clipboardMode: ClipboardSyncMode,
    clipboardAccessibilityEnabled: Boolean,
    onOpenClipboardAccessibility: () -> Unit,
    batteryOptimizationIgnored: Boolean,
    oplusAppFreezeSupported: Boolean,
    onRequestBatteryOptimization: () -> Unit,
    onOpenOplusFreezeSettings: () -> Unit,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onPair: (String?) -> Unit,
    onScanPairQr: (((String) -> Unit) -> Unit),
    onCancelPair: () -> Unit,
    onForget: () -> Unit,
    onSendFiles: (List<android.net.Uri>) -> Unit,
    onClearHistory: () -> Unit,
) {
    var page by rememberSaveable { mutableStateOf("main") }
    var tabName by rememberSaveable { mutableStateOf(MainTab.Home.name) }
    var showPairDialog by rememberSaveable { mutableStateOf(false) }
    var pairCode by rememberSaveable { mutableStateOf("") }
    val tab = runCatching { MainTab.valueOf(tabName) }.getOrDefault(MainTab.Home)
    val requestPair = {
        pairCode = ""
        showPairDialog = true
    }

    if (showPairDialog) {
        AlertDialog(
            onDismissRequest = { showPairDialog = false },
            title = { Text("Pair desktop") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(
                        "On Windows, open Conduit Settings and choose Pair phone. " +
                            "Scan the QR code shown there, or enter its six-digit code. " +
                            "Both methods work through Conduit Relay even when the devices are on different networks.",
                    )
                    FilledTonalButton(
                        onClick = {
                            onScanPairQr { code ->
                                pairCode = code
                                showPairDialog = false
                                onPair(code)
                            }
                        },
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Text("Scan QR code")
                    }
                    OutlinedTextField(
                        value = pairCode,
                        onValueChange = { pairCode = PairingCode.normalize(it).take(PairingCode.LENGTH) },
                        label = { Text("Pairing code") },
                        placeholder = { Text("123456") },
                        supportingText = { Text("6 digits · valid for two minutes") },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
                        modifier = Modifier.fillMaxWidth(),
                    )
                    TextButton(
                        onClick = {
                            showPairDialog = false
                            onPair(null)
                        },
                    ) {
                        Text("Search the same LAN instead")
                    }
                }
            },
            confirmButton = {
                Button(
                    enabled = PairingCode.isValid(pairCode),
                    onClick = {
                        val code = PairingCode.normalize(pairCode)
                        showPairDialog = false
                        onPair(code)
                    },
                ) {
                    Text("Pair")
                }
            },
            dismissButton = {
                TextButton(onClick = { showPairDialog = false }) { Text("Cancel") }
            },
        )
    }

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
        containerColor = MaterialTheme.colorScheme.surfaceContainer,
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.surfaceContainer,
                    titleContentColor = MaterialTheme.colorScheme.onSurface,
                ),
                title = {
                    Column(
                        verticalArrangement = Arrangement.spacedBy(1.dp),
                    ) {
                        Text(
                            if (tab == MainTab.Home) "Conduit" else "Settings",
                            style = MaterialTheme.typography.headlineMedium,
                            fontWeight = FontWeight.SemiBold,
                        )
                        Text(
                            if (tab == MainTab.Home) "Your devices, in sync" else "Personalize your companion",
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                },
            )
        },
        bottomBar = {
            ExpressiveNavigationDock(
                selectedSettings = tab == MainTab.Settings,
                onHome = { tabName = MainTab.Home.name },
                onSettings = { tabName = MainTab.Settings.name },
            )
        },
    ) { insets ->
        when (tab) {
            MainTab.Home -> HomeTab(
                modifier = Modifier.padding(insets),
                peerName = peerName,
                pairedDeviceId = pairedDeviceId,
                pairing = pairing,
                path = path,
                state = state,
                history = history,
                transfers = transfers,
                onCancelTransfer = onCancelTransfer,
                onConnect = onConnect,
                onDisconnect = onDisconnect,
                onPair = requestPair,
                onCancelPair = onCancelPair,
                onSendFiles = onSendFiles,
                onOpenHistory = { page = "history" },
            )
            MainTab.Settings -> SettingsTab(
                modifier = Modifier.padding(insets),
                appVersion = appVersion,
                pairedDeviceId = pairedDeviceId,
                peerName = peerName,
                pairing = pairing,
                historyCount = history.size,
                hideNotifications = hideNotifications,
                onHideNotifications = onHideNotifications,
                clipboardMode = clipboardMode,
                clipboardAccessibilityEnabled = clipboardAccessibilityEnabled,
                onOpenClipboardAccessibility = onOpenClipboardAccessibility,
                batteryOptimizationIgnored = batteryOptimizationIgnored,
                oplusAppFreezeSupported = oplusAppFreezeSupported,
                onRequestBatteryOptimization = onRequestBatteryOptimization,
                onOpenOplusFreezeSettings = onOpenOplusFreezeSettings,
                onPair = requestPair,
                onCancelPair = onCancelPair,
                onForget = onForget,
                onOpenHistory = { page = "history" },
            )
        }
    }
}

@Composable
private fun HomeTab(
    modifier: Modifier,
    peerName: String?,
    pairedDeviceId: String?,
    pairing: Boolean,
    path: String?,
    state: LinkState,
    history: List<HistoryEntry>,
    transfers: List<FileTransfer>,
    onCancelTransfer: (String) -> Unit,
    onConnect: () -> Unit,
    onDisconnect: () -> Unit,
    onPair: () -> Unit,
    onCancelPair: () -> Unit,
    onSendFiles: (List<android.net.Uri>) -> Unit,
    onOpenHistory: () -> Unit,
) {
    val pickFiles = rememberLauncherForActivityResult(GetMultipleContents()) { uris ->
        if (uris.isNotEmpty()) onSendFiles(uris)
    }
    val pickPhoto = rememberLauncherForActivityResult(GetContent()) { uri ->
        if (uri != null) onSendFiles(listOf(uri))
    }

    LazyColumn(
        modifier = modifier.fillMaxWidth(),
        contentPadding = PaddingValues(
            start = ExpressiveLayout.pageInset,
            top = 12.dp,
            end = ExpressiveLayout.pageInset,
            bottom = 26.dp,
        ),
        verticalArrangement = Arrangement.spacedBy(ExpressiveLayout.cardSpacing),
    ) {
        item {
            ExpressiveDeviceCard(
                state = state,
                peerName = peerName,
                pairedDeviceId = pairedDeviceId,
                pairing = pairing,
                path = path,
                onConnect = onConnect,
                onDisconnect = onDisconnect,
                onPair = onPair,
                onCancelPair = onCancelPair,
                onSendFiles = { pickFiles.launch("*/*") },
                onSendPhoto = { pickPhoto.launch("image/*") },
            )
        }
        if (transfers.isNotEmpty()) {
            item {
                ExpressiveSectionTitle(
                    title = "Transfers",
                    detail = transfers.count { !it.phase.terminal }.takeIf { it > 0 }?.let { it.toString() + " active" },
                )
            }
            items(transfers, key = { "${it.direction}:${it.id}" }) {
                ExpressiveTransferCard(it, peerName, onCancelTransfer)
            }
        }
        item {
            ExpressiveSectionTitle("Clipboard", "Recent activity")
        }
        item {
            DeviceActionsCard(
                history = history,
                onOpenHistory = onOpenHistory,
            )
        }
    }
}

@Composable
private fun SettingsTab(
    modifier: Modifier,
    appVersion: String,
    pairedDeviceId: String?,
    peerName: String?,
    pairing: Boolean,
    historyCount: Int,
    hideNotifications: Boolean,
    onHideNotifications: (Boolean) -> Unit,
    clipboardMode: ClipboardSyncMode,
    clipboardAccessibilityEnabled: Boolean,
    onOpenClipboardAccessibility: () -> Unit,
    batteryOptimizationIgnored: Boolean,
    oplusAppFreezeSupported: Boolean,
    onRequestBatteryOptimization: () -> Unit,
    onOpenOplusFreezeSettings: () -> Unit,
    onPair: () -> Unit,
    onCancelPair: () -> Unit,
    onForget: () -> Unit,
    onOpenHistory: () -> Unit,
) {
    var confirmForget by rememberSaveable { mutableStateOf(false) }
    if (confirmForget) {
        AlertDialog(
            onDismissRequest = { confirmForget = false },
            title = { Text("Forget ${peerName ?: "desktop"}?") },
            text = {
                Text(
                    "This removes the pairing from this phone and tells the connected desktop to forget it too. " +
                        "Your phone identity, clipboard history, and settings are kept.",
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    confirmForget = false
                    onForget()
                }) { Text("Forget") }
            },
            dismissButton = {
                TextButton(onClick = { confirmForget = false }) { Text("Cancel") }
            },
        )
    }

    LazyColumn(
        modifier = modifier.fillMaxWidth(),
        contentPadding = PaddingValues(
            start = ExpressiveLayout.pageInset,
            top = 8.dp,
            end = ExpressiveLayout.pageInset,
            bottom = 26.dp,
        ),
        verticalArrangement = Arrangement.spacedBy(ExpressiveLayout.cardSpacing),
    ) {
        item { ExpressiveSectionTitle("Appearance") }
        item {
            ExpressiveCard(modifier = Modifier.fillMaxWidth()) {
                SwitchPreferenceRow(
                    icon = R.drawable.ic_brand_sync,
                    title = "Liquid glass",
                    subtitle = "Floating glass navigation and device surfaces",
                    checked = Settings.liquidGlassEnabled,
                    onCheckedChange = { Settings.liquidGlassEnabled = it },
                )
            }
        }
        item {
            ExpressiveSectionTitle("Devices")
        }
        item {
            DeviceManagementCard(
                peerName = peerName,
                pairedDeviceId = pairedDeviceId,
                pairing = pairing,
                onPair = onPair,
                onCancelPair = onCancelPair,
                onForget = { confirmForget = true },
            )
        }
        item {
            ExpressiveSectionTitle("Sync & privacy")
        }
        item {
            ExpressiveCard(modifier = Modifier.fillMaxWidth()) {
                Column {
                    PreferenceRow(
                        icon = R.drawable.ic_history,
                        title = "Clipboard history",
                        subtitle = if (historyCount == 0) "No saved items" else "$historyCount saved items",
                        onClick = onOpenHistory,
                    )
                    HorizontalDivider(modifier = Modifier.padding(horizontal = 16.dp))
                    PreferenceRow(
                        icon = R.drawable.ic_sync,
                        title = "Clipboard sync mode",
                        subtitle = when (clipboardMode) {
                            ClipboardSyncMode.Lsposed -> if (clipboardAccessibilityEnabled) {
                                "LSPosed (root) · Accessibility standby"
                            } else {
                                "LSPosed (root) · Active"
                            }
                            ClipboardSyncMode.Accessibility ->
                                "Accessibility · Non-root compatibility"
                            ClipboardSyncMode.Unavailable ->
                                "Unavailable · Enable Accessibility service"
                        },
                        onClick = onOpenClipboardAccessibility,
                    )
                    HorizontalDivider(modifier = Modifier.padding(horizontal = 16.dp))
                    SwitchPreferenceRow(
                        icon = R.drawable.ic_notifications,
                        title = "Hide notification content",
                        subtitle = "Show app names only on Windows",
                        checked = hideNotifications,
                        onCheckedChange = onHideNotifications,
                    )
                }
            }
        }
        item {
            ExpressiveSectionTitle("Background delivery")
        }
        item {
            ExpressiveCard(modifier = Modifier.fillMaxWidth()) {
                PreferenceRow(
                    icon = R.drawable.ic_settings,
                    title = "Battery optimization",
                    subtitle = if (batteryOptimizationIgnored) {
                        "Ignored · screen-off delivery is allowed"
                    } else {
                        "Tap to allow Conduit to run while the screen is off"
                    },
                    onClick = onRequestBatteryOptimization,
                )
                if (Build.MANUFACTURER.equals("oneplus", ignoreCase = true) ||
                    Build.MANUFACTURER.equals("oppo", ignoreCase = true) ||
                    Build.BRAND.equals("oneplus", ignoreCase = true) ||
                    Build.BRAND.equals("oppo", ignoreCase = true)
                ) {
                    HorizontalDivider(modifier = Modifier.padding(horizontal = 16.dp))
                    PreferenceRow(
                        icon = R.drawable.ic_settings,
                        title = if (oplusAppFreezeSupported) {
                            "ColorOS app freeze"
                        } else {
                            "Background battery settings"
                        },
                        subtitle = if (oplusAppFreezeSupported) {
                            "Disable Quick Freeze for Conduit"
                        } else {
                            "Allow Conduit to run while the screen is off"
                        },
                        onClick = onOpenOplusFreezeSettings,
                    )
                }
            }
        }
        item {
            ExpressiveSectionTitle("About")
        }
        item {
            ExpressiveCard(modifier = Modifier.fillMaxWidth()) {
                PreferenceRow(
                    icon = R.drawable.ic_brand_sync,
                    title = "Conduit",
                    subtitle = "Version $appVersion · Fast, lightweight companion",
                )
            }
        }
    }
}

@Composable
private fun DeviceManagementCard(
    peerName: String?,
    pairedDeviceId: String?,
    pairing: Boolean,
    onPair: () -> Unit,
    onCancelPair: () -> Unit,
    onForget: () -> Unit,
) {
    ExpressiveCard(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Surface(
                    modifier = Modifier.size(48.dp),
                    shape = RoundedCornerShape(16.dp),
                    color = MaterialTheme.colorScheme.secondaryContainer,
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            painter = painterResource(R.drawable.ic_desktop),
                            contentDescription = null,
                            modifier = Modifier.size(22.dp),
                            tint = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                    }
                }
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(
                        peerName ?: if (pairedDeviceId != null) "Paired desktop" else "No paired desktop",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                    if (pairing || pairedDeviceId == null) {
                        Text(
                            if (pairing) {
                                "Pairing open · same Wi-Fi required"
                            } else {
                                "Pair a desktop to enable LAN and Relay reconnect"
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (pairedDeviceId != null && !pairing) {
                    TextButton(onClick = onForget) { Text("Forget") }
                }
                FilledTonalButton(
                    onClick = if (pairing) onCancelPair else onPair,
                    shape = RoundedCornerShape(19.dp),
                ) {
                    Text(
                        when {
                            pairing -> "Cancel pairing"
                            pairedDeviceId != null -> "Pair new desktop"
                            else -> "Pair desktop"
                        },
                    )
                }
            }
        }
    }
}

@Composable
private fun DeviceActionsCard(history: List<HistoryEntry>, onOpenHistory: () -> Unit) {
    val latest = history.firstOrNull()
    ExpressiveCard(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onOpenHistory),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(19.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Surface(
                    modifier = Modifier.size(46.dp),
                    shape = RoundedCornerShape(17.dp),
                    color = MaterialTheme.colorScheme.secondaryContainer,
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            painter = painterResource(R.drawable.ic_history),
                            contentDescription = null,
                            modifier = Modifier.size(23.dp),
                            tint = MaterialTheme.colorScheme.onSecondaryContainer,
                        )
                    }
                }
                Spacer(Modifier.width(10.dp))
                Text(
                    "Latest item",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                if (history.isNotEmpty()) {
                    Spacer(Modifier.width(6.dp))
                    Surface(
                        shape = CircleShape,
                        color = MaterialTheme.colorScheme.primary.copy(alpha = 0.12f),
                    ) {
                        Text(
                            "${history.size}",
                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.primary,
                            fontWeight = FontWeight.Bold,
                        )
                    }
                }
                Spacer(Modifier.weight(1f))
                latest?.let {
                    Text(
                        it.ago().toString(),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }

            Text(
                latest?.preview?.takeIf(String::isNotBlank) ?: "No clipboard history yet",
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 3,
                overflow = TextOverflow.Ellipsis,
                color = if (latest == null) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
            )

            latest?.let { clip ->
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        val sent = clip.direction == Direction.Sent
                        Icon(
                            painter = painterResource(if (sent) R.drawable.ic_stat_upload else R.drawable.ic_stat_download),
                            contentDescription = null,
                            modifier = Modifier.size(14.dp),
                            tint = MaterialTheme.colorScheme.primary,
                        )
                        Text(
                            if (sent) "Sent to desktop" else "Received from desktop",
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
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
            .heightIn(min = 68.dp)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            painter = painterResource(icon),
            contentDescription = null,
            modifier = Modifier.padding(start = 20.dp, end = 10.dp).size(24.dp),
            tint = MaterialTheme.colorScheme.primary,
        )
        Column(
                modifier = Modifier.weight(1f).padding(vertical = 14.dp),
        ) {
            Text(
                title,
                modifier = Modifier.padding(end = 14.dp),
                style = MaterialTheme.typography.titleLarge,
                fontSize = 16.sp,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            subtitle?.takeIf(String::isNotBlank)?.let {
                Text(
                    it,
                    modifier = Modifier.padding(end = 14.dp),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
                )
            }
        }
        trailing?.let { Box(Modifier.padding(end = 18.dp), contentAlignment = Alignment.Center) { it() } }
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
        containerColor = MaterialTheme.colorScheme.surfaceContainer,
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        "Clipboard history",
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.SemiBold,
                    )
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.surfaceContainer,
                ),
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
            contentPadding = PaddingValues(
                start = ExpressiveLayout.pageInset,
                top = 12.dp,
                end = ExpressiveLayout.pageInset,
                bottom = 28.dp,
            ),
            verticalArrangement = Arrangement.spacedBy(ExpressiveLayout.cardSpacing),
        ) {
            item {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    modifier = Modifier.fillMaxWidth(),
                    shape = RoundedCornerShape(19.dp),
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
    val directionLabel = if (entry.direction == Direction.Sent) "Sent" else "Received"
    ExpressiveCard(modifier = Modifier.fillMaxWidth()) {
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
