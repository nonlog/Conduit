package com.conduit.sync

import com.conduit.sync.proto.Envelope
import com.conduit.sync.proto.Kind
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.InputStream
import java.io.OutputStream

/** A Noise transport message cannot exceed this, so it is the frame ceiling. */
const val MAX_FRAME = 65535

/** ChaChaPoly's 16-byte tag comes out of the same budget. */
const val MAX_PLAINTEXT = MAX_FRAME - 16

/**
 * One frame carries one Envelope: 4-byte big-endian length, then Noise ciphertext.
 * Mirrors `wire.rs` on the desktop.
 *
 * Blocking by design. The link owns exactly one thread and closing the socket is what
 * unblocks it — no interrupt flags, no polling, and nothing to leak if the thread dies.
 */
class WireSession private constructor(
    private val noise: NoiseSession,
    /** The peer's Noise static public key — the thing worth pinning. */
    val peerStatic: ByteArray,
    private val input: DataInputStream,
    private val output: DataOutputStream,
) {
    /** Per-session counter from 1, so dedup state resets with the session. */
    private var nextId = 1L

    /** Allocated once at the ceiling, so per-session memory is a constant. */
    private val frame = ByteArray(MAX_FRAME)

    fun send(kind: Kind, payload: ByteArray = EMPTY): Long {
        val id = nextId++
        val envelope = Envelope.newBuilder()
            .setMessageId(id)
            .setKind(kind)
            .setPayload(com.google.protobuf.ByteString.copyFrom(payload))
            .build()
        val plain = envelope.toByteArray()
        require(plain.size <= MAX_PLAINTEXT) {
            "$kind envelope is ${plain.size} B, ceiling is $MAX_PLAINTEXT B"
        }
        writeFrame(output, noise.writeMessage(plain))
        return id
    }

    fun recv(): Envelope {
        val length = readLength(input)
        input.readFully(frame, 0, length)
        return Envelope.parseFrom(noise.readMessage(frame.copyOf(length)))
    }

    companion object {
        private val EMPTY = ByteArray(0)

        /**
         * Noise XX. The phone dials, so it is normally the initiator; the parameter
         * exists because both ends run this same code.
         */
        fun handshake(
            rawIn: InputStream,
            rawOut: OutputStream,
            localPriv: ByteArray,
            initiator: Boolean,
        ): WireSession {
            val input = DataInputStream(rawIn)
            // Buffered so the length prefix and body leave as one segment.
            val output = DataOutputStream(BufferedOutputStream(rawOut))
            val hs = NoiseXX(initiator, KeyPair.fromPrivate(localPriv))

            var myTurn = initiator
            while (!hs.isFinished) {
                if (myTurn) {
                    writeFrame(output, hs.writeMessage())
                } else {
                    val length = readLength(input)
                    val message = ByteArray(length)
                    input.readFully(message)
                    hs.readMessage(message)
                }
                myTurn = !myTurn
            }

            val peer = requireNotNull(hs.remoteStatic) {
                "XX completed without a remote static key"
            }
            return WireSession(hs.intoTransport(), peer, input, output)
        }

        /** Rejects an impossible length *before* allocating anything for it. */
        private fun readLength(input: DataInputStream): Int {
            // Signed read, so 0xFFFFFFFF arrives as -1 and is refused by the range.
            val n = input.readInt()
            require(n in 1..MAX_FRAME) { "frame length $n outside 1..$MAX_FRAME" }
            return n
        }

        private fun writeFrame(output: DataOutputStream, body: ByteArray) {
            output.writeInt(body.size)
            output.write(body)
            output.flush()
        }
    }
}
