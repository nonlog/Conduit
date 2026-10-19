package com.conduit.sync

import com.conduit.sync.proto.FileChunk
import com.conduit.sync.proto.FileOffer

/** Pure receive boundary: rejects arithmetic/order/size errors before writing to Downloads. */
internal class FileReceiveCursor(private val offer: FileOffer) {
    var written = 0L
        private set
    private var next = 0L
    val complete: Boolean get() = next == offer.chunkCount && written == offer.totalBytes

    init {
        require(offer.totalBytes in 1..MAX_FILE) { "File size is outside 1..$MAX_FILE" }
        require(offer.chunkSize in 1..32768) { "Invalid chunk size" }
        require(offer.chunkCount == Files.chunkCount(offer.totalBytes, offer.chunkSize)) { "Invalid chunk count" }
        require(offer.transferId.size() in 1..64) { "Invalid transfer id" }
    }

    fun matches(chunk: FileChunk): Boolean = chunk.transferId == offer.transferId

    /** Wrong IDs leave the active cursor unchanged, including after a targeted cancellation. */
    fun accept(chunk: FileChunk): Boolean {
        if (!matches(chunk)) return false
        require(next < offer.chunkCount && chunk.index == next) { "Unexpected chunk index" }
        require(chunk.data.size() == minOf(offer.chunkSize.toLong(), offer.totalBytes - written).toInt()) {
            "Invalid chunk length"
        }
        written += chunk.data.size()
        next++
        return true
    }
}
