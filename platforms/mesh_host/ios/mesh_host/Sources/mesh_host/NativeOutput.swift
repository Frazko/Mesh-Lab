import Foundation
#if canImport(MeshEngine)
import MeshEngine
#endif

enum NativeFailure: Error { case status(Int32) }

/// Wire frames and completed durable objects are different boundaries. A
/// voice note crosses many 4 KiB frames but returns as one verified object.
enum NativeOutput {
  static let linkRecordLimit = 4163
  // 48 KiB plaintext plus completion header and the signed receipt.
  static let durableDeliveryLimit = 48 * 1024 + 2048

  static func copy(_ buffer: MeshBuffer, status: Int32, maximumBytes: Int) throws -> [UInt8] {
    guard status == 0 else { throw NativeFailure.status(status) }
    guard buffer.len <= maximumBytes, buffer.len == 0 || buffer.ptr != nil else {
      throw NativeFailure.status(1)
    }
    guard let pointer = buffer.ptr else { return [] }
    return Array(UnsafeBufferPointer(start: pointer, count: buffer.len))
  }
}
