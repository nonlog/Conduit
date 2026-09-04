package com.conduit.sync

import java.security.MessageDigest
import java.util.Base64

/** Human code -> temporary Relay rendezvous. The Relay itself needs no pairing-specific protocol. */
object PairingCode {
    const val LENGTH = 10
    private const val DOMAIN = "conduit-pair-v1:"

    fun normalize(value: String): String =
        value.filter(Char::isLetterOrDigit).uppercase()

    fun isValid(value: String): Boolean = normalize(value).length == LENGTH

    fun display(value: String): String {
        val normalized = normalize(value)
        return if (normalized.length == LENGTH) {
            normalized.take(5) + "-" + normalized.drop(5)
        } else {
            normalized
        }
    }

    /** BASE64URL(SHA256(domain || normalized-code)), same 43-byte rendezvous shape as a device id. */
    fun rendezvous(value: String): String {
        val normalized = normalize(value)
        require(normalized.length == LENGTH) { "pairing code must contain $LENGTH characters" }
        val digest = MessageDigest.getInstance("SHA-256")
            .digest((DOMAIN + normalized).toByteArray(Charsets.UTF_8))
        return Base64.getUrlEncoder().withoutPadding().encodeToString(digest)
    }
}
