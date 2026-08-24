package com.conduit.sync

import android.content.Context
import android.text.format.DateUtils
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
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
 * The clipboard history, in memory for the UI and in `SharedPreferences` for the next run.
 *
 * `apply()` rather than a thread of our own: it updates the in-memory map immediately and
 * hands the disk write to the platform's own background writer, so recording a clip costs
 * no file IO on the calling thread and adds no thread to the process. That matters because
 * the callers are the main thread (a local copy) and [Link]'s reader thread (a remote one),
 * and neither may block.
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

    private const val FILE = "history"
    private const val KEY = "entries"

    /**
     * Snapshot state, so Compose recomposes on its own. Same pattern as [LinkStatus], and
     * for the same reason: writes arrive from background threads and Compose handles that.
     */
    var entries by mutableStateOf<List<HistoryEntry>>(emptyList())
        private set

    /** Set once; `apply()` needs a context and the callers are two different components. */
    private var prefs: android.content.SharedPreferences? = null

    /** Idempotent, because whichever of the service and the activity starts first calls it. */
    @Synchronized
    fun load(context: Context) {
        if (prefs != null) return
        val store = context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)
        prefs = store
        entries = runCatching { decode(store.getString(KEY, null)) }.getOrDefault(emptyList())
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
        prefs?.edit()?.putString(KEY, encode(entries))?.apply()
    }

    @Synchronized
    fun clear() {
        entries = emptyList()
        prefs?.edit()?.remove(KEY)?.apply()
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
