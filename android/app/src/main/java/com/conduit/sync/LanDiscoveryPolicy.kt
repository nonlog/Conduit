package com.conduit.sync

import java.net.Inet4Address
import java.net.InetAddress

/** Pure address selection: the desktop LAN listener currently accepts IPv4 only. */
internal object LanDiscoveryPolicy {
    data class Subnet(val address: InetAddress, val prefixLength: Int)

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
