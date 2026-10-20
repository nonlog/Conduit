package com.conduit.sync

import android.net.Network
import android.os.SystemClock
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.SocketException
import java.net.SocketTimeoutException
import java.util.Locale

/**
 * One-shot physical-network mDNS A query for devices whose VPN DNS substitutes a fake IP for
 * `*.local`. NSD has already identified the paired service using its TXT device ID; this only
 * obtains an on-link address, and Noise still authenticates the actual desktop before linking.
 *
 * The query asks for a unicast reply to an ephemeral port. Nothing listens while idle, and no
 * service, periodic discovery, VPN bypass for Relay, or new Android permission is required.
 */
internal object LanMdnsResolver {
    private const val TIMEOUT_MS = 1_800L
    private const val MAX_PACKET = 4_096
    private val multicast = InetAddress.getByAddress(byteArrayOf(224.toByte(), 0, 0, 251.toByte()))

    fun hostname(value: String?): String? {
        val host = value?.trim()?.trimEnd('.')?.lowercase(Locale.ROOT) ?: return null
        if (!host.endsWith(".local") || host.length > 253) return null
        val labels = host.split('.')
        if (labels.size < 2 || labels.any { label ->
                label.length !in 1..63 || label.first() == '-' || label.last() == '-' ||
                    label.any { !it.isLetterOrDigit() && it != '-' || it.code > 127 }
            }) return null
        return host
    }

    /** NsdServiceInfo.getHostname() omits the trailing `.local` domain (API 36+). */
    fun nsdHostname(value: String?): String? {
        val label = value?.trim()?.trimEnd('.')?.takeIf { it.isNotEmpty() } ?: return null
        if ('.' in label && !label.endsWith(".local", ignoreCase = true)) return null
        return hostname(if (label.endsWith(".local", ignoreCase = true)) label else "$label.local")
    }

    /** Called only by Discovery's bounded miss handler, never on Android's main thread. */
    fun query(network: Network, rawHostname: String): List<InetAddress> {
        val host = hostname(rawHostname) ?: return emptyList()
        val question = question(host)
        return DatagramSocket(null).use { socket ->
            socket.bind(InetSocketAddress(0))
            try {
                network.bindSocket(socket)
            } catch (denied: SocketException) {
                if (!isNetworkBindingDenied(denied)) throw denied
                // Some VPNs reject explicit physical-network socket binds. A regular socket
                // uses Android/VPN routing instead; if multicast is blocked there, this one
                // bounded probe simply fails and the existing Relay path stays available.
                // Never forge a VPN exemption or change the device's network policy.
            }
            socket.send(DatagramPacket(question, question.size, multicast, 5353))
            val result = linkedSetOf<InetAddress>()
            val deadline = SystemClock.elapsedRealtime() + TIMEOUT_MS
            val buffer = ByteArray(MAX_PACKET)
            while (SystemClock.elapsedRealtime() < deadline && result.isEmpty()) {
                val remaining = (deadline - SystemClock.elapsedRealtime()).coerceAtLeast(1L)
                socket.soTimeout = remaining.toInt()
                val response = DatagramPacket(buffer, buffer.size)
                try {
                    socket.receive(response)
                } catch (_: SocketTimeoutException) {
                    break
                }
                result.addAll(addresses(host, response.data.copyOf(response.length)))
            }
            result.toList()
        }
    }

    /** DNS A/IN with QU set so an ephemeral socket receives a unicast response on this Network. */
    internal fun question(host: String): ByteArray {
        val labels = requireNotNull(hostname(host)).split('.')
        val body = labels.flatMap { label -> listOf(label.length.toByte()) + label.toByteArray(Charsets.US_ASCII).toList() }
        return byteArrayOf(
            0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0,
            *body.toByteArray(), 0, 0, 1, 0x80.toByte(), 1,
        )
    }

    /** Safe bounded parser: malformed or unrelated mDNS answers are not candidate addresses. */
    internal fun addresses(host: String, data: ByteArray): List<InetAddress> {
        if (data.size < 12 || data.size > MAX_PACKET || (u16(data, 2) and 0x8000) == 0) return emptyList()
        val questions = u16(data, 4)
        val records = u16(data, 6) + u16(data, 8) + u16(data, 10)
        if (questions > 8 || records > 64) return emptyList()
        var pos = 12
        repeat(questions) {
            val name = readName(data, pos) ?: return emptyList()
            pos = name.second + 4
            if (pos > data.size) return emptyList()
        }
        val found = linkedSetOf<InetAddress>()
        repeat(records) {
            val name = readName(data, pos) ?: return emptyList()
            pos = name.second
            if (pos + 10 > data.size) return emptyList()
            val type = u16(data, pos)
            val clazz = u16(data, pos + 2) and 0x7fff // cache-flush bit is not a class
            val length = u16(data, pos + 8)
            pos += 10
            if (pos + length > data.size) return emptyList()
            if (type == 1 && clazz == 1 && length == 4 && name.first.equals(host, ignoreCase = true)) {
                found.add(InetAddress.getByAddress(data.copyOfRange(pos, pos + 4)))
            }
            pos += length
        }
        return found.toList()
    }

    private fun u16(bytes: ByteArray, offset: Int): Int =
        ((bytes[offset].toInt() and 255) shl 8) or (bytes[offset + 1].toInt() and 255)

    private fun readName(data: ByteArray, start: Int): Pair<String, Int>? {
        var offset = start
        var end = -1
        val labels = mutableListOf<String>()
        var totalLength = 0
        repeat(32) {
            if (offset !in data.indices) return null
            val size = data[offset].toInt() and 255
            when {
                size == 0 -> return labels.joinToString(".") to (if (end >= 0) end else offset + 1)
                (size and 0xc0) == 0xc0 -> {
                    if (offset + 1 >= data.size) return null
                    val target = ((size and 0x3f) shl 8) or (data[offset + 1].toInt() and 255)
                    if (target >= offset || target < 12) return null // reject cycles/forward pointers
                    if (end < 0) end = offset + 2
                    offset = target
                }
                size in 1..63 && offset + size < data.size -> {
                    val label = String(data, offset + 1, size, Charsets.US_ASCII)
                    totalLength += size + 1
                    if (totalLength > 253) return null
                    labels.add(label)
                    offset += size + 1
                }
                else -> return null
            }
        }
        return null
    }
}
