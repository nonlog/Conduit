package com.conduit.sync

import java.util.ArrayDeque

/** One writer, one bulk frame per turn. No timer or wakeup while there is no work. */
internal class TransferScheduler(
    private val capacity: Int = 64,
    private val bulkCapacity: Int = 16,
    private val onMaintenance: () -> Unit = {},
    private val onTaskFailure: (Exception) -> Unit = { throw it },
) {
    private val lock = Object()
    private val controls = ArrayDeque<() -> Unit>()
    private val urgent = ArrayDeque<() -> Unit>()
    private val bulk = ArrayDeque<() -> Boolean>()
    private var active: (() -> Boolean)? = null
    private var bulkPaused = false
    private var stopped = false
    private var controlBurst = 0
    private var maintenancePending = false

    /** Coalesced lifecycle cleanup cannot be evicted by traffic or queue saturation. */
    fun requestMaintenance() = synchronized(lock) { maintenancePending = true; lock.notifyAll() }

    fun control(priority: Boolean = false, task: () -> Unit): Boolean = synchronized(lock) {
        val queue = if (priority) urgent else controls
        if (stopped || queue.size >= capacity) return@synchronized false
        queue.addLast(task)
        lock.notifyAll()
        true
    }

    fun bulk(task: () -> Boolean): Boolean = synchronized(lock) {
        if (stopped || bulk.size + (if (active == null) 0 else 1) >= bulkCapacity) return@synchronized false
        bulk.addLast(task)
        lock.notifyAll()
        true
    }

    fun removeQueuedBulk(task: () -> Boolean) = synchronized(lock) { bulk.remove(task) }

    /** Keep the FIFO head admitted while waiting for its receiver's publication.
     * Controls still run; the writer sleeps instead of sending the next offer or spinning. */
    fun pauseBulk() = synchronized(lock) { bulkPaused = true }

    fun resumeBulk(task: () -> Boolean) = synchronized(lock) {
        if (active === task) bulkPaused = false
        lock.notifyAll()
    }

    /** Also used by deterministic JVM tests; true means there was work. */
    fun step(): Boolean {
        val maintenance = synchronized(lock) { maintenancePending.also { maintenancePending = false } }
        if (maintenance) onMaintenance()
        var control: (() -> Unit)? = null
        var transfer: (() -> Boolean)? = null
        synchronized(lock) {
            if (urgent.isNotEmpty()) control = urgent.removeFirst()
            else if (controls.isNotEmpty() && (bulkPaused || controlBurst < 16 || (active == null && bulk.isEmpty()))) {
                control = controls.removeFirst()
                controlBurst++
            } else if (!bulkPaused) {
                if (active == null && bulk.isNotEmpty()) active = bulk.removeFirst()
                transfer = active
                controlBurst = 0
            }
        }
        control?.let { it(); return true }
        val task = transfer ?: return maintenance
        if (!task()) synchronized(lock) {
            if (active === task) { active = null; bulkPaused = false }
        }
        return true
    }

    fun run() {
        while (true) {
            synchronized(lock) {
                while (!stopped && !maintenancePending && urgent.isEmpty() && controls.isEmpty() &&
                    (bulkPaused || (active == null && bulk.isEmpty()))) {
                    lock.wait()
                }
                if (stopped && !maintenancePending && urgent.isEmpty() && controls.isEmpty() && active == null && bulk.isEmpty()) return
            }
            try { step() } catch (error: Exception) { onTaskFailure(error) }
        }
    }

    fun close() = synchronized(lock) {
        stopped = true; bulkPaused = false; maintenancePending = true; lock.notifyAll()
    }
}
