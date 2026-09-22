package com.frazko.mesh_host

import org.junit.Assert.*
import org.junit.Test

class BleWriteQueueTest {
    @Test fun newQuickMessageOvertakesReconnectBacklogAtRecordBoundary() {
        fun record(id: Int) = ByteArray(100).apply { this[0]=0x72; this[1]=1; this[99]=id.toByte() }
        val scheduler = DurableRecordHistory()
        val backlog = (0 until 100).map { record(it) }
        scheduler.enqueue(backlog)
        assertArrayEquals(backlog.first(), scheduler.next())
        scheduler.enqueue(listOf(record(101)) + backlog)
        assertArrayEquals(record(101), scheduler.next())
        backlog.drop(1).forEach { assertArrayEquals(it, scheduler.next()) }
        assertNull(scheduler.next())
    }

    @Test fun anotherSendCannotInterruptAnOutstandingAttWrite() {
        val queue = BleWriteQueue()
        assertTrue(queue.idle)
        queue.complete()
        queue.addAll(listOf(byteArrayOf(1), byteArrayOf(2)))
        assertFalse(queue.idle)
        assertArrayEquals(byteArrayOf(1), queue.begin())
        queue.addAll(listOf(byteArrayOf(3)))
        repeat(10) { assertNull(queue.begin()) }
        queue.complete()
        assertArrayEquals(byteArrayOf(2), queue.begin())
        queue.complete()
        assertArrayEquals(byteArrayOf(3), queue.begin())
        queue.complete()
        assertNull(queue.begin())
        assertTrue(queue.idle)
    }

    @Test fun backpressureRetainsTheExactFragmentUntilAccepted() {
        val queue = BleWriteQueue()
        queue.addAll(listOf(byteArrayOf(7), byteArrayOf(8)))
        repeat(20) {
            assertArrayEquals(byteArrayOf(7), queue.begin())
            queue.refused()
        }
        queue.begin(); queue.complete()
        assertArrayEquals(byteArrayOf(8), queue.begin())
    }

    @Test fun voiceAndFollowingQuickMessageSurviveFragmentationAndBackpressure() {
        val queue = BleWriteQueue()
        val expected = (0 until 48).map { index -> ByteArray(1024) { (index + it).toByte() } } + listOf(byteArrayOf(9, 8, 7))
        expected.forEach { queue.addAll(BleFrameCodec.split(it, 185)) }
        val receiver = BleFrameCodec.Assembler()
        val received = mutableListOf<ByteArray>()
        var fragments = 0
        while (true) {
            val fragment = queue.begin() ?: break
            if (fragments % 7 == 0) {
                queue.refused()
                assertArrayEquals(fragment, queue.begin())
            }
            assertNull(queue.begin())
            receiver.accept(fragment)?.let { received += it }
            queue.complete()
            fragments++
        }
        assertEquals(expected.size, received.size)
        expected.zip(received).forEach { (a, b) -> assertArrayEquals(a, b) }
    }

    @Test fun resendingTheOutboxDoesNotMultiplyQueuedRecordsAndReconnectAllowsReplay() {
        val history = DurableRecordHistory()
        val records = (0 until 520).map { ByteArray(100).apply { this[0]=0x72; this[1]=1; this[98]=(it shr 8).toByte(); this[99]=it.toByte() } }
        assertEquals(520, records.count { history.admit(it) })
        repeat(20) { assertEquals(0, records.count { history.admit(it) }) }
        val newer = records.first().clone().apply { this[50] = 1 }
        assertTrue(history.admit(newer))
        assertTrue(DurableRecordHistory().admit(records.first()))
        repeat(2) { assertTrue(history.admit(byteArrayOf(0x7d))) }
    }

    @Test fun historyRemainsBoundedAcrossLongSessions() {
        val history = DurableRecordHistory()
        fun record(id: Int) = ByteArray(100).apply { this[0]=0x72; this[1]=1; this[98]=(id shr 8).toByte(); this[99]=id.toByte() }
        for (id in 0..8192) assertTrue(history.admit(record(id)))
        assertTrue(history.admit(record(0)))
    }
}
