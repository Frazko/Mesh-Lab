import Foundation

/// Ordered, bounded BLE segmentation around one already-framed Mesh Link record.
enum BleFrameCodec {
  private static let marker: UInt8 = 0x4d
  private static let start: UInt8 = 1
  private static let end: UInt8 = 2
  private static let header = 4
  private static let maxWire = 4163

  static func split(_ frame: [UInt8], maximumWrite: Int) -> [[UInt8]]? {
    guard (3...maxWire).contains(frame.count) else { return nil }
    let payload = max(1, maximumWrite - header)
    return stride(from: 0, to: frame.count, by: payload).map { offset in
      let count = min(payload, frame.count - offset)
      let flags: UInt8 = (offset == 0 ? start : 0) | (offset + count == frame.count ? end : 0)
      // The lower length byte is deliberately truncated. `UInt8(frame.count)`
      // traps in a release iOS build for every protected record above 255 bytes,
      // which made a normal voice fragment terminate the whole app.
      return [marker, flags, UInt8(frame.count >> 8), UInt8(truncatingIfNeeded: frame.count)] + Array(frame[offset..<(offset + count)])
    }
  }

  final class Assembler {
    private var expected = 0
    private var bytes = [UInt8]()

    func accept(_ fragment: Data) -> [UInt8]? { accept(Array(fragment)) }
    func accept(_ fragment: [UInt8]) -> [UInt8]? {
      guard fragment.count > BleFrameCodec.header, fragment[0] == BleFrameCodec.marker else { reset(); return nil }
      let flags = fragment[1]
      let total = Int(fragment[2]) << 8 | Int(fragment[3])
      guard (3...BleFrameCodec.maxWire).contains(total) else { reset(); return nil }
      if flags & BleFrameCodec.start != 0 { expected = total; bytes = [] }
      guard expected == total, expected != 0, bytes.count + fragment.count - BleFrameCodec.header <= expected else { reset(); return nil }
      bytes += fragment[BleFrameCodec.header...]
      guard flags & BleFrameCodec.end != 0 else { return nil }
      let output = bytes.count == expected ? bytes : nil
      reset()
      return output
    }
    func reset() { expected = 0; bytes = [] }
  }
}
