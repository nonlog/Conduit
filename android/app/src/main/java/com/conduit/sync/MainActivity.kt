package com.conduit.sync

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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp

/** What the one screen has to say. Nothing here is live yet — M0 wires it up. */
enum class LinkState(val label: String) {
    Idle("Not linked"),
    Discovering("Looking for the desktop"),
    Connected("Linked"),
}

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            ConduitTheme {
                HomeScreen(
                    fingerprint = "-- : -- : -- : -- : -- : -- : -- : --",
                    peerName = null,
                    state = LinkState.Idle,
                    onLink = {},
                )
            }
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
    Card(modifier = Modifier.fillMaxWidth()) {
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
