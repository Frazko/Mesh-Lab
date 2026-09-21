import 'dart:async';
import 'dart:convert';
import 'dart:math';
import 'dart:typed_data';

import 'package:mesh_host/mesh_host.dart';

import 'field_mesh_models.dart';

FieldDeliveryState _deliveryState(String value) => switch (value) {
  'queued' => FieldDeliveryState.queued,
  'partial' => FieldDeliveryState.partial,
  'delivered' => FieldDeliveryState.delivered,
  'expired' => FieldDeliveryState.expired,
  _ => throw StateError('Unexpected native delivery state'),
};

/// Native completion payload for an encrypted voice note. It remains an
/// adapter type: the public client validates every field before exposing it.
final class FieldCertifiedVoicePayload {
  const FieldCertifiedVoicePayload({
    required this.authorId,
    required this.objectId,
    required this.logicalId,
    required this.verifiedAt,
    required this.duration,
    required this.context,
  });

  final String authorId;
  final String objectId;
  final String logicalId;
  final DateTime verifiedAt;
  final Duration duration;
  final String context;
}

/// Completion payload supplied by the native host. It is kept internal to the
/// adapter; an app receives [FieldVerifiedIncomingText] only after the SDK
/// validates its encrypted product envelope too.
final class FieldCertifiedPayload {
  const FieldCertifiedPayload({
    required this.authorId,
    required this.objectId,
    required this.verifiedAt,
    required this.body,
  });

  final String authorId;
  final String objectId;
  final DateTime verifiedAt;
  final String body;
}

/// Narrow native boundary that can be substituted in product tests without a
/// Flutter channel or a physical radio.
abstract interface class FieldMeshGateway {
  Future<FieldIdentity> prepareIdentity();
  Future<FieldGroup> groupInfo();
  Future<FieldGroup> createGroup();
  Future<FieldBluetoothStatus> bluetoothStatus();
  Future<FieldBluetoothStatus> prepareBluetooth();
  Future<FieldBluetoothStatus> startBluetoothDiscovery();
  Future<FieldBluetoothStatus> stopBluetoothDiscovery();
  Future<FieldAwareStatus> awareStatus();
  Future<FieldAwareStatus> startAwareDiscovery();
  Future<FieldAwareStatus> stopAwareDiscovery();
  Future<FieldVoiceStatus> voiceStatus();
  Future<bool> sendText(String message, String logicalId);
  Future<FieldDelivery?> delivery(String logicalId);
  Future<List<FieldCertifiedPayload>> drainVerifiedIncomingText();
  Future<List<FieldCertifiedVoicePayload>> drainVerifiedIncomingVoice();
  Future<bool> sendVoice(Uint8List audio, int durationMillis, String logicalId);
  Future<bool> sendVoiceWithContext(
    Uint8List audio,
    int durationMillis,
    String logicalId,
    String context,
  );
  Future<bool> playLastVoice();
  Future<bool> playVoice(String objectId);
}

/// Optional native gateway extension for a product-authorized enrollment
/// roster. Keeping it separate avoids widening the base test and transport
/// contract for applications that do not own membership server-side.
abstract interface class FieldMeshEnrollmentAccessGateway {
  Future<void> configureEnrollmentAccess(FieldEnrollmentAccessPolicy policy);
  Future<void> clearEnrollmentAccess();
  Future<bool> canIssueEnrollment();
}

/// Narrow native extension for a product-managed leadership transfer. All
/// payloads are public signed bytes and remain validated by the host.
abstract interface class FieldMeshAuthorityHandoffGateway {
  Future<Uint8List> prepareAuthorityHandoff(
    Uint8List successor,
    int validUntilSeconds,
  );
  Future<Uint8List> rotateAuthority(Uint8List handoff);
  Future<Uint8List> exportCurrentPolicy();
  Future<FieldGroup> installRotatedPolicy(Uint8List policy, Uint8List handoff);
}

abstract interface class FieldMeshCloudRelayGateway {
  Future<FieldCloudRelayProof> signCloudRelay(Uint8List canonical);
}

/// Default gateway for iOS and Android. It maps mutable Pigeon DTOs into
/// immutable product DTOs at the package boundary.
final class MeshHostGateway
    implements
        FieldMeshGateway,
        FieldMeshEnrollmentAccessGateway,
        FieldMeshAuthorityHandoffGateway,
        FieldMeshCloudRelayGateway {
  MeshHostGateway({MeshHostApi? api}) : _api = api ?? MeshHostApi();

  final MeshHostApi _api;

  FieldIdentity _identity(IdentityInfo value) =>
      FieldIdentity(fingerprint: value.fingerprint, storage: value.storage);
  FieldGroup _group(GroupInfo value) =>
      FieldGroup(configured: value.configured, epoch: value.epoch);
  FieldBluetoothStatus _bluetooth(BluetoothInfo value) => FieldBluetoothStatus(
    available: value.available,
    authorized: value.authorized,
    enabled: value.enabled,
    active: value.active,
    detectedPeers: value.peerCount,
    authenticated: value.authenticated,
    receivedMessages: value.messageCount,
    lastMessage: value.lastMessage,
    detail: value.detail,
  );
  FieldAwareStatus _aware(AwareInfo value) => FieldAwareStatus(
    available: value.available,
    enabled: value.enabled,
    active: value.active,
    detectedPeers: value.peerCount,
    maxDirectPeers: value.maxPeers,
    state: value.state,
    detail: value.detail,
  );
  FieldVoiceStatus _voice(VoiceInfo value) => FieldVoiceStatus(
    receivedCount: value.receivedCount,
    lastDuration: Duration(milliseconds: value.lastDurationMillis),
    ready: value.ready,
    detail: value.detail,
  );

  @override
  Future<FieldIdentity> prepareIdentity() async =>
      _identity(await _api.prepareIdentity());
  @override
  Future<FieldGroup> groupInfo() async => _group(await _api.groupInfo());
  @override
  Future<FieldGroup> createGroup() async => _group(await _api.createGroup());
  @override
  Future<void> configureEnrollmentAccess(
    FieldEnrollmentAccessPolicy policy,
  ) async {
    await _api.configureEnrollmentAccess(
      EnrollmentAccessPolicy(
        scopeId: policy.scopeId,
        authorizedMemberIds: policy.authorizedMemberIds.toList(growable: false),
        authorityEnabled: policy.authorityEnabled,
      ),
    );
  }

  @override
  Future<void> clearEnrollmentAccess() => _api.clearEnrollmentAccess();

  @override
  Future<bool> canIssueEnrollment() => _api.canIssueEnrollment();
  @override
  Future<Uint8List> prepareAuthorityHandoff(
    Uint8List successor,
    int validUntilSeconds,
  ) => _api.prepareAuthorityHandoff(successor, validUntilSeconds);
  @override
  Future<Uint8List> rotateAuthority(Uint8List handoff) =>
      _api.rotateAuthority(handoff);
  @override
  Future<Uint8List> exportCurrentPolicy() => _api.exportInvitation();
  @override
  Future<FieldGroup> installRotatedPolicy(
    Uint8List policy,
    Uint8List handoff,
  ) async => _group(await _api.installRotatedPolicy(policy, handoff));
  @override
  Future<FieldCloudRelayProof> signCloudRelay(Uint8List canonical) async {
    if (canonical.isEmpty || canonical.length > 64 * 1024) {
      throw ArgumentError.value(canonical, 'canonical', 'must be bounded');
    }
    final proof = await _api.signCloudRelay(canonical);
    if (proof.groupId.length != 32 ||
        proof.epoch <= 0 ||
        proof.signature.length != 64) {
      throw StateError('Native cloud-relay proof is invalid');
    }
    return FieldCloudRelayProof(
      groupId: Uint8List.fromList(proof.groupId),
      epoch: proof.epoch,
      signature: Uint8List.fromList(proof.signature),
    );
  }

  @override
  Future<FieldBluetoothStatus> bluetoothStatus() async =>
      _bluetooth(await _api.bluetoothInfo());
  @override
  Future<FieldBluetoothStatus> prepareBluetooth() async =>
      _bluetooth(await _api.prepareBluetooth());
  @override
  Future<FieldBluetoothStatus> startBluetoothDiscovery() async =>
      _bluetooth(await _api.startBluetoothDiscovery());
  @override
  Future<FieldBluetoothStatus> stopBluetoothDiscovery() async =>
      _bluetooth(await _api.stopBluetoothDiscovery());
  @override
  Future<FieldAwareStatus> awareStatus() async =>
      _aware(await _api.awareInfo());
  @override
  Future<FieldAwareStatus> startAwareDiscovery() async =>
      _aware(await _api.startAwareDiscovery());
  @override
  Future<FieldAwareStatus> stopAwareDiscovery() async =>
      _aware(await _api.stopAwareDiscovery());
  @override
  Future<List<FieldCertifiedPayload>> drainVerifiedIncomingText() async =>
      (await _api.drainVerifiedIncomingText())
          .map(
            (value) => FieldCertifiedPayload(
              authorId: value.authorId,
              objectId: value.objectId,
              verifiedAt: DateTime.fromMillisecondsSinceEpoch(
                value.verifiedAtUnixSeconds * 1000,
                isUtc: true,
              ),
              body: value.body,
            ),
          )
          .toList(growable: false);

  @override
  Future<List<FieldCertifiedVoicePayload>> drainVerifiedIncomingVoice() async =>
      (await _api.drainVerifiedIncomingVoice())
          .map(
            (value) => FieldCertifiedVoicePayload(
              authorId: value.authorId,
              objectId: value.objectId,
              logicalId: value.logicalId,
              verifiedAt: DateTime.fromMillisecondsSinceEpoch(
                value.verifiedAtUnixSeconds * 1000,
                isUtc: true,
              ),
              duration: Duration(milliseconds: value.durationMillis),
              context: value.context,
            ),
          )
          .toList(growable: false);

  @override
  Future<FieldVoiceStatus> voiceStatus() async =>
      _voice(await _api.voiceInfo());
  @override
  Future<bool> sendText(String message, String logicalId) =>
      _api.sendText(message, logicalId);
  @override
  Future<FieldDelivery?> delivery(String logicalId) async {
    final value = await _api.deliveryInfo(logicalId);
    if (value.logicalId.isEmpty) return null;
    return FieldDelivery(
      logicalId: value.logicalId,
      targets: value.targetCount,
      delivered: value.deliveredCount,
      state: _deliveryState(value.state),
    );
  }

  @override
  Future<bool> sendVoice(
    Uint8List audio,
    int durationMillis,
    String logicalId,
  ) => _api.sendVoice(audio, durationMillis, logicalId);
  @override
  Future<bool> sendVoiceWithContext(
    Uint8List audio,
    int durationMillis,
    String logicalId,
    String context,
  ) => _api.sendVoiceWithContext(audio, durationMillis, logicalId, context);
  @override
  Future<bool> playLastVoice() => _api.playLastVoice();
  @override
  Future<bool> playVoice(String objectId) => _api.playVoice(objectId);
}

/// Reusable client for Convoy and other product applications.
final class FieldMeshClient
    implements
        FieldMeshSdk,
        FieldMeshPresenceController,
        FieldMeshActionSender,
        FieldMeshVoiceContextSender,
        FieldMeshEnrollmentAccessController,
        FieldMeshAuthorityHandoffController,
        FieldMeshCloudRelaySigner,
        FieldMeshVerifiedIncomingSource,
        FieldMeshVerifiedIncomingVoiceSource {
  FieldMeshClient({FieldMeshGateway? gateway})
    : _gateway = gateway ?? MeshHostGateway();

  static const _actionPrefix = 'field-action-v1:';

  final FieldMeshGateway _gateway;
  final Random _random = Random.secure();

  @override
  Future<FieldIdentity> prepareIdentity() => _gateway.prepareIdentity();
  @override
  Future<FieldGroup> groupInfo() => _gateway.groupInfo();
  @override
  Future<FieldGroup> createGroup() => _gateway.createGroup();

  @override
  Future<void> configureEnrollmentAccess(
    FieldEnrollmentAccessPolicy policy,
  ) async {
    final gateway = _gateway is FieldMeshEnrollmentAccessGateway
        ? _gateway as FieldMeshEnrollmentAccessGateway
        : null;
    if (gateway == null) {
      throw UnsupportedError('El host no admite control de incorporaciones.');
    }
    if (!RegExp(r'^[0-9a-f]{32}$').hasMatch(policy.scopeId) ||
        policy.authorizedMemberIds.length > 50 ||
        policy.authorizedMemberIds.any(
          (member) => !RegExp(r'^[0-9a-f]{64}$').hasMatch(member),
        )) {
      throw ArgumentError.value(
        policy,
        'policy',
        'Lista de miembros no válida',
      );
    }
    await gateway.configureEnrollmentAccess(policy);
  }

  @override
  Future<void> clearEnrollmentAccess() async {
    final gateway = _gateway is FieldMeshEnrollmentAccessGateway
        ? _gateway as FieldMeshEnrollmentAccessGateway
        : null;
    if (gateway == null) return;
    await gateway.clearEnrollmentAccess();
  }

  @override
  Future<bool> canIssueEnrollment() async {
    final gateway = _gateway is FieldMeshEnrollmentAccessGateway
        ? _gateway as FieldMeshEnrollmentAccessGateway
        : null;
    if (gateway == null) {
      throw UnsupportedError('El host no admite comprobar la autoridad.');
    }
    return gateway.canIssueEnrollment();
  }

  @override
  Future<FieldCloudRelayProof> signCloudRelay(Uint8List canonical) async {
    final gateway = _gateway is FieldMeshCloudRelayGateway
        ? _gateway as FieldMeshCloudRelayGateway
        : null;
    if (gateway == null) {
      throw UnsupportedError('El host no admite firmas de relevo a nube.');
    }
    if (canonical.isEmpty || canonical.length > 64 * 1024) {
      throw ArgumentError.value(canonical, 'canonical', 'must be bounded');
    }
    final proof = await gateway.signCloudRelay(canonical);
    if (proof.groupId.length != 32 ||
        proof.epoch <= 0 ||
        proof.signature.length != 64) {
      throw StateError('La prueba de relevo nativa no es válida.');
    }
    return proof;
  }

  @override
  Future<Uint8List> prepareAuthorityHandoff(
    String successorMemberId,
    DateTime validUntil,
  ) async {
    final gateway = _gateway is FieldMeshAuthorityHandoffGateway
        ? _gateway as FieldMeshAuthorityHandoffGateway
        : null;
    if (gateway == null) {
      throw UnsupportedError('El host no admite transferir autoridad.');
    }
    if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(successorMemberId)) {
      throw ArgumentError.value(
        successorMemberId,
        'successorMemberId',
        'Miembro sucesor no válido',
      );
    }
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final validUntilSeconds = validUntil.millisecondsSinceEpoch ~/ 1000;
    if (validUntilSeconds <= now) {
      throw ArgumentError.value(
        validUntil,
        'validUntil',
        'El relevo ya venció',
      );
    }
    final bytes = await gateway.prepareAuthorityHandoff(
      Uint8List.fromList([
        for (var index = 0; index < successorMemberId.length; index += 2)
          int.parse(successorMemberId.substring(index, index + 2), radix: 16),
      ]),
      validUntilSeconds,
    );
    if (bytes.isEmpty || bytes.length > 512) {
      throw StateError('El host devolvió un relevo de autoridad inválido');
    }
    return Uint8List.fromList(bytes);
  }

  @override
  Future<Uint8List> rotateAuthority(Uint8List handoff) async {
    final gateway = _gateway is FieldMeshAuthorityHandoffGateway
        ? _gateway as FieldMeshAuthorityHandoffGateway
        : null;
    if (gateway == null) {
      throw UnsupportedError('El host no admite rotar autoridad.');
    }
    if (handoff.isEmpty || handoff.length > 512) {
      throw ArgumentError.value(
        handoff,
        'handoff',
        'Relevo de autoridad no válido',
      );
    }
    final policy = await gateway.rotateAuthority(Uint8List.fromList(handoff));
    if (policy.isEmpty || policy.length > 12 * 1024) {
      throw StateError('El host devolvió una política rotada inválida');
    }
    return Uint8List.fromList(policy);
  }

  @override
  Future<Uint8List> exportCurrentPolicy() async {
    final gateway = _gateway is FieldMeshAuthorityHandoffGateway
        ? _gateway as FieldMeshAuthorityHandoffGateway
        : null;
    if (gateway == null) {
      throw UnsupportedError('El host no admite recuperar la política rotada.');
    }
    final policy = await gateway.exportCurrentPolicy();
    if (policy.isEmpty || policy.length > 12 * 1024) {
      throw StateError('El host devolvió una política rotada inválida');
    }
    return Uint8List.fromList(policy);
  }

  @override
  Future<FieldGroup> installRotatedPolicy(
    Uint8List policy,
    Uint8List handoff,
  ) async {
    final gateway = _gateway is FieldMeshAuthorityHandoffGateway
        ? _gateway as FieldMeshAuthorityHandoffGateway
        : null;
    if (gateway == null) {
      throw UnsupportedError(
        'El host no admite instalar una autoridad rotada.',
      );
    }
    if (policy.isEmpty ||
        policy.length > 12 * 1024 ||
        handoff.isEmpty ||
        handoff.length > 512) {
      throw ArgumentError('Política o relevo de autoridad no válido');
    }
    return gateway.installRotatedPolicy(
      Uint8List.fromList(policy),
      Uint8List.fromList(handoff),
    );
  }

  @override
  Future<FieldSessionStatus> status() async {
    final values = await Future.wait([
      _gateway.bluetoothStatus(),
      _gateway.awareStatus(),
      _gateway.voiceStatus(),
    ]);
    return _session(
      values[0] as FieldBluetoothStatus,
      values[1] as FieldAwareStatus,
      values[2] as FieldVoiceStatus,
    );
  }

  @override
  Future<FieldSessionStatus> connect() async {
    await _gateway.prepareBluetooth();
    final bluetooth = await _gateway.startBluetoothDiscovery();
    final aware = await _gateway.startAwareDiscovery();
    final voice = await _gateway.voiceStatus();
    return _session(bluetooth, aware, voice);
  }

  /// Keeps the protected nearby group discoverable while a product continues
  /// using its cloud route. The host already owns battery-aware BLE/Wi-Fi
  /// Aware discovery and native multi-hop relay custody, so this is deliberately
  /// the same safe start path as [connect], not a second radio session.
  @override
  Future<FieldSessionStatus> maintainPresence() => connect();

  @override
  Future<FieldSessionStatus> leave() async {
    final bluetooth = await _gateway.stopBluetoothDiscovery();
    final aware = await _gateway.stopAwareDiscovery();
    final voice = await _gateway.voiceStatus();
    return _session(bluetooth, aware, voice);
  }

  @override
  Future<FieldDelivery?> sendText(String message) => _sendText(message);

  @override
  Future<FieldDelivery?> sendTextWithLogicalId(
    String message,
    String logicalId,
  ) => _sendText(message, logicalId: logicalId);

  Future<FieldDelivery?> _sendText(String message, {String? logicalId}) async {
    final body = message.trim();
    if (body.isEmpty || !(await status()).secure) return null;
    final actionId = _actionId(logicalId);
    if (actionId == null) return null;
    final encoded = _encodeAction(body, actionId);
    // The native radio admits at most 2 KiB. Measure the sealed product
    // envelope, not just visible characters, so authenticated metadata cannot
    // turn an apparently valid message into a silent transport failure.
    if (utf8.encode(encoded).length > 2048 ||
        !await _gateway.sendText(encoded, actionId)) {
      return null;
    }
    return await delivery(actionId) ??
        FieldDelivery(
          logicalId: actionId,
          targets: 0,
          delivered: 0,
          state: FieldDeliveryState.queued,
        );
  }

  @override
  Future<FieldDelivery?> sendLocation(FieldLocation location) =>
      _sendLocation(location);

  @override
  Future<FieldDelivery?> sendLocationWithLogicalId(
    FieldLocation location,
    String logicalId,
  ) => _sendLocation(location, logicalId: logicalId);

  Future<FieldDelivery?> _sendLocation(
    FieldLocation location, {
    String? logicalId,
  }) {
    if (location.latitude < -90 ||
        location.latitude > 90 ||
        location.longitude < -180 ||
        location.longitude > 180 ||
        location.accuracyMeters < 0 ||
        (location.headingDegrees != null &&
            (location.headingDegrees! < 0 ||
                location.headingDegrees! >= 360))) {
      return Future<FieldDelivery?>.value(null);
    }
    return _sendText(
      'field-location-v1:${base64UrlEncode(utf8.encode(jsonEncode({'lat': location.latitude, 'lng': location.longitude, 'accuracy': location.accuracyMeters, 'capturedAt': location.capturedAt.toUtc().millisecondsSinceEpoch, if (location.headingDegrees != null) 'heading': location.headingDegrees})))}',
      logicalId: logicalId,
    );
  }

  @override
  Future<FieldDelivery?> delivery(String logicalId) async {
    if (!RegExp(r'^[0-9a-f]{32}$').hasMatch(logicalId)) return null;
    final value = await _gateway.delivery(logicalId);
    if (value == null ||
        value.logicalId != logicalId ||
        value.targets < 0 ||
        value.targets > 49 ||
        value.delivered < 0 ||
        value.delivered > value.targets) {
      return null;
    }
    return value;
  }

  @override
  Future<FieldDelivery?> sendVoice(Uint8List audio, Duration duration) =>
      _sendVoice(audio, duration);

  @override
  Future<FieldDelivery?> sendVoiceWithLogicalId(
    Uint8List audio,
    Duration duration,
    String logicalId,
  ) => _sendVoice(audio, duration, logicalId: logicalId);

  @override
  Future<FieldDelivery?> sendVoiceWithLogicalIdAndContext(
    Uint8List audio,
    Duration duration,
    String logicalId,
    String context,
  ) => _sendVoice(audio, duration, logicalId: logicalId, context: context);

  Future<FieldDelivery?> _sendVoice(
    Uint8List audio,
    Duration duration, {
    String? logicalId,
    String? context,
  }) async {
    if (audio.isEmpty ||
        duration <= Duration.zero ||
        !(await status()).secure) {
      return null;
    }
    final actionId = _actionId(logicalId);
    final contextBytes = context == null ? null : utf8.encode(context);
    if (actionId == null ||
        (contextBytes != null &&
            (contextBytes.isEmpty || contextBytes.length > 512)) ||
        !(context == null
            ? await _gateway.sendVoice(audio, duration.inMilliseconds, actionId)
            : await _gateway.sendVoiceWithContext(
                audio,
                duration.inMilliseconds,
                actionId,
                context,
              ))) {
      return null;
    }
    return await delivery(actionId) ??
        FieldDelivery(
          logicalId: actionId,
          targets: 0,
          delivered: 0,
          state: FieldDeliveryState.queued,
        );
  }

  @override
  Future<bool> playLastVoice() => _gateway.playLastVoice();

  @override
  Future<bool> playVerifiedVoice(String objectId) {
    if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(objectId)) {
      return Future<bool>.value(false);
    }
    return _gateway.playVoice(objectId);
  }

  String? _actionId(String? supplied) {
    if (supplied == null) return _nextLogicalId();
    return RegExp(r'^[0-9a-f]{32}$').hasMatch(supplied) ? supplied : null;
  }

  @override
  Stream<FieldSessionStatus> watch({
    Duration interval = const Duration(seconds: 1),
  }) async* {
    if (interval <= Duration.zero) {
      throw ArgumentError.value(interval, 'interval', 'must be positive');
    }
    FieldSessionStatus? previous;
    while (true) {
      final next = await status();
      if (!_sameSession(previous, next)) {
        yield next;
        previous = next;
      }
      await Future<void>.delayed(interval);
    }
  }

  @override
  Stream<FieldIncomingEvent> watchIncoming({
    Duration interval = const Duration(seconds: 1),
  }) async* {
    if (interval <= Duration.zero) {
      throw ArgumentError.value(interval, 'interval', 'must be positive');
    }
    var previousCount = (await status()).bluetooth.receivedMessages;
    while (true) {
      await Future<void>.delayed(interval);
      final next = await status();
      final bluetooth = next.bluetooth;
      if (bluetooth.receivedMessages > previousCount &&
          bluetooth.lastMessage.isNotEmpty) {
        yield _incoming(bluetooth.lastMessage);
      }
      previousCount = bluetooth.receivedMessages;
    }
  }

  @override
  Stream<FieldVerifiedIncomingText> watchVerifiedIncomingText({
    Duration interval = const Duration(seconds: 1),
  }) async* {
    if (interval <= Duration.zero) {
      throw ArgumentError.value(interval, 'interval', 'must be positive');
    }
    while (true) {
      final events = await _gateway.drainVerifiedIncomingText();
      for (final event in events) {
        final certified = _decodeCertifiedAction(event);
        if (certified != null) yield certified;
      }
      await Future<void>.delayed(interval);
    }
  }

  @override
  Stream<FieldVerifiedIncomingVoice> watchVerifiedIncomingVoice({
    Duration interval = const Duration(seconds: 1),
  }) async* {
    if (interval <= Duration.zero) {
      throw ArgumentError.value(interval, 'interval', 'must be positive');
    }
    while (true) {
      final events = await _gateway.drainVerifiedIncomingVoice();
      for (final event in events) {
        final verified = _decodeCertifiedVoice(event);
        if (verified != null) yield verified;
      }
      await Future<void>.delayed(interval);
    }
  }

  FieldVerifiedIncomingVoice? _decodeCertifiedVoice(
    FieldCertifiedVoicePayload event,
  ) {
    if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(event.authorId) ||
        !RegExp(r'^[0-9a-f]{64}$').hasMatch(event.objectId) ||
        !RegExp(r'^[0-9a-f]{32}$').hasMatch(event.logicalId) ||
        event.verifiedAt.millisecondsSinceEpoch <= 0 ||
        event.duration <= Duration.zero ||
        event.duration > const Duration(seconds: 8) ||
        utf8.encode(event.context).length > 512) {
      return null;
    }
    return FieldVerifiedIncomingVoice(
      authorId: event.authorId,
      objectId: event.objectId,
      logicalId: event.logicalId,
      verifiedAt: event.verifiedAt,
      duration: event.duration,
      context: event.context,
    );
  }

  FieldVerifiedIncomingText? _decodeCertifiedAction(
    FieldCertifiedPayload event,
  ) {
    if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(event.authorId) ||
        !RegExp(r'^[0-9a-f]{64}$').hasMatch(event.objectId) ||
        event.verifiedAt.millisecondsSinceEpoch <= 0 ||
        !event.body.startsWith(_actionPrefix)) {
      return null;
    }
    try {
      final raw = utf8.decode(
        base64Url.decode(
          base64Url.normalize(event.body.substring(_actionPrefix.length)),
        ),
      );
      final value = jsonDecode(raw);
      if (value is! Map<String, dynamic> ||
          value.length != 3 ||
          value['type'] != 'text' ||
          value['id'] is! String ||
          value['body'] is! String ||
          !RegExp(r'^[0-9a-f]{32}$').hasMatch(value['id'] as String) ||
          (value['body'] as String).isEmpty) {
        return null;
      }
      return FieldVerifiedIncomingText(
        authorId: event.authorId,
        objectId: event.objectId,
        logicalId: value['id'] as String,
        verifiedAt: event.verifiedAt,
        body: value['body'] as String,
      );
    } on FormatException {
      return null;
    }
  }

  String _encodeAction(String body, String logicalId) =>
      '$_actionPrefix${base64UrlEncode(utf8.encode(jsonEncode({'id': logicalId, 'type': 'text', 'body': body})))}';

  FieldSessionStatus _session(
    FieldBluetoothStatus bluetooth,
    FieldAwareStatus aware,
    FieldVoiceStatus voice,
  ) {
    final secure = bluetooth.authenticated || aware.state == 'connected';
    final radioAvailable = bluetooth.available || aware.available;
    final state = secure
        ? FieldConnectionState.connected
        : !radioAvailable
        ? FieldConnectionState.unavailable
        : bluetooth.active || aware.active
        ? FieldConnectionState.connecting
        : FieldConnectionState.disconnected;
    return FieldSessionStatus(
      connection: state,
      bluetooth: bluetooth,
      aware: aware,
      voice: voice,
    );
  }

  bool _sameSession(FieldSessionStatus? a, FieldSessionStatus b) =>
      a != null &&
      a.connection == b.connection &&
      a.bluetooth.active == b.bluetooth.active &&
      a.bluetooth.detectedPeers == b.bluetooth.detectedPeers &&
      a.bluetooth.authenticated == b.bluetooth.authenticated &&
      a.aware.active == b.aware.active &&
      a.aware.detectedPeers == b.aware.detectedPeers &&
      a.aware.state == b.aware.state &&
      a.voice.receivedCount == b.voice.receivedCount &&
      a.bluetooth.receivedMessages == b.bluetooth.receivedMessages;

  FieldIncomingEvent _incoming(String payload) {
    if (payload.startsWith(_actionPrefix)) {
      try {
        final value = jsonDecode(
          utf8.decode(
            base64Url.decode(
              base64Url.normalize(payload.substring(_actionPrefix.length)),
            ),
          ),
        );
        if (value is Map<String, dynamic> &&
            value.length == 3 &&
            value['type'] == 'text' &&
            value['id'] is String &&
            value['body'] is String &&
            RegExp(r'^[0-9a-f]{32}$').hasMatch(value['id'] as String)) {
          return _incoming(value['body'] as String);
        }
      } on FormatException {
        // Keep malformed product payload opaque to the UI path.
      }
      return FieldIncomingText(body: payload);
    }
    const prefix = 'field-location-v1:';
    if (!payload.startsWith(prefix)) return FieldIncomingText(body: payload);
    try {
      final value = jsonDecode(
        utf8.decode(
          base64Url.decode(
            base64Url.normalize(payload.substring(prefix.length)),
          ),
        ),
      );
      if (value is! Map<String, dynamic>) throw const FormatException();
      final lat = value['lat'];
      final lng = value['lng'];
      final accuracy = value['accuracy'];
      final capturedAt = value['capturedAt'];
      final heading = value['heading'];
      if (lat is! num ||
          lng is! num ||
          accuracy is! num ||
          capturedAt is! int ||
          (heading != null && heading is! num) ||
          lat < -90 ||
          lat > 90 ||
          lng < -180 ||
          lng > 180 ||
          accuracy < 0 ||
          (heading is num && (heading < 0 || heading >= 360))) {
        throw const FormatException();
      }
      return FieldIncomingLocation(
        location: FieldLocation(
          latitude: lat.toDouble(),
          longitude: lng.toDouble(),
          accuracyMeters: accuracy.toDouble(),
          capturedAt: DateTime.fromMillisecondsSinceEpoch(
            capturedAt,
            isUtc: true,
          ),
          headingDegrees: (heading as num?)?.toDouble(),
        ),
      );
    } on FormatException {
      return FieldIncomingText(body: payload);
    }
  }

  String _nextLogicalId() => List<String>.generate(
    16,
    (_) => _random.nextInt(256).toRadixString(16).padLeft(2, '0'),
  ).join();
}
