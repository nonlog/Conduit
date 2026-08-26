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
import com.conduit.sync.proto.Kind
import com.google.protobuf.ByteString
import java.io.InputStream
import java.io.OutputStream
import java.security.MessageDigest

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
 * Nothing here holds a file. [send] reads one 32 KiB buffer at a time and writes each one
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

    /**
     * Writes one offer followed by its chunks, on the caller's thread.
     *
     * Takes the session rather than the [Link] for the same reason [Images.send] does:
     * routing thousands of frames through `Link.send` would queue them onto a bounded,
     * discard-oldest queue and deliver a file with holes in it.
     *
     * Throws if the stream runs out early. That is a real failure — the provider declared a
     * size it could not deliver — and the receiver is already committed to a total, so there
     * is nothing to do but let [Link] tear the session down. The desktop's transfer dies
     * with it and deletes its own partial file.
     */
    fun send(
        session: WireSession,
        input: InputStream,
        meta: Meta,
        onProgress: (transferred: Long, total: Long) -> Unit = { _, _ -> },
    ) {
        val id = ByteString.copyFrom(
            MessageDigest.getInstance("SHA-256")
                .digest("${meta.name}:${meta.size}:${System.currentTimeMillis()}".toByteArray()),
            0,
            16,
        )
        val count = chunkCount(meta.size)
        val offer = FileOffer.newBuilder()
            .setName(meta.name)
            .setMime(meta.mime)
            .setTotalBytes(meta.size)
            .setChunkSize(CHUNK)
            .setChunkCount(count)
            .setTransferId(id)
            .setTimestampMs(System.currentTimeMillis())
            .build()
        onProgress(0L, meta.size)
        session.send(Kind.FILE_OFFER, offer.toByteArray())

        val buffer = ByteArray(CHUNK)
        var sent = 0L
        var index = 0L
        while (sent < meta.size) {
            val want = minOf(CHUNK.toLong(), meta.size - sent).toInt()
            // Filled completely, not just read once: a short read mid-file would make every
            // later chunk index disagree with the count the offer declared, and the receiver
            // would sit waiting for a chunk that never comes.
            val got = fill(input, buffer, want)
            require(got == want) {
                "$sent B in, ${meta.name} gave $got B where $want was declared"
            }
            val chunk = FileChunk.newBuilder()
                .setIndex(index)
                .setData(ByteString.copyFrom(buffer, 0, got))
                .setTransferId(id)
                .build()
            session.send(Kind.FILE_CHUNK, chunk.toByteArray())
            sent += got
            index++
            onProgress(sent, meta.size)
        }
        Log.i(TAG, "sent ${meta.name}, ${meta.size} B as $count chunks")
    }

    /** [InputStream.read] may return short for any reason; this one only stops at EOF. */
    private fun fill(input: InputStream, buffer: ByteArray, want: Int): Int {
        var got = 0
        while (got < want) {
            val n = input.read(buffer, got, want - got)
            if (n < 0) break
            got += n
        }
        return got
    }

    /**
     * The chunk count the desktop will check the offer against.
     *
     * Exists so the arithmetic can be tested without a session or a file: get it wrong and
     * every transfer is refused by the receiver with nothing on this side to show why.
     */
    internal fun chunkCount(size: Long): Long = (size + CHUNK - 1) / CHUNK

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
        private val id: ByteArray,
        private val total: Long,
        private val chunks: Long,
    ) : AutoCloseable {
        private var next = 0L
        private var written = 0L
        private var published = false

        data class Progress(
            val transferred: Long,
            val total: Long,
            val complete: Boolean,
        )

        /** Writes one in-order chunk and reports exact byte progress. */
        fun push(chunk: FileChunk): Progress {
            require(chunk.transferId.toByteArray().contentEquals(id)) {
                "chunk belongs to a different transfer"
            }
            require(chunk.index == next) { "chunk ${chunk.index} arrived, expected $next" }
            val bytes = chunk.data.toByteArray()
            require(written + bytes.size <= total) {
                "chunk ${chunk.index} would take the file past the $total B it declared"
            }
            val sink = requireNotNull(output) { "transfer already finished" }
            sink.write(bytes)
            next++
            written += bytes.size
            if (next < chunks) return Progress(written, total, complete = false)

            require(written == total) { "file ended at $written B, offer said $total" }
            sink.flush()
            sink.close()
            output = null
            val values = ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }
            require(resolver.update(uri, values, null, null) == 1) {
                "Downloads provider did not publish $name"
            }
            published = true
            Log.i(TAG, "received $name, $written B as $chunks chunks")
            return Progress(written, total, complete = true)
        }

        override fun close() {
            runCatching { output?.close() }
            output = null
            if (!published) {
                runCatching { resolver.delete(uri, null, null) }
                Log.w(TAG, "incoming $name abandoned at $written/$total B, deleted pending row")
            }
        }

        companion object {
            /** Validates [offer] before creating a MediaStore row. */
            fun begin(resolver: ContentResolver, offer: FileOffer): Incoming {
                require(offer.totalBytes in 1..MAX_FILE) {
                    "file of ${offer.totalBytes} B is outside 1..$MAX_FILE"
                }
                require(offer.chunkSize in 1..CHUNK) {
                    "implausible chunk size ${offer.chunkSize}"
                }
                val expect = chunkCount(offer.totalBytes)
                require(offer.chunkCount == expect) {
                    "offer claims ${offer.chunkCount} chunks, ${offer.totalBytes} B needs $expect"
                }
                require(!offer.transferId.isEmpty) { "offer has no transfer id" }

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
