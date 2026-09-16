package com.frazko.mesh_host

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction

data class FoundationInfo(val version: String, val abi: Long, val api: Long, val phase: String, val build: String)
data class FoundationEvent(val sequence: Long, val request: Long, val kind: Long)
data class FoundationSnapshot(val runtime: Long, val cursor: Long, val probes: Long, val state: Long, val reset: Boolean, val events: List<FoundationEvent>)

/** Strict bounded local API codec. Never parses a mesh radio frame. */
class MeshEnvelope(private val bytes: ByteArray) {
    private var offset = 0
    init { require(bytes.size <= 16384) }
    private fun byte(): Int { require(offset < bytes.size); return bytes[offset++].toInt() and 255 }
    private fun value(major: Int): Long {
        val h = byte(); require(h shr 5 == major)
        val tag = h and 31
        if (tag < 24) return tag.toLong()
        val (count, minimum) = when (tag) {
            24 -> 1 to 24L; 25 -> 2 to 256L; 26 -> 4 to 65536L; 27 -> 8 to 4294967296L
            else -> error("Invalid CBOR")
        }
        var n = 0L
        repeat(count) {
            require(n <= MAX_COUNTER ushr 8)
            n = (n shl 8) or byte().toLong()
        }
        require(n in minimum..MAX_COUNTER); return n
    }
    private fun uint() = value(0)
    private fun array(size: Long) { require(value(4) == size) }
    private fun string(): String {
        val count = value(3); require(count <= 128 && count <= bytes.size - offset)
        val decoder = Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT).onUnmappableCharacter(CodingErrorAction.REPORT)
        val text = decoder.decode(ByteBuffer.wrap(bytes, offset, count.toInt())).toString()
        offset += count.toInt(); return text
    }
    private fun prefix(method: Long) { array(3); require(uint() == 1L && uint() == method) }
    private fun end() { require(offset == bytes.size) }
    fun info(): FoundationInfo {
        prefix(0); array(5)
        val info = FoundationInfo(string(), uint(), uint(), string(), string())
        require(info.abi == 1L && info.api == 1L && info.phase == "F0"); end(); return info
    }
    fun snapshot(method: Long): FoundationSnapshot {
        prefix(method); array(6)
        val runtime = uint(); val cursor = uint(); val probes = uint(); val state = uint()
        require(runtime > 0 && cursor == probes && state == 0L)
        val flag = byte(); require(flag == 0xf4 || flag == 0xf5)
        val count = value(4); require(count <= 64)
        val events = mutableListOf<FoundationEvent>()
        repeat(count.toInt()) {
            array(3)
            val event = FoundationEvent(uint(), uint(), uint())
            require(event.sequence in 1..cursor && event.request > 0 && event.kind == 0L)
            require(events.lastOrNull()?.let { it.sequence + 1 == event.sequence } ?: true)
            events.add(event)
        }
        require(flag != 0xf5 || events.isEmpty()); end()
        return FoundationSnapshot(runtime, cursor, probes, state, flag == 0xf5, events)
    }
    companion object {
        const val MAX_COUNTER = 9007199254740991L
        fun request(method: Int, argument: Long = 0): ByteArray {
            require(method in 0..2 && argument in 0..MAX_COUNTER)
            val out = ByteArrayOutputStream(); out.write(0x83); out.write(1); out.write(method)
            if (argument < 24) out.write(argument.toInt()) else {
                val (count, tag) = when {
                    argument <= 255 -> 1 to 24
                    argument <= 65535 -> 2 to 25
                    argument <= 4294967295L -> 4 to 26
                    else -> 8 to 27
                }
                out.write(tag)
                for (i in count - 1 downTo 0) out.write((argument ushr (i * 8)).toInt() and 255)
            }
            return out.toByteArray()
        }
    }
}
