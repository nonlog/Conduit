package com.conduit.sync

import android.Manifest
import android.content.ContentUris
import android.content.Context
import android.content.pm.PackageManager
import android.database.ContentObserver
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.util.Log
import java.io.ByteArrayOutputStream

private const val TAG = "conduit.photo"

/**
 * Longest edge after downscaling.
 *
 * A Windows toast renders its hero image at 364x180 dip, so most of a 12 MP frame is
 * bytes nobody sees — and over the relay those bytes are cellular data. It is not
 * smaller than this because the toast is only the doorway: clicking it opens the photo
 * in Snipping Tool, and that is the resolution you then have to mark up.
 */
private const val MAX_EDGE = 1280

/** Photographs, so JPEG. 85 is where the artefacts stop being visible at this size. */
private const val QUALITY = 85

/**
 * New camera photos, mirrored to the desktop.
 *
 * Edge-triggered like everything else here: MediaStore notifies a [ContentObserver] when
 * the camera app publishes a picture, and nothing polls. No thread of its own either —
 * the callback does nothing but hand the work to [Link]'s existing sender thread, where
 * the query, the decode and the JPEG encode all happen. Idle cost is therefore zero.
 */
class Photos(private val context: Context, private val link: Link) {

    /**
     * Seconds, matching `DATE_ADDED`. The photos already on the phone when the service
     * started are not news, and mirroring a library on first launch would be the worst
     * possible first impression.
     */
    private val since = System.currentTimeMillis() / 1000

    /**
     * Highest MediaStore id already sent. Touched only from inside the load lambda, which
     * runs on the one sender thread, so it needs no lock. It is also what makes the media
     * scanner's several writes per file cost one transfer instead of several.
     */
    private var lastId = 0L

    private val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
        override fun onChange(selfChange: Boolean, uri: Uri?) {
            // Deliberately nothing here. The query, the decode and the encode are all far
            // too heavy for a callback, and `sendImage` already runs its lambda on the
            // thread dedicated to outbound frames. The notified URI is ignored on purpose:
            // it is not always the item, so the newest row is looked up either way.
            link.sendImage("photo", photo = true) { next() }
        }
    }

    fun start() {
        if (!granted(context)) {
            // Not fatal, and not worth refusing to register over: the permission can be
            // granted while the service runs, and this is then the only clue as to why
            // nothing arrived until it was.
            Log.w(TAG, "$READ not granted, so new photos will be noticed but not readable")
        }
        context.contentResolver.registerContentObserver(
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
            // Descendants too, because the notification names the individual item.
            true,
            observer,
        )
        Log.i(TAG, "watching for photos added after $since")
    }

    fun stop() = context.contentResolver.unregisterContentObserver(observer)

    /** The newest camera photo not yet sent, downscaled, or null if there is nothing new. */
    private fun next(): Images.Payload? {
        val (id, uri) = newest() ?: return null
        if (id <= lastId) return null
        lastId = id
        Log.i(TAG, "new photo $id")
        return downscale(uri)
    }

    private fun newest(): Pair<Long, Uri>? = runCatching {
        context.contentResolver.query(
            MediaStore.Images.Media.EXTERNAL_CONTENT_URI,
            arrayOf(MediaStore.Images.Media._ID),
            // DCIM is where cameras write. Pictures/Screenshots deliberately is not: the
            // feature is "a photo was taken", and a screenshot is not that.
            //
            // Pending rows are excluded for free — a half-written capture belongs to the
            // camera app, and another app's pending row is invisible to this query.
            "${MediaStore.Images.Media.RELATIVE_PATH} LIKE ? AND " +
                "${MediaStore.Images.Media.DATE_ADDED} > ?",
            arrayOf("DCIM/%", since.toString()),
            // Ids are monotonic, so this orders by arrival and matches the dedupe key
            // exactly. No LIMIT in the sort order: Android 11 rejects that.
            "${MediaStore.Images.Media._ID} DESC",
        )?.use { cursor ->
            if (!cursor.moveToFirst()) return@use null
            val id = cursor.getLong(0)
            id to ContentUris.withAppendedId(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, id)
        }
    }.onFailure { Log.w(TAG, "could not query MediaStore", it) }.getOrNull()

    private fun downscale(uri: Uri): Images.Payload? = runCatching {
        val source = ImageDecoder.createSource(context.contentResolver, uri)
        // ImageDecoder rather than BitmapFactory: it subsamples during the decode instead
        // of allocating the full frame first, and it applies the EXIF rotation a phone
        // camera always writes — without which half the photos arrive on their side.
        val bitmap = ImageDecoder.decodeBitmap(source) { decoder, info, _ ->
            val edge = maxOf(info.size.width, info.size.height)
            decoder.setTargetSampleSize(maxOf(1, edge / MAX_EDGE))
            // compress() needs pixels this process can read back, and the default
            // allocator is free to hand out a GPU-only bitmap.
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        }
        val out = ByteArrayOutputStream()
        bitmap.compress(Bitmap.CompressFormat.JPEG, QUALITY, out)
        bitmap.recycle()
        Images.Payload(out.toByteArray(), "image/jpeg")
            .also { Log.i(TAG, "photo is ${it.bytes.size} B after downscaling") }
    }.onFailure { Log.w(TAG, "could not read $uri", it) }.getOrNull()

    companion object {
        /**
         * Reading photos split away from reading storage in Android 13, so which one to
         * ask for is a version question. Defined here rather than in the activity because
         * this is the code that needs it.
         */
        val READ = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            Manifest.permission.READ_MEDIA_IMAGES
        } else {
            Manifest.permission.READ_EXTERNAL_STORAGE
        }

        fun granted(context: Context) =
            context.checkSelfPermission(READ) == PackageManager.PERMISSION_GRANTED
    }
}
