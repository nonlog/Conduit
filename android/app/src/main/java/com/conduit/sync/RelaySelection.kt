package com.conduit.sync

import android.util.Log
import java.io.File

private const val RELAY_TAG = "conduit.relay"
private const val RELAYS_FILE = "relays.txt"
private const val QUALITY_FILE = "relay-quality.txt"
private const val COOLDOWN_MS = 30L * 60L * 1000L

data class RelayEndpoint(
    val id: String,
    val host: String,
    val port: Int,
    val fallbackIpv4: String? = null,
)

/**
 * User/runtime relay inventory. It is intentionally a tiny app-private text file rather than a
 * service or discovery protocol: reading it happens only when SyncService starts and costs no idle
 * work. The production default remains TYO until the other compatible relay services are deployed.
 */
object RelayCatalog {
    val default = RelayEndpoint("tyo", "tyo.414222.xyz", 41113, "138.3.214.175")

    fun load(dir: File): List<RelayEndpoint> {
        val file = File(dir, RELAYS_FILE)
        val parsed = runCatching {
            file.takeIf(File::isFile)?.readLines().orEmpty()
                .mapNotNull(::parse)
                .distinctBy(RelayEndpoint::id)
        }.onFailure { Log.w(RELAY_TAG, "could not read relay inventory", it) }
            .getOrDefault(emptyList())
        return parsed.ifEmpty { listOf(default) }
    }

    internal fun parse(line: String): RelayEndpoint? {
        val clean = line.substringBefore('#').trim()
        if (clean.isEmpty()) return null
        val fields = clean.split('|').map(String::trim)
        if (fields.size !in 3..4) return null
        val id = fields[0]
        val host = fields[1]
        val port = fields[2].toIntOrNull() ?: return null
        if (id.isEmpty() || host.isEmpty() || port !in 1..65535) return null
        return RelayEndpoint(id, host, port, fields.getOrNull(3)?.takeIf(String::isNotEmpty))
    }
}

internal data class RelayQuality(
    var successes: Int = 0,
    var dialFailures: Int = 0,
    var unstableSessions: Int = 0,
    var failureStreak: Int = 0,
    var cooldownUntilMs: Long = 0,
    var lastSuccessMs: Long = 0,
    var goodputBps: Double = 0.0,
)

/**
 * Passive relay quality memory. There is no timer and no probe method by design.
 *
 * The only writers are events Conduit already had to process: a real handshake succeeds/fails, a
 * live session dies unexpectedly, or real image/file bytes finish moving. That preserves the
 * product's idle-cost rule while still learning that a low-latency but lossy relay is bad.
 */
class RelayQualityStore(private val dir: File) {
    private val quality = mutableMapOf<String, RelayQuality>()

    init {
        load()
    }

    @Synchronized
    fun candidates(networkClass: String, endpoints: List<RelayEndpoint>, nowMs: Long): List<RelayEndpoint> {
        if (endpoints.size <= 1) return endpoints
        val indexed = endpoints.withIndex().toList()
        val available = indexed.filter { (_, endpoint) ->
            record(networkClass, endpoint.id).cooldownUntilMs <= nowMs
        }
        val pool = available.ifEmpty { indexed }
        return pool.sortedWith(
            compareBy<IndexedValue<RelayEndpoint>>(
                { record(networkClass, it.value.id).failureStreak },
                { -record(networkClass, it.value.id).lastSuccessMs },
                { -record(networkClass, it.value.id).goodputBps },
                { record(networkClass, it.value.id).unstableSessions },
                { it.index },
            ),
        ).map { it.value }
    }

    @Synchronized
    fun connected(networkClass: String, endpoint: RelayEndpoint, nowMs: Long) {
        record(networkClass, endpoint.id).apply {
            successes++
            failureStreak = 0
            cooldownUntilMs = 0
            lastSuccessMs = nowMs
        }
        save()
    }

    @Synchronized
    fun dialFailed(networkClass: String, endpoint: RelayEndpoint, nowMs: Long) {
        record(networkClass, endpoint.id).apply {
            dialFailures++
            failureStreak++
            if (failureStreak >= 2) cooldownUntilMs = nowMs + COOLDOWN_MS
        }
        save()
    }

    @Synchronized
    fun unstable(networkClass: String, endpoint: RelayEndpoint) {
        record(networkClass, endpoint.id).apply {
            unstableSessions++
            failureStreak = failureStreak.coerceAtLeast(1)
        }
        save()
    }

    @Synchronized
    fun goodput(networkClass: String, endpoint: RelayEndpoint, bytes: Long, elapsedMs: Long) {
        if (bytes <= 0L || elapsedMs <= 0L) return
        val observed = bytes.toDouble() * 1000.0 / elapsedMs.toDouble()
        record(networkClass, endpoint.id).apply {
            // A deliberately slow EWMA. One fast screenshot must not erase hours of bad routing.
            goodputBps = if (goodputBps <= 0.0) observed else goodputBps * 0.75 + observed * 0.25
        }
        save()
    }

    @Synchronized
    internal fun snapshot(networkClass: String, endpoint: RelayEndpoint): RelayQuality =
        record(networkClass, endpoint.id).copy()

    private fun key(networkClass: String, id: String) = "$networkClass\t$id"

    private fun record(networkClass: String, id: String): RelayQuality =
        quality.getOrPut(key(networkClass, id)) { RelayQuality() }

    private fun load() {
        runCatching {
            File(dir, QUALITY_FILE).takeIf(File::isFile)?.forEachLine { line ->
                val f = line.split('\t')
                if (f.size != 10) return@forEachLine
                quality[key(f[0], f[1])] = RelayQuality(
                    successes = f[2].toIntOrNull() ?: return@forEachLine,
                    dialFailures = f[3].toIntOrNull() ?: return@forEachLine,
                    unstableSessions = f[4].toIntOrNull() ?: return@forEachLine,
                    failureStreak = f[5].toIntOrNull() ?: return@forEachLine,
                    cooldownUntilMs = f[6].toLongOrNull() ?: return@forEachLine,
                    lastSuccessMs = f[7].toLongOrNull() ?: return@forEachLine,
                    goodputBps = f[8].toDoubleOrNull() ?: return@forEachLine,
                )
            }
        }.onFailure { Log.w(RELAY_TAG, "could not read relay quality", it) }
    }

    private fun save() {
        runCatching {
            val body = quality.entries.joinToString("\n") { (key, q) ->
                val (networkClass, id) = key.split('\t', limit = 2)
                listOf(
                    networkClass,
                    id,
                    q.successes,
                    q.dialFailures,
                    q.unstableSessions,
                    q.failureStreak,
                    q.cooldownUntilMs,
                    q.lastSuccessMs,
                    q.goodputBps,
                    "v1",
                ).joinToString("\t")
            }
            File(dir, QUALITY_FILE).writeText(if (body.isEmpty()) "" else "$body\n")
        }.onFailure { Log.w(RELAY_TAG, "could not persist relay quality", it) }
    }
}
