package com.conduit.sync

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import java.io.File

/**
 * Serves the one cached clipboard image over `content://`.
 *
 * Android's clipboard carries a URI rather than bytes, and the app pasting it has to be
 * able to open that URI. `ClipboardManager` grants the reader temporary permission to
 * whatever the clip points at, but only for a provider that allows it — hence
 * `grantUriPermissions` in the manifest, and `exported="false"`, so a grant from the
 * clipboard is the *only* way in.
 *
 * Hand-written rather than androidx `FileProvider`: this serves exactly one file from
 * `cacheDir` and needs neither a paths XML nor a dependency to do it. The read side is
 * all that is implemented, because the clipboard only ever reads.
 */
class ImageProvider : ContentProvider() {

    override fun onCreate() = true

    /**
     * Resolves [uri] inside `cacheDir` and refuses anything that escapes it.
     *
     * The path is attacker-controlled in principle — a grant leaks one URI, and a
     * consumer could try walking it — so the canonical path is compared rather than the
     * string, which is what stops `..` from reaching the rest of the app's data.
     */
    private fun resolve(uri: Uri): File? {
        val cache = context?.cacheDir?.canonicalFile ?: return null
        val name = uri.lastPathSegment ?: return null
        val file = File(cache, name).canonicalFile
        return file.takeIf { it.parentFile == cache && it.isFile }
    }

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor? {
        // Read-only regardless of what was asked for: nothing needs to write here.
        val file = resolve(uri) ?: return null
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun getType(uri: Uri): String = "image/png"

    /**
     * Name and size. Plenty of paste targets query these before opening the stream, and
     * a provider that returns nothing here looks to them like an empty image.
     */
    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor? {
        val file = resolve(uri) ?: return null
        val columns = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        val values = columns.map { column ->
            when (column) {
                OpenableColumns.DISPLAY_NAME -> file.name
                OpenableColumns.SIZE -> file.length()
                else -> null
            }
        }
        return MatrixCursor(columns, 1).apply { addRow(values) }
    }

    override fun insert(uri: Uri, values: ContentValues?): Uri? = null

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ) = 0

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?) = 0

    companion object {
        /** Must match `android:authorities` in the manifest. */
        const val AUTHORITY = "com.conduit.sync.images"

        fun uriFor(file: File): Uri = Uri.Builder()
            .scheme("content")
            .authority(AUTHORITY)
            .appendPath(file.name)
            .build()

        /**
         * Whether [uri] is one of ours.
         *
         * This is image echo suppression, and it is exact rather than heuristic: setting
         * the clipboard fires our own listener, and the clip we get back points here.
         * Comparing authorities beats comparing bytes, which would mean reading the file
         * back on the main thread to discover we wrote it.
         */
        fun isOurs(uri: Uri?) = uri?.authority == AUTHORITY
    }
}
