package com.conduit.sync

import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
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
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp

/** What the one screen has to say. */
enum class LinkState(val label: String) {
    Idle("Not linked"),
    Discovering("Looking for the desktop"),
    Connected("Linked"),
}

/**
 * The screen's whole state. Snapshot state rather than a flow because writes come from
 * [Link]'s threads and Compose already handles that; a repository layer for three
 * fields would be scaffolding.
 */
object LinkStatus {
    var state by mutableStateOf(LinkState.Idle)
    var peer by mutableStateOf<String?>(null)
    var fingerprint by mutableStateOf("-- : -- : -- : -- : -- : -- : -- : --")
}

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Shown before anything is running, so the desktop can be paired against it.
        LinkStatus.fingerprint = Identity.fingerprint(Identity.loadOrCreate(filesDir).public)
        // A host on the launch intent pins the address and links straight away. It has to
        // go through the activity: Android 12+ refuses a foreground service started from
        // the background, so `am start-foreground-service` cannot drive the service itself.
        intent.getStringExtra("host")?.let(::startLink)
        setContent {
            ConduitTheme {
                HomeScreen(
                    fingerprint = LinkStatus.fingerprint,
                    peerName = LinkStatus.peer,
                    state = LinkStatus.state,
                    onLink = { startLink(null) },
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
    state: LinkState,
    onLink: () -> Unit,
) {
    Scaffold(topBar = { TopAppBar(title = { Text("conduit") }) }) { insets ->
        Column(
            modifier = Modifier
                .padding(insets)
                .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            StatusCard(state, peerName, onLink)
            IdentityCard(fingerprint)
        }
    }
}

@Composable
private fun StatusCard(state: LinkState, peerName: String?, onLink: () -> Unit) {
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
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Column {
                Text(state.label, style = MaterialTheme.typography.titleMedium)
                peerName?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall)
                }
            }
            if (state != LinkState.Connected) {
                Button(onClick = onLink) { Text("Link") }
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
