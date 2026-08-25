package com.conduit.sync

import java.io.ByteArrayOutputStream
import java.security.SecureRandom
import org.bouncycastle.crypto.digests.Blake2sDigest
import org.bouncycastle.crypto.macs.HMac
import org.bouncycastle.crypto.modes.ChaCha20Poly1305
import org.bouncycastle.crypto.params.AEADParameters
import org.bouncycastle.crypto.params.KeyParameter
import org.bouncycastle.math.ec.rfc7748.X25519

/**
 * `Noise_XX_25519_ChaChaPoly_BLAKE2s`, and nothing else.
 *
 * Written here rather than taken from a library because no Noise implementation is
 * published to Maven Central for the JVM. Only one pattern and one cipher suite are
 * supported, which is the whole reason it fits in one file. Correctness rests on the
 * interop fixture in `NoiseInteropTest` — a transcript produced by Rust `snow` and
 * replayed here byte for byte.
 *
 * Follows the Noise spec revision 34.
 */
private const val DHLEN = 32
private const val HASHLEN = 32
private const val TAGLEN = 16

const val NOISE_XX = "Noise_XX_25519_ChaChaPoly_BLAKE2s"

/** Mixed into the handshake hash so a peer speaking another dialect fails here. */
val PROLOGUE: ByteArray = "conduit/1".toByteArray()

private val EMPTY = ByteArray(0)

class KeyPair(val private: ByteArray, val public: ByteArray) {
    companion object {
        fun generate(rng: SecureRandom = SecureRandom()): KeyPair {
            val priv = ByteArray(DHLEN)
            X25519.generatePrivateKey(rng, priv)
            return fromPrivate(priv)
        }

        fun fromPrivate(priv: ByteArray): KeyPair {
            require(priv.size == DHLEN) { "private key is ${priv.size} bytes, expected $DHLEN" }
            val pub = ByteArray(DHLEN)
            X25519.generatePublicKey(priv, 0, pub, 0)
            return KeyPair(priv.copyOf(), pub)
        }
    }
}

private fun blake2s(vararg parts: ByteArray): ByteArray {
    val digest = Blake2sDigest(HASHLEN * 8)
    for (part in parts) digest.update(part, 0, part.size)
    return ByteArray(HASHLEN).also { digest.doFinal(it, 0) }
}

private fun hmac(key: ByteArray, vararg data: ByteArray): ByteArray {
    val mac = HMac(Blake2sDigest(HASHLEN * 8))
    mac.init(KeyParameter(key))
    for (part in data) mac.update(part, 0, part.size)
    return ByteArray(mac.macSize).also { mac.doFinal(it, 0) }
}

/** Noise HKDF with two outputs. */
private fun hkdf2(chainingKey: ByteArray, ikm: ByteArray): Pair<ByteArray, ByteArray> {
    val temp = hmac(chainingKey, ikm)
    val first = hmac(temp, byteArrayOf(1))
    return first to hmac(temp, first, byteArrayOf(2))
}

private fun dh(priv: ByteArray, pub: ByteArray): ByteArray {
    val out = ByteArray(DHLEN)
    // calculateAgreement, not scalarMult: it returns false for a low-order point, which
    // would otherwise agree on an all-zero secret. snow lets that through and fails on
    // the tag instead; failing here is stricter and indistinguishable to honest peers.
    require(X25519.calculateAgreement(priv, 0, pub, 0, out, 0)) {
        "X25519 rejected the peer's key"
    }
    return out
}

private class CipherState {
    private var key: ByteArray? = null
    private var counter = 0L

    fun initializeKey(k: ByteArray) {
        key = k
        counter = 0
    }

    fun hasKey() = key != null

    /** 4 zero bytes then the counter, little-endian — Noise's ChaChaPoly nonce. */
    private fun nonce(): ByteArray {
        val iv = ByteArray(12)
        var v = counter
        for (i in 4 until 12) {
            iv[i] = (v and 0xff).toByte()
            v = v ushr 8
        }
        return iv
    }

    private fun run(forEncryption: Boolean, ad: ByteArray, input: ByteArray): ByteArray {
        val k = key ?: return input
        val cipher = ChaCha20Poly1305()
        cipher.init(forEncryption, AEADParameters(KeyParameter(k), TAGLEN * 8, nonce(), ad))
        val out = ByteArray(cipher.getOutputSize(input.size))
        var off = cipher.processBytes(input, 0, input.size, out, 0)
        off += cipher.doFinal(out, off)
        counter++
        return if (off == out.size) out else out.copyOf(off)
    }

    fun encrypt(ad: ByteArray, plaintext: ByteArray) = run(true, ad, plaintext)

    fun decrypt(ad: ByteArray, ciphertext: ByteArray) = run(false, ad, ciphertext)
}

/** Transport mode: two keys, two counters, no negotiation left to do. */
class NoiseSession internal constructor(sendKey: ByteArray, recvKey: ByteArray) {
    private val tx = CipherState().apply { initializeKey(sendKey) }
    private val rx = CipherState().apply { initializeKey(recvKey) }

    fun writeMessage(plaintext: ByteArray): ByteArray = tx.encrypt(EMPTY, plaintext)

    fun readMessage(ciphertext: ByteArray): ByteArray = rx.decrypt(EMPTY, ciphertext)
}

class NoiseXX(
    private val initiator: Boolean,
    private val s: KeyPair,
    private val rng: SecureRandom = SecureRandom(),
) {
    private var chainingKey: ByteArray
    private var handshakeHash: ByteArray
    private val cipher = CipherState()
    private var e: KeyPair? = null
    private var re: ByteArray? = null
    private var messageIndex = 0

    /** The peer's static public key. Null until message 2 (initiator) or 3 (responder). */
    var remoteStatic: ByteArray? = null
        private set

    init {
        // The protocol name is 33 bytes, past HASHLEN, so it is hashed rather than padded.
        handshakeHash = blake2s(NOISE_XX.toByteArray())
        chainingKey = handshakeHash.copyOf()
        mixHash(PROLOGUE)
    }

    val isFinished: Boolean get() = messageIndex >= PATTERNS.size

    private fun mixHash(data: ByteArray) {
        handshakeHash = blake2s(handshakeHash, data)
    }

    private fun mixKey(ikm: ByteArray) {
        val (ck, tempKey) = hkdf2(chainingKey, ikm)
        chainingKey = ck
        cipher.initializeKey(tempKey)
    }

    private fun encryptAndHash(plaintext: ByteArray): ByteArray =
        cipher.encrypt(handshakeHash, plaintext).also { mixHash(it) }

    private fun decryptAndHash(ciphertext: ByteArray): ByteArray =
        cipher.decrypt(handshakeHash, ciphertext).also { mixHash(ciphertext) }

    private fun diffieHellman(token: String): ByteArray = when (token) {
        "ee" -> dh(e!!.private, re!!)
        "es" -> if (initiator) dh(e!!.private, remoteStatic!!) else dh(s.private, re!!)
        "se" -> if (initiator) dh(s.private, re!!) else dh(e!!.private, remoteStatic!!)
        else -> error("token $token is not in XX")
    }

    fun writeMessage(payload: ByteArray = EMPTY): ByteArray {
        check(!isFinished) { "handshake already finished" }
        val out = ByteArrayOutputStream()
        for (token in PATTERNS[messageIndex]) {
            when (token) {
                "e" -> {
                    // Each role writes `e` exactly once, so a pre-set key is the
                    // fixture's pin and never a reused ephemeral in production.
                    val ephemeral = e ?: KeyPair.generate(rng)
                    e = ephemeral
                    out.write(ephemeral.public)
                    mixHash(ephemeral.public)
                }
                "s" -> out.write(encryptAndHash(s.public))
                else -> mixKey(diffieHellman(token))
            }
        }
        out.write(encryptAndHash(payload))
        messageIndex++
        return out.toByteArray()
    }

    fun readMessage(message: ByteArray): ByteArray {
        check(!isFinished) { "handshake already finished" }
        var off = 0
        for (token in PATTERNS[messageIndex]) {
            when (token) {
                "e" -> {
                    val ephemeral = message.at(off, DHLEN)
                    off += DHLEN
                    re = ephemeral
                    mixHash(ephemeral)
                }
                "s" -> {
                    val len = DHLEN + if (cipher.hasKey()) TAGLEN else 0
                    remoteStatic = decryptAndHash(message.at(off, len))
                    off += len
                }
                else -> mixKey(diffieHellman(token))
            }
        }
        messageIndex++
        return decryptAndHash(message.at(off, message.size - off))
    }

    /**
     * [len] bytes at [off], or a protocol error that says what was short.
     *
     * A handshake message is bytes off the network, so one that is too short is untrusted
     * input, not an internal bug — and `copyOfRange` would raise IndexOutOfBoundsException,
     * which reads in a log like a defect in this class rather than a peer sending nonsense.
     *
     * It happens for real: a relay that splices two *initiators* to each other hands each one
     * the other's 32-byte first message where an 80-byte second one was due.
     */
    private fun ByteArray.at(off: Int, len: Int): ByteArray {
        require(off >= 0 && len >= 0 && off + len <= size) {
            "handshake message $messageIndex is $size B, needs ${off + len} B"
        }
        return copyOfRange(off, off + len)
    }

    fun intoTransport(): NoiseSession {
        check(isFinished) { "handshake is not finished" }
        val (first, second) = hkdf2(chainingKey, EMPTY)
        return if (initiator) NoiseSession(first, second) else NoiseSession(second, first)
    }

    /** For the interop fixture only: pins the ephemeral so a transcript is reproducible. */
    internal fun forceEphemeral(priv: ByteArray) {
        e = KeyPair.fromPrivate(priv)
    }

    private companion object {
        val PATTERNS = listOf(
            listOf("e"),
            listOf("e", "ee", "s", "es"),
            listOf("s", "se"),
        )
    }
}
