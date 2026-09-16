import 'package:pigeon/pigeon.dart';

@ConfigurePigeon(
  PigeonOptions(
    dartOut: 'lib/src/mesh_api.g.dart',
    swiftOut: 'ios/mesh_host/Sources/mesh_host/MeshApi.g.swift',
    kotlinOut: 'android/src/main/kotlin/com/frazko/mesh_host/MeshApi.g.kt',
    kotlinOptions: KotlinOptions(package: 'com.frazko.mesh_host'),
    dartPackageName: 'mesh_host',
  ),
)
class EngineInfo {
  EngineInfo({
    required this.engineVersion,
    required this.abiVersion,
    required this.apiVersion,
    required this.phase,
    required this.buildId,
  });
  String engineVersion;
  int abiVersion;
  int apiVersion;
  String phase;
  String buildId;
}

class DiagnosticEvent {
  DiagnosticEvent({
    required this.sequence,
    required this.requestId,
    required this.kind,
  });
  int sequence;
  int requestId;
  int kind;
}

class EngineSnapshot {
  EngineSnapshot({
    required this.runtimeId,
    required this.cursor,
    required this.probeCount,
    required this.foundationState,
    required this.cursorReset,
    required this.events,
  });
  int runtimeId;
  int cursor;
  int probeCount;
  int foundationState;
  bool cursorReset;
  List<DiagnosticEvent> events;
}

class IdentityInfo {
  IdentityInfo({required this.fingerprint, required this.storage});
  String fingerprint;
  String storage;
}

class GroupInfo {
  GroupInfo({required this.configured, required this.epoch});
  bool configured;
  int epoch;
}

class BluetoothInfo {
  BluetoothInfo({
    required this.available,
    required this.authorized,
    required this.enabled,
    required this.active,
    required this.peerCount,
    required this.probeCount,
    required this.authenticated,
    required this.messageCount,
    required this.lastMessage,
    required this.detail,
  });
  bool available;
  bool authorized;
  bool enabled;
  bool active;
  int peerCount;
  int probeCount;
  bool authenticated;
  int messageCount;
  String lastMessage;
  String detail;
}

class VoiceInfo {
  VoiceInfo({
    required this.receivedCount,
    required this.lastDurationMillis,
    required this.ready,
    required this.detail,
  });
  int receivedCount;
  int lastDurationMillis;
  bool ready;
  String detail;
}

/// Origin-local receipt evidence for one visible action. No object ID,
/// recipient identity, receipt or secret crosses into Flutter.
class DeliveryInfo {
  DeliveryInfo({
    required this.logicalId,
    required this.targetCount,
    required this.deliveredCount,
    required this.state,
  });
  String logicalId;
  int targetCount;
  int deliveredCount;
  String state;
}

/// Capability and discovery state for the direct Wi-Fi Aware radio.
/// It never exposes an IP address, SSID, router, or hotspot.
class AwareInfo {
  AwareInfo({
    required this.available,
    required this.enabled,
    required this.active,
    required this.peerCount,
    required this.maxPeers,
    required this.state,
    required this.detail,
  });
  bool available;
  bool enabled;
  bool active;
  int peerCount;
  int maxPeers;
  String state;
  String detail;
}

@HostApi()
abstract class MeshHostApi {
  @async
  EngineInfo engineInfo();
  @async
  IdentityInfo prepareIdentity();
  @async
  GroupInfo groupInfo();
  @async
  GroupInfo createGroup();
  @async
  Uint8List exportInvitation();
  @async
  Uint8List createEnrollmentRequest(Uint8List invitation);
  @async
  Uint8List issueEnrollment(Uint8List request);
  @async
  GroupInfo installPolicy(Uint8List policy);
  @async
  BluetoothInfo bluetoothInfo();
  @async
  BluetoothInfo prepareBluetooth();
  @async
  BluetoothInfo startBluetoothDiscovery();
  @async
  BluetoothInfo stopBluetoothDiscovery();
  @async
  AwareInfo awareInfo();
  @async
  AwareInfo startAwareDiscovery();
  @async
  AwareInfo stopAwareDiscovery();

  @async
  bool sendText(String message, String logicalId);
  @async
  DeliveryInfo deliveryInfo(String logicalId);
  @async
  VoiceInfo voiceInfo();
  @async
  bool sendVoice(Uint8List audio, int durationMillis, String logicalId);
  @async
  bool playLastVoice();
  @async
  EngineSnapshot subscribe(int cursor);
  @async
  EngineSnapshot verifyBridge(int requestId);
}
