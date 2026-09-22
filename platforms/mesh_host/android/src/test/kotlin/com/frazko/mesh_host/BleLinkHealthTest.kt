package com.frazko.mesh_host

import org.junit.Assert.*
import org.junit.Test

class BleLinkHealthTest {
    @Test fun probeWaitsForCurrentFrameButOvertakesVoiceBacklogWithoutDroppingAudio() {
        val scheduler = DurableRecordHistory()
        val writes = BleWriteQueue()
        val receiver = BleFrameCodec.Assembler()
        val voice = (0..2).map { index -> ByteArray(4096) { (it + index).toByte() } }
        val probe = byteArrayOf(0x7d, 0, 0, 0, 1)
        scheduler.enqueue(voice)
        writes.addAll(BleFrameCodec.split(scheduler.next()!!, 185))
        val first = writes.begin()!!
        assertNull(receiver.accept(first))
        scheduler.enqueue(listOf(probe))
        assertNull(writes.begin())
        writes.complete()
        val received = mutableListOf<ByteArray>()
        while (true) {
            if (writes.idle) {
                val record = scheduler.next() ?: break
                writes.addAll(BleFrameCodec.split(record, 185))
            }
            receiver.accept(writes.begin()!!)?.let { received += it }
            writes.complete()
        }
        val expected = listOf(voice[0], probe, voice[1], voice[2])
        assertEquals(expected.size, received.size)
        expected.zip(received).forEach { (a, b) -> assertArrayEquals(a, b) }
    }
    @Test fun handshakeCannotWaitForeverAndWritesDoNotProveLiveness() {
        val health = BleLinkHealth(100)
        health.received(11_000, false)
        assertFalse(health.probeDue(11_000))
        assertFalse(health.expired(12_099))
        assertTrue(health.expired(12_100))
    }
    @Test fun verifiedResponsesKeepLongAudioAndIdleSessionsAlive() {
        val health = BleLinkHealth(0)
        health.received(1_000, true)
        assertFalse(health.probeDue(2_999))
        for (time in 3_000L..60_000L step 3_000) {
            assertTrue(health.probeDue(time))
            assertFalse(health.probeDue(time + 1))
            health.received(time + 100, true)
            assertFalse(health.expired(time + 11_999))
        }
        assertTrue(health.expired(72_100))
        assertFalse(health.probeDue(72_100))
    }
    @Test fun restartRequiresNewProofAndIndependentPeersDoNotRefreshEachOther() {
        val stale = BleLinkHealth(0)
        val healthy = BleLinkHealth(0)
        stale.received(1, true)
        healthy.received(11_000, true)
        assertTrue(stale.expired(12_001))
        assertFalse(healthy.expired(12_001))
        val replacement = BleLinkHealth(12_001)
        assertFalse(replacement.probeDue(15_001))
        replacement.received(15_001, true)
        assertTrue(replacement.probeDue(15_001))
        assertFalse(BleLinkHealth().expired(BleLinkHealth.now()))
    }
}
