package com.conduit.sync

import com.conduit.sync.proto.ClipImageChunk
import com.conduit.sync.proto.ClipImageHeader
import com.google.protobuf.ByteString
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test

/**
 * Every field of a header and a chunk is chosen by the peer, so this is the boundary
 * where a hostile or broken desktop could make the phone allocate what it likes. Plain
 * JVM test: [Images.Assembly] deliberately touches no Android API.
 */
class ImageAssemblyTest {

    @Test
    fun `incremental image sending allows controls between frames and uses a fresh id`() {
        val payload = Images.Payload(ByteArray(32769) { 3 }, "image/png")
        val first = Images.Outgoing(payload, photo = false, screenshot = false)
        val second = Images.Outgoing(payload, photo = false, screenshot = false)
        val scheduler = TransferScheduler()
        val trace = mutableListOf<String>()
        val headers = mutableListOf<ClipImageHeader>()
        val chunks = mutableListOf<ClipImageChunk>()
        val send: (com.conduit.sync.proto.Kind, ByteArray) -> Unit = { kind, bytes ->
            trace += kind.name
            if (kind == com.conduit.sync.proto.Kind.CLIP_IMAGE_HEADER) headers += ClipImageHeader.parseFrom(bytes)
            else chunks += ClipImageChunk.parseFrom(bytes)
        }
        scheduler.bulk { !first.step(send) }
        scheduler.bulk { !second.step(send) }
        scheduler.step()
        scheduler.control(priority = true) { trace += "cancel-other-file" }
        scheduler.step()
        while (scheduler.step()) { }
        org.junit.Assert.assertEquals("cancel-other-file", trace[1])
        org.junit.Assert.assertEquals(2, headers.size)
        org.junit.Assert.assertNotEquals(headers[0].headerId, headers[1].headerId)
        org.junit.Assert.assertEquals(listOf(32768, 1, 32768, 1), chunks.map { it.data.size() })
        assertArrayEquals(payload.bytes, chunks[0].data.toByteArray() + chunks[1].data.toByteArray())
    }

    private val id = ByteString.copyFrom(ByteArray(16) { 7 })
    private val chunkSize = 32 * 1024

    private fun header(
        total: Int,
        chunk: Int = chunkSize,
        count: Int = (total + chunk - 1) / chunk,
        photo: Boolean = false,
        screenshot: Boolean = false,
        headerId: ByteString = id,
    ): ClipImageHeader = ClipImageHeader.newBuilder()
        .setMime("image/png")
        .setTotalBytes(total)
        .setChunkSize(chunk)
        .setChunkCount(count)
        .setHeaderId(headerId)
        .setPhoto(photo)
        .setScreenshot(screenshot)
        .build()

    private fun chunk(index: Int, data: ByteArray, headerId: ByteString = id): ClipImageChunk =
        ClipImageChunk.newBuilder()
            .setIndex(index)
            .setData(ByteString.copyFrom(data))
            .setHeaderId(headerId)
            .setStreamId(1)
            .build()

    @Test
    fun `chunks reassemble to exactly what was sent`() {
        val payload = ByteArray(chunkSize + 100) { it.toByte() }
        val assembly = Images.Assembly.begin(header(payload.size))

        var out: ByteArray? = null
        var index = 0
        var from = 0
        while (from < payload.size) {
            val to = minOf(from + chunkSize, payload.size)
            out = assembly.push(chunk(index, payload.copyOfRange(from, to)))
            // Nothing may be handed back before the final chunk, or a truncated image
            // would reach the clipboard looking complete.
            if (to < payload.size) assertNull(out)
            index++
            from = to
        }
        assertArrayEquals(payload, out)
    }

    @Test
    fun `a header that would allocate too much is refused`() {
        // The point of validating before the buffer is sized: a peer claiming more than
        // the ceiling must not be able to make this process ask for it.
        assertThrows(IllegalArgumentException::class.java) {
            Images.Assembly.begin(header(MAX_IMAGE + 1, count = 999))
        }
        assertThrows(IllegalArgumentException::class.java) {
            Images.Assembly.begin(header(0, count = 0))
        }
        assertThrows(IllegalArgumentException::class.java) {
            Images.Assembly.begin(header(100, chunk = 0, count = 1))
        }
        // A chunk larger than the ceiling could never have been sent in one frame.
        assertThrows(IllegalArgumentException::class.java) {
            Images.Assembly.begin(header(100, chunk = chunkSize * 2, count = 1))
        }
        // chunk_count must agree with the arithmetic, or the last chunk never arrives
        // and the buffer is held for the life of the session.
        assertThrows(IllegalArgumentException::class.java) {
            Images.Assembly.begin(header(chunkSize * 2, count = 1))
        }
    }

    @Test
    fun `a peer cannot exceed reorder or truncate what it declared`() {
        assertThrows(IllegalArgumentException::class.java) {
            Images.Assembly.begin(header(10)).push(chunk(0, ByteArray(11)))
        }
        assertThrows(IllegalArgumentException::class.java) {
            Images.Assembly.begin(header(10)).push(chunk(1, ByteArray(10)))
        }
        assertThrows(IllegalArgumentException::class.java) {
            val stranger = ByteString.copyFrom(ByteArray(16) { 9 })
            Images.Assembly.begin(header(10)).push(chunk(0, ByteArray(10), stranger))
        }
        // The chunk count is satisfied but the image is short, so it must not be handed
        // over as a valid PNG.
        assertThrows(IllegalArgumentException::class.java) {
            Images.Assembly.begin(header(10)).push(chunk(0, ByteArray(9)))
        }
    }

    @Test
    fun `the capture flags survive the transfer`() {
        // They decide whether the image lands on the clipboard or becomes a capture toast,
        // so losing either can silently overwrite whatever the user had copied.
        val assembly = Images.Assembly.begin(header(4, photo = true))
        assertArrayEquals(byteArrayOf(1, 2, 3, 4), assembly.push(chunk(0, byteArrayOf(1, 2, 3, 4))))
        org.junit.Assert.assertTrue(assembly.photo)
        org.junit.Assert.assertFalse(Images.Assembly.begin(header(4)).photo)

        val screenshot = Images.Assembly.begin(header(4, photo = true, screenshot = true))
        org.junit.Assert.assertTrue(screenshot.photo)
        org.junit.Assert.assertTrue(screenshot.screenshot)
        org.junit.Assert.assertFalse(Images.Assembly.begin(header(4)).screenshot)
    }
}
