package com.frazko.mesh_host

/**
 * Domain-separated public enrollment records carried before a Noise session.
 *
 * A single marker byte is not sufficient: Noise handshake records are opaque
 * and can legitimately start with any byte. The 128-bit prefix makes an
 * encrypted handshake record unambiguously different from enrollment traffic.
 */
internal object EnrollmentRecordCodec {
    const val hello = 0xf0
    const val invitation = 0xf1
    const val request = 0xf2
    const val policy = 0xf3

    private val domain = byteArrayOf(
        0x4d, 0x45, 0x53, 0x48, 0x2d, 0x45, 0x4e, 0x52,
        0x4f, 0x4c, 0x4c, 0x2d, 0x76, 0x31, 0x00, 0x7f,
    )
    private const val maxRawBytes = 4160

    data class Record(val kind: Int, val payload: ByteArray)

    fun encode(kind: Int, payload: ByteArray = ByteArray(0)): ByteArray? {
        if (kind !in hello..policy || domain.size + 1 + payload.size > maxRawBytes) return null
        return domain + byteArrayOf(kind.toByte()) + payload
    }

    fun decode(raw: ByteArray): Record? {
        if (raw.size < domain.size + 1) return null
        if (!raw.copyOfRange(0, domain.size).contentEquals(domain)) return null
        val kind = raw[domain.size].toInt() and 255
        if (kind !in hello..policy) return null
        return Record(kind, raw.copyOfRange(domain.size + 1, raw.size))
    }
}
