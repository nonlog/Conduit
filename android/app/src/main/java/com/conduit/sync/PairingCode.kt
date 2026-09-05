package com.conduit.sync

import java.net.URI
import java.net.URLDecoder
import java.security.MessageDigest
import java.util.Base64

/** Human code -> temporary Relay rendezvous. The Relay itself needs no pairing-specific protocol. */
object PairingCode {
    const val LENGTH = 6
    private const val DOMAIN = "conduit-pair-v2:"

    fun normalize(value: String): String =
        value.filter { it in '0'..'9' }

    fun isValid(value: String): Boolean =
        value.length == LENGTH && value.all { it in '0'..'9' }

    fun display(value: String): String = normalize(value)

    /** Parse the exact payload rendered by the Windows pairing QR code. */
    fun fromQrPayload(value: String): String? {
        val uri = runCatching { URI(value.trim()) }.getOrNull() ?: return null
        if (!uri.scheme.equals("conduit", ignoreCase = true) ||
            !uri.host.equals("pair", ignoreCase = true)) return null

        val rawCode = uri.rawQuery
            ?.split('&')
            ?.firstOrNull { it.substringBefore('=').equals("code", ignoreCase = true) }
            ?.substringAfter('=', "")
            ?: return null
        val decoded = runCatching { URLDecoder.decode(rawCode, Charsets.UTF_8.name()) }.getOrNull()
            ?: return null
        val code = normalize(decoded)
        return code.takeIf(::isValid)
    }

    /** BASE64URL(SHA256(domain || normalized-code)), same 43-byte rendezvous shape as a device id. */
    fun rendezvous(value: String): String {
        val normalized = normalize(value)
        require(normalized.length == LENGTH) { "pairing code must contain $LENGTH digits" }
        val digest = MessageDigest.getInstance("SHA-256")
            .digest((DOMAIN + normalized).toByteArray(Charsets.UTF_8))
        return Base64.getUrlEncoder().withoutPadding().encodeToString(digest)
    }
}
