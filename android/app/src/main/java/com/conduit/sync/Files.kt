package com.conduit.sync

import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.util.Log
import com.conduit.sync.proto.FileChunk
import com.conduit.sync.proto.FileOffer
import com.google.protobuf.ByteString
import java.io.InputStream
import java.io.OutputStream

private const val TAG = "conduit.file"

/**
 * 32 KiB, matching the desktop. A larger chunk plus protobuf framing overflows the
 * 65519-byte Noise plaintext ceiling, and [WireSession.send] throws on an oversized frame —
 * which [Link] turns into a teardown of the session carrying the clipboard.
 */
private const val CHUNK = 32 * 1024

/** Mirrors `MAX_FILE` in the daemon's `file.rs`. Refused here so nothing is read at all. */
const val MAX_FILE = 512L * 1024 * 1024

/**
 * Files going to the desktop.
 *
 * [Outgoing] reads one 32 KiB buffer at a time and writes each one
 * out as a frame, so sharing a 400 MB video costs this process the buffer and not the
 * video — the same reason the desktop writes each chunk straight to disk.
 *
 * No thread of its own: [Link]'s sender thread is the only caller, which is also the only
 * thread allowed to block on a binder call into the app that owns the URI.
 */
object Files {

    /** What the provider says about a URI, resolved before a byte is read. */
    class Meta(val name: String, val mime: String, val size: Long)

    /**
     * An open file and what is known about it.
     *
     * The two are produced together on purpose. Both the size query and the stream open are
     * binder calls into whichever app owns the URI, so both belong on [Link]'s sender thread
     * rather than on the main thread the share intent arrives on.
     */
    class Source(val meta: Meta, val stream: java.io.InputStream)

    /** Resolves and opens [uri], or null if it cannot be offered. Closes nothing. */
    fun open(context: Context, uri: Uri): Source? {
        val meta = meta(context, uri) ?: return null
        val stream = runCatching { context.contentResolver.openInputStream(uri) }
            .onFailure { Log.w(TAG, "could not open $uri", it) }
            .getOrNull() ?: return null
        return Source(meta, stream)
    }

    /**
     * Asks the provider for a name and a size, or null if it will not say how big the file
     * is.
     *
     * The size has to be known up front: [FileOffer] declares a total and a chunk count,
     * and the receiver refuses an offer whose arithmetic does not hold. A provider that
     * reports no size is rare and the honest answer is to decline rather than to buffer the
     * whole stream to measure it.
     */
    fun meta(context: Context, uri: Uri): Meta? {
        val resolver = context.contentResolver
        var name: String? = null
        var size = -1L
        runCatching {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)
                ?.use { cursor: Cursor ->
                    if (cursor.moveToFirst()) {
                        val nameAt = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                        val sizeAt = cursor.getColumnIndex(OpenableColumns.SIZE)
                        if (nameAt >= 0 && !cursor.isNull(nameAt)) name = cursor.getString(nameAt)
                        if (sizeAt >= 0 && !cursor.isNull(sizeAt)) size = cursor.getLong(sizeAt)
                    }
                }
        }.onFailure { Log.w(TAG, "could not query $uri", it) }

        // Not every provider implements OpenableColumns; a file descriptor knows its own
        // length, and for a `file://` URI that is the only thing that does.
        if (size < 0) {
            size = runCatching {
                resolver.openAssetFileDescriptor(uri, "r")?.use { it.length }
            }.getOrNull() ?: -1L
        }
        if (size < 0) {
            Log.w(TAG, "$uri reports no size, so it cannot be offered")
            return null
        }
        if (size == 0L) {
            Log.w(TAG, "$uri is empty, skipped")
            return null
        }
        if (size > MAX_FILE) {
            Log.w(TAG, "$uri is $size B, past the $MAX_FILE B ceiling")
            return null
        }
        return Meta(
            // The last path segment is a poor name but a better one than nothing, and the
            // desktop sanitises whatever arrives regardless.
            name = name ?: uri.lastPathSegment ?: "file",
            mime = resolver.getType(uri) ?: "application/octet-stream",
            size = size,
        )
    }

    /** Reads at most one wire chunk per scheduler turn, with a single fixed-size buffer. */
    class Outgoing(val id: ByteString, private val input: InputStream, val meta: Meta) : AutoCloseable {
        private val buffer = ByteArray(CHUNK)
        var sent = 0L
            private set
        private var index = 0L
        val offer: FileOffer

        init {
            require(id.size() == 16) { "invalid transfer id" }
            require(meta.size in 1..MAX_FILE) { "file is empty, unknown-size or over 512 MB" }
            offer = FileOffer.newBuilder()
                .setName(meta.name.take(200)).setMime(meta.mime.take(200))
                .setTotalBytes(meta.size).setChunkSize(CHUNK).setChunkCount(chunkCount(meta.size))
                .setTransferId(id).setTimestampMs(System.currentTimeMillis()).build()
        }

        fun next(): FileChunk {
            check(sent < meta.size) { "source already finished" }
            val want = minOf(CHUNK.toLong(), meta.size - sent).toInt()
            val got = fill(input, buffer, want)
            require(got == want) { "Source ended at ${sent + got} B; expected ${meta.size} B" }
            val chunk = FileChunk.newBuilder().setIndex(index)
                .setData(ByteString.copyFrom(buffer, 0, got)).setTransferId(id).build()
            sent += got
            index++
            return chunk
        }

        override fun close() = input.close()
    }

    /** [InputStream.read] may return short for any reason; this one only stops at EOF. */
    internal fun fill(input: InputStream, buffer: ByteArray, want: Int): Int {
        require(want in 0..buffer.size)
        var got = 0
        while (got < want) {
            val n = input.read(buffer, got, want - got)
            if (n < 0) break
            if (n == 0) {
                // Broken providers sometimes return zero; make progress without spinning.
                val byte = input.read()
                if (byte < 0) break
                buffer[got++] = byte.toByte()
            } else got += n
        }
        return got
    }

    /**
     * The chunk count the desktop will check the offer against.
     *
     * Exists so the arithmetic can be tested without a session or a file: get it wrong and
     * every transfer is refused by the receiver with nothing on this side to show why.
     */
    internal fun chunkCount(size: Long, chunk: Int = CHUNK): Long {
        require(size in 1..MAX_FILE && chunk in 1..CHUNK)
        return (size - 1) / chunk + 1
    }

    /**
     * One file arriving from the desktop.
     *
     * Android's Downloads MediaStore gives us the same transaction shape the Windows receiver
     * has with its `.part` file: insert with `IS_PENDING=1`, stream chunks straight into the
     * provider, then publish only after the declared byte/chunk counts are exact. [close] deletes
     * an unpublished row, so a dead Noise session cannot leave a half-file visible in Downloads.
     */
    class Incoming private constructor(
        private val resolver: ContentResolver,
        private val uri: Uri,
        private var output: OutputStream?,
        val name: String,
        val id: ByteArray,
        val total: Long,
        private val chunks: Long,
        private val cursor: FileReceiveCursor,
    ) : AutoCloseable {
        private var published = false
        val transferred: Long get() = cursor.written

        data class Progress(
            val transferred: Long,
            val total: Long,
            val complete: Boolean,
        )

        /** Writes one in-order chunk and reports exact byte progress. */
        fun push(chunk: FileChunk): Progress {
            if (!cursor.accept(chunk)) return Progress(transferred, total, complete = false)
            val bytes = chunk.data.toByteArray()
            val sink = requireNotNull(output) { "transfer already finished" }
            sink.write(bytes)
            if (!cursor.complete) return Progress(transferred, total, complete = false)

            sink.flush()
            sink.close()
            output = null
            val values = ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }
            require(resolver.update(uri, values, null, null) == 1) {
                "Downloads provider did not publish $name"
            }
            published = true
            Log.i(TAG, "received $name, $transferred B as $chunks chunks")
            return Progress(transferred, total, complete = true)
        }

        override fun close() {
            runCatching { output?.close() }
            output = null
            if (!published) {
                runCatching { resolver.delete(uri, null, null) }
                Log.w(TAG, "incoming $name abandoned at $transferred/$total B, deleted pending row")
            }
        }

        companion object {
            /** Validates [offer] before creating a MediaStore row. */
            fun begin(resolver: ContentResolver, offer: FileOffer): Incoming {
                val cursor = FileReceiveCursor(offer)

                val name = safeName(offer.name)
                val values = ContentValues().apply {
                    put(MediaStore.Downloads.DISPLAY_NAME, name)
                    put(
                        MediaStore.Downloads.MIME_TYPE,
                        offer.mime.takeIf { it.isNotBlank() } ?: "application/octet-stream",
                    )
                    put(
                        MediaStore.Downloads.RELATIVE_PATH,
                        Environment.DIRECTORY_DOWNLOADS + "/",
                    )
                    put(MediaStore.Downloads.IS_PENDING, 1)
                }
                val uri = requireNotNull(
                    resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values),
                ) { "Downloads provider refused $name" }
                val stream = try {
                    requireNotNull(resolver.openOutputStream(uri, "w")) {
                        "Downloads provider did not open $uri"
                    }
                } catch (t: Throwable) {
                    runCatching { resolver.delete(uri, null, null) }
                    throw t
                }
                return Incoming(
                    resolver = resolver,
                    uri = uri,
                    output = stream,
                    name = name,
                    id = offer.transferId.toByteArray(),
                    total = offer.totalBytes,
                    chunks = offer.chunkCount,
                    cursor = cursor,
                )
            }
        }
    }

    /** A peer-supplied basename suitable for MediaStore Downloads. */
    internal fun safeName(name: String): String {
        val base = name.substringAfterLast('/').substringAfterLast('\\')
        val cleaned = buildString {
            for (c in base) {
                append(
                    when {
                        c.code < 0x20 || c in "<>:\"/\\|?*" -> '_'
                        else -> c
                    },
                )
            }
        }.trim().trimEnd('.', ' ')
        if (cleaned.isEmpty() || cleaned == "." || cleaned == "..") return "file"

        val dot = cleaned.lastIndexOf('.')
        val stem = if (dot > 0) cleaned.substring(0, dot) else cleaned
        val ext = if (dot > 0) cleaned.substring(dot) else ""
        return stem.take(120) + ext.take(80)
    }
}
