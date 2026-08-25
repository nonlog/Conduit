package com.conduit.sync

import android.content.ContentUris
import android.content.Context
import android.database.ContentObserver
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log

private const val TAG = "conduit.screenshot"

/**
 * The target OnePlus/ColorOS device publishes screenshots here. Keep this narrow rather than
 * treating every image whose name happens to contain "Screenshot" as a capture.
 */
private const val PATH = "Pictures/Screenshots/%"
private const val NAME_PREFIX = "Screenshot_"

/**
 * New phone screenshots mirrored to the desktop.
 *
 * The observer is edge-triggered and owns no thread/timer. Its callback only queues work on
 * [Link]'s existing sender. The query then walks new MediaStore rows in id order, which means
 * two screenshots taken before the sender drains its queue are delivered one per callback
 * instead of the newest one causing the older one to disappear.
 */
class Screenshots(private val context: Context, private val link: Link) {

    /** Existing screenshots are not news when the service starts. */
    private val since = System.currentTimeMillis() / 1000

    /** Last row consumed, touched only by the sender thread through [next]. */
    private var lastId = 0L

    private val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
        override fun onChange(selfChange: Boolean, uri: Uri?) {
            // `photo=true` is intentional compatibility: an older desktop that does not
            // know the screenshot field still keeps the image out of the clipboard.
            link.sendImage("screenshot", photo = true, screenshot = true) { next() }
        }
    }

    fun start() {
        if (!Photos.granted(context)) {
            Log.w(TAG, "${Photos.READ} not granted, so screenshots cannot be read")
        }
        context.contentResolver.registerContentObserver(
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
            true,
            observer,
        )
        Log.i(TAG, "watching $PATH for screenshots added after $since")
    }

    fun stop() = context.contentResolver.unregisterContentObserver(observer)

    /** The oldest new screenshot not yet consumed, or null when this callback was unrelated. */
    private fun next(): Images.Payload? {
        val (id, uri) = nextRow() ?: return null
        // Consume the row before I/O. A corrupt/unreadable screenshot must not be retried on
        // every later MediaStore event forever.
        lastId = id
        val mime = context.contentResolver.getType(uri) ?: "image/png"
        Log.i(TAG, "new screenshot $id ($mime)")
        return Images.read(context, uri, mime)
    }

    private fun nextRow(): Pair<Long, Uri>? = runCatching {
        context.contentResolver.query(
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
            arrayOf(
                MediaStore.Images.Media._ID,
                MediaStore.Images.Media.DISPLAY_NAME,
            ),
            "${MediaStore.Images.Media.RELATIVE_PATH} LIKE ? AND " +
                "${MediaStore.Images.Media.DATE_ADDED} > ? AND " +
                "${MediaStore.Images.Media._ID} > ?",
            arrayOf(PATH, since.toString(), lastId.toString()),
            // Oldest first so a burst is drained rather than collapsed to its last row.
            "${MediaStore.Images.Media._ID} ASC",
        )?.use { cursor ->
            while (cursor.moveToNext()) {
                val id = cursor.getLong(0)
                val name = cursor.getString(1).orEmpty()
                if (!name.startsWith(NAME_PREFIX)) continue
                return@use id to ContentUris.withAppendedId(
                    MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
                    id,
                )
            }
            null
        }
    }.onFailure { Log.w(TAG, "could not query screenshots", it) }.getOrNull()
}
