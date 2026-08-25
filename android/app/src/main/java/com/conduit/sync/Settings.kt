package com.conduit.sync

import android.content.Context
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.File

private const val TAG = "conduit.settings"

/**
 * The handful of choices that are the user's rather than the design's.
 *
 * Two `key=value` lines in `filesDir` — the same way [Identity] keeps the peer's name, and for
 * a blunt reason: `getSharedPreferences` never creates its directory on the phone this was
 * built against, so every `apply()` is silently dropped and the choice resets on the next
 * launch. No error, no exception, and it looks exactly like a switch that was never touched.
 * `filesDir` demonstrably works there. Two booleans do not need a key-value store anyway.
 *
 * Note what is *not* here. Android decides on its own whether a notification listener may see
 * sensitive content, and when it decides no, it substitutes the string "Sensitive notification
 * content" before this app is ever called — so no setting here can turn that off. That one is
 * granted per-install with:
 *
 *   adb shell cmd appops set com.conduit.sync RECEIVE_SENSITIVE_NOTIFICATIONS allow
 */
object Settings {

    private const val FILE = "settings.txt"
    private const val HIDE_NOTIFICATION_CONTENT = "hide_notification_content"
    private const val LINK_WANTED = "link_wanted"

    /**
     * Whether a mirrored notification arrives on the desktop with its text removed.
     *
     * Off by default. A companion app that silently blanked what it mirrored would be
     * indistinguishable from a broken one, so hiding is a decision the user makes.
     *
     * Written through to disk by the setter, so nothing has to remember to save.
     */
    var hideNotificationContent: Boolean
        get() = hidden
        set(value) {
            hidden = value
            save()
        }

    /**
     * Whether the user wants a link at all, which is the one thing here that is not a
     * preference so much as a memory of a tap.
     *
     * On by default, and persisted because `START_STICKY` lets the system restart
     * [SyncService] with a null intent — a restart that silently reconnects is a restart that
     * overrides the disconnect the user asked for.
     */
    var linkWanted: Boolean
        get() = wanted
        set(value) {
            wanted = value
            save()
        }

    /** The snapshot state behind [hideNotificationContent]; the settings screen reads it. */
    private var hidden by mutableStateOf(false)

    /**
     * Volatile rather than snapshot state: nothing recomposes on it, but [SyncService] reads
     * it from [Link]'s reader thread while the taps that write it arrive on the main one.
     */
    @Volatile private var wanted = true

    /** Set once by [load]; null until then, which is why the setters cannot write before it. */
    private var dir: File? = null

    /** Idempotent: the activity, the service and the listener each start independently. */
    @Synchronized
    fun load(context: Context) {
        if (dir != null) return
        val files = context.applicationContext.filesDir
        dir = files
        val stored = runCatching { File(files, FILE).readText() }.getOrNull()
            ?.lineSequence()
            ?.map { it.trim().split('=', limit = 2) }
            ?.filter { it.size == 2 }
            ?.associate { it[0] to it[1] }
            .orEmpty()
        // Absent means the default, and so does unparseable: a file that has been corrupted
        // should leave the link willing to come up, because that is one tap to undo, where a
        // link stuck down reads as an app that stopped working.
        hidden = stored[HIDE_NOTIFICATION_CONTENT].toBoolean()
        wanted = stored[LINK_WANTED]?.toBooleanStrictOrNull() ?: true
        // Says what was restored, because a setting that silently fails to persist looks
        // exactly like one that was never toggled.
        Log.i(TAG, "settings loaded, hide=$hidden wanted=$wanted")
    }

    /**
     * Rewrites the whole file, which for two lines is the simplest thing that cannot
     * half-succeed. Best effort: a setting that fails to save is worth less than a crash on
     * the main thread, and [load] falls back to the defaults.
     */
    private fun save() {
        val files = dir
        if (files == null) {
            Log.w(TAG, "not loaded, so hide=$hidden wanted=$wanted is not being saved")
            return
        }
        runCatching {
            File(files, FILE).writeText(
                "$HIDE_NOTIFICATION_CONTENT=$hidden\n$LINK_WANTED=$wanted\n",
            )
        }.onFailure { Log.w(TAG, "could not save settings", it) }
    }
}
