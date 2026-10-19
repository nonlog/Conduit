package com.conduit.sync

import com.conduit.sync.proto.FileChunk
import com.conduit.sync.proto.FileOffer
import com.google.protobuf.ByteString
import java.io.ByteArrayInputStream
import java.io.InputStream
import org.junit.Assert.*
import org.junit.Test

class FileStreamingTest {
    private val id = ByteString.copyFrom(ByteArray(16) { 5 })
    private fun offer(size: Long = 5, chunk: Int = 3) = FileOffer.newBuilder()
        .setName("test.bin").setTotalBytes(size).setChunkSize(chunk)
        .setChunkCount(Files.chunkCount(size, chunk)).setTransferId(id).build()
    private fun chunk(index: Long, size: Int, transferId: ByteString = id) = FileChunk.newBuilder()
        .setIndex(index).setData(ByteString.copyFrom(ByteArray(size))).setTransferId(transferId).build()

    @Test fun sourceShortReadsAndZeroReadsStillFillExactWireChunks() {
        val bytes = ByteArray(32769) { it.toByte() }
        val base = ByteArrayInputStream(bytes)
        val source = object : InputStream() {
            var zero = true
            override fun read() = base.read()
            override fun read(b: ByteArray, off: Int, len: Int): Int {
                zero = !zero
                return if (!zero) 0 else base.read(b, off, minOf(7, len))
            }
        }
        Files.Outgoing(id, source, Files.Meta("x", "application/octet-stream", bytes.size.toLong())).use {
            assertEquals(2L, it.offer.chunkCount)
            val first = it.next()
            val last = it.next()
            assertEquals(0L, first.index)
            assertEquals(1L, last.index)
            assertEquals(id, first.transferId)
            assertArrayEquals(bytes, first.data.toByteArray() + last.data.toByteArray())
            assertEquals(bytes.size.toLong(), it.sent)
        }
    }

    @Test fun aSourceShorterThanItsDeclaredSizeFailsWithoutEmittingTheShortChunk() {
        Files.Outgoing(id, ByteArrayInputStream(ByteArray(3)), Files.Meta("short", "x", 4)).use {
            assertThrows(IllegalArgumentException::class.java) { it.next() }
            assertEquals(0L, it.sent)
        }
        val buffer = ByteArray(3)
        assertEquals(2, Files.fill(ByteArrayInputStream(byteArrayOf(1, 2)), buffer, 3))
    }

    @Test fun invalidOrUnboundedSourcesAreRefusedBeforeReading() {
        val source = object : InputStream() { override fun read(): Int = error("must not read") }
        for (size in listOf(-1L, 0L, MAX_FILE + 1, Long.MAX_VALUE)) {
            assertThrows(IllegalArgumentException::class.java) {
                Files.Outgoing(id, source, Files.Meta("bad", "x", size))
            }
        }
    }

    @Test fun receiveArithmeticUsesTheAdvertisedChunkSizeAndWrongIdsDoNotPoisonIt() {
        val receiver = FileReceiveCursor(offer())
        val other = ByteString.copyFrom(ByteArray(16) { 6 })
        assertFalse(receiver.accept(chunk(0, 3, other)))
        assertEquals(0L, receiver.written)
        assertTrue(receiver.accept(chunk(0, 3)))
        assertFalse(receiver.complete)
        assertFalse(receiver.accept(chunk(99, 3, other)))
        assertTrue(receiver.accept(chunk(1, 2)))
        assertTrue(receiver.complete)
        assertEquals(5L, receiver.written)
    }

    @Test fun malformedAndDuplicateChunksAreRejectedBeforePublication() {
        assertThrows(IllegalArgumentException::class.java) {
            FileReceiveCursor(offer().toBuilder().setChunkCount(1).build())
        }
        val receiver = FileReceiveCursor(offer())
        assertThrows(IllegalArgumentException::class.java) { receiver.accept(chunk(0, 2)) }
        assertThrows(IllegalArgumentException::class.java) { receiver.accept(chunk(1, 3)) }
        assertTrue(receiver.accept(chunk(0, 3)))
        assertThrows(IllegalArgumentException::class.java) { receiver.accept(chunk(0, 3)) }
        assertTrue(receiver.accept(chunk(1, 2)))
        assertThrows(IllegalArgumentException::class.java) { receiver.accept(chunk(2, 0)) }
    }
}
