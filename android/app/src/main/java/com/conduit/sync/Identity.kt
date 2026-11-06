package com.conduit.sync

import java.io.File
import java.security.MessageDigest
import java.util.Base64

/**
 * The phone's Noise static key, and the two names derived from it.
 *
 * Mirrors `wire.rs` byte for byte — same file layout, same derivations. The desktop
 * shows the same fingerprint for the same key, or pairing is unverifiable.
 */
object Identity {
    /** 64 bytes: private ‖ public. Public is stored, not re-derived. */
    private const val FILE = "identity.bin"

    /** The desktop's [deviceId], which is also the relay rendezvous. */
    private const val PEER_FILE = "peer.txt"

    /**
     * The desktop's own name for itself. Stored next to its id because the two arrive
     * together and both have to survive a restart: the id so the relay has a rendezvous, the
     * name so the share sheet has a label before the first session of the day connects.
     */
    private const val PEER_NAME_FILE = "peer-name.txt"

    fun loadOrCreate(dir: File): KeyPair {
        val file = File(dir, FILE)
        val raw = if (file.isFile) file.readBytes() else null
        if (raw != null) {
            require(raw.size == 64) { "$file is ${raw.size} bytes, expected 64" }
            return KeyPair(raw.copyOfRange(0, 32), raw.copyOfRange(32, 64))
        }
        val fresh = KeyPair.generate()
        dir.mkdirs()
        // Written whole, then moved: a torn identity file would lock out the desktop
        // pairing that is keyed on the public half.
        val tmp = File(dir, "$FILE.tmp")
        tmp.writeBytes(fresh.private + fresh.public)
        require(tmp.renameTo(file)) { "could not move $tmp into place" }
        return fresh
    }

    /** BASE64URL(SHA256(static_pub)), unpadded. The relay pairs on this. */
    fun deviceId(staticPub: ByteArray): String =
        Base64.getUrlEncoder().withoutPadding().encodeToString(sha256(staticPub))

    /** First 8 bytes of the same hash, hex, colon-joined — the out-of-band comparison. */
    fun fingerprint(staticPub: ByteArray): String =
        sha256(staticPub).take(8).joinToString(":") {
            (it.toInt() and 0xff).toString(16).padStart(2, '0')
        }

    private fun sha256(data: ByteArray): ByteArray =
        MessageDigest.getInstance("SHA-256").digest(data)

    /**
     * Records which desktop we last completed a handshake with. Throws rather than
     * swallowing: the caller logs it, and a phone that cannot remember its peer simply
     * has no relay path.
     */
    fun rememberPeer(dir: File, deviceId: String) {
        PairedDesktops.remember(dir, deviceId)
    }

    /** Compatibility pointer to the main Link; all trusted computers are in [PairedDesktops]. */
    fun peer(dir: File): String? = runCatching { File(dir, PEER_FILE).readText().trim() }
        .getOrNull()
        ?.takeIf { it.length == ID_LEN }

    fun peers(dir: File): List<PairedDesktop> = PairedDesktops.list(dir)

    /** Forget the current desktop, never every trusted device on the phone. */
    fun forgetPeer(dir: File) {
        peer(dir)?.let { PairedDesktops.remove(dir, it) }
    }

    fun rememberPeerName(dir: File, name: String) {
        peer(dir)?.let { PairedDesktops.rename(dir, it, name) }
    }

    fun peerName(dir: File): String? =
        peer(dir)?.let { current -> PairedDesktops.list(dir).firstOrNull { it.id == current }?.name }
            ?.takeIf { it.isNotEmpty() }
            ?: runCatching { File(dir, PEER_NAME_FILE).readText().trim() }
                .getOrNull()?.takeIf { it.isNotEmpty() }
}

/** BASE64URL of a SHA-256 digest, unpadded, is always this long. */
const val ID_LEN = 43
