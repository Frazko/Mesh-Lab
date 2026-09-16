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
  Future<bool> sendVoice(Uint8List audio, int durationMillis, String logicalId);
  Future<bool> playLastVoice();
}

/// Default gateway for iOS and Android. It maps mutable Pigeon DTOs into
/// immutable product DTOs at the package boundary.
final class MeshHostGateway implements FieldMeshGateway {
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
  Future<bool> playLastVoice() => _api.playLastVoice();
}

/// Reusable client for Convoy and other product applications.
final class FieldMeshClient implements FieldMeshSdk, FieldMeshActionSender {
  FieldMeshClient({FieldMeshGateway? gateway})
    : _gateway = gateway ?? MeshHostGateway();

  final FieldMeshGateway _gateway;
  final Random _random = Random.secure();

  @override
  Future<FieldIdentity> prepareIdentity() => _gateway.prepareIdentity();
  @override
  Future<FieldGroup> groupInfo() => _gateway.groupInfo();
  @override
  Future<FieldGroup> createGroup() => _gateway.createGroup();

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
    if (body.isEmpty || body.length > 500 || !(await status()).secure) {
      return null;
    }
    final actionId = _actionId(logicalId);
    if (actionId == null || !await _gateway.sendText(body, actionId)) {
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

  Future<FieldDelivery?> _sendVoice(
    Uint8List audio,
    Duration duration, {
    String? logicalId,
  }) async {
    if (audio.isEmpty ||
        duration <= Duration.zero ||
        !(await status()).secure) {
      return null;
    }
    final actionId = _actionId(logicalId);
    if (actionId == null ||
        !await _gateway.sendVoice(audio, duration.inMilliseconds, actionId)) {
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
