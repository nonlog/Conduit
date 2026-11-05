package com.conduit.sync

/**
 * Resolve the desktop the UI and default share actions should use.
 *
 * The selected id is a durable user preference, but it can point at a computer
 * that is currently offline while another trusted computer is already linked.
 * In that case showing the offline preference as the active connection makes
 * the phone say "Connecting" and sends clipboard/share work into a dead socket.
 */
internal fun resolveActiveDevice(
    devices: List<ConnectedDevice>,
    selectedId: String?,
    legacyId: String? = null,
): ConnectedDevice? {
    val selected = devices.firstOrNull { it.id == selectedId }
    return selected?.takeIf { it.state == LinkState.Connected }
        ?: devices.firstOrNull { it.state == LinkState.Connected }
        ?: selected
        ?: devices.firstOrNull { it.id == legacyId }
        ?: devices.firstOrNull()
}

internal fun resolveActiveDeviceId(
    devices: List<ConnectedDevice>,
    selectedId: String?,
    legacyId: String? = null,
): String? = resolveActiveDevice(devices, selectedId, legacyId)?.id
