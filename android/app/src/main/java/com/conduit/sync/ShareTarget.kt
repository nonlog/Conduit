package com.conduit.sync

import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.drawable.Icon
import android.util.Log

private const val TAG = "conduit.share"

/** One immutable shortcut id per trusted Noise identity, not one shared desktop slot. */
object ShareTarget {
    private const val LEGACY_ID = "desktop"
    private const val PREFIX = "desktop-"
    private const val CATEGORY = "com.conduit.sync.category.SEND"

    /** A direct-share shortcut is optional; every paired computer remains in the in-app chooser. */
    fun publishAll(context: Context, peers: List<PairedDesktop>) {
        val manager = context.getSystemService(ShortcutManager::class.java) ?: return
        val max = manager.maxShortcutCountPerActivity.coerceAtMost(4).coerceAtLeast(0)
        val shortcuts = peers.take(max).map { peer ->
            val label = peer.name.ifBlank { "Conduit computer" }.take(25)
            ShortcutInfo.Builder(context, PREFIX + peer.id.take(20))
                .setShortLabel(label)
                .setLongLabel(peer.name.ifBlank { label })
                .setIcon(Icon.createWithResource(context, R.drawable.ic_share_target))
                .setCategories(setOf(CATEGORY))
                .setIntent(Intent(context, MainActivity::class.java)
                    .setAction(Intent.ACTION_VIEW).putExtra(EXTRA_PEER_ID, peer.id))
                .setLongLived(true)
                .build()
        }
        runCatching {
            manager.setDynamicShortcuts(shortcuts)
            // Old releases used the same shortcut id for every new computer. Once the
            // per-peer list has been published, remove that stale long-lived legacy shortcut.
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R) {
                manager.removeLongLivedShortcuts(listOf(LEGACY_ID))
            }
        }.onFailure { Log.w(TAG, "cannot publish per-computer share targets", it) }
    }

    /** Compatibility entrypoint for older callers; preserves peers already in the registry. */
    fun publish(context: Context, name: String) {
        val peers = PairedDesktops.list(context.filesDir)
        publishAll(context, peers)
    }

    fun clear(context: Context) = publishAll(context, emptyList())
}
