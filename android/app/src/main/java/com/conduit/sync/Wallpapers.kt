package com.conduit.sync

import android.app.WallpaperManager
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.Log
import java.io.ByteArrayOutputStream
import java.security.MessageDigest
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

private const val WALLPAPER_TAG = "conduit.wallpaper"
private const val MAX_PREVIEW_BYTES = 56 * 1024
private const val MAX_PREVIEW_WIDTH = 180
private const val MAX_PREVIEW_HEIGHT = 400
private const val ROOT_READ_TIMEOUT_MS = 2_500L
private const val MAX_SOURCE_BYTES = 8 * 1024 * 1024

/**
 * Produces one small cached home-wallpaper preview for the Windows phone frame.
 *
 * There is deliberately no poller. The service asks once after an authenticated session and
 * invalidates this cache only from WallpaperManager's existing change callback. On Android builds
 * that still expose WallpaperManager#getWallpaperFile we use it directly. New Android releases
 * hide the bitmap from ordinary apps, so a rooted device falls back to the stable system wallpaper
 * files. Root is optional: failure simply means the desktop keeps its previous/fallback preview.
 */
object Wallpapers {
    class Preview(val jpeg: ByteArray, val sha256: ByteArray)

    private val rootSources = listOf(
        "/data/system/users/0/wallpaper_screenshot.png",
        "/data/system/users/0/wallpaper",
        "/data/system/users/0/wallpaper_orig",
    )

    @Volatile private var cached: Preview? = null

    fun invalidate() {
        cached = null
    }

    fun preview(context: Context, force: Boolean = false): Preview? {
        if (!force) cached?.let { return it }
        val bitmap = loadPlatformWallpaper(context) ?: loadRootWallpaper() ?: return null
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
        Log.d(WALLPAPER_TAG, "platform wallpaper pixels unavailable: ${it.javaClass.simpleName}")
    }.getOrNull()

    /**
     * Reads a constant system path through KernelSU/Magisk without ever placing wallpaper bytes in
     * a shell argument. The temporary executor exists only for the one bounded read: it gives a
     * root manager that is waiting for approval a hard timeout without adding a resident thread.
     */
    private fun loadRootWallpaper(): Bitmap? {
        for (path in rootSources) {
            val bytes = readRoot(path) ?: continue
            val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
            if (bitmap != null) {
                Log.i(WALLPAPER_TAG, "loaded system wallpaper source $path")
                return bitmap
            }
        }
        Log.i(WALLPAPER_TAG, "no readable system wallpaper source")
        return null
    }

    private fun readRoot(path: String): ByteArray? {
        val process = runCatching { ProcessBuilder("su", "-c", "cat $path").start() }.getOrNull() ?: return null
        val executor = Executors.newSingleThreadExecutor { r -> Thread(r, "conduit-wallpaper-read") }
        return try {
            val future = executor.submit<ByteArray?> {
                process.inputStream.use { input ->
                    val out = ByteArrayOutputStream()
                    val buffer = ByteArray(16 * 1024)
                    while (true) {
                        val n = input.read(buffer)
                        if (n <= 0) break
                        if (out.size() + n > MAX_SOURCE_BYTES) return@submit null
                        out.write(buffer, 0, n)
                    }
                    out.toByteArray().takeIf { it.isNotEmpty() }
                }
            }
            val bytes = future.get(ROOT_READ_TIMEOUT_MS, TimeUnit.MILLISECONDS)
            if (!process.waitFor(300, TimeUnit.MILLISECONDS) || process.exitValue() != 0) null else bytes
        } catch (_: Exception) {
            null
        } finally {
            process.destroyForcibly()
            executor.shutdownNow()
        }
    }

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
