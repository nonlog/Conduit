package com.conduit.sync

import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentResolver
import android.content.Context
import android.net.Uri
import android.util.Log
import com.conduit.sync.proto.ClipImageChunk
import com.conduit.sync.proto.ClipImageHeader
import com.conduit.sync.proto.Kind
import com.google.protobuf.ByteString
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileOutputStream
import java.security.MessageDigest

private const val TAG = "conduit.img"

/**
 * 32 KiB, matching the desktop. A 64 KiB chunk plus its protobuf framing overflows the
 * 65519-byte Noise plaintext ceiling, and [WireSession.send] refuses an oversized frame —
 * which would tear the session down over a pasted screenshot.
 */
private const val CHUNK = 32 * 1024

/**
 * The ceiling on one image, matching `MAX_IMAGE` on the desktop. A clipboard is not a file
 * transfer; this is a generous phone screenshot and a cheap thing to refuse before
 * allocating anything for it.
 */
const val MAX_IMAGE = 10 * 1024 * 1024

/**
 * Clipboard images, both directions.
 *
 * PNG on the wire in both directions, so nothing here has to negotiate a format. Android
 * hands over a `content://` URI whose bytes are usually already PNG or JPEG, and JPEG is
 * re-encoded rather than sent as-is: Windows apps that read the registered PNG clipboard
 * format expect a PNG, and one decode on the sending side is cheaper than teaching the
 * desktop to sniff.
 *
 * No thread of its own. Sending runs on [Link]'s sender thread and receiving on its
 * reader thread, which is what keeps this feature from adding to the idle cost.
 */
object Images {

    /** Image bytes with the MIME type the provider reported for them. */
    class Payload(val bytes: ByteArray, val mime: String)

    /**
     * Reads the first image on the clipboard, or null if there is none.
     *
     * The [ContentResolver] call is why this belongs on a worker: opening a `content://`
     * URI crosses a binder to whichever app owns the provider, and that app may be cold.
     */
    fun fromClipboard(context: Context, clip: ClipData): Payload? {
        val item = clip.takeIf { it.itemCount > 0 }?.getItemAt(0) ?: return null
        val uri = item.uri ?: return null
        val mime = context.contentResolver.getType(uri)
        if (mime == null || !mime.startsWith("image/")) {
            Log.d(TAG, "clipboard URI is $mime, not an image")
            return null
        }
        return read(context, uri, mime)
    }

    /**
     * Reads [uri] as-is, without re-encoding.
     *
     * Deliberately no conversion here. Re-encoding a camera JPEG as PNG on the phone
     * costs a full decode and encode on a battery, and a 4 MB photo can come back as
     * 20 MB of PNG — past the ceiling, so the transfer would be refused after all that
     * work. The desktop decodes whatever arrives anyway, so it normalises instead.
     */
    fun read(context: Context, uri: Uri, mime: String): Payload? {
        val bytes = runCatching {
            context.contentResolver.openInputStream(uri)?.use { input ->
                // Bounded read: a provider can report any size it likes, or none, so the
                // ceiling is enforced against the bytes actually delivered.
                val out = ByteArrayOutputStream()
                val buffer = ByteArray(CHUNK)
                while (true) {
                    val n = input.read(buffer)
                    if (n <= 0) break
                    if (out.size() + n > MAX_IMAGE) {
                        Log.w(TAG, "image over $MAX_IMAGE B, skipped")
                        return null
                    }
                    out.write(buffer, 0, n)
                }
                out.toByteArray()
            }
        }.onFailure { Log.w(TAG, "could not read $uri", it) }.getOrNull() ?: return null

        return if (bytes.isEmpty()) null else Payload(bytes, mime)
    }

    /**
     * Writes one PNG as a header followed by chunks, on the caller's thread.
     *
     * Takes the session rather than the [Link] on purpose. Routing each frame through
     * `Link.send` would queue several hundred tasks onto a bounded, discard-oldest queue
     * and quietly drop most of a large image; the caller is already the sender thread, so
     * the frames are written straight out, in order, as one unit of work.
     */
    fun send(session: WireSession, payload: Payload, photo: Boolean) {
        val bytes = payload.bytes
        val id = ByteString.copyFrom(MessageDigest.getInstance("SHA-256").digest(bytes), 0, 16)
        val count = (bytes.size + CHUNK - 1) / CHUNK

        val header = ClipImageHeader.newBuilder()
            .setMime(payload.mime)
            .setTotalBytes(bytes.size)
            .setChunkSize(CHUNK)
            .setChunkCount(count)
            .setTimestampMs(System.currentTimeMillis())
            .setHeaderId(id)
            .setPhoto(photo)
            .build()
        session.send(Kind.CLIP_IMAGE_HEADER, header.toByteArray())

        for (index in 0 until count) {
            val from = index * CHUNK
            val to = minOf(from + CHUNK, bytes.size)
            val chunk = ClipImageChunk.newBuilder()
                .setIndex(index)
                .setData(ByteString.copyFrom(bytes, from, to - from))
                .setHeaderId(id)
                .setStreamId(1)
                .build()
            session.send(Kind.CLIP_IMAGE_CHUNK, chunk.toByteArray())
        }
        Log.i(TAG, "sent ${bytes.size} B of ${payload.mime} as $count chunks, photo=$photo")
    }

    /**
     * Puts a received PNG on the clipboard.
     *
     * Android's clipboard carries a URI, not bytes, so the image has to be written
     * somewhere a `content://` URI can point at. That is what [ImageProvider] is for.
     * One file, overwritten each time: a clipboard has no history, so neither does this.
     */
    fun toClipboard(context: Context, clipboard: ClipboardManager, png: ByteArray): Uri? {
        val file = File(context.cacheDir, "clip.png")
        return runCatching {
            FileOutputStream(file).use { it.write(png) }
            val uri = ImageProvider.uriFor(file)
            clipboard.setPrimaryClip(
                ClipData.newUri(context.contentResolver, "conduit", uri),
            )
            uri
        }.onFailure { Log.w(TAG, "could not put the image on the clipboard", it) }.getOrNull()
    }

    /**
     * One image being reassembled from chunks.
     *
     * Bounded twice over: a header claiming more than [MAX_IMAGE] is refused before
     * anything is allocated for it, and every chunk is checked against the total the
     * peer declared. Chunks must arrive in order — they travel on one TCP stream inside
     * one Noise session, so out of order is not a network condition, it is a broken or
     * hostile peer, and dropping the transfer is the right answer.
     */
    class Assembly private constructor(
        private val id: ByteString,
        /** Announced as a camera photo rather than a clipboard copy. */
        val photo: Boolean,
        private val expect: Int,
        private val total: Int,
    ) {
        private val bytes = ByteArrayOutputStream(total)
        private var next = 0

        /** Adds a chunk, returning the whole image once the last one arrives. */
        fun push(chunk: ClipImageChunk): ByteArray? {
            require(chunk.headerId == id) { "chunk belongs to a different image" }
            require(chunk.index == next) { "chunk ${chunk.index} arrived, expected $next" }
            require(bytes.size() + chunk.data.size() <= total) {
                "chunk ${chunk.index} would take the image past the $total B it declared"
            }
            chunk.data.writeTo(bytes)
            next++

            if (next < expect) return null
            require(bytes.size() == total) { "image ended at ${bytes.size()} B, header said $total" }
            return bytes.toByteArray()
        }

        companion object {
            /** Starts a transfer, or throws if the header is not self-consistent. */
            fun begin(header: ClipImageHeader): Assembly {
                val total = header.totalBytes
                require(total in 1..MAX_IMAGE) { "image of $total B is outside 1..$MAX_IMAGE" }
                val chunk = header.chunkSize
                require(chunk in 1..CHUNK) { "implausible chunk size $chunk" }
                val expect = (total + chunk - 1) / chunk
                require(header.chunkCount == expect) {
                    "header claims ${header.chunkCount} chunks, $total B in $chunk B needs $expect"
                }
                return Assembly(header.headerId, header.photo, expect, total)
            }
        }
    }
}
