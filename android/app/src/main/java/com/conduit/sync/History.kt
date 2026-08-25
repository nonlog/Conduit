package com.conduit.sync

import android.content.Context
import android.text.format.DateUtils
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.File
import org.json.JSONArray
import org.json.JSONObject

/** Which way a clip travelled. */
enum class Direction { Sent, Received }

/**
 * One clipboard event worth remembering.
 *
 * [preview] is already truncated — the full text is on the clipboard, not here, and a
 * history that stored 64 000-character clips would be the one unbounded thing in the app.
 */
data class HistoryEntry(
    val direction: Direction,
    val preview: String,
    val at: Long,
    /** An image clip; [preview] then describes it rather than quoting it. */
    val image: Boolean = false,
) {
    fun ago(): CharSequence = DateUtils.getRelativeTimeSpanString(
        at,
        System.currentTimeMillis(),
        DateUtils.MINUTE_IN_MILLIS,
        DateUtils.FORMAT_ABBREV_RELATIVE,
    )
}

/**
 * The clipboard history, in memory for the UI and in a file for the next run.
 *
 * One JSON file in `filesDir`, the same as [Settings] and [Identity], and for the same blunt
 * reason: `getSharedPreferences` never creates its directory on the phone this was built
 * against, so every `apply()` was silently dropped — no error, no exception — and the history
 * came back empty on every launch. It looked exactly like a history that was never recorded.
 *
 * The write is synchronous on the calling thread, where `apply()` handed it to the platform's
 * own background writer. That is affordable only because of the bounds below: the file cannot
 * exceed a few tens of kB, and a clip happens at human speed rather than in a loop. The
 * callers are the main thread (a local copy) and [Link]'s reader thread (a remote one), so the
 * alternative would be a thread of our own, which this project spends its effort not having.
 *
 * Bounded twice over, because this is the one structure here that grows with use: at most
 * [MAX] entries, each at most [PREVIEW] characters. Worst case on disk is well under 50 kB
 * and cannot grow past it however long the app runs.
 */
object History {

    /** Two screens of scrolling. Past this, older clips are of no interest to anyone. */
    private const val MAX = 100

    /** Enough to recognise a clip by eye; the clipboard holds the real thing. */
    private const val PREVIEW = 200

    private const val FILE = "history.json"

    /**
     * Snapshot state, so Compose recomposes on its own. Same pattern as [LinkStatus], and
     * for the same reason: writes arrive from background threads and Compose handles that.
     */
    var entries by mutableStateOf<List<HistoryEntry>>(emptyList())
        private set

    /** Set once by [load]; null until then, which is why [record] cannot write before it. */
    private var file: File? = null

    /** Idempotent, because whichever of the service and the activity starts first calls it. */
    @Synchronized
    fun load(context: Context) {
        if (file != null) return
        val store = File(context.applicationContext.filesDir, FILE)
        file = store
        entries = runCatching { decode(store.takeIf { it.isFile }?.readText()) }
            .getOrDefault(emptyList())
    }

    /**
     * Records a clip. Newest first, so the UI needs no sorting and the trim drops the tail.
     *
     * Synchronized because the two callers are on different threads and this is a
     * read-modify-write; the critical section is a list copy of at most [MAX] entries.
     */
    @Synchronized
    fun record(direction: Direction, text: String, image: Boolean = false) {
        val entry = HistoryEntry(
            direction = direction,
            preview = text.take(PREVIEW),
            at = System.currentTimeMillis(),
            image = image,
        )
        entries = (listOf(entry) + entries).take(MAX)
        save()
    }

    @Synchronized
    fun clear() {
        entries = emptyList()
        file?.let { runCatching { it.delete() } }
    }

    /**
     * Best effort, like [Settings]: a history that fails to write is worth less than an
     * exception on the clipboard path.
     *
     * Returns before [encode] rather than after, which is not just an early exit — nothing
     * must touch `org.json` when there is nowhere to write, because it is a stub that throws
     * on the JVM and [HistoryBoundTest] exercises exactly this object with no directory. The
     * old `prefs?.edit()?.putString(KEY, encode(entries))` got that for free from the
     * safe-call chain; an argument would have been evaluated eagerly.
     *
     * ponytail: whole-file rewrite, not write-then-rename. A process killed mid-write leaves
     * JSON that will not parse, and [load] then starts empty — losing a list of previews of
     * clips the user still has. Make it atomic if that ever costs anything real.
     */
    private fun save() {
        val store = file ?: return
        runCatching { store.writeText(encode(entries)) }
    }

    /**
     * `org.json` is part of the platform, so the format costs no dependency. One array of
     * flat objects; an entry that will not parse is skipped rather than failing the load,
     * because a corrupt history must not stop the app from starting.
     */
    private fun encode(list: List<HistoryEntry>): String {
        val array = JSONArray()
        list.forEach {
            array.put(
                JSONObject()
                    .put("d", it.direction.name)
                    .put("t", it.preview)
                    .put("a", it.at)
                    .put("i", it.image),
            )
        }
        return array.toString()
    }

    private fun decode(raw: String?): List<HistoryEntry> {
        if (raw == null) return emptyList()
        val array = JSONArray(raw)
        return (0 until array.length()).mapNotNull { i ->
            runCatching {
                val o = array.getJSONObject(i)
                HistoryEntry(
                    direction = Direction.valueOf(o.getString("d")),
                    preview = o.getString("t"),
                    at = o.getLong("a"),
                    image = o.optBoolean("i", false),
                )
            }.getOrNull()
        }.take(MAX)
    }
}
