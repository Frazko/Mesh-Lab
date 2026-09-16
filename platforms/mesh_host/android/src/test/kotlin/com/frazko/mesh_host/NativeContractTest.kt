package com.frazko.mesh_host

import org.junit.Assert.*
import org.junit.Test

class NativeContractTest {
    @Test fun identityPublicKeyUsesNativeRustAndRejectsWrongSize() {
        val expected = "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29"
        assertEquals(expected, NativeBridge.identityPublic(ByteArray(32)).joinToString("") { "%02x".format(it.toInt() and 255) })
        for (size in listOf(0,31,33,1000)) {
            try { NativeBridge.identityPublic(ByteArray(size)); fail("Invalid seed accepted") }
            catch (e: IllegalStateException) { assertEquals("MESH_1", e.message) }
        }
    }
    @Test fun jniRoundTripAndIdempotentReplay() {
        assertEquals(1, NativeBridge.abiVersion())
        repeat(1000) {
            val h = NativeBridge.create(1)
            try {
                val info = MeshEnvelope(NativeBridge.request(h, MeshEnvelope.request(0))).info()
                assertEquals("F0", info.phase)
                val before = MeshEnvelope(NativeBridge.request(h, MeshEnvelope.request(1))).snapshot(1)
                assertEquals(0L, before.cursor)
                val after = MeshEnvelope(NativeBridge.request(h, MeshEnvelope.request(2, 42))).snapshot(2)
                assertEquals(h, after.runtime); assertEquals(1L, after.cursor)
                assertEquals(42L, after.events.single().request)
                val retry = MeshEnvelope(NativeBridge.request(h, MeshEnvelope.request(2, 42))).snapshot(2)
                assertEquals(1L, retry.cursor); assertTrue(retry.events.isEmpty())
            } finally { NativeBridge.release(h) }
            try { NativeBridge.request(h, MeshEnvelope.request(0)); fail("Stale handle accepted") }
            catch (e: IllegalStateException) { assertEquals("MESH_3", e.message) }
        }
    }
    @Test fun fixedVectorAndMalformedEnvelopes() {
        val valid = byteArrayOf(0x83.toByte(),1,1,0x86.toByte(),1,0,0,0,0xf4.toByte(),0x80.toByte())
        assertEquals(1L, MeshEnvelope(valid).snapshot(1).runtime)
        assertArrayEquals(byteArrayOf(0x83.toByte(),1,2,0x18,0x18), MeshEnvelope.request(2,24))
        for (end in 0 until valid.size) {
            try { MeshEnvelope(valid.copyOf(end)).snapshot(1); fail("Truncation accepted") }
            catch (_: IllegalArgumentException) { }
        }
        try { MeshEnvelope(valid + byteArrayOf(0)).snapshot(1); fail("Trailing byte accepted") }
        catch (_: IllegalArgumentException) { }
    }
    @Test fun routedRecordJniKeepsTheRouteAttachedToTheDurablePayload() {
        val route = ByteArray(91)
        route[0] = 1
        (1..16).forEach { route[it] = 5 }
        (17..48).forEach { route[it] = 1 }
        (49..80).forEach { route[it] = 2 }
        route[81] = 1; route[82] = 4; route[90] = 100
        val record = byteArrayOf(0x6d, 1, 1, 0, 3, 9, 8, 7)
        val encoded = NativeBridge.routedRecordEncode(route, record)
        assertEquals(2 + route.size + record.size, encoded.size)
        val decoded = NativeBridge.routedRecordDecode(encoded)
        assertArrayEquals(route + record, decoded)
        try { NativeBridge.routedRecordEncode(route.copyOf(90), record); fail("Short route accepted") }
        catch (e: IllegalStateException) { assertEquals("MESH_1", e.message) }
        try { NativeBridge.routedRecordDecode(encoded.copyOf(encoded.size - 1)); fail("Truncated record accepted") }
        catch (e: IllegalStateException) { assertEquals("MESH_1", e.message) }
    }
}
