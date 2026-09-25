package com.conduit.sync

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log

private const val TAG = "conduit.boot"

internal fun shouldStartLinkOnBoot(linkWanted: Boolean, hasPairedPeer: Boolean): Boolean =
    linkWanted && hasPairedPeer

/**
 * Restores the user's existing companion-link request after Android finishes a normal boot.
 *
 * BOOT_COMPLETED is used rather than LOCKED_BOOT_COMPLETED because Conduit's identity and settings
 * live in credential-encrypted filesDir. The connectedDevice foreground-service type is permitted
 * from BOOT_COMPLETED on current Android releases, and SyncService immediately owns the foreground
 * lifecycle once started.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return

        Settings.load(context)
        val hasPairedPeer = Identity.peer(context.filesDir) != null
        if (!shouldStartLinkOnBoot(Settings.linkWanted, hasPairedPeer)) {
            Log.i(
                TAG,
                "boot completed; link restore skipped (wanted=${Settings.linkWanted}, paired=$hasPairedPeer)",
            )
            return
        }

        runCatching {
            context.startForegroundService(
                Intent(context, SyncService::class.java).setAction(ACTION_BOOT_CONNECT),
            )
        }.onSuccess {
            Log.i(TAG, "boot completed; companion link restore requested")
        }.onFailure {
            Log.w(TAG, "could not restore companion link after boot", it)
        }
    }
}
