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
        val running = SyncService.activeLink != null && Settings.linkWanted
        if (running) {
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
        val running = SyncService.activeLink != null && Settings.linkWanted
        tile.icon = Icon.createWithResource(this, R.drawable.ic_stat_link)
        tile.label = "Conduit"
        tile.state = if (running) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.subtitle = when {
            LinkStatus.state == LinkState.Connected -> "Linked to $peer"
            running -> "Connecting to $peer"
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
