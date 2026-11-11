package com.conduit.sync

import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress

/** Pure address selection: the desktop LAN listener currently accepts IPv4 only. */
internal object LanDiscoveryPolicy {
    data class Subnet(val address: InetAddress, val prefixLength: Int)

    /**
     * The desktop's own "ip:port" for its LAN listener. It arrived over the Noise-authenticated
     * session, so the pairing is trusted; the address is still checked, because a stale interface
     * must cost one failed dial rather than a wrong subnet or a synthetic VPN address.
     */
    fun hintTarget(
        hint: String?,
        subnets: List<Subnet>,
    ): InetSocketAddress? {
        val text = hint?.trim().orEmpty()
        val colon = text.lastIndexOf(':')
        if (colon <= 0) return null
        val port = text.substring(colon + 1).toIntOrNull() ?: return null
        if (port !in 1..65535) return null
        val address = parseIpv4(text.substring(0, colon)) ?: return null
        if (address.isAnyLocalAddress || address.isLoopbackAddress || address.isMulticastAddress) return null
        if (isSynthetic(address)) return null
        val local = subnets.filter { it.address is Inet4Address && it.prefixLength in 1..32 }
        if (local.isEmpty() || local.none { onLinkHost(address, it) }) return null
        return InetSocketAddress(address, port)
    }

    /** Numeric only: a hinted hostname would be a DNS lookup on the main thread, so it is refused. */
    private fun parseIpv4(text: String): Inet4Address? {
        val parts = text.split('.')
        if (parts.size != 4) return null
        val bytes = ByteArray(4)
        parts.forEachIndexed { index, part ->
            val value = part.toIntOrNull() ?: return null
            if (value !in 0..255) return null
            bytes[index] = value.toByte()
        }
        return InetAddress.getByAddress(bytes) as Inet4Address
    }

    fun acceptsPeer(expected: String?, advertised: String?): Boolean =
        expected == null || expected == advertised

    fun selectAddress(candidates: List<InetAddress>, subnets: List<Subnet>): InetAddress? {
        val local = subnets.filter { it.address is Inet4Address && it.prefixLength in 1..32 }
        return candidates.asSequence()
            .filterIsInstance<Inet4Address>()
            .filter { !it.isAnyLocalAddress && !it.isLoopbackAddress && !it.isMulticastAddress }
            .filterNot(::isSynthetic)
            .filter { address -> local.none { it.address == address } }
            // Never pick a desktop's Ethernet/VPN address when discovering it over a hotspot.
            .filter { address -> local.isEmpty() || local.any { onLinkHost(address, it) } }
            .distinct()
            .sortedBy { ipv4(it) }
            .firstOrNull()
    }

    private fun isSynthetic(address: Inet4Address): Boolean {
        val bytes = address.address
        return (bytes[0].toInt() and 255) == 198 && (bytes[1].toInt() and 255) in 18..19
    }

    private fun onLinkHost(address: Inet4Address, subnet: Subnet): Boolean {
        val mask = (0xffff_ffffL shl (32 - subnet.prefixLength)) and 0xffff_ffffL
        val candidate = ipv4(address)
        if ((candidate and mask) != (ipv4(subnet.address) and mask)) return false
        if (subnet.prefixLength >= 31) return true
        val host = candidate and (mask xor 0xffff_ffffL)
        return host != 0L && host != (mask xor 0xffff_ffffL)
    }

    private fun ipv4(address: InetAddress): Long =
        address.address.fold(0L) { value, byte -> (value shl 8) or (byte.toLong() and 255) }
}

/** Stops late resolve/deadline callbacks from completing or cancelling a newer burst. */
internal class DiscoveryEpoch {
    private var next = 0L
    private var active: Long? = null

    @Synchronized fun begin(): Long = (++next).also { active = it }
    @Synchronized fun isCurrent(epoch: Long): Boolean = active == epoch
    @Synchronized fun finish(epoch: Long): Boolean {
        if (active != epoch) return false
        active = null
        return true
    }
    @Synchronized fun stop() { active = null }
}
