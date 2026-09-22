package com.frazko.mesh_host

import org.junit.Assert.*
import org.junit.Test

class EnrollmentRecordCodecTest {
    @Test fun enrollmentRoundTripKeepsKindAndPayload() {
        val payload = byteArrayOf(1, 2, 3, 0xf3.toByte())
        val encoded = EnrollmentRecordCodec.encode(EnrollmentRecordCodec.policy, payload)
        assertNotNull(encoded)
        val decoded = EnrollmentRecordCodec.decode(encoded!!)
        assertEquals(EnrollmentRecordCodec.policy, decoded?.kind)
        assertArrayEquals(payload, decoded?.payload)
    }

    @Test fun noiseRecordBeginningWithLegacyEnrollmentByteIsNotEnrollment() {
        val opaqueNoiseRecord = byteArrayOf(0xf3.toByte()) + ByteArray(38) { it.toByte() }
        assertNull(EnrollmentRecordCodec.decode(opaqueNoiseRecord))
    }

    @Test fun corruptedDomainAndOversizedRecordsAreRejected() {
        val encoded = EnrollmentRecordCodec.encode(EnrollmentRecordCodec.hello)!!
        encoded[0] = (encoded[0].toInt() xor 1).toByte()
        assertNull(EnrollmentRecordCodec.decode(encoded))
        assertNull(EnrollmentRecordCodec.encode(EnrollmentRecordCodec.policy, ByteArray(4160)))
    }
}
