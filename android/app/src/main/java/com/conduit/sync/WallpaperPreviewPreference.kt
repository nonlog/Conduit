package com.conduit.sync

import android.content.Intent
import android.net.Uri
import android.provider.Settings as AndroidSettings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext

/** A user-initiated permission explanation, not a silent broad-storage grant. */
@Composable
internal fun WallpaperPreviewPreference() {
    val context = LocalContext.current
    var allowed by remember { mutableStateOf(Wallpapers.accessGranted(context)) }
    var confirm by remember { mutableStateOf(false) }
    fun refresh() {
        allowed = Wallpapers.accessGranted(context)
        Wallpapers.invalidate()
        if (allowed && LinkStatus.devices.any { it.state == LinkState.Connected }) {
            context.startService(Intent(context, SyncService::class.java).setAction(ACTION_REFRESH_WALLPAPER))
        }
    }
    val settings = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { refresh() }
    val legacyPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { refresh() }
    PreferenceRow(
        icon = R.drawable.ic_palette,
        title = "Device wallpaper preview",
        subtitle = if (allowed) "Allowed · tap to refresh on connected computers" else "Not allowed · enable to show your phone wallpaper on computers",
        onClick = { if (allowed) refresh() else confirm = true },
    )
    MotionAlertDialog(
        visible = confirm,
        onDismissRequest = { confirm = false },
        title = { Text("Allow wallpaper preview?") },
        text = { Text("Android requires its All files access permission to read the actual wallpaper. Conduit sends only a small encrypted preview to your paired computers. This is optional; file transfer and pairing do not need it.") },
        confirmButton = {
            TextButton(onClick = {
                confirm = false
                if (android.os.Build.VERSION.SDK_INT >= 33) {
                    runCatching { settings.launch(Intent(AndroidSettings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                        Uri.parse("package:${context.packageName}"))) }
                        .onFailure { android.widget.Toast.makeText(context, "Open Android Settings > Conduit > All files access to allow the wallpaper preview", android.widget.Toast.LENGTH_LONG).show() }
                } else {
                    legacyPermission.launch(android.Manifest.permission.READ_EXTERNAL_STORAGE)
                }
            }) { Text("Open permission settings") }
        },
        dismissButton = { TextButton(onClick = { confirm = false }) { Text("Not now") } },
    )
}
