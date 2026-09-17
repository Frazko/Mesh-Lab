import Flutter
import UIKit

public class MeshHostPlugin: NSObject, FlutterPlugin, MeshHostApi {
  private let bluetooth: BluetoothAccess
  private let aware: WifiAwareAccess

  override init() {
    bluetooth = BluetoothAccess()
    aware = WifiAwareAccess()
    super.init()
    bluetooth.setAwarePayloadSender { [weak aware] payload in
      aware?.sendProtectedPayload(payload) ?? false
    }
    aware.setPayloadReceiver { [weak bluetooth] payload, handle, reply in
      bluetooth?.acceptAwarePayload(payload, noiseHandle: handle, reply: reply)
    }
  }

  public static func register(with registrar: FlutterPluginRegistrar) {
    MeshHostApiSetup.setUp(binaryMessenger: registrar.messenger(), api: MeshHostPlugin())
  }
  private func execute<T>(_ work: @escaping () throws -> T) async throws -> T {
    try await withCheckedThrowingContinuation { continuation in
      NativeRuntime.shared.queue.async {
        do { continuation.resume(returning: try work()) }
        catch NativeFailure.status(let code) {
          continuation.resume(throwing: PigeonError(code: "MESH_\(code)", message: "El motor rechazó la operación.", details: nil))
        } catch is KeyStorageFailure {
          continuation.resume(throwing: PigeonError(code: "KEY_STORAGE_UNAVAILABLE", message: "No se pudo abrir la identidad protegida.", details: nil))
        } catch {
          continuation.resume(throwing: PigeonError(code: "INVALID_ENVELOPE", message: "Respuesta incompatible del motor.", details: nil))
        }
      }
    }
  }
  func prepareIdentity() async throws -> IdentityInfo {
    try await execute {
      do {
        let identity = SecureIdentity()
        let fingerprint = try identity.prepare()
        try NativeRuntime.shared.prepareStore(identity.storeMaterial())
        return IdentityInfo(fingerprint: fingerprint, storage: "Keychain")
      }
      catch { throw KeyStorageFailure.unavailable }
    }
  }
  func groupInfo() async throws -> GroupInfo {
    try await execute {
      guard NativeRuntime.shared.hasStore else { return GroupInfo(configured: false, epoch: 0) }
      let epoch = try NativeRuntime.shared.policyEpoch()
      return GroupInfo(configured: epoch != 0, epoch: epoch)
    }
  }
  func createGroup() async throws -> GroupInfo {
    try await execute {
      guard NativeRuntime.shared.hasStore else { throw NativeFailure.status(3) }
      let epoch = try NativeRuntime.shared.createGroup(try SecureIdentity().groupMaterial())
      return GroupInfo(configured: epoch > 0, epoch: epoch)
    }
  }
  func exportInvitation() async throws -> FlutterStandardTypedData {
    try await execute { FlutterStandardTypedData(bytes: Data(try NativeRuntime.shared.exportInvitation())) }
  }
  func createEnrollmentRequest(invitation: FlutterStandardTypedData) async throws -> FlutterStandardTypedData {
    try await execute {
      FlutterStandardTypedData(bytes: Data(try NativeRuntime.shared.createEnrollmentRequest(
        try SecureIdentity().groupMaterial(), invitation: Array(invitation.data))))
    }
  }
  func issueEnrollment(request: FlutterStandardTypedData) async throws -> FlutterStandardTypedData {
    try await execute {
      FlutterStandardTypedData(bytes: Data(try NativeRuntime.shared.issueEnrollment(
        try SecureIdentity().groupMaterial(), request: Array(request.data))))
    }
  }
  func installPolicy(policy: FlutterStandardTypedData) async throws -> GroupInfo {
    try await execute {
      let epoch = try NativeRuntime.shared.installPolicy(Array(policy.data))
      return GroupInfo(configured: epoch > 0, epoch: epoch)
    }
  }
  func bluetoothInfo() async throws -> BluetoothInfo {
    if Thread.isMainThread { return bluetooth.info() }
    return DispatchQueue.main.sync { bluetooth.info() }
  }
  func prepareBluetooth() async throws -> BluetoothInfo {
    if Thread.isMainThread { return bluetooth.prepare() }
    return DispatchQueue.main.sync { bluetooth.prepare() }
  }
  func startBluetoothDiscovery() async throws -> BluetoothInfo {
    if Thread.isMainThread { return bluetooth.startDiscovery() }
    return DispatchQueue.main.sync { bluetooth.startDiscovery() }
  }
  func stopBluetoothDiscovery() async throws -> BluetoothInfo {
    if Thread.isMainThread { return bluetooth.stopDiscovery() }
    return DispatchQueue.main.sync { bluetooth.stopDiscovery() }
  }
  /// The encrypted-store runtime has a single serial queue. Pigeon calls can
  /// arrive on any cooperative executor, so every policy lookup must cross that
  /// queue before the UI-owned Wi‑Fi Aware object is touched on the main queue.
  private func currentHasGroup() async -> Bool {
    (try? await execute {
      guard NativeRuntime.shared.hasStore else { return false }
      return try NativeRuntime.shared.policyEpoch() > 0
    }) ?? false
  }
  private func onMain<T>(_ work: @escaping () -> T) -> T {
    Thread.isMainThread ? work() : DispatchQueue.main.sync(execute: work)
  }
  func awareInfo() async throws -> AwareInfo {
    let hasGroup = await currentHasGroup()
    return onMain { self.aware.info(hasGroup: hasGroup) }
  }
  func startAwareDiscovery() async throws -> AwareInfo {
    let hasGroup = await currentHasGroup()
    return onMain { self.aware.start(hasGroup: hasGroup) }
  }
  func stopAwareDiscovery() async throws -> AwareInfo {
    let hasGroup = await currentHasGroup()
    return onMain { self.aware.stop(hasGroup: hasGroup) }
  }
  func sendText(message: String, logicalId: String) async throws -> Bool {
    if Thread.isMainThread { return bluetooth.sendText(message, logicalId: logicalId) }
    return DispatchQueue.main.sync { bluetooth.sendText(message, logicalId: logicalId) }
  }
  func drainVerifiedIncomingText() async throws -> [VerifiedIncomingText] {
    onMain {
      self.bluetooth.drainVerifiedIncomingText().map { event in
        VerifiedIncomingText(
          authorId: event.authorId,
          objectId: event.objectId,
          verifiedAtUnixSeconds: event.verifiedAtUnixSeconds,
          body: event.body
        )
      }
    }
  }

  func drainVerifiedIncomingVoice() async throws -> [VerifiedIncomingVoice] {
    onMain {
      self.bluetooth.drainVerifiedIncomingVoice().map { event in
        VerifiedIncomingVoice(
          authorId: event.authorId,
          objectId: event.objectId,
          logicalId: event.logicalId,
          verifiedAtUnixSeconds: event.verifiedAtUnixSeconds,
          durationMillis: event.durationMillis,
          context: event.context
        )
      }
    }
  }

  func deliveryInfo(logicalId: String) async throws -> DeliveryInfo {
    try await execute {
      guard let requested = BluetoothAccess.decodeLogicalId(logicalId) else { throw NativeFailure.status(1) }
      let bytes = try NativeRuntime.shared.deliverySummary(requested)
      guard bytes.isEmpty || bytes.count == 19 else { throw NativeFailure.status(1) }
      guard !bytes.isEmpty else { return DeliveryInfo(logicalId: "", targetCount: 0, deliveredCount: 0, state: "none") }
      let id = bytes.prefix(16).map { String(format: "%02x", $0) }.joined()
      let state: String
      switch bytes[18] { case 0: state = "queued"; case 1: state = "partial"; case 2: state = "delivered"; case 3: state = "expired"; default: throw NativeFailure.status(1) }
      return DeliveryInfo(logicalId: id, targetCount: Int64(bytes[16]), deliveredCount: Int64(bytes[17]), state: state)
    }
  }
  func voiceInfo() async throws -> VoiceInfo {
    if Thread.isMainThread { return bluetooth.voiceInfo() }
    return DispatchQueue.main.sync { bluetooth.voiceInfo() }
  }
  func sendVoice(audio: FlutterStandardTypedData, durationMillis: Int64, logicalId: String) async throws -> Bool {
    if Thread.isMainThread { return bluetooth.sendVoice(Array(audio.data), durationMillis: durationMillis, logicalId: logicalId) }
    return DispatchQueue.main.sync { bluetooth.sendVoice(Array(audio.data), durationMillis: durationMillis, logicalId: logicalId) }
  }
  func sendVoiceWithContext(audio: FlutterStandardTypedData, durationMillis: Int64, logicalId: String, context: String) async throws -> Bool {
    if Thread.isMainThread { return bluetooth.sendVoiceWithContext(Array(audio.data), durationMillis: durationMillis, logicalId: logicalId, context: context) }
    return DispatchQueue.main.sync { bluetooth.sendVoiceWithContext(Array(audio.data), durationMillis: durationMillis, logicalId: logicalId, context: context) }
  }
  func playLastVoice() async throws -> Bool {
    if Thread.isMainThread { return bluetooth.playLastVoice() }
    return DispatchQueue.main.sync { bluetooth.playLastVoice() }
  }
  func playVoice(objectId: String) async throws -> Bool {
    if Thread.isMainThread { return bluetooth.playVoice(objectId) }
    return DispatchQueue.main.sync { bluetooth.playVoice(objectId) }
  }
  func engineInfo() async throws -> EngineInfo {
    try await execute {
      var decoder = try MeshEnvelope(NativeRuntime.shared.request(method: 0))
      let info = try decoder.info()
      return EngineInfo(engineVersion: info.version, abiVersion: info.abi, apiVersion: info.api, phase: info.phase, buildId: info.build)
    }
  }
  func subscribe(cursor: Int64) async throws -> EngineSnapshot {
    try await snapshot(method: 1, argument: cursor)
  }
  func verifyBridge(requestId: Int64) async throws -> EngineSnapshot {
    try await snapshot(method: 2, argument: requestId)
  }
  private func snapshot(method: UInt8, argument: Int64) async throws -> EngineSnapshot {
    try await execute {
      var decoder = try MeshEnvelope(NativeRuntime.shared.request(method: method, argument: argument))
      let s = try decoder.snapshot(method: Int64(method))
      return EngineSnapshot(runtimeId: s.runtime, cursor: s.cursor, probeCount: s.probes,
        foundationState: s.state, cursorReset: s.reset,
        events: s.events.map { DiagnosticEvent(sequence: $0.sequence, requestId: $0.request, kind: $0.kind) })
    }
  }
}
