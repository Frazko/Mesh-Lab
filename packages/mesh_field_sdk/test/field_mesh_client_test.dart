import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mesh_host/mesh_host.dart';
import 'package:mesh_field_sdk/mesh_field_sdk.dart';

class FakeGateway
    implements FieldMeshGateway, FieldMeshEnrollmentAccessGateway {
  bool secure = false;
  bool awareSecure = false;
  bool active = false;
  bool available = true;
  bool acceptText = true;
  bool acceptVoice = true;
  bool noDeliveryEvidence = false;
  bool playableVoice = false;
  int receivedMessages = 0;
  String lastText = '';
  String lastLogicalId = '';
  String lastVoiceLogicalId = '';
  FieldDelivery? nextDelivery;
  List<FieldCertifiedPayload> verifiedIncoming = const [];
  List<FieldCertifiedVoicePayload> verifiedIncomingVoice = const [];
  FieldEnrollmentAccessPolicy? enrollmentAccess;

  FieldBluetoothStatus get bluetooth => FieldBluetoothStatus(
    available: available,
    authorized: true,
    enabled: true,
    active: active,
    detectedPeers: secure ? 1 : 0,
    authenticated: secure,
    receivedMessages: receivedMessages,
    lastMessage: lastText,
    detail: 'test',
  );
  FieldAwareStatus get aware => FieldAwareStatus(
    available: available,
    enabled: true,
    active: active,
    detectedPeers: awareSecure ? 1 : 0,
    maxDirectPeers: 2,
    state: awareSecure ? 'connected' : 'discovering',
    detail: 'test',
  );
  FieldVoiceStatus get voice => const FieldVoiceStatus(
    receivedCount: 0,
    lastDuration: Duration.zero,
    ready: false,
    detail: 'test',
  );

  @override
  Future<FieldAwareStatus> awareStatus() async => aware;
  @override
  Future<FieldBluetoothStatus> bluetoothStatus() async => bluetooth;
  @override
  Future<FieldGroup> createGroup() async =>
      const FieldGroup(configured: true, epoch: 1);
  @override
  Future<FieldGroup> groupInfo() async =>
      const FieldGroup(configured: true, epoch: 1);
  @override
  Future<void> configureEnrollmentAccess(
    FieldEnrollmentAccessPolicy policy,
  ) async {
    enrollmentAccess = policy;
  }

  @override
  Future<void> clearEnrollmentAccess() async {
    enrollmentAccess = null;
  }

  @override
  Future<FieldDelivery?> delivery(String logicalId) async => noDeliveryEvidence
      ? null
      : nextDelivery ??
            FieldDelivery(
              logicalId: logicalId,
              targets: 1,
              delivered: 0,
              state: FieldDeliveryState.queued,
            );
  @override
  Future<bool> playLastVoice() async => playableVoice;
  @override
  Future<List<FieldCertifiedPayload>> drainVerifiedIncomingText() async {
    final next = verifiedIncoming;
    verifiedIncoming = const [];
    return next;
  }

  @override
  Future<List<FieldCertifiedVoicePayload>> drainVerifiedIncomingVoice() async {
    final next = verifiedIncomingVoice;
    verifiedIncomingVoice = const [];
    return next;
  }

  @override
  Future<bool> playVoice(String objectId) async =>
      RegExp(r'^[0-9a-f]{64}$').hasMatch(objectId) && playableVoice;

  @override
  Future<FieldIdentity> prepareIdentity() async =>
      const FieldIdentity(fingerprint: 'a', storage: 'test');
  @override
  Future<FieldAwareStatus> startAwareDiscovery() async {
    active = true;
    return aware;
  }

  @override
  Future<FieldBluetoothStatus> startBluetoothDiscovery() async {
    active = true;
    return bluetooth;
  }

  @override
  Future<FieldAwareStatus> stopAwareDiscovery() async {
    active = false;
    return aware;
  }

  @override
  Future<FieldBluetoothStatus> stopBluetoothDiscovery() async {
    active = false;
    return bluetooth;
  }

  @override
  Future<FieldBluetoothStatus> prepareBluetooth() async => bluetooth;
  @override
  Future<bool> sendText(String message, String logicalId) async {
    lastText = message;
    lastLogicalId = logicalId;
    return acceptText;
  }

  @override
  Future<bool> sendVoice(
    Uint8List audio,
    int durationMillis,
    String logicalId,
  ) async {
    lastVoiceLogicalId = logicalId;
    return acceptVoice;
  }

  @override
  Future<bool> sendVoiceWithContext(
    Uint8List audio,
    int durationMillis,
    String logicalId,
    String context,
  ) async {
    lastVoiceLogicalId = logicalId;
    return acceptVoice;
  }

  @override
  Future<FieldVoiceStatus> voiceStatus() async => voice;
}

class FakeHostApi extends MeshHostApi {
  FakeHostApi(this.value);

  DeliveryInfo value;
  List<VerifiedIncomingText> verifiedIncoming = const [];
  List<VerifiedIncomingVoice> verifiedIncomingVoice = const [];

  final bluetooth = BluetoothInfo(
    available: true,
    authorized: true,
    enabled: true,
    active: true,
    peerCount: 2,
    probeCount: 3,
    authenticated: true,
    messageCount: 4,
    lastMessage: 'campo',
    detail: 'Bluetooth seguro.',
  );
  final aware = AwareInfo(
    available: true,
    enabled: true,
    active: true,
    peerCount: 5,
    maxPeers: 8,
    state: 'connected',
    detail: 'WFA seguro.',
  );
  final voice = VoiceInfo(
    receivedCount: 6,
    lastDurationMillis: 7000,
    ready: true,
    detail: 'Voz lista.',
  );

  @override
  Future<IdentityInfo> prepareIdentity() async =>
      IdentityInfo(fingerprint: 'fingerprint', storage: 'keychain');

  @override
  Future<GroupInfo> groupInfo() async => GroupInfo(configured: true, epoch: 2);

  @override
  Future<GroupInfo> createGroup() async =>
      GroupInfo(configured: true, epoch: 3);

  @override
  Future<BluetoothInfo> bluetoothInfo() async => bluetooth;

  @override
  Future<BluetoothInfo> prepareBluetooth() async => bluetooth;

  @override
  Future<BluetoothInfo> startBluetoothDiscovery() async => bluetooth;

  @override
  Future<BluetoothInfo> stopBluetoothDiscovery() async => bluetooth;

  @override
  Future<AwareInfo> awareInfo() async => aware;

  @override
  Future<AwareInfo> startAwareDiscovery() async => aware;

  @override
  Future<AwareInfo> stopAwareDiscovery() async => aware;

  @override
  Future<List<VerifiedIncomingText>> drainVerifiedIncomingText() async {
    final next = verifiedIncoming;
    verifiedIncoming = const [];
    return next;
  }

  @override
  Future<List<VerifiedIncomingVoice>> drainVerifiedIncomingVoice() async {
    final next = verifiedIncomingVoice;
    verifiedIncomingVoice = const [];
    return next;
  }

  @override
  Future<VoiceInfo> voiceInfo() async => voice;

  @override
  Future<bool> sendText(String message, String logicalId) async => true;

  @override
  Future<DeliveryInfo> deliveryInfo(String logicalId) async => value;

  @override
  Future<bool> sendVoice(
    Uint8List audio,
    int durationMillis,
    String logicalId,
  ) async => true;

  @override
  Future<bool> sendVoiceWithContext(
    Uint8List audio,
    int durationMillis,
    String logicalId,
    String context,
  ) async => true;

  @override
  Future<bool> playLastVoice() async => true;

  @override
  Future<bool> playVoice(String objectId) async => true;
}

final class _LegacyGateway implements FieldMeshGateway {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  test(
    'identity and group operations remain available before radio activation',
    () async {
      final sdk = FieldMeshClient(gateway: FakeGateway());

      expect((await sdk.prepareIdentity()).storage, 'test');
      expect((await sdk.groupInfo()).configured, isTrue);
      expect((await sdk.createGroup()).epoch, 1);
    },
  );

  test(
    'automatic enrollment access accepts only a bounded public roster',
    () async {
      final gateway = FakeGateway();
      final sdk = FieldMeshClient(gateway: gateway);
      const member =
          '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef';

      await sdk.configureEnrollmentAccess(
        FieldEnrollmentAccessPolicy(
          scopeId: '0123456789abcdef0123456789abcdef',
          authorizedMemberIds: [member],
          authorityEnabled: true,
        ),
      );
      expect(gateway.enrollmentAccess?.authorizedMemberIds, {member});
      expect(
        gateway.enrollmentAccess?.scopeId,
        '0123456789abcdef0123456789abcdef',
      );
      await sdk.clearEnrollmentAccess();
      expect(gateway.enrollmentAccess, isNull);
      await expectLater(
        sdk.configureEnrollmentAccess(
          FieldEnrollmentAccessPolicy(
            scopeId: '0123456789abcdef0123456789abcdef',
            authorizedMemberIds: ['not-a-member'],
            authorityEnabled: false,
          ),
        ),
        throwsArgumentError,
      );
    },
  );

  test('rejects uppercase and more than fifty enrollment identities', () async {
    final sdk = FieldMeshClient(gateway: FakeGateway());
    final overCapacity = List.generate(
      51,
      (index) => index.toRadixString(16).padLeft(64, '0'),
    );

    await expectLater(
      sdk.configureEnrollmentAccess(
        FieldEnrollmentAccessPolicy(
          scopeId: '0123456789abcdef0123456789abcdef',
          authorizedMemberIds: const [
            'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
          ],
          authorityEnabled: true,
        ),
      ),
      throwsArgumentError,
    );
    await expectLater(
      sdk.configureEnrollmentAccess(
        FieldEnrollmentAccessPolicy(
          scopeId: 'not-a-product-scope',
          authorizedMemberIds: const [
            '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
          ],
          authorityEnabled: true,
        ),
      ),
      throwsArgumentError,
    );
    await expectLater(
      sdk.configureEnrollmentAccess(
        FieldEnrollmentAccessPolicy(
          scopeId: '0123456789abcdef0123456789abcdef',
          authorizedMemberIds: overCapacity,
          authorityEnabled: true,
        ),
      ),
      throwsArgumentError,
    );
  });

  test('does not pretend a legacy host can enforce product enrollment', () async {
    final sdk = FieldMeshClient(gateway: _LegacyGateway());

    await expectLater(
      sdk.configureEnrollmentAccess(
        FieldEnrollmentAccessPolicy(
          scopeId: '0123456789abcdef0123456789abcdef',
          authorizedMemberIds: const [
            '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
          ],
          authorityEnabled: false,
        ),
      ),
      throwsUnsupportedError,
    );

    // Older consumers can still close their own session during a staged host
    // upgrade; clearing an absent product gate never opens one implicitly.
    await sdk.clearEnrollmentAccess();
  });

  test(
    'products see only a connected secure session after authenticated radio',
    () async {
      final gateway = FakeGateway()..secure = true;
      final sdk = FieldMeshClient(gateway: gateway);

      expect((await sdk.status()).connection, FieldConnectionState.connected);
      final delivery = await sdk.sendText('  convoy listo  ');
      expect(delivery?.state, FieldDeliveryState.queued);
      expect(delivery?.logicalId, matches(RegExp(r'^[0-9a-f]{32}$')));
      expect(gateway.lastText, startsWith('field-action-v1:'));
      expect(gateway.lastLogicalId, delivery?.logicalId);
    },
  );

  test(
    'preserves a product logical ID for text, location, and voice',
    () async {
      const actionId = '0123456789abcdef0123456789abcdef';
      final gateway = FakeGateway()..secure = true;
      final sdk = FieldMeshClient(gateway: gateway);
      final location = FieldLocation(
        latitude: 10.06794,
        longitude: -84.15198,
        accuracyMeters: 15,
        capturedAt: DateTime.utc(2026, 9, 15, 12),
        headingDegrees: 90,
      );

      expect(
        (await sdk.sendTextWithLogicalId('mismo ID', actionId))?.logicalId,
        actionId,
      );
      expect(gateway.lastLogicalId, actionId);
      expect(
        (await sdk.sendLocationWithLogicalId(location, actionId))?.logicalId,
        actionId,
      );
      expect(gateway.lastLogicalId, actionId);
      expect(
        (await sdk.sendVoiceWithLogicalId(
          Uint8List.fromList([1]),
          const Duration(seconds: 1),
          actionId,
        ))?.logicalId,
        actionId,
      );
      expect(gateway.lastVoiceLogicalId, actionId);
      expect(await sdk.sendTextWithLogicalId('invalid', 'uuid-v4'), isNull);
    },
  );

  test('unsecured discovery cannot send product content', () async {
    final sdk = FieldMeshClient(gateway: FakeGateway());

    expect((await sdk.connect()).connection, FieldConnectionState.connecting);
    expect(await sdk.sendText('sin enlace'), isNull);
  });

  test(
    'delivery keeps a durable result scoped to its generated action',
    () async {
      final gateway = FakeGateway()..secure = true;
      final sdk = FieldMeshClient(gateway: gateway);
      final queued = await sdk.sendText('estado de convoy');
      gateway.nextDelivery = FieldDelivery(
        logicalId: queued!.logicalId,
        targets: 4,
        delivered: 4,
        state: FieldDeliveryState.delivered,
      );

      final delivered = await sdk.delivery(queued.logicalId);

      expect(delivered?.complete, isTrue);
      expect(delivered?.delivered, 4);
      expect(delivered?.targets, 4);
    },
  );

  test(
    'malformed or foreign delivery evidence never reaches a product',
    () async {
      final gateway = FakeGateway()
        ..secure = true
        ..nextDelivery = const FieldDelivery(
          logicalId: 'ffffffffffffffffffffffffffffffff',
          targets: 1,
          delivered: 2,
          state: FieldDeliveryState.delivered,
        );
      final sdk = FieldMeshClient(gateway: gateway);

      expect(await sdk.delivery('invalid'), isNull);
      expect(await sdk.delivery('00000000000000000000000000000000'), isNull);
    },
  );

  test(
    'accepted text remains queued when host evidence arrives later',
    () async {
      final gateway = FakeGateway()
        ..secure = true
        ..noDeliveryEvidence = true;
      final delivery = await FieldMeshClient(gateway: gateway)
          .sendText('offline');

      expect(delivery?.state, FieldDeliveryState.queued);
      expect(delivery?.targets, 0);
      expect(delivery?.retrying, isTrue);
    },
  );

  test(
    'product actions reject invalid input and native queue failure',
    () async {
      final gateway = FakeGateway()..secure = true;
      final sdk = FieldMeshClient(gateway: gateway);

      expect(await sdk.sendText('   '), isNull);
      expect(await sdk.sendText('x' * 501), isNull);
      gateway.acceptText = false;
      expect(await sdk.sendText('queue failure'), isNull);
      expect(
        await sdk.sendVoice(Uint8List(0), const Duration(seconds: 1)),
        isNull,
      );
      expect(
        await sdk.sendVoice(Uint8List.fromList([1]), Duration.zero),
        isNull,
      );
      gateway.acceptVoice = false;
      expect(
        await sdk.sendVoice(
          Uint8List.fromList([1]),
          const Duration(seconds: 1),
        ),
        isNull,
      );
    },
  );

  test(
    'session state distinguishes disconnected, unavailable and aware links',
    () async {
      final gateway = FakeGateway();
      final sdk = FieldMeshClient(gateway: gateway);

      expect(
        (await sdk.status()).connection,
        FieldConnectionState.disconnected,
      );
      gateway.available = false;
      expect((await sdk.status()).connection, FieldConnectionState.unavailable);
      gateway.available = true;
      gateway.active = true;
      expect((await sdk.status()).connection, FieldConnectionState.connecting);
      gateway.secure = true;
      expect((await sdk.status()).connection, FieldConnectionState.connected);
      expect((await sdk.leave()).connection, FieldConnectionState.connected);
    },
  );

  test(
    'watch rejects a non-positive interval before polling forever',
    () async {
      final sdk = FieldMeshClient(gateway: FakeGateway());

      await expectLater(
        sdk.watch(interval: Duration.zero).first,
        throwsArgumentError,
      );
    },
  );

  test(
    'host gateway maps all durable delivery states without Pigeon leakage',
    () async {
      const id = '0123456789abcdef0123456789abcdef';
      for (final state in ['queued', 'partial', 'delivered', 'expired']) {
        final gateway = MeshHostGateway(
          api: FakeHostApi(
            DeliveryInfo(
              logicalId: id,
              targetCount: 2,
              deliveredCount: state == 'delivered' ? 2 : 0,
              state: state,
            ),
          ),
        );
        final delivery = await gateway.delivery(id);
        expect(delivery?.logicalId, id);
        expect(delivery?.state.name, state);
      }

      final unsupported = MeshHostGateway(
        api: FakeHostApi(
          DeliveryInfo(
            logicalId: id,
            targetCount: 0,
            deliveredCount: 0,
            state: 'unexpected',
          ),
        ),
      );
      await expectLater(unsupported.delivery(id), throwsStateError);
    },
  );

  test(
    'host gateway preserves native certified evidence without Pigeon DTOs',
    () async {
      final host =
          FakeHostApi(
              DeliveryInfo(
                logicalId: '',
                targetCount: 0,
                deliveredCount: 0,
                state: 'queued',
              ),
            )
            ..verifiedIncoming = [
              VerifiedIncomingText(
                authorId: 'a' * 64,
                objectId: 'b' * 64,
                verifiedAtUnixSeconds: 1700000000,
                body: 'field-action-v1:payload',
              ),
            ];
      final gateway = MeshHostGateway(api: host);

      final received = await gateway.drainVerifiedIncomingText();

      expect(received, hasLength(1));
      expect(received.single.authorId, 'a' * 64);
      expect(received.single.objectId, 'b' * 64);
      expect(
        received.single.verifiedAt,
        DateTime.fromMillisecondsSinceEpoch(1700000000000, isUtc: true),
      );
      expect(received.single.body, 'field-action-v1:payload');
    },
  );

  test(
    'host gateway maps certified voice metadata and object playback',
    () async {
      final host =
          FakeHostApi(
              DeliveryInfo(
                logicalId: '',
                targetCount: 0,
                deliveredCount: 0,
                state: 'queued',
              ),
            )
            ..verifiedIncomingVoice = [
              VerifiedIncomingVoice(
                authorId: 'a' * 64,
                objectId: 'b' * 64,
                logicalId: 'c' * 32,
                verifiedAtUnixSeconds: 1700000000,
                durationMillis: 2500,
                context: 'convoy-mesh-action-v1:trusted',
              ),
            ];
      final gateway = MeshHostGateway(api: host);

      final received = await gateway.drainVerifiedIncomingVoice();

      expect(received, hasLength(1));
      expect(received.single.authorId, 'a' * 64);
      expect(received.single.objectId, 'b' * 64);
      expect(received.single.logicalId, 'c' * 32);
      expect(received.single.duration, const Duration(milliseconds: 2500));
      expect(await gateway.playVoice('b' * 64), isTrue);
    },
  );

  test('host gateway maps every public session capability', () async {
    const id = '0123456789abcdef0123456789abcdef';
    final gateway = MeshHostGateway(
      api: FakeHostApi(
        DeliveryInfo(
          logicalId: id,
          targetCount: 2,
          deliveredCount: 1,
          state: 'partial',
        ),
      ),
    );

    expect((await gateway.prepareIdentity()).fingerprint, 'fingerprint');
    expect((await gateway.groupInfo()).epoch, 2);
    expect((await gateway.createGroup()).epoch, 3);
    expect((await gateway.bluetoothStatus()).receivedMessages, 4);
    expect((await gateway.prepareBluetooth()).detectedPeers, 2);
    expect((await gateway.startBluetoothDiscovery()).authenticated, isTrue);
    expect(
      (await gateway.stopBluetoothDiscovery()).detail,
      'Bluetooth seguro.',
    );
    expect((await gateway.awareStatus()).maxDirectPeers, 8);
    expect((await gateway.startAwareDiscovery()).state, 'connected');
    expect((await gateway.stopAwareDiscovery()).detectedPeers, 5);
    expect(
      (await gateway.voiceStatus()).lastDuration,
      const Duration(seconds: 7),
    );
    expect(await gateway.sendText('texto', id), isTrue);
    expect((await gateway.delivery(id))?.state, FieldDeliveryState.partial);
    expect(
      await gateway.sendVoice(
        Uint8List.fromList([1]),
        1000,
        '0123456789abcdef0123456789abcdef',
      ),
      isTrue,
    );
    expect(await gateway.playLastVoice(), isTrue);
  });

  test(
    'watch projects a session update and delegates voice playback',
    () async {
      final gateway = FakeGateway()..playableVoice = true;
      final sdk = FieldMeshClient(gateway: gateway);

      final first = await sdk
          .watch(interval: const Duration(milliseconds: 1))
          .first;

      expect(first.connection, FieldConnectionState.disconnected);
      expect(await sdk.playLastVoice(), isTrue);
    },
  );

  test(
    'location is validated, queued as an encrypted action and decoded',
    () async {
      final gateway = FakeGateway()..secure = true;
      final sdk = FieldMeshClient(gateway: gateway);
      final location = FieldLocation(
        latitude: 10.06794,
        longitude: -84.15198,
        accuracyMeters: 15,
        capturedAt: DateTime.utc(2026, 9, 14, 12),
        headingDegrees: 273.5,
      );

      final delivery = await sdk.sendLocation(location);

      expect(delivery, isNotNull);
      expect(gateway.lastText, startsWith('field-action-v1:'));
      expect(
        await sdk.sendLocation(
          FieldLocation(
            latitude: 91,
            longitude: 0,
            accuracyMeters: 1,
            capturedAt: DateTime.utc(2026),
          ),
        ),
        isNull,
      );
    },
  );

  test(
    'voice receives the same durable logical-delivery contract as text',
    () async {
      final gateway = FakeGateway()..secure = true;
      final sdk = FieldMeshClient(gateway: gateway);

      final delivery = await sdk.sendVoice(
        Uint8List.fromList([1, 2, 3]),
        const Duration(seconds: 1),
      );

      expect(delivery, isNotNull);
      expect(delivery?.state, FieldDeliveryState.queued);
      expect(delivery?.logicalId, matches(RegExp(r'^[0-9a-f]{32}$')));
      expect(gateway.lastVoiceLogicalId, delivery?.logicalId);

      gateway.nextDelivery = FieldDelivery(
        logicalId: delivery!.logicalId,
        targets: 2,
        delivered: 1,
        state: FieldDeliveryState.partial,
      );
      expect(
        (await sdk.delivery(delivery.logicalId))?.state,
        FieldDeliveryState.partial,
      );

      gateway.nextDelivery = FieldDelivery(
        logicalId: delivery.logicalId,
        targets: 2,
        delivered: 2,
        state: FieldDeliveryState.delivered,
      );
      expect((await sdk.delivery(delivery.logicalId))?.complete, isTrue);
    },
  );

  test(
    'a verified Wi-Fi Aware link alone accepts text, location and voice',
    () async {
      final gateway = FakeGateway()
        ..active = true
        ..awareSecure = true;
      final sdk = FieldMeshClient(gateway: gateway);
      final location = FieldLocation(
        latitude: 10.06794,
        longitude: -84.15198,
        accuracyMeters: 15,
        capturedAt: DateTime.utc(2026, 9, 15, 12),
        headingDegrees: 273.5,
      );

      final session = await sdk.status();

      expect(session.connection, FieldConnectionState.connected);
      expect(session.bluetooth.authenticated, isFalse);
      expect(session.aware.state, 'connected');
      expect(await sdk.sendText('mensaje por WFA'), isNotNull);
      expect(await sdk.sendLocation(location), isNotNull);
      expect(gateway.lastText, startsWith('field-action-v1:'));
      expect(
        await sdk.sendVoice(
          Uint8List.fromList([1, 2, 3]),
          const Duration(seconds: 1),
        ),
        isNotNull,
      );
    },
  );

  test('incoming watcher projects plain text and a typed location', () async {
    final gateway = FakeGateway();
    final sdk = FieldMeshClient(gateway: gateway);
    final events = sdk.watchIncoming(interval: const Duration(milliseconds: 1));
    final first = events.first;
    await Future<void>.delayed(const Duration(milliseconds: 2));
    gateway
      ..lastText = 'mensaje recibido'
      ..receivedMessages = 1;

    expect(await first, isA<FieldIncomingText>());
  });

  test(
    'incoming location retains vehicle heading for map projection',
    () async {
      final gateway = FakeGateway()..secure = true;
      final sdk = FieldMeshClient(gateway: gateway);
      final location = FieldLocation(
        latitude: 10.06794,
        longitude: -84.15198,
        accuracyMeters: 15,
        capturedAt: DateTime.utc(2026, 9, 14, 12),
        headingDegrees: 273.5,
      );
      await sdk.sendLocation(location);
      final incoming = sdk
          .watchIncoming(interval: const Duration(milliseconds: 1))
          .first;
      await Future<void>.delayed(const Duration(milliseconds: 2));
      gateway.receivedMessages = 1;

      final event = await incoming;

      expect(event, isA<FieldIncomingLocation>());
      final received = (event as FieldIncomingLocation).location;
      expect(received.latitude, location.latitude);
      expect(received.longitude, location.longitude);
      expect(received.headingDegrees, location.headingDegrees);
    },
  );

  test('outgoing product text seals its exact logical ID inside the encrypted payload', () async {
    final gateway = FakeGateway()..secure = true;
    final sdk = FieldMeshClient(gateway: gateway);
    const logicalId = 'a47d6e8525f34ef6bb0102092f27d05c';

    await sdk.sendTextWithLogicalId('  misma acción  ', logicalId);

    final encoded = gateway.lastText.substring('field-action-v1:'.length);
    final decoded = jsonDecode(
      utf8.decode(base64Url.decode(base64Url.normalize(encoded))),
    );
    expect(decoded, {'id': logicalId, 'type': 'text', 'body': 'misma acción'});
  });

  test(
    'verified incoming stream preserves certified native evidence',
    () async {
      final gateway = FakeGateway()
        ..verifiedIncoming = [
          FieldCertifiedPayload(
            authorId: 'a' * 64,
            objectId: 'b' * 64,
            verifiedAt: DateTime.fromMillisecondsSinceEpoch(
              1700000000000,
              isUtc: true,
            ),
            body: 'field-action-v1:eyJpZCI6ImNjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjY2NjIiwidHlwZSI6InRleHQiLCJib2R5IjoiYWNjaVx1MDBmM24gY2VydGlmaWNhZGEifQ',
          ),
        ];
      final sdk = FieldMeshClient(gateway: gateway);
      final event = await sdk
          .watchVerifiedIncomingText(interval: const Duration(milliseconds: 1))
          .first;
      expect(event.authorId, 'a' * 64);
      expect(event.objectId, 'b' * 64);
      expect(event.logicalId, 'c' * 32);
      expect(event.body, 'acción certificada');
    },
  );

  test('verified incoming stream drops malformed host evidence', () async {
    final gateway = FakeGateway()
      ..verifiedIncoming = [
        FieldCertifiedPayload(
          authorId: 'a' * 64,
          objectId: 'b' * 64,
          verifiedAt: DateTime.fromMillisecondsSinceEpoch(
            1700000000000,
            isUtc: true,
          ),
          body: 'field-action-v1:not-base64',
        ),
      ];
    final sdk = FieldMeshClient(gateway: gateway);
    await expectLater(
      sdk
          .watchVerifiedIncomingText(interval: const Duration(milliseconds: 1))
          .first
          .timeout(const Duration(milliseconds: 10)),
      throwsA(isA<TimeoutException>()),
    );
  });

  test(
    'emits only receipt-certified voice with a scoped playback handle',
    () async {
      final gateway = FakeGateway()
        ..verifiedIncomingVoice = [
          FieldCertifiedVoicePayload(
            authorId: 'a' * 64,
            objectId: 'b' * 64,
            logicalId: 'c' * 32,
            verifiedAt: DateTime.utc(2026, 9, 16, 12),
            duration: const Duration(seconds: 3),
            context: 'convoy-mesh-action-v1:trusted',
          ),
        ]
        ..playableVoice = true;
      final sdk = FieldMeshClient(gateway: gateway);

      final event = await sdk
          .watchVerifiedIncomingVoice(interval: const Duration(milliseconds: 1))
          .first;

      expect(event.authorId, 'a' * 64);
      expect(event.objectId, 'b' * 64);
      expect(event.logicalId, 'c' * 32);
      expect(event.duration, const Duration(seconds: 3));
      expect(await sdk.playVerifiedVoice(event.objectId), isTrue);
      expect(await sdk.playVerifiedVoice('not-a-certified-object'), isFalse);
    },
  );

  test(
    'preserves bounded encrypted voice context with the logical ID',
    () async {
      const id = '0123456789abcdef0123456789abcdef';
      final gateway = FakeGateway()..secure = true;
      final sdk = FieldMeshClient(gateway: gateway);

      final delivery = await sdk.sendVoiceWithLogicalIdAndContext(
        Uint8List.fromList([1, 2]),
        const Duration(seconds: 1),
        id,
        'convoy-mesh-action-v1:trusted',
      );

      expect(delivery?.logicalId, id);
      expect(gateway.lastVoiceLogicalId, id);
      expect(
        await sdk.sendVoiceWithLogicalIdAndContext(
          Uint8List.fromList([1]),
          const Duration(seconds: 1),
          id,
          'x' * 513,
        ),
        isNull,
      );
    },
  );

  test(
    'fails closed for malformed or out-of-policy certified voice metadata',
    () async {
      final gateway = FakeGateway()
        ..verifiedIncomingVoice = [
          FieldCertifiedVoicePayload(
            authorId: 'a' * 64,
            objectId: 'b' * 64,
            logicalId: 'broken',
            verifiedAt: DateTime.utc(2026, 9, 16, 12),
            duration: const Duration(seconds: 3),
            context: 'convoy-mesh-action-v1:trusted',
          ),
          FieldCertifiedVoicePayload(
            authorId: 'a' * 64,
            objectId: 'd' * 64,
            logicalId: 'c' * 32,
            verifiedAt: DateTime.utc(2026, 9, 16, 12),
            duration: const Duration(seconds: 9),
            context: '',
          ),
        ];
      final sdk = FieldMeshClient(gateway: gateway);
      final stream = sdk.watchVerifiedIncomingVoice(
        interval: const Duration(milliseconds: 1),
      );

      await expectLater(
        stream.first.timeout(const Duration(milliseconds: 30)),
        throwsA(isA<TimeoutException>()),
      );
    },
  );
}
