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
 * Snapshot state so the UI reads it directly and recomposes on a change, over a one-line file
 * in `filesDir` — the same way [Identity] keeps the peer's name, and for a blunt reason:
 * `getSharedPreferences` never creates its directory on the phone this was built against, so
 * every `apply()` is silently dropped and the setting resets on the next launch. `filesDir`
 * demonstrably works there. A boolean does not need a key-value store anyway.
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

    /** The snapshot state behind the property above; separate only so the setter can persist. */
    private var hidden by mutableStateOf(false)

    /** Set once by [load]; null until then, which is why the setter cannot write before it. */
    private var dir: File? = null

    /** Idempotent: the activity, the service and the listener each start independently. */
    @Synchronized
    fun load(context: Context) {
        if (dir != null) return
        val files = context.applicationContext.filesDir
        dir = files
        hidden = runCatching { File(files, FILE).readText() }
            .getOrNull()
            ?.lineSequence()
            ?.any { it.trim() == "$HIDE_NOTIFICATION_CONTENT=true" }
            ?: false
        // Says what was restored, because a setting that silently fails to persist looks
        // exactly like one that was never toggled.
        Log.i(TAG, "settings loaded, hide=$hidden")
    }

    /**
     * Rewrites the whole file, which for one line is the simplest thing that cannot
     * half-succeed. Best effort: a setting that fails to save is worth less than a crash on
     * the main thread, and [load] falls back to the default.
     */
    private fun save() {
        val files = dir
        if (files == null) {
            Log.w(TAG, "not loaded, so hide=$hidden is not being saved")
            return
        }
        runCatching { File(files, FILE).writeText("$HIDE_NOTIFICATION_CONTENT=$hidden\n") }
            .onFailure { Log.w(TAG, "could not save settings", it) }
    }
}
