package com.conduit.sync

import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.drawable.Icon
import android.util.Log

private const val TAG = "conduit.share"

/**
 * The entry Conduit puts in Android's share sheet.
 *
 * The point of this file is one detail: the share sheet's ordinary app row can only show
 * the label baked into the manifest, which would be "Conduit" forever. A *sharing shortcut*
 * can be labelled at runtime, and the system draws those in the Direct Share row above the
 * app list — so the desktop's own name is what the user actually taps. That is the whole
 * reason a shortcut is involved rather than just an intent filter.
 *
 * Platform [ShortcutManager] rather than `ShortcutManagerCompat`: sharing shortcuts became
 * a framework feature in API 29, which is this app's minimum, so the AndroidX wrapper would
 * only be a dependency to do the same call.
 *
 * The shortcut is republished whenever the desktop names itself. `addDynamicShortcuts`
 * replaces the one with a matching id, so this cannot accumulate — there is exactly one
 * desktop and therefore exactly one shortcut, forever.
 */
object ShareTarget {

    /** One desktop, one shortcut, one id. Re-publishing the same id replaces it. */
    private const val ID = "desktop"

    /**
     * Must match the `<category>` under `<share-target>` in `res/xml/shortcuts.xml`, or the
     * system pairs the shortcut with no share target and never offers it.
     */
    private const val CATEGORY = "com.conduit.sync.category.SEND"

    /**
     * Publishes, or updates, the share-sheet entry for a desktop called [name].
     *
     * Never throws: a launcher that refuses the shortcut costs the Direct Share row, and the
     * plain `ACTION_SEND` filter on [ShareActivity] still works. `setLongLived` is what lets
     * the system cache it and rank it in the share sheet after the app has been idle.
     */
    fun publish(context: Context, name: String) {
        val manager = context.getSystemService(ShortcutManager::class.java) ?: return
        val shortcut = ShortcutInfo.Builder(context, ID)
            .setShortLabel(name)
            .setLongLabel(name)
            .setIcon(Icon.createWithResource(context, R.drawable.ic_share_target))
            .setCategories(setOf(CATEGORY))
            // A shortcut needs an intent even as a share target, because tapping it from a
            // launcher's shortcut menu has to do something. Sharing nothing means there is
            // nothing to send, so it opens the app.
            .setIntent(Intent(context, MainActivity::class.java).setAction(Intent.ACTION_VIEW))
            .setLongLived(true)
            .build()
        runCatching { manager.addDynamicShortcuts(listOf(shortcut)) }
            .onSuccess { Log.i(TAG, "share target is now '$name'") }
            .onFailure { Log.w(TAG, "could not publish the share target", it) }
    }
}
