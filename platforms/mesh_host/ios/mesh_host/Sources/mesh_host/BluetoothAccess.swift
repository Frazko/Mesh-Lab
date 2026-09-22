import CoreBluetooth
import Foundation
import AVFoundation

/// BLE discovery, enrollment, and the protected GATT transport.
struct CertifiedIncomingText {
  let authorId: String
  let objectId: String
  let verifiedAtUnixSeconds: Int64
  let body: String
}

/// Metadata for audio that native code has already verified and committed.
/// Audio stays private to the host and is addressed only by this object ID.
struct CertifiedIncomingVoice {
  let authorId: String
  let objectId: String
  let logicalId: String
  let verifiedAtUnixSeconds: Int64
  let durationMillis: Int64
  let context: String
}

final class BluetoothAccess: NSObject, CBCentralManagerDelegate, CBPeripheralManagerDelegate, CBPeripheralDelegate {
  private let serviceUUID = CBUUID(string: "3C2865E0-1B51-49B4-9F22-4F15D5667761")
  private let rxUUID = CBUUID(string: "3C2865E1-1B51-49B4-9F22-4F15D5667761")
  private let txUUID = CBUUID(string: "3C2865E2-1B51-49B4-9F22-4F15D5667761")
  private var central: CBCentralManager?
  private var peripheral: CBPeripheralManager?
  private var centralState: CBManagerState = .unknown
  private var peripheralState: CBManagerState = .unknown
  private var requested = false
  private var advertising = false
  private var peers = Set<UUID>()
  private var connected = [UUID: CBPeripheral]()
  private var probes: Int64 = 0
  private var messages: Int64 = 0
  private var lastMessage = ""
  // Product adapters drain this FIFO; unlike `lastMessage`, it cannot collapse
  // a burst of already-certified durable deliveries into one mutable value.
  private var verifiedIncoming = [CertifiedIncomingText]()
  private var verifiedIncomingVoice = [CertifiedIncomingVoice]()
  private var recentTextIds = Set<UInt32>()
  private var recentTextOrder = [UInt32]()
  private var forwardedRelayFrames = [String: [UInt8]]()
  private let maxDurableRelaySlots = 65
  private let maxDurableOriginSlots = 520
  private final class SessionLink { let handle: UInt64; let initiator: Bool; var stage: Int = 0; var originOutboxDrained = false
    init(_ handle: UInt64, initiator: Bool) { self.handle = handle; self.initiator = initiator }
  }
  private var clientSessions = [UUID: SessionLink]()
  private var serverSessions = [UUID: SessionLink]()
  private var clientWrites = [UUID: [[UInt8]]]()
  private var serverWrites = [UUID: [[UInt8]]]()
  private var inbound = [String: BleFrameCodec.Assembler]()
  private var subscribers = [UUID: CBCentral]()
  private var txCharacteristic: CBMutableCharacteristic?
  private var enrollmentDetail = ""
  // nil is the Mesh Lab's deliberate open-enrollment behavior. Product
  // adapters set an exact server-authorized roster before discovery; then a
  // public request must prove it belongs to that roster before issuance.
  private var enrollmentAllowedMembers: Set<String>?
  private var enrollmentAuthorityEnabled = true
  private var recoveredStalePolicy = false
  private var reconnectWorkItem: DispatchWorkItem?
  private var receivedVoices: Int64 = 0
  private var lastVoiceDurationMillis: Int64 = 0
  private var lastVoiceFile: URL?
  private var voicePlayer: AVAudioPlayer?
  private final class VoiceAssembly {
    let total: Int
    let durationMillis: Int64
    var chunks = [Int: [UInt8]]()
    init(total: Int, durationMillis: Int64) {
      self.total = total
      self.durationMillis = durationMillis
    }
  }
  private var voiceAssemblies = [UInt32: VoiceAssembly]()
  // Wi‑Fi Aware owns its own NetworkConnection<TLS> and Noise session. This
  // closure carries only already-authenticated application payloads to that
  // transport; it never exposes a LAN endpoint to Flutter.
  private var awarePayloadSender: (([UInt8]) -> Bool)?

  // These records carry only public policy or a signed enrollment request. They
  // run before a protected session exists, then the normal Noise handshake takes
  // over on the same GATT link. The full domain prefix is required because an
  // opaque Noise record can legitimately start with any single byte.
  private let enrollmentDomain: [UInt8] = [
    0x4d, 0x45, 0x53, 0x48, 0x2d, 0x45, 0x4e, 0x52,
    0x4f, 0x4c, 0x4c, 0x2d, 0x76, 0x31, 0x00, 0x7f,
  ]
  private let enrollmentHello: UInt8 = 0xf0
  private let enrollmentInvitation: UInt8 = 0xf1
  private let enrollmentRequest: UInt8 = 0xf2
  private let enrollmentPolicy: UInt8 = 0xf3
  private let voiceMarker: UInt8 = 0x56
  private let durableVoiceVersion: UInt8 = 2
  private let durableVoiceContextVersion: UInt8 = 3
  private let durableVoiceHeaderBytes = 22
  private let durableVoiceContextHeaderBytes = 24
  private let maxVoiceContextBytes = 512
  private let textMarker: UInt8 = 0x7f
  private let receiptMarker: UInt8 = 0x7e
  private let heartbeatMarker: UInt8 = 0x7d
  private let heartbeatAckMarker: UInt8 = 0x7c
  private let presenceMarker: UInt8 = 0x7b
  private let textHeaderBytes = 5
  private let maxRecentTextIds = 64
  private let voiceHeaderBytes = 11
  private let maxVoicePayload = 4096
  private let maxVoiceChunks = 16
  private let maxVoiceBytes = 48 * 1024
  private let maxVoiceDurationMillis: Int64 = 10_000

  private func log(_ message: String) {
    NSLog("[MeshBle] %@", message)
  }

  func info() -> BluetoothInfo {
    if centralState == .unsupported || peripheralState == .unsupported {
      return BluetoothInfo(available: false, authorized: false, enabled: false, active: false,
                           peerCount: 0, probeCount: 0, authenticated: false, messageCount: messages, lastMessage: lastMessage, detail: "Este iPhone no tiene Bluetooth disponible.")
    }
    let authorization = CBCentralManager.authorization
    guard authorization == .allowedAlways else {
      let message = authorization == .notDetermined
        ? "Autoriza Bluetooth para descubrir teléfonos del laboratorio."
        : "Autoriza Bluetooth en Ajustes para continuar."
      return BluetoothInfo(available: true, authorized: false, enabled: false, active: false,
                           peerCount: 0, probeCount: 0, authenticated: false, messageCount: messages, lastMessage: lastMessage, detail: message)
    }
    guard centralState == .poweredOn && peripheralState == .poweredOn else {
      let message = centralState == .poweredOff || peripheralState == .poweredOff
        ? "Bluetooth está apagado. Wi‑Fi Aware puede seguir activo."
        : "Bluetooth está preparando su estado. Vuelve a comprobarlo."
      return BluetoothInfo(available: true, authorized: true, enabled: false, active: false,
                           peerCount: 0, probeCount: 0, authenticated: false, messageCount: messages, lastMessage: lastMessage, detail: message)
    }
    let visiblePeers = Set(peers).union(subscribers.keys).union(connected.keys)
    let active = central?.isScanning == true || advertising
    let authenticated = clientSessions.values.contains { link in
      runtime { try NativeRuntime.shared.sessionAuthenticated(link.handle) } == true
    } || serverSessions.values.contains { link in
      runtime { try NativeRuntime.shared.sessionAuthenticated(link.handle) } == true
    }
    let message = active
      ? (authenticated ? "Teléfono del grupo conectado de forma segura."
        : (!enrollmentDetail.isEmpty ? enrollmentDetail : "Buscando Mesh Lab compatibles: \(visiblePeers.count) detectado(s)."))
      : "Bluetooth listo. La conexión segura se inicia al tener un grupo."
    return BluetoothInfo(available: true, authorized: true, enabled: true, active: active,
                         peerCount: Int64(visiblePeers.count), probeCount: probes, authenticated: authenticated,
                         messageCount: messages, lastMessage: lastMessage, detail: message)
  }

  func prepare() -> BluetoothInfo {
    ensureManagers()
    return info()
  }

  func startDiscovery() -> BluetoothInfo {
    requested = true
    peers.removeAll()
    probes = 0
    ensureManagers()
    beginCentralScanIfReady()
    beginPeripheralIfReady()
    return info()
  }

  func stopDiscovery() -> BluetoothInfo {
    requested = false
    reconnectWorkItem?.cancel()
    reconnectWorkItem = nil
    central?.stopScan()
    peripheral?.stopAdvertising()
    peripheral?.removeAllServices()
    advertising = false
    peers.removeAll()
    probes = 0
    for peer in connected.values { central?.cancelPeripheralConnection(peer) }
    connected.removeAll()
    for link in clientSessions.values { runtime { NativeRuntime.shared.releaseSession(link.handle) } }
    for link in serverSessions.values { runtime { NativeRuntime.shared.releaseSession(link.handle) } }
    clientSessions.removeAll(); serverSessions.removeAll(); clientWrites.removeAll(); serverWrites.removeAll()
    inbound.removeAll(); subscribers.removeAll(); txCharacteristic = nil
    enrollmentDetail = ""
    return info()
  }

  private func ensureManagers() {
    if central == nil {
      central = CBCentralManager(delegate: self, queue: .main,
                                 options: [CBCentralManagerOptionShowPowerAlertKey: true])
    }
    if peripheral == nil { peripheral = CBPeripheralManager(delegate: self, queue: .main) }
  }

  private func beginCentralScanIfReady() {
    guard requested, centralState == .poweredOn, central?.isScanning == false else { return }
    // Some Android stacks advertise a 128-bit UUID in a legacy record that
    // CoreBluetooth's hardware service filter misses. Scan locally, then keep
    // only Mesh Lab advertisements before connecting to anything.
    central?.scanForPeripherals(withServices: nil,
                                options: [CBCentralManagerScanOptionAllowDuplicatesKey: false])
    log("Started local Mesh Lab scan")
  }

  private func beginPeripheralIfReady() {
    guard requested, peripheralState == .poweredOn, peripheral?.isAdvertising == false else { return }
    // The client drains this queue with `.withResponse`; advertise the same
    // capability or CoreBluetooth drops the enrollment and session frames.
    let rx = CBMutableCharacteristic(type: rxUUID, properties: [.write],
                                     value: nil, permissions: [.writeable])
    let tx = CBMutableCharacteristic(type: txUUID, properties: [.notify], value: nil, permissions: [])
    txCharacteristic = tx
    let service = CBMutableService(type: serviceUUID, primary: true)
    service.characteristics = [rx, tx]
    peripheral?.add(service)
  }

  @discardableResult private func runtime<T>(_ work: @escaping () throws -> T) -> T? {
    try? NativeRuntime.shared.queue.sync { try work() }
  }
  private func startClient(_ peripheral: CBPeripheral) {
    guard clientSessions[peripheral.identifier] == nil else { return }
    guard hasGroup else {
      log("Starting Bluetooth enrollment with \(peripheral.identifier.uuidString)")
      enrollmentDetail = "Teléfono Mesh Lab encontrado. Incorporando el grupo por Bluetooth…"
      enqueueClient(peripheral, raw: [enrollmentFrame(enrollmentHello, [])])
      return
    }
    guard let handle = runtime({ try NativeRuntime.shared.startSession(try SecureIdentity().groupMaterial(), initiator: true) }),
          let first = runtime({ try NativeRuntime.shared.sessionWrite(handle) }) else { return }
    log("Starting Noise handshake with \(peripheral.identifier.uuidString)")
    clientSessions[peripheral.identifier] = SessionLink(handle, initiator: true)
    enqueueClient(peripheral, raw: [first])
  }
  private func receiveFromServer(_ central: CBCentral, fragment: Data) {
    let key = "s:\(central.identifier.uuidString)"
    // Dictionary's `default:` subscript does not retain a newly-created
    // reference type when only one of its methods mutates it. Keep the
    // assembler explicitly, otherwise every BLE fragment starts over.
    let assembler: BleFrameCodec.Assembler
    if let existing = inbound[key] { assembler = existing }
    else { assembler = BleFrameCodec.Assembler(); inbound[key] = assembler }
    guard let framed = assembler.accept(fragment),
          let raw = runtime({ try NativeRuntime.shared.linkDecode(framed) }) else { return }
    if handleEnrollmentFromClient(raw, central: central) { return }
    var link = serverSessions[central.identifier]
    if link == nil, let handle = runtime({ try NativeRuntime.shared.startSession(try SecureIdentity().groupMaterial(), initiator: false) }) {
      link = SessionLink(handle, initiator: false); serverSessions[central.identifier] = link
    }
    guard let link else { return }
    if link.stage == 0 {
      guard runtime({ try NativeRuntime.shared.sessionRead(link.handle, frame: raw) }) != nil,
            let response = runtime({ try NativeRuntime.shared.sessionWrite(link.handle) }) else { closeServer(central.identifier); return }
      link.stage = 1; enqueueServer(central, raw: [response])
    } else if link.stage == 1 {
      guard runtime({ try NativeRuntime.shared.sessionRead(link.handle, frame: raw) }) != nil,
            runtime({ try NativeRuntime.shared.sessionFinish(link.handle) }) != nil,
            let proof = runtime({ try NativeRuntime.shared.sessionAuthenticate(link.handle, material: try SecureIdentity().groupMaterial()) }) else { closeServer(central.identifier); return }
      link.stage = 2; enqueueServer(central, raw: [proof])
    } else { acceptProtected(link, raw: raw) }
  }
  private func receiveFromClient(_ peripheral: CBPeripheral, fragment: Data) {
    let key = "c:\(peripheral.identifier.uuidString)"
    let assembler: BleFrameCodec.Assembler
    if let existing = inbound[key] { assembler = existing }
    else { assembler = BleFrameCodec.Assembler(); inbound[key] = assembler }
    guard let framed = assembler.accept(fragment) else { return }
    guard let raw = runtime({ try NativeRuntime.shared.linkDecode(framed) }) else {
      log("Rejected \(framed.count)-byte Android wire frame")
      return
    }
    log("Received \(raw.count)-byte GATT record from \(peripheral.identifier.uuidString)")
    if handleEnrollmentFromServer(raw, peripheral: peripheral) { return }
    guard let link = clientSessions[peripheral.identifier] else { return }
    if link.stage == 0 {
      guard runtime({ try NativeRuntime.shared.sessionRead(link.handle, frame: raw) }) != nil,
            let third = runtime({ try NativeRuntime.shared.sessionWrite(link.handle) }),
            runtime({ try NativeRuntime.shared.sessionFinish(link.handle) }) != nil,
            let proof = runtime({ try NativeRuntime.shared.sessionAuthenticate(link.handle, material: try SecureIdentity().groupMaterial()) }) else {
        recoverFromStalePolicy(peripheral)
        return
      }
      link.stage = 1
      enqueueClient(peripheral, raw: [third, proof])
    } else { acceptProtected(link, raw: raw) }
  }

  /// A prior test could have created a different group on this iPhone. Android
  /// is the only authority in this lab, so recover once by keeping the
  /// Keychain identity and replacing only the stale local group policy.
  private func recoverFromStalePolicy(_ peripheral: CBPeripheral) {
    guard !recoveredStalePolicy else { closeClient(peripheral); return }
    recoveredStalePolicy = true
    if let old = clientSessions.removeValue(forKey: peripheral.identifier) {
      runtime { NativeRuntime.shared.releaseSession(old.handle) }
    }
    clientWrites.removeValue(forKey: peripheral.identifier)
    inbound.removeValue(forKey: "c:\(peripheral.identifier.uuidString)")
    guard runtime({ try NativeRuntime.shared.resetPolicyForLab() }) != nil else {
      enrollmentDetail = "No se pudo recuperar el grupo de Android."
      closeClient(peripheral)
      return
    }
    enrollmentDetail = "Configuración anterior recuperada. Incorporando el grupo de Android…"
    log("Discarded stale local group; starting Android enrollment")
    startClient(peripheral)
  }
  private func acceptProtected(_ link: SessionLink, raw: [UInt8]) {
    guard let data = runtime({ try NativeRuntime.shared.sessionReceive(link.handle, frame: raw) }) else { return }
    if runtime({ try NativeRuntime.shared.sessionAuthenticated(link.handle) }) == true {
      probes += 1
      if !link.originOutboxDrained {
        link.originOutboxDrained = true
        _ = drainDurableOriginOutbox()
        _ = drainDurableReceiptOutbox()
        drainDurableRelayQueue()
        drainDurableRelayReceiptQueue()
        drainDurableRelayReceiptAckQueue()
      }
    }
    if acceptRoutedPayload(data, noiseHandle: link.handle) { return }
    if data.first == receiptMarker, data.count == textHeaderBytes {
      log("Bluetooth text receipt received")
      return
    }
    if data.first == heartbeatMarker || data.first == heartbeatAckMarker || data.first == presenceMarker { return }
    if !data.isEmpty, !acceptVoiceChunk(data), let text = unwrapText(data),
       let message = String(bytes: text, encoding: .utf8) {
      messages += 1
      lastMessage = message
      log("Protected text received via Bluetooth")
    }
  }

  /// Ingests only canonical durable records after the per-radio Noise session.
  /// The normal chat path remains below this boundary until it creates durable
  /// objects itself, so a legacy text cannot accidentally enter relay custody.
  private func acceptRoutedPayload(_ data: [UInt8], noiseHandle: UInt64) -> Bool {
    guard data.count >= 96, data[0] == 0x72, data[1] == 1 else { return false }
    let frame = Array(data[2..<93])
    let relayKey = frame[1..<17].map { String(format: "%02x", $0) }.joined()
    let kind = Int(data[95])
    let forwarded: [UInt8]
    switch kind {
    case 1, 3, 4:
      guard let material = try? SecureIdentity().groupMaterial() else { return true }
      defer { var material = material; material.wipe() }
      guard runtime({ try NativeRuntime.shared.openRelayGate(member: material.member) }) != nil,
            let via = runtime({ try NativeRuntime.shared.sessionPeer(noiseHandle) }),
            let decision = runtime({ try NativeRuntime.shared.acceptRelayFrame(frame, via: via) }),
            decision.count == 92, decision[0] == 1 else { return true }
      forwarded = Array(decision[1..<92])
      if forwardedRelayFrames.count >= 128, let oldest = forwardedRelayFrames.keys.first {
        forwardedRelayFrames.removeValue(forKey: oldest)
      }
      forwardedRelayFrames[relayKey] = forwarded
    case 2:
      guard let prior = forwardedRelayFrames[relayKey] else { return true }
      forwarded = prior
    default:
      return true
    }
    guard let via = runtime({ try NativeRuntime.shared.sessionPeer(noiseHandle) }) else { return true }
    let persisted = [0x72, 1] + forwarded + Array(data[93..<data.count])
    guard let accepted = runtime({ try NativeRuntime.shared.acceptRoutedRecord(persisted, receivedFrom: via) }) else { return true }
    if accepted == 2 {
      finalizeDurableText()
      drainDurableRelayQueue()
    } else if accepted == 3 {
      drainDurableRelayReceiptQueue()
      drainDurableReceiptAckOutbox()
    } else if accepted == 4 {
      drainDurableRelayReceiptAckQueue()
    }
    return true
  }

  /// BLE forwarding is available now. Wi‑Fi Aware provides the same ingress
  /// contract above, but it intentionally does not use its single broadcast
  /// sender here until it can select an individual WFA neighbor and exclude
  /// the ingress link as precisely as BLE does.
  private func drainDurableRelayQueue() {
    guard let receivedFrom = runtime({ try NativeRuntime.shared.relayReceivedFrom() }),
          receivedFrom.count == 32 else { return }
    var records = [[UInt8]]()
    for slot in 0..<maxDurableRelaySlots {
      guard let record = runtime({ try NativeRuntime.shared.relayRecord(slot: UInt16(slot)) }) else { return }
      if record.isEmpty { break }
      guard record.count <= 4096 else { return }
      records.append(record)
    }
    guard !records.isEmpty else { return }
    for (id, link) in clientSessions {
      guard !isIngressNeighbor(link, receivedFrom), let peer = connected[id],
            let frames = protectedRelayFrames(records, link: link) else { continue }
      enqueueClient(peer, raw: frames)
    }
    for (id, link) in serverSessions {
      guard !isIngressNeighbor(link, receivedFrom), let peer = subscribers[id],
            let frames = protectedRelayFrames(records, link: link) else { continue }
      enqueueServer(peer, raw: frames)
    }
  }

  /// Receipt custody follows the same BLE neighbor rules as object custody.
  /// It is restored from SQLCipher after a process restart and never goes back
  /// to the authenticated neighbor that supplied it.
  private func drainDurableRelayReceiptQueue() {
    guard let receivedFrom = runtime({ try NativeRuntime.shared.relayReceiptReceivedFrom() }),
          receivedFrom.count == 32 else { return }
    var records = [[UInt8]]()
    for slot in 0..<maxDurableOriginSlots {
      guard let record = runtime({ try NativeRuntime.shared.relayReceiptRecord(slot: UInt16(slot)) }) else { return }
      if record.isEmpty { break }
      guard record.count <= 4096 else { return }
      records.append(record)
    }
    guard !records.isEmpty else { return }
    for (id, link) in clientSessions {
      guard !isIngressNeighbor(link, receivedFrom), let peer = connected[id],
            let frames = protectedRelayFrames(records, link: link) else { continue }
      enqueueClient(peer, raw: frames)
    }
    for (id, link) in serverSessions {
      guard !isIngressNeighbor(link, receivedFrom), let peer = subscribers[id],
            let frames = protectedRelayFrames(records, link: link) else { continue }
      enqueueServer(peer, raw: frames)
    }
  }

  /// A signed ACK is stored before it crosses another BLE edge, so a relay
  /// restart cannot force the final recipient to retry until message expiry.
  private func drainDurableRelayReceiptAckQueue() {
    guard let receivedFrom = runtime({ try NativeRuntime.shared.relayReceiptAckReceivedFrom() }),
          receivedFrom.count == 32 else { return }
    var records = [[UInt8]]()
    for slot in 0..<maxDurableOriginSlots {
      guard let record = runtime({ try NativeRuntime.shared.relayReceiptAckRecord(slot: UInt16(slot)) }) else { return }
      if record.isEmpty { break }
      guard record.count <= 4096 else { return }
      records.append(record)
    }
    guard !records.isEmpty else { return }
    for (id, link) in clientSessions {
      guard !isIngressNeighbor(link, receivedFrom), let peer = connected[id],
            let frames = protectedRelayFrames(records, link: link) else { continue }
      enqueueClient(peer, raw: frames)
    }
    for (id, link) in serverSessions {
      guard !isIngressNeighbor(link, receivedFrom), let peer = subscribers[id],
            let frames = protectedRelayFrames(records, link: link) else { continue }
      enqueueServer(peer, raw: frames)
    }
  }

  /// Native completion packet: plaintext is made visible only once all
  /// encrypted chunks verified and the local signed receipt is committed.
  /// Parses completion packet v2 from Rust. It has no product-controlled
  /// author field: every exported metadata value comes from the verified proof
  /// after the receipt transaction has committed.
  private func finalizeDurableText() {
    guard let material = try? SecureIdentity().groupMaterial(),
          let packet = runtime({ try NativeRuntime.shared.finalizeNextDurableText(material: material) }),
          packet.count >= 76, packet[0] == 0x74, packet[1] == 2 else { return }
    let objectId = packet[2..<34].map { String(format: "%02x", $0) }.joined()
    let authorId = packet[34..<66].map { String(format: "%02x", $0) }.joined()
    var verifiedAt: Int64 = 0
    for byte in packet[66..<74] { verifiedAt = (verifiedAt << 8) | Int64(byte) }
    guard verifiedAt > 0 else { return }
    let receiptLength = (Int(packet[74]) << 8) | Int(packet[75])
    let textLengthOffset = 76 + receiptLength
    guard textLengthOffset + 2 <= packet.count else { return }
    let textLength = (Int(packet[textLengthOffset]) << 8) | Int(packet[textLengthOffset + 1])
    let textOffset = textLengthOffset + 2
    guard textLength > 0, textOffset + textLength == packet.count else { return }
    let payload = Array(packet[textOffset..<packet.count])
    if payload.count >= durableVoiceHeaderBytes, payload[0] == voiceMarker,
       payload[1] == durableVoiceVersion || payload[1] == durableVoiceContextVersion {
      let duration = (Int64(payload[2]) << 24) | (Int64(payload[3]) << 16) | (Int64(payload[4]) << 8) | Int64(payload[5])
      let logicalId = payload[6..<22].map { String(format: "%02x", $0) }.joined()
      let contextLength: Int
      if payload[1] == durableVoiceContextVersion {
        guard payload.count >= durableVoiceContextHeaderBytes else { return }
        contextLength = (Int(payload[22]) << 8) | Int(payload[23])
      } else {
        contextLength = 0
      }
      let audioOffset = payload[1] == durableVoiceContextVersion ? durableVoiceContextHeaderBytes + contextLength : durableVoiceHeaderBytes
      guard contextLength <= maxVoiceContextBytes, audioOffset < payload.count else { return }
      let context = contextLength == 0 ? "" : String(bytes: payload[durableVoiceContextHeaderBytes..<audioOffset], encoding: .utf8)
      let audio = Array(payload.dropFirst(audioOffset))
      guard duration > 0, duration <= maxVoiceDurationMillis,
            logicalId.range(of: "^[0-9a-f]{32}$", options: .regularExpression) != nil,
            context != nil, !audio.isEmpty, audio.count <= maxVoiceBytes else { return }
      do {
        let folder = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
          .appendingPathComponent("mesh-voice", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let file = folder.appendingPathComponent("\(objectId).m4a")
        try Data(audio).write(to: file, options: .atomic)
        lastVoiceFile = file
        lastVoiceDurationMillis = duration
        receivedVoices += 1
        if verifiedIncomingVoice.count == 64 { verifiedIncomingVoice.removeFirst() }
        verifiedIncomingVoice.append(CertifiedIncomingVoice(authorId: authorId, objectId: objectId, logicalId: logicalId, verifiedAtUnixSeconds: verifiedAt, durationMillis: duration, context: context!))
        _ = drainDurableReceiptOutbox()
      } catch { }
      return
    }
    guard let text = String(bytes: payload, encoding: .utf8) else { return }
    messages += 1
    lastMessage = text
    if verifiedIncoming.count == 64 { verifiedIncoming.removeFirst() }
    verifiedIncoming.append(CertifiedIncomingText(authorId: authorId, objectId: objectId, verifiedAtUnixSeconds: verifiedAt, body: text))
    log("Certified durable text committed locally")
    _ = drainDurableReceiptOutbox()
  }

  func drainVerifiedIncomingText() -> [CertifiedIncomingText] {
    let result = verifiedIncoming
    verifiedIncoming.removeAll(keepingCapacity: true)
    return result
  }

  func drainVerifiedIncomingVoice() -> [CertifiedIncomingVoice] {
    let result = verifiedIncomingVoice
    verifiedIncomingVoice.removeAll(keepingCapacity: true)
    return result
  }

  /// Starts each persisted source record across live authenticated edges.
  /// If all radios are unavailable the encrypted outbox remains for a future
  /// reconnect; the user action is not duplicated.
  private func drainDurableOriginOutbox() -> Bool {
    var records = [[UInt8]]()
    for slot in 0..<maxDurableOriginSlots {
      guard let record = runtime({ try NativeRuntime.shared.outboxRecord(slot: UInt16(slot)) }) else { return false }
      if record.isEmpty { break }
      guard record.count <= 4096 else { return false }
      records.append(record)
    }
    guard !records.isEmpty else { return false }
    _ = sendProtected(records)
    return true
  }

  private func drainDurableReceiptOutbox() -> Bool {
    var records = [[UInt8]]()
    for slot in 0..<maxDurableOriginSlots {
      guard let record = runtime({ try NativeRuntime.shared.receiptRecord(slot: UInt16(slot)) }) else { return false }
      if record.isEmpty { break }
      guard record.count <= 4096 else { return false }
      records.append(record)
    }
    guard !records.isEmpty else { return false }
    _ = sendProtected(records)
    return true
  }

  /// The origin only signs this after committing a valid recipient receipt.
  /// A recipient keeps audit evidence but stops replaying that receipt once it
  /// sees this origin-signed ACK.
  private func drainDurableReceiptAckOutbox() -> Bool {
    var records = [[UInt8]]()
    for slot in 0..<maxDurableOriginSlots {
      guard let material = try? SecureIdentity().groupMaterial(),
            let record = runtime({ try NativeRuntime.shared.receiptAckRecord(slot: UInt16(slot), material: material) }) else { return false }
      if record.isEmpty { break }
      guard record.count <= 4096 else { return false }
      records.append(record)
    }
    guard !records.isEmpty else { return false }
    _ = sendProtected(records)
    return true
  }

  private func isIngressNeighbor(_ link: SessionLink, _ receivedFrom: [UInt8]) -> Bool {
    guard let member = runtime({ try NativeRuntime.shared.sessionPeer(link.handle) }) else { return true }
    return member == receivedFrom
  }

  private func protectedRelayFrames(_ records: [[UInt8]], link: SessionLink) -> [[UInt8]]? {
    guard runtime({ try NativeRuntime.shared.sessionAuthenticated(link.handle) }) == true else { return nil }
    var frames = [[UInt8]]()
    for record in records {
      guard let frame = runtime({ try NativeRuntime.shared.sessionSend(link.handle, bytes: record) }) else { return nil }
      frames.append(frame)
    }
    return frames
  }

  /// Receipt forwarding uses the post-Noise routed frame generated for this
  /// hop. As with object custody, the ingress link is excluded; WFA output is
  /// intentionally left to its per-neighbor scheduler until iOS can select
  /// one NDP peer precisely.
  private func relayPayload(_ sourceHandle: UInt64, payload: [UInt8]) {
    for (id, link) in clientSessions {
      guard link.handle != sourceHandle, let peer = connected[id],
            let frames = protectedRelayFrames([payload], link: link) else { continue }
      enqueueClient(peer, raw: frames)
    }
    for (id, link) in serverSessions {
      guard link.handle != sourceHandle, let peer = subscribers[id],
            let frames = protectedRelayFrames([payload], link: link) else { continue }
      enqueueServer(peer, raw: frames)
    }
  }

  private func textPacket(_ bytes: [UInt8]) -> [UInt8] {
    let id = UInt32.random(in: UInt32.min...UInt32.max)
    return [textMarker, UInt8((id >> 24) & 0xff), UInt8((id >> 16) & 0xff),
            UInt8((id >> 8) & 0xff), UInt8(id & 0xff)] + bytes
  }

  /// Returns nil for a repeat sent by the second encrypted radio link.
  private func unwrapText(_ data: [UInt8]) -> [UInt8]? {
    guard data.first == textMarker, data.count >= textHeaderBytes else { return data }
    let id = (UInt32(data[1]) << 24) | (UInt32(data[2]) << 16) |
      (UInt32(data[3]) << 8) | UInt32(data[4])
    guard !recentTextIds.contains(id) else { return nil }
    recentTextIds.insert(id); recentTextOrder.append(id)
    if recentTextOrder.count > maxRecentTextIds {
      recentTextIds.remove(recentTextOrder.removeFirst())
    }
    return Array(data.dropFirst(textHeaderBytes))
  }

  private var hasGroup: Bool {
    runtime { try NativeRuntime.shared.policyEpoch() > 0 } == true
  }

  /// Installs the product roster allowed to join automatically. `nil` is
  /// reserved for Mesh Lab's explicit open-enrollment mode; an empty set denies
  /// every applicant. This method must run on the UI/main queue with BLE state.
  func setEnrollmentAllowedMembers(_ members: Set<String>?, authorityEnabled: Bool = true) {
    enrollmentAllowedMembers = members
    enrollmentAuthorityEnabled = authorityEnabled
  }

  private func enrollmentRequestAllowed(_ request: [UInt8]) -> Bool {
    guard let allowed = enrollmentAllowedMembers else { return true }
    guard enrollmentAuthorityEnabled else { return false }
    guard runtime({ try NativeRuntime.shared.canIssueEnrollment(try SecureIdentity().groupMaterial()) }) == true else {
      enrollmentDetail = "La autoridad del convoy cambió y requiere rotación segura."
      return false
    }
    guard let member = runtime({ try NativeRuntime.shared.enrollmentRequestMember(request) }) else {
      return false
    }
    let fingerprint = member.map { String(format: "%02x", $0) }.joined()
    return allowed.contains(fingerprint)
  }

  /// Returns true when the frame was reserved for the one-time public enrollment exchange.
  private func handleEnrollmentFromClient(_ raw: [UInt8], central: CBCentral) -> Bool {
    guard let record = decodeEnrollmentFrame(raw) else { return false }
    let kind = record.kind
    guard hasGroup else { return true }
    switch kind {
    case enrollmentHello:
      guard let policy = runtime({ try NativeRuntime.shared.exportInvitation() }) else {
        enrollmentDetail = "No se pudo preparar la incorporación Bluetooth."; return true
      }
      enrollmentDetail = "Enviando invitación segura por Bluetooth…"
      enqueueServer(central, raw: [enrollmentFrame(enrollmentInvitation, policy)])
    case enrollmentRequest:
      let request = record.payload
      guard !request.isEmpty, request.count <= 512 else { return true }
      guard enrollmentRequestAllowed(request) else {
        enrollmentDetail = "Solicitud no autorizada para este convoy."
        return true
      }
      // Repeating the same attempt is safe: if Android already authorized this
      // iPhone, the current public policy is exactly the bundle it needs.
      let policy = runtime { try NativeRuntime.shared.issueEnrollment(try SecureIdentity().groupMaterial(), request: request) }
        ?? runtime { try NativeRuntime.shared.exportInvitation() }
      guard let policy else { enrollmentDetail = "Android no pudo autorizar este teléfono."; return true }
      enrollmentDetail = "Segundo teléfono autorizado por Bluetooth."
      enqueueServer(central, raw: [enrollmentFrame(enrollmentPolicy, policy)])
    default:
      break
    }
    return true
  }

  private func handleEnrollmentFromServer(_ raw: [UInt8], peripheral: CBPeripheral) -> Bool {
    guard let record = decodeEnrollmentFrame(raw) else { return false }
    let kind = record.kind
    guard !hasGroup else { return true }
    let payload = record.payload
    switch kind {
    case enrollmentInvitation:
      guard let request = runtime({ try NativeRuntime.shared.createEnrollmentRequest(try SecureIdentity().groupMaterial(), invitation: payload) }) else {
        enrollmentDetail = "No se pudo validar la invitación Bluetooth."; return true
      }
      enrollmentDetail = "Solicitando incorporación al Android…"
      enqueueClient(peripheral, raw: [enrollmentFrame(enrollmentRequest, request)])
    case enrollmentPolicy:
      guard runtime({ try NativeRuntime.shared.installPolicy(payload) }) != nil else {
        enrollmentDetail = "Android entregó un grupo no válido."; return true
      }
      enrollmentDetail = "✓ Grupo incorporado. Protegiendo la conexión Bluetooth…"
      startClient(peripheral)
    default:
      break
    }
    return true
  }

  private func enrollmentFrame(_ kind: UInt8, _ payload: [UInt8]) -> [UInt8] {
    // A two-phone group is well below this ceiling. The bound is also enforced
    // by BleFrameCodec before it reaches CoreBluetooth.
    guard enrollmentDomain.count + 1 + payload.count <= 4160 else {
      enrollmentDetail = "El grupo es demasiado grande para esta incorporación Bluetooth."
      return enrollmentDomain + [kind]
    }
    return enrollmentDomain + [kind] + payload
  }

  private func decodeEnrollmentFrame(_ raw: [UInt8]) -> (kind: UInt8, payload: [UInt8])? {
    guard raw.count >= enrollmentDomain.count + 1,
          Array(raw.prefix(enrollmentDomain.count)) == enrollmentDomain else { return nil }
    let kind = raw[enrollmentDomain.count]
    guard kind >= enrollmentHello, kind <= enrollmentPolicy else { return nil }
    return (kind, Array(raw.dropFirst(enrollmentDomain.count + 1)))
  }
  /// Installs the direct Wi‑Fi Aware payload path. The closure may return true
  /// only after a Noise-authenticated WFA link exists; scheduling a radio scan
  /// alone never makes a text sendable.
  func setAwarePayloadSender(_ sender: @escaping ([UInt8]) -> Bool) {
    awarePayloadSender = sender
  }

  /// Handles an application payload decrypted by the independent Wi‑Fi Aware
  /// Noise session. BLE and WFA intentionally share text/voice decoding and
  /// message-ID deduplication, so a fallback cannot duplicate the chat.
  func acceptAwarePayload(_ data: [UInt8], noiseHandle: UInt64, reply: @escaping ([UInt8]) -> Void) {
    guard !data.isEmpty else { return }
    if acceptRoutedPayload(data, noiseHandle: noiseHandle) { return }
    if data.first == receiptMarker, data.count == textHeaderBytes {
      log("Wi-Fi Aware text receipt received")
      return
    }
    if data.first == heartbeatMarker, data.count == textHeaderBytes {
      reply([heartbeatAckMarker] + Array(data[1..<textHeaderBytes]))
      return
    }
    if data.first == heartbeatAckMarker, data.count == textHeaderBytes { return }
    if data.first == presenceMarker, data.count == 2 { return }
    if data.first == textMarker, data.count >= textHeaderBytes {
      reply([receiptMarker] + Array(data[1..<textHeaderBytes]))
    }
    if !acceptVoiceChunk(data), let text = unwrapText(data),
       let message = String(bytes: text, encoding: .utf8) {
      messages += 1
      lastMessage = message
      log("Protected text received via Wi-Fi Aware")
    }
  }

  func sendText(_ message: String, logicalId: String) -> Bool {
    let bytes = Array(message.utf8)
    guard let id = Self.decodeLogicalId(logicalId) else { return false }
    guard !message.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, bytes.count <= 2048,
          let material = try? SecureIdentity().groupMaterial(),
          let queued = runtime({ try NativeRuntime.shared.enqueueDurableText(bytes, logicalId: id, material: material) }),
          queued > 0 else { return false }
    _ = drainDurableOriginOutbox()
    return true
  }
  static func decodeLogicalId(_ value: String) -> [UInt8]? {
    guard value.count == 32, value.allSatisfy({ $0.isHexDigit }) else { return nil }
    var bytes: [UInt8] = []
    bytes.reserveCapacity(16)
    var index = value.startIndex
    for _ in 0..<16 {
      let next = value.index(index, offsetBy: 2)
      guard let byte = UInt8(value[index..<next], radix: 16) else { return nil }
      bytes.append(byte); index = next
    }
    return bytes
  }

  /// The app hands us an AAC/M4A file. Raw PCM is never sent through Dart or BLE.
  func sendVoice(_ audio: [UInt8], durationMillis: Int64, logicalId: String) -> Bool {
    sendVoiceInternal(audio, durationMillis: durationMillis, logicalId: logicalId, context: nil)
  }

  /// Product context is encrypted inside the durable signed object. It is
  /// bounded before allocation and has no effect on routing or authorization.
  func sendVoiceWithContext(_ audio: [UInt8], durationMillis: Int64, logicalId: String, context: String) -> Bool {
    sendVoiceInternal(audio, durationMillis: durationMillis, logicalId: logicalId, context: context)
  }

  private func sendVoiceInternal(_ audio: [UInt8], durationMillis: Int64, logicalId: String, context: String?) -> Bool {
    guard !audio.isEmpty, audio.count <= maxVoiceBytes,
          durationMillis > 0, durationMillis <= maxVoiceDurationMillis else { return false }
    guard let logical = Self.decodeLogicalId(logicalId) else { return false }
    let contextBytes = context.map { Array($0.utf8) }
    guard contextBytes == nil || (!(contextBytes?.isEmpty ?? true) && contextBytes!.count <= maxVoiceContextBytes) else { return false }
    guard durationMillis <= Int64(UInt32.max), let material = try? SecureIdentity().groupMaterial() else { return false }
    let header: [UInt8]
    if let contextBytes {
      header = [voiceMarker, durableVoiceContextVersion,
        UInt8((durationMillis >> 24) & 0xff), UInt8((durationMillis >> 16) & 0xff),
        UInt8((durationMillis >> 8) & 0xff), UInt8(durationMillis & 0xff)] + logical +
        [UInt8((contextBytes.count >> 8) & 0xff), UInt8(contextBytes.count & 0xff)] + contextBytes
    } else {
      header = [voiceMarker, durableVoiceVersion,
        UInt8((durationMillis >> 24) & 0xff), UInt8((durationMillis >> 16) & 0xff),
        UInt8((durationMillis >> 8) & 0xff), UInt8(durationMillis & 0xff)] + logical
    }
    guard header.count + audio.count <= maxVoiceBytes else { return false }
    let durable = header + audio
    guard let queued = runtime({ try NativeRuntime.shared.enqueueDurableText(durable, logicalId: logical, material: material) }),
          queued > 0 else { return false }
    _ = drainDurableOriginOutbox()
    return true
  }

  func voiceInfo() -> VoiceInfo {
    let ready = lastVoiceFile.map { FileManager.default.fileExists(atPath: $0.path) } ?? false
    return VoiceInfo(receivedCount: receivedVoices, lastDurationMillis: lastVoiceDurationMillis,
                     ready: ready,
                     detail: ready ? "Nota de voz recibida. Lista para reproducir."
                                   : "Aún no hay una nota de voz recibida.")
  }

  func playLastVoice() -> Bool {
    playVoiceFile(lastVoiceFile)
  }

  /// The product never receives a file path. It can replay only an immutable,
  /// receipt-certified voice object returned by the native FIFO.
  func playVoice(_ objectId: String) -> Bool {
    guard objectId.range(of: "^[0-9a-f]{64}$", options: .regularExpression) != nil else { return false }
    let file = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
      .appendingPathComponent("mesh-voice", isDirectory: true)
      .appendingPathComponent("\(objectId).m4a")
    return playVoiceFile(file)
  }

  private func playVoiceFile(_ file: URL?) -> Bool {
    guard let file, FileManager.default.fileExists(atPath: file.path) else { return false }
    do {
      voicePlayer = try AVAudioPlayer(contentsOf: file)
      guard voicePlayer?.prepareToPlay() == true else { return false }
      return voicePlayer?.play() == true
    } catch {
      voicePlayer = nil
      return false
    }
  }

  private func sendProtected(_ payloads: [[UInt8]]) -> Bool {
    if let awarePayloadSender,
       payloads.allSatisfy({ awarePayloadSender($0) }) {
      return true
    }
    guard let pair = clientSessions.first(where: { pair in
      runtime { try NativeRuntime.shared.sessionAuthenticated(pair.value.handle) } == true
    }), let peer = connected[pair.key] else { return false }
    var frames = [[UInt8]]()
    for payload in payloads {
      guard let frame = runtime({ try NativeRuntime.shared.sessionSend(pair.value.handle, bytes: payload) }) else { return false }
      frames.append(frame)
    }
    enqueueClient(peer, raw: frames)
    return true
  }

  /// Consumes only valid voice envelopes. Invalid binary data is left out of text history.
  private func acceptVoiceChunk(_ data: [UInt8]) -> Bool {
    guard data.first == voiceMarker else { return false }
    guard data.count > voiceHeaderBytes else { return true }
    let id = (UInt32(data[1]) << 24) | (UInt32(data[2]) << 16) | (UInt32(data[3]) << 8) | UInt32(data[4])
    let index = (Int(data[5]) << 8) | Int(data[6])
    let total = (Int(data[7]) << 8) | Int(data[8])
    let durationMillis = Int64((Int(data[9]) << 8) | Int(data[10]))
    guard total > 0, total <= maxVoiceChunks, index >= 0, index < total,
          durationMillis > 0, durationMillis <= maxVoiceDurationMillis else { return true }
    let part = Array(data.dropFirst(voiceHeaderBytes))
    if let old = voiceAssemblies[id], (old.total != total || old.durationMillis != durationMillis) {
      voiceAssemblies.removeValue(forKey: id)
      return true
    }
    if voiceAssemblies[id] == nil {
      if voiceAssemblies.count >= 2 { voiceAssemblies.removeAll() }
      voiceAssemblies[id] = VoiceAssembly(total: total, durationMillis: durationMillis)
    }
    guard let assembly = voiceAssemblies[id] else { return true }
    if assembly.chunks[index] == nil { assembly.chunks[index] = part }
    guard assembly.chunks.count == assembly.total else { return true }
    var audio = [UInt8]()
    for position in 0..<assembly.total {
      guard let chunk = assembly.chunks[position] else { return true }
      audio += chunk
    }
    voiceAssemblies.removeValue(forKey: id)
    guard !audio.isEmpty, audio.count <= maxVoiceBytes else { return true }
    do {
      let folder = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("mesh-voice", isDirectory: true)
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      let file = folder.appendingPathComponent("latest.m4a")
      try Data(audio).write(to: file, options: .atomic)
      lastVoiceFile = file
      lastVoiceDurationMillis = assembly.durationMillis
      receivedVoices += 1
    } catch { }
    return true
  }

  private func voicePacket(id: UInt32, index: Int, total: Int, durationMillis: Int64,
                           audio: [UInt8]) -> [UInt8] {
    var packet: [UInt8] = [voiceMarker,
      UInt8((id >> 24) & 0xff), UInt8((id >> 16) & 0xff),
      UInt8((id >> 8) & 0xff), UInt8(id & 0xff),
      UInt8((index >> 8) & 0xff), UInt8(index & 0xff),
      UInt8((total >> 8) & 0xff), UInt8(total & 0xff),
      UInt8((durationMillis >> 8) & 0xff), UInt8(durationMillis & 0xff)]
    packet += audio
    return packet
  }
  private func enqueueClient(_ peripheral: CBPeripheral, raw: [[UInt8]]) {
    let maximum = peripheral.maximumWriteValueLength(for: .withResponse)
    var queue = clientWrites[peripheral.identifier, default: []]
    for value in raw { guard let framed = runtime({ try NativeRuntime.shared.linkEncode(value) }), let parts = BleFrameCodec.split(framed, maximumWrite: maximum) else { return }; queue += parts }
    clientWrites[peripheral.identifier] = queue; pumpClient(peripheral)
  }
  private func pumpClient(_ peripheral: CBPeripheral) {
    guard var queue = clientWrites[peripheral.identifier], !queue.isEmpty,
          let service = peripheral.services?.first(where: { $0.uuid == serviceUUID }),
          let rx = service.characteristics?.first(where: { $0.uuid == rxUUID }) else { return }
    let next = queue.removeFirst(); clientWrites[peripheral.identifier] = queue
    peripheral.writeValue(Data(next), for: rx, type: .withResponse)
  }
  private func enqueueServer(_ central: CBCentral, raw: [[UInt8]]) {
    var queue = serverWrites[central.identifier, default: []]
    for value in raw { guard let framed = runtime({ try NativeRuntime.shared.linkEncode(value) }), let parts = BleFrameCodec.split(framed, maximumWrite: 185) else { return }; queue += parts }
    serverWrites[central.identifier] = queue; pumpServer(central)
  }
  private func pumpServer(_ central: CBCentral) {
    guard var queue = serverWrites[central.identifier], !queue.isEmpty, let tx = txCharacteristic else { return }
    let next = queue.removeFirst()
    if peripheral?.updateValue(Data(next), for: tx, onSubscribedCentrals: [central]) == true { serverWrites[central.identifier] = queue }
  }
  private func closeClient(_ peripheral: CBPeripheral) { if let link = clientSessions.removeValue(forKey: peripheral.identifier) { runtime { NativeRuntime.shared.releaseSession(link.handle) } }; clientWrites.removeValue(forKey: peripheral.identifier); inbound.removeValue(forKey: "c:\(peripheral.identifier.uuidString)"); central?.cancelPeripheralConnection(peripheral) }
  private func closeServer(_ id: UUID) { if let link = serverSessions.removeValue(forKey: id) { runtime { NativeRuntime.shared.releaseSession(link.handle) } }; serverWrites.removeValue(forKey: id); inbound.removeValue(forKey: "s:\(id.uuidString)") }

  /// Bluetooth can disappear underneath an active GATT connection without a
  /// `didDisconnectPeripheral` callback. A new Android GATT session must never
  /// reuse the old Noise handle after the radio returns.
  private func resetVolatileLinksForRadioRestart() {
    for link in clientSessions.values { runtime { NativeRuntime.shared.releaseSession(link.handle) } }
    for link in serverSessions.values { runtime { NativeRuntime.shared.releaseSession(link.handle) } }
    clientSessions.removeAll(); serverSessions.removeAll()
    clientWrites.removeAll(); serverWrites.removeAll(); inbound.removeAll()
    connected.removeAll(); peers.removeAll(); subscribers.removeAll()
    enrollmentDetail = "Bluetooth se reinició. Reconectando automáticamente…"
  }

  func centralManagerDidUpdateState(_ central: CBCentralManager) {
    centralState = central.state
    log("Central state \(central.state.rawValue)")
    if central.state == .poweredOn { beginCentralScanIfReady() }
    else { resetVolatileLinksForRadioRestart() }
  }

  func centralManager(_ central: CBCentralManager, didDiscover peripheral: CBPeripheral,
                      advertisementData: [String: Any], rssi RSSI: NSNumber) {
    let services = advertisementData[CBAdvertisementDataServiceUUIDsKey] as? [CBUUID] ?? []
    guard services.contains(serviceUUID) else { return }
    if peers.insert(peripheral.identifier).inserted {
      log("Found Mesh Lab advertisement \(peripheral.identifier.uuidString)")
      connected[peripheral.identifier] = peripheral
      peripheral.delegate = self
      central.connect(peripheral)
    }
  }

  func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
    log("Connected to Android GATT \(peripheral.identifier.uuidString)")
    peripheral.discoverServices([serviceUUID])
  }

  func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral,
                      error: Error?) {
    let reason = error?.localizedDescription ?? "none"
    log("Disconnected from Android GATT \(peripheral.identifier.uuidString): \(reason)")
    connected.removeValue(forKey: peripheral.identifier)
    peers.remove(peripheral.identifier)
    closeClient(peripheral)
    scheduleReconnect()
  }

  /// A BLE disconnect is normal when either OS restarts its radio or the phone
  /// briefly leaves range. Keep the recovery local and visible; the user must
  /// never recreate a group to restore an already authenticated peer.
  private func scheduleReconnect() {
    guard requested else { return }
    reconnectWorkItem?.cancel()
    enrollmentDetail = "Conexión interrumpida. Reconectando automáticamente…"
    // CoreBluetooth reports `isScanning` until the current run-loop turn ends.
    // Restarting synchronously here is silently ignored, leaving the app with
    // no scanner after a temporary disconnect.
    central?.stopScan()
    let work = DispatchWorkItem { [weak self] in
      guard let self, self.requested else { return }
      self.beginCentralScanIfReady()
    }
    reconnectWorkItem = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 1, execute: work)
  }

  func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
    guard error == nil else { log("Service discovery failed: \(error!.localizedDescription)"); return }
    for service in peripheral.services ?? [] where service.uuid == serviceUUID {
      peripheral.discoverCharacteristics([rxUUID, txUUID], for: service)
    }
  }

  func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService,
                  error: Error?) {
    guard error == nil, let tx = service.characteristics?.first(where: { $0.uuid == txUUID }) else { log("Characteristic discovery failed"); return }
    log("Subscribing to Android notifications")
    peripheral.setNotifyValue(true, for: tx)
  }

  func peripheral(_ peripheral: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic,
                  error: Error?) {
    if let error { log("Notification subscription failed: \(error.localizedDescription)"); return }
    if characteristic.uuid == txUUID, characteristic.isNotifying {
      log("Android notifications enabled")
      startClient(peripheral)
    }
  }

  func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic,
                  error: Error?) {
    if let error {
      log("Android notification failed: \(error.localizedDescription)")
      return
    }
    log("Android value callback \(characteristic.uuid.uuidString), \(characteristic.value?.count ?? 0) bytes")
    if characteristic.uuid == txUUID, let value = characteristic.value { receiveFromClient(peripheral, fragment: value) }
  }

  func peripheral(_ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic,
                  error: Error?) {
    if error == nil, characteristic.uuid == rxUUID { pumpClient(peripheral) } else { closeClient(peripheral) }
  }

  func peripheralManagerDidUpdateState(_ peripheral: CBPeripheralManager) {
    peripheralState = peripheral.state
    beginPeripheralIfReady()
  }

  func peripheralManager(_ peripheral: CBPeripheralManager, didAdd service: CBService, error: Error?) {
    guard requested, error == nil else { return }
    peripheral.startAdvertising([CBAdvertisementDataServiceUUIDsKey: [serviceUUID]])
    advertising = true
  }

  func peripheralManager(_ peripheral: CBPeripheralManager,
                         didReceiveWrite requests: [CBATTRequest]) {
    for request in requests {
      guard request.characteristic.uuid == rxUUID, request.offset == 0, let value = request.value else { continue }
      receiveFromServer(request.central, fragment: value)
      peripheral.respond(to: request, withResult: .success)
    }
  }

  func peripheralManager(_ peripheral: CBPeripheralManager, central: CBCentral,
                         didSubscribeTo characteristic: CBCharacteristic) {
    if characteristic.uuid == txUUID { subscribers[central.identifier] = central }
  }
  func peripheralManager(_ peripheral: CBPeripheralManager, central: CBCentral,
                         didUnsubscribeFrom characteristic: CBCharacteristic) {
    if characteristic.uuid == txUUID { subscribers.removeValue(forKey: central.identifier); closeServer(central.identifier) }
  }
  func peripheralManagerIsReady(toUpdateSubscribers peripheral: CBPeripheralManager) {
    for central in subscribers.values { pumpServer(central) }
  }
}
