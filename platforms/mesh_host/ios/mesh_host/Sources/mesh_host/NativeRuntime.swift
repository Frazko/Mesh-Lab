import Foundation
import MeshEngine

enum NativeFailure: Error { case status(Int32) }
/// Process-owned diagnostic runtime. The serial queue survives Dart hot restarts.
final class NativeRuntime {
  static let shared = NativeRuntime()
  let queue = DispatchQueue(label: "com.frazko.mesh-lab.runtime", qos: .userInitiated)
  private var handle: UInt64 = 0
  private var storeHandle: UInt64 = 0
  private var relayGate: UInt64 = 0
  private let labScope = "lab"
  private var storeScope = "lab"
  var hasStore: Bool { storeHandle != 0 }

  func request(method: UInt8, argument: Int64 = 0) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    if handle == 0 {
      guard mesh_abi_version() == 1 else { throw NativeFailure.status(2) }
      let status = mesh_runtime_create(1, &handle)
      guard status == 0 else { throw NativeFailure.status(status) }
    }
    let bytes = try MeshEnvelope.request(method: method, argument: argument)
    var buffer = MeshBuffer(ptr: nil, len: 0)
    let status = bytes.withUnsafeBufferPointer { input in
      mesh_runtime_request(handle, input.baseAddress, input.count, &buffer)
    }
    defer { mesh_buffer_release(buffer) }
    guard status == 0 else { throw NativeFailure.status(status) }
    guard buffer.len <= 16384, let ptr = buffer.ptr else { throw NativeFailure.status(1) }
    return Array(UnsafeBufferPointer(start: ptr, count: buffer.len))
  }
  func prepareStore(_ material: SecureStoreMaterial) throws {
    dispatchPrecondition(condition: .onQueue(queue))
    var material = material
    defer { material.wipe() }
    if storeHandle != 0 { return }
    let root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask,
                                           appropriateFor: nil, create: true)
    var directory = root.appendingPathComponent("mesh-store", isDirectory: true)
      .appendingPathComponent(storeScope, isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true,
                                            attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
    var values = URLResourceValues(); values.isExcludedFromBackup = true
    try directory.setResourceValues(values)
    let path = directory.appendingPathComponent("state-v1.db").path.data(using: .utf8)!
    var opened: UInt64 = 0
    let status = material.databaseKey.withUnsafeBytes { key in
      material.member.withUnsafeBufferPointer { member in
        path.withUnsafeBytes { path in
          mesh_secure_store_open(key.bindMemory(to: UInt8.self).baseAddress, key.count,
                                 member.baseAddress, member.count,
                                 path.bindMemory(to: UInt8.self).baseAddress, path.count, &opened)
        }
      }
    }
    guard status == 0, opened != 0 else { throw NativeFailure.status(status) }
    storeHandle = opened
  }

  private func releaseStore() {
    dispatchPrecondition(condition: .onQueue(queue))
    if storeHandle != 0 {
      _ = mesh_secure_store_release(storeHandle)
      storeHandle = 0
    }
    if relayGate != 0 {
      _ = mesh_relay_gate_release(relayGate)
      relayGate = 0
    }
  }

  func productScopeWillChange(_ scope: String) throws -> Bool {
    dispatchPrecondition(condition: .onQueue(queue))
    guard scope.range(of: "^[0-9a-f]{32}$", options: .regularExpression) != nil else {
      throw NativeFailure.status(1)
    }
    return scope != storeScope
  }

  /// Uses an encrypted store selected by an opaque product scope. A previous
  /// Convoy group remains isolated on disk and cannot become the active radio
  /// group for a later convoy.
  func selectProductScope(_ scope: String, material rawMaterial: SecureStoreMaterial) throws {
    dispatchPrecondition(condition: .onQueue(queue))
    var material = rawMaterial
    defer { material.wipe() }
    guard scope.range(of: "^[0-9a-f]{32}$", options: .regularExpression) != nil else {
      throw NativeFailure.status(1)
    }
    if storeScope == scope && storeHandle != 0 {
      return
    }
    releaseStore()
    storeScope = scope
    try prepareStore(material)
  }

  /// Returning to the lab scope closes the former product radio policy. The
  /// encrypted product database stays available only under its original scope
  /// for local audit/rejoin decisions made by the product.
  func clearProductScope() {
    dispatchPrecondition(condition: .onQueue(queue))
    releaseStore()
    storeScope = labScope
  }

  /// This lab has one Android authority. If an iPhone still carries a group
  /// created during an earlier failed setup, discard only the encrypted policy
  /// database and reopen it with the same Keychain-backed identity. The
  /// identity itself is deliberately never reset.
  func resetPolicyForLab() throws {
    dispatchPrecondition(condition: .onQueue(queue))
    releaseStore()
    storeScope = labScope
    let root = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask,
                                           appropriateFor: nil, create: true)
    let database = root.appendingPathComponent("mesh-store", isDirectory: true)
      .appendingPathComponent(labScope, isDirectory: true)
      .appendingPathComponent("state-v1.db")
    for suffix in ["", "-wal", "-shm"] {
      try? FileManager.default.removeItem(atPath: database.path + suffix)
    }
    try prepareStore(try SecureIdentity().storeMaterial())
  }
  func policyEpoch() throws -> Int64 {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    var epoch: UInt64 = 0
    let status = mesh_secure_store_policy_epoch(storeHandle, UInt64(Date().timeIntervalSince1970), &epoch)
    guard status == 0, epoch <= UInt64(Int64.max) else { throw NativeFailure.status(status) }
    return Int64(epoch)
  }
  /// Persists one routed record after the caller's Noise session authenticated
  /// its neighbor. Returns a scheduling hint only; it is not UI delivery.
  func acceptRoutedRecord(_ bytes: [UInt8], receivedFrom: [UInt8]) throws -> UInt8 {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 && !bytes.isEmpty && bytes.count <= 4096 && receivedFrom.count == 32 else {
      throw NativeFailure.status(1)
    }
    var accepted: UInt8 = 0
    let status = bytes.withUnsafeBufferPointer { input in
      receivedFrom.withUnsafeBufferPointer { receivedFrom in
        mesh_secure_store_accept_routed(storeHandle, input.baseAddress, input.count,
                                        receivedFrom.baseAddress, receivedFrom.count,
                                        UInt64(Date().timeIntervalSince1970), &accepted)
      }
    }
    guard status == 0 else { throw NativeFailure.status(status) }
    return accepted
  }
  /// Atomically seals and queues every certified recipient audience for one
  /// visible text action. Keychain material is wiped as soon as native work ends.
  func enqueueDurableText(_ bytes: [UInt8], material: GroupMaterial) throws -> UInt16 {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 && !bytes.isEmpty && bytes.count <= 48 * 1024 else {
      throw NativeFailure.status(1)
    }
    var material = material
    defer { material.wipe() }
    var count: UInt16 = 0
    let status = material.identitySeed.withUnsafeBytes { identity in
      bytes.withUnsafeBufferPointer { text in
        mesh_secure_store_enqueue_text(storeHandle,
                                       identity.bindMemory(to: UInt8.self).baseAddress, identity.count,
                                       text.baseAddress, text.count,
                                       UInt64(Date().timeIntervalSince1970), &count)
      }
    }
    guard status == 0 else { throw NativeFailure.status(status) }
    return count
  }
  func enqueueDurableText(_ bytes: [UInt8], logicalId: [UInt8], material: GroupMaterial) throws -> UInt16 {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 && !bytes.isEmpty && bytes.count <= 48 * 1024 && logicalId.count == 16 else {
      throw NativeFailure.status(1)
    }
    var material = material
    defer { material.wipe() }
    var count: UInt16 = 0
    let status = material.identitySeed.withUnsafeBytes { identity in
      bytes.withUnsafeBufferPointer { text in
        logicalId.withUnsafeBufferPointer { logical in
          mesh_secure_store_enqueue_text_with_logical_id(storeHandle,
            identity.bindMemory(to: UInt8.self).baseAddress, identity.count,
            text.baseAddress, text.count, logical.baseAddress, logical.count,
            UInt64(Date().timeIntervalSince1970), &count)
        }
      }
    }
    guard status == 0 else { throw NativeFailure.status(status) }
    return count
  }
  func latestDeliverySummary() throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    let bytes = try sessionOutput {
      mesh_secure_store_latest_delivery_summary(storeHandle, UInt64(Date().timeIntervalSince1970), $0)
    }
    guard bytes.isEmpty || bytes.count == 19 else { throw NativeFailure.status(1) }
    return bytes
  }
  func deliverySummary(_ logicalId: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0, logicalId.count == 16 else { throw NativeFailure.status(3) }
    let bytes = try logicalId.withUnsafeBufferPointer { logical in
      try sessionOutput {
        mesh_secure_store_delivery_summary(storeHandle, logical.baseAddress, logical.count,
                                           UInt64(Date().timeIntervalSince1970), $0)
      }
    }
    guard bytes.isEmpty || bytes.count == 19 else { throw NativeFailure.status(1) }
    return bytes
  }
  func outboxRecord(slot: UInt16) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    return try sessionOutput {
      mesh_secure_store_outbox_record(storeHandle, slot,
                                      UInt64(Date().timeIntervalSince1970), $0)
    }
  }
  func receiptRecord(slot: UInt16) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    return try sessionOutput {
      mesh_secure_store_receipt_record(storeHandle, slot,
                                       UInt64(Date().timeIntervalSince1970), $0)
    }
  }
  func receiptAckRecord(slot: UInt16, material: GroupMaterial) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    var material = material
    defer { material.wipe() }
    return try sessionOutput { output in
      material.identitySeed.withUnsafeBytes { identity in
        mesh_secure_store_receipt_ack_record(storeHandle,
                                              identity.bindMemory(to: UInt8.self).baseAddress, identity.count,
                                              slot, UInt64(Date().timeIntervalSince1970), output)
      }
    }
  }
  /// The returned completion packet remains in the native host until the chat
  /// reducer has parsed it; an empty packet means no complete incoming text.
  func finalizeNextDurableText(material: GroupMaterial) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    var material = material
    defer { material.wipe() }
    return try sessionOutput { output in
      material.identitySeed.withUnsafeBytes { identity in
        material.deliverySeed.withUnsafeBytes { delivery in
          mesh_secure_store_finalize_next_text(storeHandle,
                                                identity.bindMemory(to: UInt8.self).baseAddress, identity.count,
                                                delivery.bindMemory(to: UInt8.self).baseAddress, delivery.count,
                                                UInt64(Date().timeIntervalSince1970), output)
        }
      }
    }
  }
  /// Empty is the normal result when the durable relay queue has no record at
  /// this slot. This remains host-only until a verified neighbor is selected.
  func relayRecord(slot: UInt16) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    return try sessionOutput {
      mesh_secure_store_relay_record(storeHandle, slot,
                                     UInt64(Date().timeIntervalSince1970), $0)
    }
  }
  func relayReceivedFrom() throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    let member = try sessionOutput {
      mesh_secure_store_relay_received_from(storeHandle,
                                             UInt64(Date().timeIntervalSince1970), $0)
    }
    guard member.isEmpty || member.count == 32 else { throw NativeFailure.status(1) }
    return member
  }
  func relayReceiptRecord(slot: UInt16) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    return try sessionOutput {
      mesh_secure_store_relay_receipt_record(storeHandle, slot,
                                             UInt64(Date().timeIntervalSince1970), $0)
    }
  }
  func relayReceiptReceivedFrom() throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    let member = try sessionOutput {
      mesh_secure_store_relay_receipt_received_from(storeHandle,
                                                     UInt64(Date().timeIntervalSince1970), $0)
    }
    guard member.isEmpty || member.count == 32 else { throw NativeFailure.status(1) }
    return member
  }
  func relayReceiptAckRecord(slot: UInt16) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    return try sessionOutput {
      mesh_secure_store_relay_receipt_ack_record(storeHandle, slot,
                                                  UInt64(Date().timeIntervalSince1970), $0)
    }
  }
  func relayReceiptAckReceivedFrom() throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    let member = try sessionOutput {
      mesh_secure_store_relay_receipt_ack_received_from(storeHandle,
                                                         UInt64(Date().timeIntervalSince1970), $0)
    }
    guard member.isEmpty || member.count == 32 else { throw NativeFailure.status(1) }
    return member
  }
  func openRelayGate(member: [UInt8]) throws {
    dispatchPrecondition(condition: .onQueue(queue))
    guard member.count == 32 else { throw NativeFailure.status(1) }
    if relayGate != 0 { return }
    let status = member.withUnsafeBufferPointer { mesh_relay_gate_open($0.baseAddress, $0.count, &relayGate) }
    guard status == 0 && relayGate != 0 else { throw NativeFailure.status(status) }
  }
  func acceptRelayFrame(_ frame: [UInt8], via: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard relayGate != 0 && frame.count == 91 && via.count == 32 else { throw NativeFailure.status(1) }
    return try sessionOutput { output in
      frame.withUnsafeBufferPointer { frame in via.withUnsafeBufferPointer { via in
        mesh_relay_gate_accept(relayGate, frame.baseAddress, frame.count, via.baseAddress, via.count,
                               UInt64(Date().timeIntervalSince1970), output)
      }}
    }
  }
  func createGroup(_ material: GroupMaterial) throws -> Int64 {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    var material = material
    defer { material.wipe() }
    var epoch: UInt64 = 0
    let now = UInt64(Date().timeIntervalSince1970)
    let status = material.identitySeed.withUnsafeBytes { identity in
      material.deliverySeed.withUnsafeBytes { delivery in
        material.member.withUnsafeBufferPointer { member in
          mesh_secure_store_create_group(
            storeHandle,
            identity.bindMemory(to: UInt8.self).baseAddress, identity.count,
            delivery.bindMemory(to: UInt8.self).baseAddress, delivery.count,
            member.baseAddress, member.count, now, &epoch)
        }
      }
    }
    guard status == 0, epoch > 0, epoch <= UInt64(Int64.max) else {
      throw NativeFailure.status(status)
    }
    return Int64(epoch)
  }
  func exportInvitation() throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    var buffer = MeshBuffer(ptr: nil, len: 0)
    let status = mesh_secure_store_export_policy(storeHandle, UInt64(Date().timeIntervalSince1970), &buffer)
    defer { mesh_buffer_release(buffer) }
    guard status == 0, buffer.len <= 12 * 1024, let pointer = buffer.ptr else { throw NativeFailure.status(status) }
    return Array(UnsafeBufferPointer(start: pointer, count: buffer.len))
  }
  func createEnrollmentRequest(_ material: GroupMaterial, invitation: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    var material = material
    defer { material.wipe() }
    guard !invitation.isEmpty && invitation.count <= 12 * 1024 else { throw NativeFailure.status(1) }
    var buffer = MeshBuffer(ptr: nil, len: 0)
    let status = material.identitySeed.withUnsafeBytes { identity in
      material.deliverySeed.withUnsafeBytes { delivery in
        invitation.withUnsafeBufferPointer { invitation in
          mesh_create_enrollment_request(identity.bindMemory(to: UInt8.self).baseAddress, identity.count,
                                          delivery.bindMemory(to: UInt8.self).baseAddress, delivery.count,
                                          invitation.baseAddress, invitation.count,
                                          UInt64(Date().timeIntervalSince1970), &buffer)
        }
      }
    }
    defer { mesh_buffer_release(buffer) }
    guard status == 0, buffer.len <= 512, let pointer = buffer.ptr else { throw NativeFailure.status(status) }
    return Array(UnsafeBufferPointer(start: pointer, count: buffer.len))
  }
  func issueEnrollment(_ material: GroupMaterial, request: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 && !request.isEmpty && request.count <= 512 else { throw NativeFailure.status(1) }
    var material = material
    defer { material.wipe() }
    var buffer = MeshBuffer(ptr: nil, len: 0)
    let status = material.identitySeed.withUnsafeBytes { identity in
      request.withUnsafeBufferPointer { request in
        mesh_secure_store_issue_enrollment(storeHandle, identity.bindMemory(to: UInt8.self).baseAddress,
                                            identity.count, request.baseAddress, request.count,
                                            UInt64(Date().timeIntervalSince1970), &buffer)
      }
    }
    defer { mesh_buffer_release(buffer) }
    guard status == 0, buffer.len <= 12 * 1024, let pointer = buffer.ptr else { throw NativeFailure.status(status) }
    return Array(UnsafeBufferPointer(start: pointer, count: buffer.len))
  }
  /// Returns only whether this Keychain-backed identity still matches the
  /// authority certified in the active policy. Keys and roster remain native.
  func canIssueEnrollment(_ material: GroupMaterial) throws -> Bool {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    var material = material
    defer { material.wipe() }
    var canIssue: UInt8 = 0
    let status = material.identitySeed.withUnsafeBytes { identity in
      mesh_secure_store_can_issue_enrollment(
        storeHandle,
        identity.bindMemory(to: UInt8.self).baseAddress,
        identity.count,
        UInt64(Date().timeIntervalSince1970),
        &canIssue
      )
    }
    guard status == 0 else { throw NativeFailure.status(status) }
    return canIssue == 1
  }
  func signCloudRelay(_ canonical: [UInt8], material: GroupMaterial) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0, !canonical.isEmpty, canonical.count <= 64 * 1024 else { throw NativeFailure.status(3) }
    var material = material
    defer { material.wipe() }
    let proof = try sessionOutput { output in
      material.identitySeed.withUnsafeBytes { identity in
        canonical.withUnsafeBufferPointer { canonical in
          mesh_secure_store_sign_cloud_relay(storeHandle,
            identity.bindMemory(to: UInt8.self).baseAddress, identity.count,
            canonical.baseAddress, canonical.count,
            UInt64(Date().timeIntervalSince1970), output)
        }
      }
    }
    guard proof.count == 104 else { throw NativeFailure.status(1) }
    return proof
  }
  /// Signs a public, short-lived authority handoff. The successor has to be an
  /// already certified local group member; private material remains in Keychain.
  func prepareAuthorityHandoff(_ material: GroupMaterial, successor: [UInt8], validUntil: UInt64) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    let now = UInt64(Date().timeIntervalSince1970)
    guard storeHandle != 0 && successor.count == 32 && validUntil > now else { throw NativeFailure.status(1) }
    var material = material
    defer { material.wipe() }
    var buffer = MeshBuffer(ptr: nil, len: 0)
    let status = material.identitySeed.withUnsafeBytes { identity in
      successor.withUnsafeBufferPointer { successor in
        mesh_secure_store_prepare_authority_handoff(
          storeHandle, identity.bindMemory(to: UInt8.self).baseAddress, identity.count,
          successor.baseAddress, successor.count, validUntil, now, &buffer
        )
      }
    }
    defer { mesh_buffer_release(buffer) }
    guard status == 0, buffer.len <= 512, let pointer = buffer.ptr else { throw NativeFailure.status(status) }
    return Array(UnsafeBufferPointer(start: pointer, count: buffer.len))
  }
  func enrollmentRequestMember(_ request: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard !request.isEmpty && request.count <= 512 else { throw NativeFailure.status(1) }
    var member = [UInt8](repeating: 0, count: 32)
    let status = request.withUnsafeBufferPointer { request in
      member.withUnsafeMutableBufferPointer { output in
        mesh_enrollment_request_member(request.baseAddress, request.count,
                                       UInt64(Date().timeIntervalSince1970), output.baseAddress)
      }
    }
    guard status == 0 else { throw NativeFailure.status(status) }
    return member
  }
  func installPolicy(_ policy: [UInt8]) throws -> Int64 {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 && !policy.isEmpty && policy.count <= 12 * 1024 else { throw NativeFailure.status(1) }
    var epoch: UInt64 = 0
    let status = policy.withUnsafeBufferPointer {
      mesh_secure_store_install_policy(storeHandle, $0.baseAddress, $0.count,
                                       UInt64(Date().timeIntervalSince1970), &epoch)
    }
    guard status == 0, epoch > 0, epoch <= UInt64(Int64.max) else { throw NativeFailure.status(status) }
    return Int64(epoch)
  }
  /// The promoted member reissues the public policy at the immediate next epoch.
  func rotateAuthority(_ material: GroupMaterial, handoff: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 && !handoff.isEmpty && handoff.count <= 512 else { throw NativeFailure.status(1) }
    var material = material
    defer { material.wipe() }
    var buffer = MeshBuffer(ptr: nil, len: 0)
    let status = material.identitySeed.withUnsafeBytes { identity in
      handoff.withUnsafeBufferPointer { handoff in
        mesh_secure_store_rotate_authority(
          storeHandle, identity.bindMemory(to: UInt8.self).baseAddress, identity.count,
          handoff.baseAddress, handoff.count, UInt64(Date().timeIntervalSince1970), &buffer
        )
      }
    }
    defer { mesh_buffer_release(buffer) }
    guard status == 0, buffer.len <= 12 * 1024, let pointer = buffer.ptr else { throw NativeFailure.status(status) }
    return Array(UnsafeBufferPointer(start: pointer, count: buffer.len))
  }
  /// Existing members converge only when the old-authority handoff accompanies
  /// the new public policy.
  func installRotatedPolicy(_ policy: [UInt8], handoff: [UInt8]) throws -> Int64 {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 && !policy.isEmpty && policy.count <= 12 * 1024
      && !handoff.isEmpty && handoff.count <= 512 else { throw NativeFailure.status(1) }
    var epoch: UInt64 = 0
    let status = policy.withUnsafeBufferPointer { policy in
      handoff.withUnsafeBufferPointer { handoff in
        mesh_secure_store_install_rotated_policy(
          storeHandle, policy.baseAddress, policy.count, handoff.baseAddress, handoff.count,
          UInt64(Date().timeIntervalSince1970), &epoch
        )
      }
    }
    guard status == 0, epoch > 0, epoch <= UInt64(Int64.max) else { throw NativeFailure.status(status) }
    return Int64(epoch)
  }
  func startSession(_ material: GroupMaterial, initiator: Bool) throws -> UInt64 {
    dispatchPrecondition(condition: .onQueue(queue))
    guard storeHandle != 0 else { throw NativeFailure.status(3) }
    var material = material
    defer { material.wipe() }
    var handle: UInt64 = 0
    let now = UInt64(Date().timeIntervalSince1970)
    let status = material.sessionSeed.withUnsafeBytes { session in
      material.member.withUnsafeBufferPointer { member in
        mesh_secure_session_start(storeHandle, session.bindMemory(to: UInt8.self).baseAddress, session.count,
                                  member.baseAddress, member.count, initiator ? 0 : 1, now, &handle)
      }
    }
    guard status == 0, handle != 0 else { throw NativeFailure.status(status) }
    return handle
  }
  private func sessionOutput(_ work: (UnsafeMutablePointer<MeshBuffer>) -> Int32) throws -> [UInt8] {
    var buffer = MeshBuffer(ptr: nil, len: 0)
    let status = work(&buffer)
    defer { mesh_buffer_release(buffer) }
    guard status == 0, buffer.len <= 4163 else { throw NativeFailure.status(status) }
    guard buffer.len == 0 || buffer.ptr != nil else { throw NativeFailure.status(1) }
    guard let pointer = buffer.ptr else { return [] }
    return Array(UnsafeBufferPointer(start: pointer, count: buffer.len))
  }
  func sessionWrite(_ handle: UInt64) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    return try sessionOutput { mesh_secure_session_write(handle, UInt64(Date().timeIntervalSince1970), $0) }
  }
  func sessionRead(_ handle: UInt64, frame: [UInt8]) throws {
    dispatchPrecondition(condition: .onQueue(queue))
    guard !frame.isEmpty && frame.count <= 96 else { throw NativeFailure.status(1) }
    let status = frame.withUnsafeBufferPointer {
      mesh_secure_session_read(handle, $0.baseAddress, $0.count, UInt64(Date().timeIntervalSince1970))
    }
    guard status == 0 else { throw NativeFailure.status(status) }
  }
  func sessionFinish(_ handle: UInt64) throws {
    dispatchPrecondition(condition: .onQueue(queue))
    let status = mesh_secure_session_finish(handle, UInt64(Date().timeIntervalSince1970))
    guard status == 0 else { throw NativeFailure.status(status) }
  }
  func sessionAuthenticate(_ handle: UInt64, material: GroupMaterial) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    var material = material
    defer { material.wipe() }
    return try sessionOutput { output in
      material.identitySeed.withUnsafeBytes { identity in
        mesh_secure_session_authenticate(handle, identity.bindMemory(to: UInt8.self).baseAddress,
                                         identity.count, UInt64(Date().timeIntervalSince1970), output)
      }
    }
  }
  func sessionSend(_ handle: UInt64, bytes: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard !bytes.isEmpty && bytes.count <= 4096 else { throw NativeFailure.status(1) }
    return try sessionOutput { output in
      bytes.withUnsafeBufferPointer {
        mesh_secure_session_send(handle, $0.baseAddress, $0.count,
                                 UInt64(Date().timeIntervalSince1970), output)
      }
    }
  }
  func sessionReceive(_ handle: UInt64, frame: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard frame.count >= 58 && frame.count <= 4154 else { throw NativeFailure.status(1) }
    return try sessionOutput { output in
      frame.withUnsafeBufferPointer {
        mesh_secure_session_receive(handle, $0.baseAddress, $0.count,
                                    UInt64(Date().timeIntervalSince1970), output)
      }
    }
  }
  func sessionAuthenticated(_ handle: UInt64) throws -> Bool {
    dispatchPrecondition(condition: .onQueue(queue))
    var authenticated: UInt8 = 0
    let status = mesh_secure_session_authenticated(handle, &authenticated)
    guard status == 0 else { throw NativeFailure.status(status) }
    return authenticated == 1
  }
  func sessionPeer(_ handle: UInt64) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    var member = [UInt8](repeating: 0, count: 32)
    let status = member.withUnsafeMutableBufferPointer { mesh_secure_session_peer(handle, $0.baseAddress) }
    guard status == 0 else { throw NativeFailure.status(status) }
    return member
  }
  func releaseSession(_ handle: UInt64) { if handle != 0 { _ = mesh_secure_session_release(handle) } }
  func linkEncode(_ bytes: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard !bytes.isEmpty && bytes.count <= 4160 else { throw NativeFailure.status(1) }
    return try sessionOutput { output in
      bytes.withUnsafeBufferPointer { mesh_link_frame_encode($0.baseAddress, $0.count, output) }
    }
  }
  func linkDecode(_ bytes: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard (3...4163).contains(bytes.count) else { throw NativeFailure.status(1) }
    return try sessionOutput { output in
      bytes.withUnsafeBufferPointer { mesh_link_frame_decode($0.baseAddress, $0.count, output) }
    }
  }
  /// Builds a canonical route + durable payload envelope before passing it to
  /// a Noise session. The result is bounded by one authenticated datagram.
  func routedRecordEncode(frame: [UInt8], record: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard frame.count == 91 && !record.isEmpty && record.count <= 4003 else {
      throw NativeFailure.status(1)
    }
    return try sessionOutput { output in
      frame.withUnsafeBufferPointer { frame in
        record.withUnsafeBufferPointer { record in
          mesh_routed_record_encode(frame.baseAddress, frame.count,
                                    record.baseAddress, record.count, output)
        }
      }
    }
  }
  /// Returns the 91-byte route frame followed by the canonical durable record.
  func routedRecordDecode(_ bytes: [UInt8]) throws -> [UInt8] {
    dispatchPrecondition(condition: .onQueue(queue))
    guard !bytes.isEmpty && bytes.count <= 4096 else { throw NativeFailure.status(1) }
    return try sessionOutput { output in
      bytes.withUnsafeBufferPointer { mesh_routed_record_decode($0.baseAddress, $0.count, output) }
    }
  }
  deinit {
    if handle != 0 { _ = mesh_runtime_release(handle) }
    if storeHandle != 0 { _ = mesh_secure_store_release(storeHandle) }
    if relayGate != 0 { _ = mesh_relay_gate_release(relayGate) }
  }
}
