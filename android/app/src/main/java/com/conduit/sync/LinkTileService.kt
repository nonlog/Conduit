package com.conduit.sync

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService

/** Quick Settings toggle for the one Conduit link. */
class LinkTileService : TileService() {

    override fun onStartListening() {
        super.onStartListening()
        Settings.load(this)
        updateTile()
    }

    override fun onClick() {
        super.onClick()
        Settings.load(this)
        // The persisted request is authoritative. A service can be between process creation and
        // Link initialisation while the tile is tapped, and using activeLink as the gate made an
        // in-progress search impossible to stop: the tap simply issued CONNECT again.
        val paired = Identity.peer(filesDir) != null
        if (!paired && !LinkStatus.pairing) {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            return
        }
        val linkRequested = Settings.linkWanted
        if (linkRequested) {
            // Persist first. Even if the service disappears between this check and startService,
            // START_STICKY cannot silently undo the user's explicit toggle-off.
            Settings.linkWanted = false
            startService(Intent(this, SyncService::class.java).setAction(ACTION_DISCONNECT))
        } else {
            startForegroundService(Intent(this, SyncService::class.java).setAction(ACTION_CONNECT))
        }
        updateTile()
    }

    private fun updateTile() {
        val tile = qsTile ?: return
        val peer = LinkStatus.peerName ?: Identity.peerName(filesDir) ?: "desktop"
        val paired = LinkStatus.pairedDeviceId ?: Identity.peer(filesDir)
        val linkRequested = Settings.linkWanted
        tile.icon = Icon.createWithResource(this, R.drawable.ic_stat_link)
        tile.label = "Conduit"
        tile.state = if (linkRequested && paired != null) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.subtitle = when {
            LinkStatus.pairing -> "Pairing on LAN"
            paired == null -> "Open Conduit to pair"
            LinkStatus.state == LinkState.Connected -> "Linked to $peer"
            LinkStatus.state == LinkState.Waiting -> "Waiting for $peer"
            LinkStatus.state == LinkState.Retrying -> "Reconnecting to $peer"
            linkRequested -> "Connecting to $peer"
            else -> "Not linked"
        }
        tile.updateTile()
    }

    companion object {
        fun refresh(context: Context) {
            requestListeningState(
                context,
                ComponentName(context, LinkTileService::class.java),
            )
        }
    }
}
