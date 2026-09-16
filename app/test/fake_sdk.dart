import 'package:mesh_host/mesh_host.dart';

import 'dart:typed_data';

import 'package:mesh_lab/core/sdk/lab_controller.dart';

/// Test-only fake, never selected by the application's entry point.
class FakeLabSdk implements LabSdk {
  int cursor = 0;
  int runtimeId = 7;
  final List<DiagnosticEvent> events = [];
  final List<int> requests = [];
  EngineSnapshot snapshot({bool reset = false, int? after}) => EngineSnapshot(
    runtimeId: runtimeId,
    cursor: cursor,
    probeCount: cursor,
    foundationState: 0,
    cursorReset: reset,
    events: reset
        ? []
        : events.where((e) => e.sequence > (after ?? 0)).toList(),
  );
  @override
  Future<IdentityInfo> prepareIdentity() async =>
      IdentityInfo(fingerprint: 'ab' * 32, storage: 'Keychain');
  @override
  Future<GroupInfo> groupInfo() async => GroupInfo(configured: false, epoch: 0);
  @override
  Future<GroupInfo> createGroup() async =>
      GroupInfo(configured: true, epoch: 1);
  @override
  Future<BluetoothInfo> bluetoothInfo() async => BluetoothInfo(
    available: true,
    authorized: true,
    enabled: true,
    active: false,
    peerCount: 0,
    probeCount: 0,
    authenticated: false,
    messageCount: 0,
    lastMessage: '',
    detail: 'Bluetooth listo.',
  );
  @override
  Future<BluetoothInfo> prepareBluetooth() => bluetoothInfo();
  @override
  Future<BluetoothInfo> startBluetoothDiscovery() async => BluetoothInfo(
    available: true,
    authorized: true,
    enabled: true,
    active: true,
    peerCount: 0,
    probeCount: 0,
    authenticated: false,
    messageCount: 0,
    lastMessage: '',
    detail: 'Buscando teléfonos del laboratorio.',
  );
  @override
  Future<BluetoothInfo> stopBluetoothDiscovery() => bluetoothInfo();
  @override
  Future<AwareInfo> awareInfo() async => AwareInfo(
    available: true,
    enabled: true,
    active: false,
    peerCount: 0,
    maxPeers: 2,
    state: 'stopped',
    detail: 'Wi-Fi Aware listo.',
  );
  @override
  Future<AwareInfo> startAwareDiscovery() => awareInfo();
  @override
  Future<AwareInfo> stopAwareDiscovery() => awareInfo();
  @override
  Future<bool> sendText(String message, String logicalId) async => false;
  @override
  Future<DeliveryInfo> deliveryInfo(String logicalId) async => DeliveryInfo(
    logicalId: '',
    targetCount: 0,
    deliveredCount: 0,
    state: 'none',
  );
  @override
  Future<VoiceInfo> voiceInfo() async => VoiceInfo(
    receivedCount: 0,
    lastDurationMillis: 0,
    ready: false,
    detail: 'Aún no hay una nota de voz recibida.',
  );
  @override
  Future<bool> sendVoice(
    Uint8List audio,
    int durationMillis,
    String logicalId,
  ) async => false;
  @override
  Future<bool> playLastVoice() async => false;
  @override
  Future<EngineInfo> engineInfo() async => EngineInfo(
    engineVersion: '0.1.0',
    abiVersion: 1,
    apiVersion: 1,
    phase: 'F0',
    buildId: 'test-only',
  );
  @override
  Future<EngineSnapshot> subscribe(int cursor) async => snapshot(after: cursor);
  @override
  Future<EngineSnapshot> verifyBridge(int requestId) async {
    requests.add(requestId);
    if (!events.any((e) => e.requestId == requestId)) {
      cursor++;
      events.add(
        DiagnosticEvent(sequence: cursor, requestId: requestId, kind: 0),
      );
    }
    return snapshot();
  }
}
