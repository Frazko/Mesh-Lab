import Foundation

/// Only authenticated inbound traffic proves the peer is still alive.
/// ATT write completion and a cached CoreBluetooth connection do not.
final class BleLinkHealth {
  private let began: TimeInterval
  private var lastVerified: TimeInterval?
  private var lastProbe: TimeInterval
  init(now: TimeInterval = ProcessInfo.processInfo.systemUptime) {
    began = now
    lastProbe = now
  }
  func received(now: TimeInterval, authenticated: Bool) {
    if authenticated { lastVerified = now }
  }
  func expired(now: TimeInterval) -> Bool { now - (lastVerified ?? began) >= 12 }
  func probeDue(now: TimeInterval) -> Bool {
    guard lastVerified != nil, !expired(now: now), now - lastProbe >= 3 else { return false }
    lastProbe = now
    return true
  }
}
