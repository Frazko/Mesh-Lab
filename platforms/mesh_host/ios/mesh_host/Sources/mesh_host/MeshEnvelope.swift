import Foundation

struct FoundationInfo { let version: String; let abi: Int64; let api: Int64; let phase: String; let build: String }
struct FoundationEvent { let sequence: Int64; let request: Int64; let kind: Int64 }
struct FoundationSnapshot {
  let runtime: Int64; let cursor: Int64; let probes: Int64; let state: Int64
  let reset: Bool; let events: [FoundationEvent]
}
enum EnvelopeError: Error { case invalid }

/// Strict, bounded decoder for schema/api/foundation.cddl, not a general CBOR parser.
struct MeshEnvelope {
  private let bytes: [UInt8]
  private var offset = 0
  init(_ bytes: [UInt8]) throws {
    guard bytes.count <= 16384 else { throw EnvelopeError.invalid }
    self.bytes = bytes
  }
  private mutating func byte() throws -> UInt8 {
    guard offset < bytes.count else { throw EnvelopeError.invalid }
    defer { offset += 1 }; return bytes[offset]
  }
  private mutating func value(_ major: UInt8) throws -> UInt64 {
    let h = try byte(); guard h >> 5 == major else { throw EnvelopeError.invalid }
    let tag = h & 31
    if tag < 24 { return UInt64(tag) }
    let count: Int; let minimum: UInt64
    switch tag {
    case 24: count = 1; minimum = 24
    case 25: count = 2; minimum = 256
    case 26: count = 4; minimum = 65536
    case 27: count = 8; minimum = 4294967296
    default: throw EnvelopeError.invalid
    }
    var n: UInt64 = 0
    for _ in 0..<count { n = (n << 8) | UInt64(try byte()) }
    guard n >= minimum && n <= 9007199254740991 else { throw EnvelopeError.invalid }
    return n
  }
  private mutating func uint() throws -> Int64 { Int64(try value(0)) }
  private mutating func array(_ count: UInt64) throws {
    guard try value(4) == count else { throw EnvelopeError.invalid }
  }
  private mutating func string() throws -> String {
    let count = Int(try value(3))
    guard count <= 128, offset + count <= bytes.count,
          let s = String(bytes: bytes[offset..<(offset + count)], encoding: .utf8) else { throw EnvelopeError.invalid }
    offset += count; return s
  }
  private mutating func prefix(_ method: Int64) throws {
    try array(3)
    guard try uint() == 1, try uint() == method else { throw EnvelopeError.invalid }
  }
  private func end() throws { guard offset == bytes.count else { throw EnvelopeError.invalid } }
  mutating func info() throws -> FoundationInfo {
    try prefix(0); try array(5)
    let info = FoundationInfo(version: try string(), abi: try uint(), api: try uint(), phase: try string(), build: try string())
    guard info.abi == 1, info.api == 1, info.phase == "F0" else { throw EnvelopeError.invalid }
    try end(); return info
  }
  mutating func snapshot(method: Int64) throws -> FoundationSnapshot {
    try prefix(method); try array(6)
    let runtime = try uint(), cursor = try uint(), probes = try uint(), state = try uint()
    guard runtime > 0, cursor == probes, state == 0 else { throw EnvelopeError.invalid }
    let flag = try byte(); guard flag == 0xf4 || flag == 0xf5 else { throw EnvelopeError.invalid }
    let count = try value(4); guard count <= 64 else { throw EnvelopeError.invalid }
    var events: [FoundationEvent] = []
    for _ in 0..<count {
      try array(3)
      let event = FoundationEvent(sequence: try uint(), request: try uint(), kind: try uint())
      guard event.sequence > 0, event.sequence <= cursor, event.request > 0, event.kind == 0,
            events.last.map({ $0.sequence + 1 == event.sequence }) ?? true else { throw EnvelopeError.invalid }
      events.append(event)
    }
    guard flag != 0xf5 || events.isEmpty else { throw EnvelopeError.invalid }
    try end()
    return FoundationSnapshot(runtime: runtime, cursor: cursor, probes: probes, state: state, reset: flag == 0xf5, events: events)
  }
  static func request(method: UInt8, argument: Int64 = 0) throws -> [UInt8] {
    guard method <= 2, argument >= 0, argument <= 9007199254740991 else { throw EnvelopeError.invalid }
    let n = UInt64(argument); var bytes: [UInt8] = [0x83, 1, method]
    if n < 24 { bytes.append(UInt8(n)) }
    else {
      let count: Int; let tag: UInt8
      if n <= 255 { count = 1; tag = 24 }
      else if n <= 65535 { count = 2; tag = 25 }
      else if n <= 4294967295 { count = 4; tag = 26 }
      else { count = 8; tag = 27 }
      bytes.append(tag)
      for i in (0..<count).reversed() { bytes.append(UInt8(truncatingIfNeeded: n >> (i * 8))) }
    }
    return bytes
  }
}
