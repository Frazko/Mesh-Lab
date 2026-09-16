import 'dart:async';
import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mesh_host/mesh_host.dart';
import 'package:mesh_lab/core/sdk/lab_controller.dart';

import 'fake_sdk.dart';

class ChatSdk extends FakeLabSdk {
  String? sent;
  @override
  Future<BluetoothInfo> bluetoothInfo() async => BluetoothInfo(
    available: true,
    authorized: true,
    enabled: true,
    active: true,
    peerCount: 1,
    probeCount: 0,
    authenticated: true,
    messageCount: 0,
    lastMessage: '',
    detail: 'Conectado.',
  );
  @override
  Future<bool> sendText(String message, String logicalId) async {
    sent = message;
    return true;
  }
}

class AwareChatSdk extends ChatSdk {
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
    detail: 'Bluetooth de respaldo listo.',
  );

  @override
  Future<AwareInfo> awareInfo() async => AwareInfo(
    available: true,
    enabled: true,
    active: true,
    peerCount: 1,
    maxPeers: 2,
    state: 'connected',
    detail: 'Wi-Fi Aware seguro activo con 1 teléfono del grupo.',
  );
}

class DeliveryChatSdk extends ChatSdk {
  final Map<String, DeliveryInfo> deliveries = {};

  @override
  Future<DeliveryInfo> deliveryInfo(String logicalId) async =>
      deliveries[logicalId] ??
      DeliveryInfo(
        logicalId: '',
        targetCount: 0,
        deliveredCount: 0,
        state: 'none',
      );
}

class GroupSdk extends FakeLabSdk {
  bool created = false;
  int createCalls = 0;

  @override
  Future<GroupInfo> groupInfo() async =>
      GroupInfo(configured: created, epoch: created ? 1 : 0);

  @override
  Future<GroupInfo> createGroup() async {
    createCalls++;
    created = true;
    return GroupInfo(configured: true, epoch: 1);
  }
}

class SessionSdk extends FakeLabSdk {
  int bluetoothStarts = 0;
  int bluetoothStops = 0;
  int awareStarts = 0;
  int awareStops = 0;

  @override
  Future<BluetoothInfo> startBluetoothDiscovery() async {
    bluetoothStarts++;
    return BluetoothInfo(
      available: true,
      authorized: true,
      enabled: true,
      active: true,
      peerCount: 0,
      probeCount: 0,
      authenticated: false,
      messageCount: 0,
      lastMessage: '',
      detail: 'Buscando grupo.',
    );
  }

  @override
  Future<BluetoothInfo> stopBluetoothDiscovery() async {
    bluetoothStops++;
    return bluetoothInfo();
  }

  @override
  Future<AwareInfo> startAwareDiscovery() async {
    awareStarts++;
    return AwareInfo(
      available: true,
      enabled: true,
      active: true,
      peerCount: 0,
      maxPeers: 2,
      state: 'discovering',
      detail: 'Buscando vecinos seguros.',
    );
  }

  @override
  Future<AwareInfo> stopAwareDiscovery() async {
    awareStops++;
    return awareInfo();
  }
}

class MediaSdk extends ChatSdk {
  int sentVoices = 0;
  bool playable = false;
  String? lastVoiceLogicalId;
  final Map<String, DeliveryInfo> voiceDeliveries = {};

  @override
  Future<bool> sendVoice(
    Uint8List audio,
    int durationMillis,
    String logicalId,
  ) async {
    sentVoices++;
    playable = true;
    lastVoiceLogicalId = logicalId;
    return true;
  }

  @override
  Future<DeliveryInfo> deliveryInfo(String logicalId) async =>
      voiceDeliveries[logicalId] ??
      DeliveryInfo(
        logicalId: '',
        targetCount: 0,
        deliveredCount: 0,
        state: 'none',
      );

  @override
  Future<bool> playLastVoice() async => playable;

  @override
  Future<VoiceInfo> voiceInfo() async => VoiceInfo(
    receivedCount: playable ? 1 : 0,
    lastDurationMillis: playable ? 500 : 0,
    ready: playable,
    detail: playable ? 'Nota lista.' : 'Sin notas.',
  );
}

class RejectingChatSdk extends ChatSdk {
  @override
  Future<bool> sendText(String message, String logicalId) async => false;
}

class IncomingPayloadSdk extends FakeLabSdk {
  int messageCount = 0;
  String lastMessage = '';

  @override
  Future<BluetoothInfo> bluetoothInfo() async => BluetoothInfo(
    available: true,
    authorized: true,
    enabled: true,
    active: true,
    peerCount: 1,
    probeCount: 0,
    authenticated: true,
    messageCount: messageCount,
    lastMessage: lastMessage,
    detail: 'Enlace seguro activo.',
  );
}

class InvalidSnapshotSdk extends FakeLabSdk {
  @override
  Future<EngineSnapshot> subscribe(int cursor) async => EngineSnapshot(
    runtimeId: 7,
    cursor: 1,
    probeCount: 0,
    foundationState: 0,
    cursorReset: false,
    events: const [],
  );
}

class DuplicateInboundChatSdk extends FakeLabSdk {
  int messageCount = 0;
  String lastMessage = '';

  @override
  Future<BluetoothInfo> bluetoothInfo() async => BluetoothInfo(
    available: true,
    authorized: true,
    enabled: true,
    active: true,
    peerCount: 1,
    probeCount: 0,
    authenticated: true,
    messageCount: messageCount,
    lastMessage: lastMessage,
    detail: 'Enlace seguro activo.',
  );
}

class LostReplySdk extends FakeLabSdk {
  bool lost = false;
  @override
  Future<EngineSnapshot> verifyBridge(int requestId) async {
    final result = await super.verifyBridge(requestId);
    if (!lost) {
      lost = true;
      throw TimeoutException('Reply lost after mutation');
    }
    return result;
  }
}

class ResetSdk extends FakeLabSdk {
  bool reset = false;
  @override
  Future<EngineSnapshot> subscribe(int cursor) async => snapshot(reset: reset);
}

class UnavailableSdk extends FakeLabSdk {
  @override
  Future<EngineInfo> engineInfo() async =>
      throw PlatformException(code: 'channel-error');
}

class DeferredSdk extends FakeLabSdk {
  final result = Completer<EngineSnapshot>();
  @override
  Future<EngineSnapshot> subscribe(int cursor) => result.future;
}

void main() {
  test('UI-09 late response cannot mark background state fresh', () async {
    final sdk = DeferredSdk();
    final c = LabController(sdk);
    addTearDown(c.dispose);
    final refresh = c.refresh();
    c.markStale();
    sdk.result.complete(sdk.snapshot());
    await refresh;
    expect(c.stateIsFresh, false);
    expect(c.snapshot, isNotNull);
  });

  test(
    'UI-01 timeout retries the same request without duplicate verification',
    () async {
      final sdk = LostReplySdk();
      final c = LabController(sdk);
      addTearDown(c.dispose);
      await c.refresh();
      await c.verify();
      expect(c.failure?.code, 'BRIDGE_TIMEOUT');
      expect(c.retryPending, true);
      expect(c.stateIsFresh, false);
      expect(sdk.cursor, 1);
      await c.verify();
      expect(sdk.requests[0], sdk.requests[1]);
      expect(sdk.cursor, 1);
      expect(c.snapshot!.probeCount, 1);
      expect(c.events.length, 1);
      expect(c.failure, isNull);
      expect(c.retryPending, false);
    },
  );
  test(
    'UI-02 cursor reset replaces projection and clears old history',
    () async {
      final sdk = ResetSdk();
      final c = LabController(sdk);
      addTearDown(c.dispose);
      await c.refresh();
      await c.verify();
      expect(c.events, hasLength(1));
      sdk.reset = true;
      sdk.cursor = 100;
      await c.refresh();
      expect(c.snapshot!.cursor, 100);
      expect(c.events, isEmpty);
      expect(c.notice, contains('Historial incompleto'));
    },
  );
  test(
    'UI-03 resubscribe preserves the native runtime and deduplicates events',
    () async {
      final sdk = FakeLabSdk();
      final c = LabController(sdk);
      addTearDown(c.dispose);
      await c.refresh();
      await c.verify();
      await c.refresh();
      expect(c.snapshot!.runtimeId, 7);
      expect(c.events.length, 1);
      final next = LabController(sdk);
      addTearDown(next.dispose);
      await next.refresh();
      expect(next.snapshot!.cursor, 1);
      expect(next.events.length, 1);
    },
  );
  test(
    'UI-04 missing native plugin fails visibly instead of fake success',
    () async {
      final c = LabController(UnavailableSdk());
      addTearDown(c.dispose);
      await c.refresh();
      expect(c.ready, false);
      expect(c.failure?.code, 'channel-error');
      expect(c.snapshot, isNull);
    },
  );
  test('UI-05 concurrent taps are serialized', () async {
    final sdk = FakeLabSdk();
    final c = LabController(sdk);
    addTearDown(c.dispose);
    await c.refresh();
    await Future.wait([c.verify(), c.verify()]);
    expect(sdk.requests, hasLength(1));
  });
  test(
    'Wi-Fi Aware authenticated link enables chat without Bluetooth',
    () async {
      final sdk = AwareChatSdk();
      final c = LabController(sdk);
      addTearDown(c.dispose);
      await c.refresh();

      expect(c.bluetooth?.authenticated, false);
      expect(c.aware?.state, 'connected');
      expect(c.secureConnected, true);
      expect(await c.sendText('solo aware'), true);
      expect(sdk.sent, contains('solo aware'));
    },
  );

  test(
    'group creation persists one valid group and rejects a duplicate tap',
    () async {
      final sdk = GroupSdk();
      final c = LabController(sdk);
      addTearDown(c.dispose);
      await c.refresh();
      expect(c.group?.configured, false);
      await c.createGroup();
      expect(c.group?.configured, true);
      expect(c.group?.epoch, 1);
      expect(sdk.createCalls, 1);
      await c.createGroup();
      expect(sdk.createCalls, 1);
      expect(c.failure?.code, 'INVALID_STATE');
    },
  );

  test('visible connect and leave operate both radios once', () async {
    final sdk = SessionSdk();
    final c = LabController(sdk);
    addTearDown(c.dispose);
    await c.refresh();
    await c.connectSession();
    expect(sdk.bluetoothStarts, 1);
    expect(sdk.awareStarts, 1);
    expect(c.bluetooth?.active, true);
    expect(c.aware?.active, true);
    await c.leaveSession();
    expect(sdk.bluetoothStops, 1);
    expect(sdk.awareStops, 1);
    expect(c.sessionActive, false);
  });

  test(
    'voice only queues a bounded recording on an authenticated link',
    () async {
      final sdk = MediaSdk();
      final c = LabController(sdk);
      addTearDown(c.dispose);
      await c.refresh();
      await c.sendVoice(Uint8List.fromList([1, 2, 3]), 500);
      expect(sdk.sentVoices, 1);
      expect(c.voice?.ready, true);
      expect(c.outgoingVoiceDeliveryLabel, 'En cola segura');
      final id = sdk.lastVoiceLogicalId!;
      sdk.voiceDeliveries[id] = DeliveryInfo(
        logicalId: id,
        targetCount: 2,
        deliveredCount: 1,
        state: 'partial',
      );
      await c.refresh();
      expect(c.outgoingVoiceDeliveryLabel, 'Entregado a 1 de 2');
      sdk.voiceDeliveries[id] = DeliveryInfo(
        logicalId: id,
        targetCount: 2,
        deliveredCount: 2,
        state: 'delivered',
      );
      await c.refresh();
      expect(c.outgoingVoiceDeliveryLabel, 'Entregado a 2 de 2');
      await c.playLastVoice();
      expect(c.failure, isNull);
      await c.sendVoice(Uint8List(0), 0);
      expect(sdk.sentVoices, 1);
      expect(c.failure?.code, 'INVALID_STATE');
    },
  );

  test('a rejected durable outbox does not create a chat bubble', () async {
    final c = LabController(RejectingChatSdk());
    addTearDown(c.dispose);
    await c.refresh();
    expect(await c.sendText('no debe aparecer'), false);
    expect(c.chatMessages, isEmpty);
    expect(c.failure?.code, 'INVALID_STATE');
  });

  test(
    'incoming encrypted location and malformed text are projected safely',
    () async {
      final sdk = IncomingPayloadSdk();
      final c = LabController(sdk);
      addTearDown(c.dispose);
      await c.refresh();
      final reading = LocationReading(
        latitude: 10.06794,
        longitude: -84.15198,
        accuracyMeters: 15,
        capturedAt: DateTime.fromMillisecondsSinceEpoch(1700000000000),
      );
      sdk.messageCount = 1;
      sdk.lastMessage = reading.encode();
      await c.refresh();
      expect(c.groupLocation?.latitude, reading.latitude);
      expect(c.groupLocation?.longitude, reading.longitude);

      sdk.messageCount = 2;
      sdk.lastMessage = '{payload roto';
      await c.refresh();
      expect(c.chatMessages.single.body, '{payload roto');
    },
  );

  test('location decoder rejects malformed and out-of-range payloads', () {
    expect(LocationReading.tryDecode('mesh-location-v1:malformed'), isNull);
    final invalid = base64UrlEncode(
      utf8.encode(jsonEncode({'v': 1, 'lat': 91, 'lon': 0, 'acc': 1, 'at': 1})),
    );
    expect(LocationReading.tryDecode('mesh-location-v1:$invalid'), isNull);
  });

  test('voice cannot queue or play before a secure link exists', () async {
    final c = LabController(FakeLabSdk());
    addTearDown(c.dispose);
    await c.refresh();
    await c.sendVoice(Uint8List.fromList([1]), 500);
    expect(c.failure?.code, 'INVALID_STATE');
    await c.playLastVoice();
    expect(c.failure?.code, 'INVALID_STATE');
  });

  test('an inconsistent native snapshot is never accepted as fresh', () async {
    final c = LabController(InvalidSnapshotSdk());
    addTearDown(c.dispose);
    await c.refresh();
    expect(c.ready, false);
    expect(c.snapshot, isNull);
    expect(c.failure?.code, 'INVALID_STATE');
  });

  test('chat uses a hybrid timestamp and stable logical message id', () async {
    final sdk = ChatSdk();
    final c = LabController(sdk);
    addTearDown(c.dispose);
    await c.refresh();
    await c.sendText('primero');
    await c.sendText('segundo');
    expect(c.chatMessages, hasLength(2));
    expect(
      c.chatMessages.first.timestamp.compareTo(c.chatMessages.last.timestamp),
      lessThan(0),
    );
    final wire = sdk.sent!;
    expect(wire, contains('"v":2'));
    expect(wire, contains('"h"'));
    expect(c.chatMessages.first.id, matches(RegExp(r'^[0-9a-f]{32}$')));
  });

  test('origin receipt updates only its matching chat bubble', () async {
    final sdk = DeliveryChatSdk();
    final c = LabController(sdk);
    addTearDown(c.dispose);
    await c.refresh();
    await c.sendText('primero');
    await c.sendText('segundo');
    final first = c.chatMessages.first.id;
    final second = c.chatMessages.last.id;
    sdk.deliveries[first] = DeliveryInfo(
      logicalId: first,
      targetCount: 2,
      deliveredCount: 1,
      state: 'partial',
    );
    await c.refresh();
    expect(
      c.chatMessages.singleWhere((entry) => entry.id == first).deliveryState,
      'partial',
    );
    expect(
      c.chatMessages.singleWhere((entry) => entry.id == second).deliveryState,
      'queued',
    );
    sdk.deliveries[first] = DeliveryInfo(
      logicalId: first,
      targetCount: 2,
      deliveredCount: 1,
      state: 'expired',
    );
    await c.refresh();
    expect(
      c.chatMessages.singleWhere((entry) => entry.id == first).deliveryState,
      'expired',
    );
  });

  test('same routed group message creates one conversation bubble', () async {
    final sdk = DuplicateInboundChatSdk();
    final c = LabController(sdk);
    addTearDown(c.dispose);
    await c.refresh();

    sdk.lastMessage = '{"v":2,"type":"chat","id":"peer-42","body":"por dos rutas","h":{"ms":1700000000000,"c":0,"o":"peer"}}';
    sdk.messageCount = 1;
    await c.refresh();
    expect(
      c.chatMessages.where((entry) => entry.id == 'peer-42'),
      hasLength(1),
    );

    // The same durable logical publication can arrive from a second audience
    // or a recovered neighbor. Radio counters may advance, but chat history
    // must keep the one signed logical ID as a single conversation entry.
    sdk.messageCount = 2;
    await c.refresh();
    expect(
      c.chatMessages.where((entry) => entry.id == 'peer-42'),
      hasLength(1),
    );
    expect(
      c.chatMessages.singleWhere((entry) => entry.id == 'peer-42').body,
      'por dos rutas',
    );
  });
}
