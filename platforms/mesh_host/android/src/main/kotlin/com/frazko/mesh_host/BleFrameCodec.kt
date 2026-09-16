package com.frazko.mesh_host

/** Ordered, bounded BLE segmentation around one already-framed Mesh Link record. */
internal object BleFrameCodec {
    private const val marker = 0x4d
    private const val start = 1
    private const val end = 2
    private const val header = 4
    private const val maxWire = 4163

    fun split(frame: ByteArray, maximumWrite: Int): List<ByteArray> {
        require(frame.size in 3..maxWire)
        val payload = (maximumWrite - header).coerceAtLeast(1)
        return frame.asList().chunked(payload).mapIndexed { index, chunk ->
            val flags = (if (index == 0) start else 0) or
                (if ((index + 1) * payload >= frame.size) end else 0)
            byteArrayOf(marker.toByte(), flags.toByte(), (frame.size ushr 8).toByte(), frame.size.toByte()) +
                chunk.toByteArray()
        }
    }

    internal class Assembler {
        private var expected = 0
        private var bytes = ByteArray(0)

        fun accept(fragment: ByteArray): ByteArray? {
            if (fragment.size <= header || fragment[0].toInt() and 255 != marker) { reset(); return null }
            val flags = fragment[1].toInt() and 255
            val total = ((fragment[2].toInt() and 255) shl 8) or (fragment[3].toInt() and 255)
            if (total !in 3..maxWire) { reset(); return null }
            if (flags and start != 0) { expected = total; bytes = ByteArray(0) }
            if (expected != total || expected == 0 || bytes.size + fragment.size - header > expected) { reset(); return null }
            bytes += fragment.copyOfRange(header, fragment.size)
            if (flags and end == 0) return null
            val output = bytes.takeIf { it.size == expected }
            reset()
            return output
        }

        fun reset() { expected = 0; bytes = ByteArray(0) }
    }
}
