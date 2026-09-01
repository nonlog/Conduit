package com.conduit.sync

import android.app.WallpaperManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.Log
import java.io.ByteArrayOutputStream
import java.security.MessageDigest

private const val WALLPAPER_TAG = "conduit.wallpaper"
private const val MAX_PREVIEW_BYTES = 56 * 1024
private const val MAX_PREVIEW_WIDTH = 180
private const val MAX_PREVIEW_HEIGHT = 400

/**
 * Produces one small cached home-wallpaper preview for the Windows phone frame.
 *
 * There is deliberately no poller. The service asks once after an authenticated session and
 * invalidates this cache only from WallpaperManager's existing change callback. Android 14+
 * protects the actual pixels behind the system "All files access" app-op. Conduit declares that
 * optional capability but never requests it automatically; without it this method simply returns
 * no preview and the desktop keeps its local fallback.
 */
object Wallpapers {
    class Preview(val jpeg: ByteArray, val sha256: ByteArray)

    @Volatile private var cached: Preview? = null

    fun invalidate() {
        cached = null
    }

    fun preview(context: Context, force: Boolean = false): Preview? {
        if (!force) cached?.let { return it }
        val bitmap = loadPlatformWallpaper(context) ?: return null
        return try {
            encodePreview(bitmap)?.also { cached = it }
        } finally {
            bitmap.recycle()
        }
    }

    @Suppress("DEPRECATION")
    private fun loadPlatformWallpaper(context: Context): Bitmap? = runCatching {
        val manager = WallpaperManager.getInstance(context)
        manager.getWallpaperFile(WallpaperManager.FLAG_SYSTEM)?.use { descriptor ->
            BitmapFactory.decodeFileDescriptor(descriptor.fileDescriptor)
        }
    }.onFailure {
        Log.d(
            WALLPAPER_TAG,
            "wallpaper pixels unavailable: ${it.javaClass.simpleName}: ${it.message}",
        )
    }.getOrNull()

    private fun encodePreview(source: Bitmap): Preview? {
        if (source.width <= 0 || source.height <= 0) return null
        val scale = minOf(
            1f,
            MAX_PREVIEW_WIDTH.toFloat() / source.width,
            MAX_PREVIEW_HEIGHT.toFloat() / source.height,
        )
        val width = maxOf(1, (source.width * scale).toInt())
        val height = maxOf(1, (source.height * scale).toInt())
        val scaled = if (width == source.width && height == source.height) source
        else Bitmap.createScaledBitmap(source, width, height, true)
        try {
            for (quality in intArrayOf(84, 76, 68, 60, 52)) {
                val out = ByteArrayOutputStream(MAX_PREVIEW_BYTES)
                if (!scaled.compress(Bitmap.CompressFormat.JPEG, quality, out)) continue
                val bytes = out.toByteArray()
                if (bytes.isNotEmpty() && bytes.size <= MAX_PREVIEW_BYTES) {
                    return Preview(bytes, MessageDigest.getInstance("SHA-256").digest(bytes))
                }
            }
        } finally {
            if (scaled !== source) scaled.recycle()
        }
        Log.w(WALLPAPER_TAG, "wallpaper preview remained over $MAX_PREVIEW_BYTES B")
        return null
    }
}
