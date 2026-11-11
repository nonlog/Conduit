package com.conduit.sync

import java.io.File
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import java.util.Base64

/** One trusted Windows identity. The id is the Noise static-public-key hash, never a hostname. */
data class PairedDesktop(val id: String, val name: String = "", val enabled: Boolean = true)

/**
 * Bounded, durable trust registry for multiple independent computers.
 *
 * The original peer.txt and peer-name.txt are deliberately retained as a compatibility pointer
 * to the last paired computer. Upgrading an existing phone imports those files without rotating
 * its Noise key or replacing the previous trust decision. A newly paired computer is appended,
 * never substituted for an existing one. Selection is a separate pointer: choosing a file target
 * does not need to tear down any transport.
 */
object PairedDesktops {
    const val MAX_PEERS = 8
    private const val REGISTRY = "paired-desktops.tsv"
    private const val LEGACY_ID = "peer.txt"
    private const val LEGACY_NAME = "peer-name.txt"
    private const val SELECTED = "selected-peer.txt"

    private fun validId(id: String): Boolean =
        id.length == ID_LEN && id.all { it in 'A'..'Z' || it in 'a'..'z' || it in '0'..'9' || it == '-' || it == '_' }

    private fun safeName(value: String): String =
        value.replace('\t', ' ').replace('\r', ' ').replace('\n', ' ').trim().take(64)

    private fun readOne(path: File): String? =
        runCatching { path.readText().trim() }.getOrNull()?.takeIf { it.isNotEmpty() }

    /** Android Direct Share supplies the published shortcut id, not the full Noise id.
     * A stale or ambiguous shortcut is never allowed to silently use another computer. */
    fun targetFromShortcut(peers: List<PairedDesktop>, shortcutId: String?): String? {
        val prefix = shortcutId?.takeIf { it.startsWith("desktop-") }
            ?.removePrefix("desktop-") ?: return null
        if (prefix.length != 20 || !prefix.all { it.isLetterOrDigit() || it == '-' || it == '_' }) return null
        return peers.singleOrNull { it.id.startsWith(prefix) }?.id
    }

    @Synchronized
    fun list(dir: File): List<PairedDesktop> {
        val registry = File(dir, REGISTRY)
        val rows = runCatching { registry.readLines() }.getOrDefault(emptyList())
        val peers = LinkedHashMap<String, PairedDesktop>()
        for (line in rows.take(MAX_PEERS * 3)) {
            // Two columns is the pre-switch v0.1.21 form, which means "connected".
            val cols = line.split('\t')
            if (cols.size < 2 || !validId(cols[0])) continue
            val name = runCatching {
                String(Base64.getUrlDecoder().decode(cols[1]), Charsets.UTF_8)
            }.getOrNull() ?: continue
            val enabled = cols.size < 3 || cols[2].trim() != "0"
            peers.putIfAbsent(cols[0], PairedDesktop(cols[0], safeName(name), enabled))
            if (peers.size == MAX_PEERS) break
        }
        // A pre-registry phone (or a separate old component) may have saved the
        // previous paired computer only in peer.txt. Preserve it during migration.
        val id = readOne(File(dir, LEGACY_ID))
        if (!registry.exists() && id != null && validId(id) && !peers.containsKey(id) && peers.size < MAX_PEERS) {
            peers[id] = PairedDesktop(id, safeName(readOne(File(dir, LEGACY_NAME)).orEmpty()))
        }
        return peers.values.toList()
    }

    @Synchronized
    fun selected(dir: File): String? {
        val peers = list(dir)
        val preference = readOne(File(dir, SELECTED))
        return preference?.takeIf { wanted -> peers.any { it.id == wanted } }
            ?: readOne(File(dir, LEGACY_ID))?.takeIf { wanted -> peers.any { it.id == wanted } }
            ?: peers.firstOrNull()?.id
    }

    @Synchronized
    fun remember(dir: File, id: String) {
        require(validId(id)) { "invalid Noise device id" }
        val peers = list(dir).toMutableList()
        val existing = peers.indexOfFirst { it.id == id }
        if (existing < 0) {
            require(peers.size < MAX_PEERS) { "maximum paired desktops reached" }
            peers.add(PairedDesktop(id))
        } else {
            // A fresh pairing handshake is an explicit "use this computer" signal.
            peers[existing] = peers[existing].copy(enabled = true)
        }
        save(dir, peers)
        // Legacy pointer is the main Link's next reconnection target, not the whole
        // trust store. Save the corresponding label so existing UI remains correct.
        val name = peers.first { it.id == id }.name
        writeAtomic(File(dir, LEGACY_ID), id + "\n")
        writeAtomic(File(dir, LEGACY_NAME), name + "\n")
        choose(dir, id)
    }

    @Synchronized
    fun rename(dir: File, id: String, rawName: String) {
        val name = safeName(rawName)
        if (name.isEmpty()) return
        val peers = list(dir).toMutableList()
        val i = peers.indexOfFirst { it.id == id }
        if (i < 0) return
        if (peers[i].name != name) {
            peers[i] = peers[i].copy(name = name)
            save(dir, peers)
        }
        if (readOne(File(dir, LEGACY_ID)) == id) writeAtomic(File(dir, LEGACY_NAME), name + "\n")
    }

    /** Per-computer Connect/Disconnect. Unlike [remove] it keeps the trust decision. */
    @Synchronized
    fun setEnabled(dir: File, id: String, enabled: Boolean) {
        val peers = list(dir).toMutableList()
        val i = peers.indexOfFirst { it.id == id }
        if (i < 0 || peers[i].enabled == enabled) return
        peers[i] = peers[i].copy(enabled = enabled)
        save(dir, peers)
    }

    /** A master Connect tap resumes every trusted computer the user paused one by one. */
    @Synchronized
    fun enableAll(dir: File) {
        val peers = list(dir)
        if (peers.all { it.enabled }) return
        save(dir, peers.map { it.copy(enabled = true) })
    }

    @Synchronized
    fun choose(dir: File, id: String) {
        require(list(dir).any { it.id == id }) { "cannot select an unpaired desktop" }
        writeAtomic(File(dir, SELECTED), id + "\n")
    }

    @Synchronized
    fun remove(dir: File, id: String) {
        val remaining = list(dir).filterNot { it.id == id }
        save(dir, remaining)
        val wasPrimary = readOne(File(dir, LEGACY_ID)) == id
        if (wasPrimary) {
            val replacement = remaining.firstOrNull()
            if (replacement == null) {
                File(dir, LEGACY_ID).delete()
                File(dir, LEGACY_NAME).delete()
            } else {
                writeAtomic(File(dir, LEGACY_ID), replacement.id + "\n")
                writeAtomic(File(dir, LEGACY_NAME), replacement.name + "\n")
            }
        }
        if (readOne(File(dir, SELECTED)) == id) {
            val next = remaining.firstOrNull()
            if (next == null) File(dir, SELECTED).delete()
            else choose(dir, next.id)
        }
    }

    private fun save(dir: File, peers: List<PairedDesktop>) {
        require(peers.size <= MAX_PEERS)
        val text = peers.joinToString("\n", postfix = if (peers.isEmpty()) "" else "\n") {
            it.id + "\t" + Base64.getUrlEncoder().withoutPadding()
                .encodeToString(it.name.toByteArray(Charsets.UTF_8)) +
                "\t" + if (it.enabled) "1" else "0"
        }
        writeAtomic(File(dir, REGISTRY), text)
    }

    private fun writeAtomic(destination: File, text: String) {
        destination.parentFile?.mkdirs()
        val temp = File(destination.parentFile, destination.name + "." + Thread.currentThread().id + ".tmp")
        temp.writeText(text)
        try {
            Files.move(temp.toPath(), destination.toPath(),
                StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
        } catch (_: AtomicMoveNotSupportedException) {
            Files.move(temp.toPath(), destination.toPath(), StandardCopyOption.REPLACE_EXISTING)
        } finally {
            temp.delete()
        }
    }
}
