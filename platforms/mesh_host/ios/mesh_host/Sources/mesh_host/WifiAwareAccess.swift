import DeviceDiscoveryUI
import Foundation
import Network
import UIKit
import WiFiAware

/// Native Wi‑Fi Aware transport for iPhone↔iPhone. It deliberately uses the
/// iOS 26 `NetworkConnection<TLS>` API rather than the retired LAN `NWConnection`
/// test channel. TLS protects the local radio path and Mesh Lab's Noise session
/// binds every payload to the current group before Bluetooth/UI can consume it.
final class WifiAwareAccess {
  private let serviceName = "_meshlab._tcp"
  private let maxFrameBytes = 64 * 1024
  private var active = false
  private var pairedCount: Int64 = 0
  private var detail = "Wi‑Fi Aware esperando una sesión de campo."
  private var refreshTimer: Timer?
  private var pairingVisible = false
  private var reconnectWorkItem: DispatchWorkItem?

  @available(iOS 26.0, *)
  private final class Link {
    let id: String
    let connection: NetworkConnection<TLS>
    let noiseHandle: UInt64
    let initiator: Bool
    var stage = 0
    var authenticated = false

    init(id: String, connection: NetworkConnection<TLS>, noiseHandle: UInt64, initiator: Bool) {
      self.id = id
      self.connection = connection
      self.noiseHandle = noiseHandle
      self.initiator = initiator
    }
  }

  /// Stream writes must stay ordered: a length header and its body cannot be
  /// interleaved with another chat, GPS, receipt or voice frame.
  @available(iOS 26.0, *)
  private actor FrameWriter {
    private let connection: NetworkConnection<TLS>
    init(_ connection: NetworkConnection<TLS>) { self.connection = connection }

    func send(_ raw: [UInt8]) async throws {
      let count = raw.count
      guard count > 0, count <= 64 * 1024 else { throw FrameError.invalidLength }
      let header = Data([
        UInt8((count >> 24) & 0xff), UInt8((count >> 16) & 0xff),
        UInt8((count >> 8) & 0xff), UInt8(count & 0xff),
      ])
      try await connection.send(header)
      try await connection.send(Data(raw))
    }
  }

  private enum FrameError: Error { case invalidLength }

  /// iOS 15 remains supported for BLE. Keep iOS-26-only storage behind an
  /// erased reference; Swift does not permit availability attributes on stored
  /// properties of an otherwise iOS-15 class.
  @available(iOS 26.0, *)
  private final class DataPlane {
    var listenerTask: Task<Void, Never>?
    var browserTask: Task<Void, Never>?
    var links = [String: Link]()
    var writers = [String: FrameWriter]()
    var connectingEndpoints = Set<String>()
  }
  private var dataPlane: AnyObject?

  @available(iOS 26.0, *)
  private func plane() -> DataPlane {
    if let existing = dataPlane as? DataPlane { return existing }
    let created = DataPlane()
    dataPlane = created
    return created
  }

  @available(iOS 26.0, *)
  private func currentPlane() -> DataPlane? { dataPlane as? DataPlane }

  /// Installed by `MeshHostPlugin`. The Bluetooth host owns shared chat/voice
  /// decoding and dedupe; Wi‑Fi Aware owns only its radio, TLS and Noise link.
  private var payloadReceiver: (([UInt8], UInt64, @escaping ([UInt8]) -> Void) -> Void)?

  func setPayloadReceiver(_ receiver: @escaping ([UInt8], UInt64, @escaping ([UInt8]) -> Void) -> Void) {
    payloadReceiver = receiver
  }

  func info(hasGroup: Bool) -> AwareInfo {
    guard #available(iOS 26.0, *) else {
      return AwareInfo(available: false, enabled: false, active: false, peerCount: 0, maxPeers: 0,
                       state: "unsupported", detail: "Este iPhone requiere iOS 26 o posterior para Wi‑Fi Aware.")
    }
    let available = WACapabilities.supportedFeatures.contains(.wifiAware)
    let secureLinks = currentPlane()?.links.values.filter(\.authenticated).count ?? 0
    let state: String
    if !available { state = "unsupported" }
    else if !hasGroup { state = "needs_group" }
    else if !active { state = "stopped" }
    else if secureLinks > 0 { state = "connected" }
    else if pairedCount == 0 { state = "pairing_required" }
    else { state = "paired_waiting" }
    let text: String
    if !available { text = "Este iPhone no soporta Wi‑Fi Aware." }
    else if !hasGroup { text = "Crea o incorpora el grupo antes de abrir Wi‑Fi Aware." }
    else { text = detail }
    return AwareInfo(available: available, enabled: available, active: active,
                     peerCount: Int64(secureLinks), maxPeers: Int64(WACapabilities.maximumConnectableDevices), state: state, detail: text)
  }

  func start(hasGroup: Bool) -> AwareInfo {
    guard hasGroup else { return info(hasGroup: false) }
    guard #available(iOS 26.0, *) else { return info(hasGroup: true) }
    guard WACapabilities.supportedFeatures.contains(.wifiAware) else { return info(hasGroup: true) }
    active = true
    detail = "Wi‑Fi Aware activo. Consultando teléfonos vinculados…"
    refreshPairedDevices()
    presentSystemPairingIfNeeded()
    refreshTimer?.invalidate()
    refreshTimer = Timer.scheduledTimer(withTimeInterval: 3, repeats: true) { [weak self] _ in
      self?.refreshPairedDevices()
    }
    return info(hasGroup: true)
  }

  /// Returns true only while there is a complete Noise-authenticated WFA path.
  /// The Pigeon send API is synchronous, so actual stream writing continues in
  /// the ordered writer actor after this admission check.
  func sendProtectedPayload(_ payload: [UInt8]) -> Bool {
    guard #available(iOS 26.0, *), !payload.isEmpty, payload.count <= maxFrameBytes,
          let link = currentPlane()?.links.values.first(where: { $0.authenticated }) else { return false }
    guard let frame = runtime({ try NativeRuntime.shared.sessionSend(link.noiseHandle, bytes: payload) }) else { return false }
    write(frame, on: link)
    return true
  }

  @available(iOS 26.0, *)
  private func refreshPairedDevices() {
    guard active else { return }
    Task { [weak self] in
      do {
        let devices = try await WAPairedDevice.allDevices.current() ?? [:]
        DispatchQueue.main.async {
          guard let self, self.active else { return }
          self.pairedCount = Int64(devices.count)
          if devices.isEmpty {
            self.detail = "Wi‑Fi Aware activo. El sistema debe vincular el otro teléfono antes del enlace."
          } else if let plane = self.currentPlane(), plane.links.values.contains(where: \.authenticated) {
            self.detail = "Wi‑Fi Aware seguro activo con \(plane.links.values.filter(\.authenticated).count) teléfono(s) del grupo."
          } else {
            self.detail = "Wi‑Fi Aware activo: \(devices.count) teléfono(s) vinculado(s). Abriendo enlace seguro…"
            self.startDataPlaneIfNeeded()
          }
        }
      } catch {
        DispatchQueue.main.async {
          guard let self, self.active else { return }
          self.detail = "Wi‑Fi Aware requiere autorización del sistema: \(error.localizedDescription)"
        }
      }
    }
  }

  /// Wi‑Fi Aware pairing is deliberately owned by iOS. `Conectar sesión` is
  /// the only Mesh Lab action; the sheet is platform consent, not another app
  /// setup flow. Once paired, this controller is not shown on every reconnect.
  private func presentSystemPairingIfNeeded() {
    guard #available(iOS 26.0, *) else { return }
    guard !pairingVisible, pairedCount == 0,
          let service = WAPublishableService.allServices[serviceName] else { return }
    let provider: WAPublisherListener = .wifiAware(
      .connecting(to: service, from: .userSpecifiedDevices)
    )
    guard DDDevicePairingViewController.isSupported(provider),
          let presenter = foregroundPresenter() else {
      detail = "Wi‑Fi Aware está disponible, pero el sistema no pudo abrir su selector de dispositivo."
      return
    }
    pairingVisible = true
    detail = "Selecciona el teléfono del grupo en el selector seguro de iOS."
    let controller = DDDevicePairingViewController(listenerProvider: provider, access: .permanent)
    presenter.present(controller, animated: true) { [weak self] in
      // The periodic paired-device read is the source of truth for completion.
      self?.pairingVisible = false
    }
  }

  @available(iOS 26.0, *)
  private func startDataPlaneIfNeeded() {
    let plane = plane()
    guard active, pairedCount > 0, plane.listenerTask == nil, plane.browserTask == nil,
          let publishing = WAPublishableService.allServices[serviceName],
          let subscribing = WASubscribableService.allServices[serviceName] else { return }
    do {
      let provider: WAPublisherListener = .wifiAware(
        .connecting(to: publishing, from: .allPairedDevices)
      )
      let listener = try NetworkListener(for: provider, using: { TLS().peerAuthentication(.none) })
      plane.listenerTask = Task { [weak self, listener, plane] in
        do {
          try await listener.run { [weak self] connection in
            self?.acceptIncoming(connection)
          }
        } catch is CancellationError {
          // `Salir de sesión` or recovery cancels this run intentionally.
        } catch {
          DispatchQueue.main.async { [weak self] in self?.dataPlaneFailed("escucha", error: error, plane: plane) }
        }
      }
      let browser: NetworkBrowser<WASubscriberBrowser> = NetworkBrowser(
        for: .wifiAware(.connecting(to: .allPairedDevices, from: subscribing))
      )
      plane.browserTask = Task { [weak self, browser, plane] in
        do {
          try await browser.run { [weak self] endpoints in
            for endpoint in endpoints { self?.openOutgoing(endpoint) }
          }
        } catch is CancellationError {
          // `Salir de sesión` or recovery cancels this run intentionally.
        } catch {
          DispatchQueue.main.async { [weak self] in self?.dataPlaneFailed("búsqueda", error: error, plane: plane) }
        }
      }
      detail = "Wi‑Fi Aware activo. Buscando el teléfono vinculado…"
    } catch {
      dataPlaneFailed("inicio", error: error)
    }
  }

  @available(iOS 26.0, *)
  private func acceptIncoming(_ connection: NetworkConnection<TLS>) {
    DispatchQueue.main.async { [weak self] in self?.startLink(connection, initiator: false) }
  }

  @available(iOS 26.0, *)
  private func openOutgoing(_ endpoint: WASubscriberBrowser.Endpoint) {
    let key = String(describing: endpoint)
    let plane = plane()
    guard active, plane.links.isEmpty, !plane.connectingEndpoints.contains(key) else { return }
    plane.connectingEndpoints.insert(key)
    let connection = NetworkConnection(to: endpoint, using: { TLS().peerAuthentication(.none) })
    startLink(connection, initiator: true, endpointKey: key)
  }

  @available(iOS 26.0, *)
  private func startLink(_ connection: NetworkConnection<TLS>, initiator: Bool, endpointKey: String? = nil) {
    guard active else { return }
    let id = connection.id
    let plane = plane()
    guard plane.links[id] == nil else { return }
    guard let handle = runtime({ try NativeRuntime.shared.startSession(try SecureIdentity().groupMaterial(), initiator: initiator) }) else {
      detail = "Wi‑Fi Aware no pudo iniciar su sesión segura."
      return
    }
    let link = Link(id: id, connection: connection, noiseHandle: handle, initiator: initiator)
    plane.links[id] = link
    plane.writers[id] = FrameWriter(connection)
    connection.onStateUpdate { [weak self, weak link] _, state in
      guard case .failed = state else {
        guard case .cancelled = state else { return }
        DispatchQueue.main.async { if let link { self?.close(link, detail: "Wi‑Fi Aware se desconectó. Buscando de nuevo…") } }
        return
      }
      DispatchQueue.main.async { if let link { self?.close(link, detail: "Wi‑Fi Aware perdió el enlace. Buscando de nuevo…") } }
    }
    if initiator {
      guard let first = runtime({ try NativeRuntime.shared.sessionWrite(handle) }) else {
        close(link, detail: "Wi‑Fi Aware no pudo iniciar Noise.")
        return
      }
      write(first, on: link)
    }
    Task { [weak self, weak link] in
      guard let self, let link else { return }
      await self.receiveLoop(link)
    }
  }

  @available(iOS 26.0, *)
  private func receiveLoop(_ link: Link) async {
    do {
      while !Task.isCancelled {
        let header = try await link.connection.receive(exactly: 4).content
        let headerBytes = [UInt8](header)
        guard headerBytes.count == 4 else { throw FrameError.invalidLength }
        let length = (Int(headerBytes[0]) << 24) | (Int(headerBytes[1]) << 16) |
          (Int(headerBytes[2]) << 8) | Int(headerBytes[3])
        guard length > 0, length <= maxFrameBytes else { throw FrameError.invalidLength }
        let raw = [UInt8](try await link.connection.receive(exactly: length).content)
        guard raw.count == length else { throw FrameError.invalidLength }
        await handleFrame(raw, on: link)
      }
    } catch is CancellationError {
      // Deliberate teardown.
    } catch {
      DispatchQueue.main.async { [weak self, weak link] in
        if let link { self?.close(link, detail: "Wi‑Fi Aware se interrumpió. Buscando de nuevo…") }
      }
    }
  }

  @available(iOS 26.0, *)
  private func handleFrame(_ raw: [UInt8], on link: Link) async {
    if !link.initiator && link.stage == 0 {
      guard runtime({ try NativeRuntime.shared.sessionRead(link.noiseHandle, frame: raw) }) != nil,
            let response = runtime({ try NativeRuntime.shared.sessionWrite(link.noiseHandle) }) else {
        DispatchQueue.main.async { [weak self] in self?.close(link, detail: "Wi‑Fi Aware rechazó la sesión Noise.") }
        return
      }
      link.stage = 1
      write(response, on: link)
      return
    }
    if link.initiator && link.stage == 0 {
      guard runtime({ try NativeRuntime.shared.sessionRead(link.noiseHandle, frame: raw) }) != nil,
            let third = runtime({ try NativeRuntime.shared.sessionWrite(link.noiseHandle) }),
            runtime({ try NativeRuntime.shared.sessionFinish(link.noiseHandle) }) != nil,
            let proof = runtime({ try NativeRuntime.shared.sessionAuthenticate(link.noiseHandle, material: try SecureIdentity().groupMaterial()) }) else {
        DispatchQueue.main.async { [weak self] in self?.close(link, detail: "Wi‑Fi Aware rechazó la sesión Noise.") }
        return
      }
      link.stage = 2
      write(third, on: link); write(proof, on: link)
      markAuthenticated(link)
      return
    }
    if !link.initiator && link.stage == 1 {
      guard runtime({ try NativeRuntime.shared.sessionRead(link.noiseHandle, frame: raw) }) != nil,
            runtime({ try NativeRuntime.shared.sessionFinish(link.noiseHandle) }) != nil,
            let proof = runtime({ try NativeRuntime.shared.sessionAuthenticate(link.noiseHandle, material: try SecureIdentity().groupMaterial()) }) else {
        DispatchQueue.main.async { [weak self] in self?.close(link, detail: "Wi‑Fi Aware rechazó la sesión Noise.") }
        return
      }
      link.stage = 2
      write(proof, on: link)
      markAuthenticated(link)
      return
    }
    guard link.stage == 2,
          let payload = runtime({ try NativeRuntime.shared.sessionReceive(link.noiseHandle, frame: raw) }) else { return }
    DispatchQueue.main.async { [weak self, weak link] in
      guard let self, let link, self.currentPlane()?.links[link.id] === link else { return }
      self.payloadReceiver?(payload, link.noiseHandle) { [weak self, weak link] reply in
        guard let self, let link, self.currentPlane()?.links[link.id] === link else { return }
        self.sendReply(reply, on: link)
      }
    }
  }

  @available(iOS 26.0, *)
  private func markAuthenticated(_ link: Link) {
    DispatchQueue.main.async { [weak self, weak link] in
      guard let self, let link, let plane = self.currentPlane(), plane.links[link.id] === link,
            self.runtime({ try NativeRuntime.shared.sessionAuthenticated(link.noiseHandle) }) == true else { return }
      link.authenticated = true
      self.detail = "Wi‑Fi Aware seguro activo con \(plane.links.values.filter(\.authenticated).count) teléfono(s) del grupo."
    }
  }

  @available(iOS 26.0, *)
  private func sendReply(_ payload: [UInt8], on link: Link) {
    guard link.authenticated,
          let frame = runtime({ try NativeRuntime.shared.sessionSend(link.noiseHandle, bytes: payload) }) else { return }
    write(frame, on: link)
  }

  @available(iOS 26.0, *)
  private func write(_ raw: [UInt8], on link: Link) {
    guard let writer = currentPlane()?.writers[link.id], !raw.isEmpty, raw.count <= maxFrameBytes else { return }
    Task { [weak self, weak link] in
      do { try await writer.send(raw) }
      catch {
        DispatchQueue.main.async { if let link { self?.close(link, detail: "Wi‑Fi Aware no pudo enviar datos. Reconectando…") } }
      }
    }
  }

  @available(iOS 26.0, *)
  private func close(_ link: Link, detail nextDetail: String? = nil) {
    guard let plane = currentPlane(), plane.links.removeValue(forKey: link.id) != nil else { return }
    plane.writers.removeValue(forKey: link.id)
    runtime { NativeRuntime.shared.releaseSession(link.noiseHandle) }
    if let nextDetail { detail = nextDetail }
    if plane.links.values.allSatisfy({ !$0.authenticated }) {
      scheduleLinkRecovery()
    }
  }

  /// iOS may keep a browser alive after a datapath disappears without emitting
  /// another endpoint update. Recreate both sides after a short bounded delay,
  /// preserving the paired-device authorization and never asking the user to
  /// press the connection control again.
  @available(iOS 26.0, *)
  private func scheduleLinkRecovery() {
    guard active, pairedCount > 0 else { return }
    reconnectWorkItem?.cancel()
    let work = DispatchWorkItem { [weak self] in
      guard let self, self.active, self.pairedCount > 0,
            let old = self.currentPlane(), old.links.values.allSatisfy({ !$0.authenticated }) else { return }
      old.listenerTask?.cancel()
      old.browserTask?.cancel()
      self.dataPlane = nil
      self.detail = "Wi‑Fi Aware reconectando automáticamente…"
      self.startDataPlaneIfNeeded()
    }
    reconnectWorkItem = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.35, execute: work)
  }

  @available(iOS 26.0, *)
  private func dataPlaneFailed(_ stage: String, error: Error, plane: DataPlane? = nil) {
    guard active, plane == nil || currentPlane() === plane else { return }
    currentPlane()?.listenerTask = nil; currentPlane()?.browserTask = nil
    detail = "Wi‑Fi Aware no pudo iniciar \(stage): \(error.localizedDescription)"
    scheduleLinkRecovery()
  }

  private func foregroundPresenter() -> UIViewController? {
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    guard let window = scenes.first(where: { $0.activationState == .foregroundActive })?
      .windows.first(where: { $0.isKeyWindow }) else { return nil }
    var controller = window.rootViewController
    while let next = controller?.presentedViewController { controller = next }
    return controller
  }

  func stop(hasGroup: Bool) -> AwareInfo {
    active = false; pairedCount = 0; pairingVisible = false
    reconnectWorkItem?.cancel(); reconnectWorkItem = nil
    refreshTimer?.invalidate(); refreshTimer = nil
    if #available(iOS 26.0, *) {
      if let plane = currentPlane() {
        plane.listenerTask?.cancel(); plane.listenerTask = nil
        plane.browserTask?.cancel(); plane.browserTask = nil
        for link in plane.links.values { close(link) }
        plane.links.removeAll(); plane.writers.removeAll(); plane.connectingEndpoints.removeAll()
      }
      dataPlane = nil
    }
    detail = "Wi‑Fi Aware detenido por el usuario."
    return info(hasGroup: hasGroup)
  }

  @discardableResult private func runtime<T>(_ work: @escaping () throws -> T) -> T? {
    try? NativeRuntime.shared.queue.sync { try work() }
  }
}
