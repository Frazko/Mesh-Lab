import Foundation
import CryptoKit

/// Keeps the head fragment until ATT completion (or notification acceptance).
/// New records cannot interrupt an in-flight write or reorder Noise counters.
final class BleWriteQueue {
  private var pending = [[UInt8]]()
  private var head = 0
  private var inFlight = false
  var idle: Bool { !inFlight && head == pending.count }

  func append(_ parts: [[UInt8]]) { pending.append(contentsOf: parts) }
  func begin() -> [UInt8]? {
    guard !inFlight, head < pending.count else { return nil }
    inFlight = true
    return pending[head]
  }
  func complete() {
    guard inFlight else { return }
    head += 1
    inFlight = false
    if head == pending.count { pending.removeAll(keepingCapacity: true); head = 0 }
  }
  func refused() { inFlight = false }
}

final class DurableRecordHistory {
  private var records = Set<Data>()
  private var order = [Data]()
  private var pending = [[UInt8]]()
  func enqueue(_ payloads: [[UInt8]]) {
    // Assign Noise counters only when removing the next record for the radio.
    pending.insert(contentsOf: payloads.filter { admit($0) }, at: 0)
  }
  func next() -> [UInt8]? { pending.isEmpty ? nil : pending.removeFirst() }
  func admit(_ payload: [UInt8]) -> Bool {
    guard payload.count >= 96, payload[0] == 0x72, payload[1] == 1 else { return true }
    let key = Data(SHA256.hash(data: Data(payload)))
    guard records.insert(key).inserted else { return false }
    order.append(key)
    if order.count > 8192 { records.remove(order.removeFirst()) }
    return true
  }
}
