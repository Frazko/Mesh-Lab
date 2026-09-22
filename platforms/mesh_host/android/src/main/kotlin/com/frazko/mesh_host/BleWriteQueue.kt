package com.frazko.mesh_host

/** ATT writes/notifications own one fragment until their completion callback.
 * Enqueuing another record must never start a second write or drop the first. */
internal class BleWriteQueue {
    private val pending = ArrayDeque<ByteArray>()
    private var inFlight = false
    val idle: Boolean get() = !inFlight && pending.isEmpty()

    fun addAll(parts: List<ByteArray>) { pending.addAll(parts) }
    fun begin(): ByteArray? {
        if (inFlight || pending.isEmpty()) return null
        inFlight = true
        return pending.first()
    }
    fun complete() {
        if (!inFlight) return
        pending.removeFirst()
        inFlight = false
    }
    fun refused() { inFlight = false }
}

/** Durable retransmission belongs to a new transport session. Repeated product
 * sends on a healthy reliable link must not append the entire outbox again. */
internal class DurableRecordHistory {
    private val records = LinkedHashSet<String>()
    private val pending = ArrayDeque<ByteArray>()
    fun enqueue(payloads: List<ByteArray>) {
        // Prioritize newly submitted records before the unsent reconnect tail.
        // This is plaintext: Noise counters are assigned only at dequeue.
        val fresh = payloads.filter { admit(it) }
        fresh.asReversed().forEach { pending.addFirst(it) }
    }
    fun next(): ByteArray? = if (pending.isEmpty()) null else pending.removeFirst()
    fun admit(payload: ByteArray): Boolean {
        if (payload.size < 96 || payload[0] != 0x72.toByte() || payload[1] != 1.toByte()) return true
        val key = java.security.MessageDigest.getInstance("SHA-256")
            .digest(payload).joinToString("") { "%02x".format(it) }
        if (!records.add(key)) return false
        if (records.size > 8192) records.remove(records.first())
        return true
    }
}
