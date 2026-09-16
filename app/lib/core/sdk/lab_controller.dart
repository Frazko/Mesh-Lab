import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:geolocator/geolocator.dart';
import 'package:mesh_host/mesh_host.dart';
import 'package:shared_preferences/shared_preferences.dart';

const _locationPrefix = 'mesh-location-v1:';

abstract interface class LabSdk {
  Future<EngineInfo> engineInfo();
  Future<IdentityInfo> prepareIdentity();
  Future<GroupInfo> groupInfo();
  Future<GroupInfo> createGroup();
  Future<BluetoothInfo> bluetoothInfo();
  Future<BluetoothInfo> prepareBluetooth();
  Future<BluetoothInfo> startBluetoothDiscovery();
  Future<BluetoothInfo> stopBluetoothDiscovery();
  Future<AwareInfo> awareInfo();
  Future<AwareInfo> startAwareDiscovery();
  Future<AwareInfo> stopAwareDiscovery();
  Future<bool> sendText(String message, String logicalId);
  Future<DeliveryInfo> deliveryInfo(String logicalId);
  Future<VoiceInfo> voiceInfo();
  Future<bool> sendVoice(Uint8List audio, int durationMillis, String logicalId);
  Future<bool> playLastVoice();
  Future<EngineSnapshot> subscribe(int cursor);
  Future<EngineSnapshot> verifyBridge(int requestId);
}

class NativeLabSdk implements LabSdk {
  NativeLabSdk({MeshHostApi? api}) : _api = api ?? MeshHostApi();
  final MeshHostApi _api;
  @override
  Future<EngineInfo> engineInfo() => _api.engineInfo();
  @override
  Future<IdentityInfo> prepareIdentity() => _api.prepareIdentity();
  @override
  Future<GroupInfo> groupInfo() => _api.groupInfo();
  @override
  Future<GroupInfo> createGroup() => _api.createGroup();
  @override
  Future<BluetoothInfo> bluetoothInfo() => _api.bluetoothInfo();
  @override
  Future<BluetoothInfo> prepareBluetooth() => _api.prepareBluetooth();
  @override
  Future<BluetoothInfo> startBluetoothDiscovery() =>
      _api.startBluetoothDiscovery();
  @override
  Future<BluetoothInfo> stopBluetoothDiscovery() =>
      _api.stopBluetoothDiscovery();
  @override
  Future<AwareInfo> awareInfo() => _api.awareInfo();
  @override
  Future<AwareInfo> startAwareDiscovery() => _api.startAwareDiscovery();
  @override
  Future<AwareInfo> stopAwareDiscovery() => _api.stopAwareDiscovery();
  @override
  Future<bool> sendText(String message, String logicalId) =>
      _api.sendText(message, logicalId);
  @override
  Future<DeliveryInfo> deliveryInfo(String logicalId) =>
      _api.deliveryInfo(logicalId);
  @override
  Future<VoiceInfo> voiceInfo() => _api.voiceInfo();
  @override
  Future<bool> sendVoice(
    Uint8List audio,
    int durationMillis,
    String logicalId,
  ) => _api.sendVoice(audio, durationMillis, logicalId);
  @override
  Future<bool> playLastVoice() => _api.playLastVoice();
  @override
  Future<EngineSnapshot> subscribe(int cursor) => _api.subscribe(cursor);
  @override
  Future<EngineSnapshot> verifyBridge(int requestId) =>
      _api.verifyBridge(requestId);
}

class LabFailure {
  const LabFailure(this.code, this.message);
  final String code;
  final String message;
  static LabFailure from(Object error) {
    if (error is TimeoutException) {
      return const LabFailure(
        'BRIDGE_TIMEOUT',
        'El motor no respondió a tiempo. Reintenta la misma operación.',
      );
    }
    if (error is PlatformException) {
      return switch (error.code) {
        'channel-error' || 'ENGINE_NOT_PACKAGED' => LabFailure(
          error.code,
          'No se encuentra el puente nativo. Esta prueba requiere la app de iOS o Android con el motor incluido.',
        ),
        'MESH_2' || 'INVALID_ENVELOPE' => LabFailure(
          error.code,
          'La app y el motor tienen contratos incompatibles. Instala una compilación completa.',
        ),
        'KEY_STORAGE_UNAVAILABLE' => LabFailure(
          error.code,
          'No se pudo abrir la identidad protegida. Desbloquea el teléfono y reintenta. Si persiste, conserva la app y sus datos para revisar el fallo.',
        ),
        'MESH_4' => LabFailure(
          error.code,
          'El motor alcanzó su límite de recursos. Recupera el estado antes de continuar.',
        ),
        'MESH_6' => LabFailure(
          error.code,
          'La verificación quedó fuera del historial. Recupera el estado del motor.',
        ),
        _ => LabFailure(
          error.code,
          'No se pudo completar la operación. Recupera el estado y vuelve a intentar.',
        ),
      };
    }
    if (error is StateError) {
      final detail = error.message.toString();
      return LabFailure(
        'INVALID_STATE',
        detail.isEmpty ? 'No se pudo validar el estado del motor.' : detail,
      );
    }
    return const LabFailure(
      'INVALID_STATE',
      'No se pudo validar el estado del motor.',
    );
  }
}

/// A single, explicit location reading. It stays in memory and travels as a
/// typed payload inside the existing encrypted Bluetooth session.
class LocationReading {
  const LocationReading({
    required this.latitude,
    required this.longitude,
    required this.accuracyMeters,
    required this.capturedAt,
  });

  final double latitude;
  final double longitude;
  final double accuracyMeters;
  final DateTime capturedAt;

  String encode() =>
      _locationPrefix +
      base64UrlEncode(
        utf8.encode(
          jsonEncode({
            'v': 1,
            'lat': latitude,
            'lon': longitude,
            'acc': accuracyMeters,
            'at': capturedAt.millisecondsSinceEpoch,
          }),
        ),
      );

  static LocationReading? tryDecode(String payload) {
    if (!payload.startsWith(_locationPrefix)) return null;
    try {
      final decoded = jsonDecode(
        utf8.decode(
          base64Url.decode(
            base64Url.normalize(payload.substring(_locationPrefix.length)),
          ),
        ),
      );
      if (decoded is! Map || decoded['v'] != 1) return null;
      final latitude = decoded['lat'];
      final longitude = decoded['lon'];
      final accuracy = decoded['acc'];
      final capturedAt = decoded['at'];
      if (latitude is! num ||
          longitude is! num ||
          accuracy is! num ||
          capturedAt is! int ||
          !latitude.isFinite ||
          !longitude.isFinite ||
          !accuracy.isFinite ||
          latitude < -90 ||
          latitude > 90 ||
          longitude < -180 ||
          longitude > 180 ||
          accuracy < 0) {
        return null;
      }
      return LocationReading(
        latitude: latitude.toDouble(),
        longitude: longitude.toDouble(),
        accuracyMeters: accuracy.toDouble(),
        capturedAt: DateTime.fromMillisecondsSinceEpoch(capturedAt),
      );
    } catch (_) {
      return null;
    }
  }
}

class HybridTimestamp implements Comparable<HybridTimestamp> {
  const HybridTimestamp(this.millis, this.counter, this.origin);
  final int millis;
  final int counter;
  final String origin;

  Map<String, Object> toJson() => {'ms': millis, 'c': counter, 'o': origin};

  static HybridTimestamp? fromJson(Object value) {
    if (value is! Map ||
        value['ms'] is! int ||
        value['c'] is! int ||
        value['o'] is! String) {
      return null;
    }
    final millis = value['ms'] as int;
    final counter = value['c'] as int;
    final origin = value['o'] as String;
    if (millis < 0 || counter < 0 || origin.isEmpty || origin.length > 64) {
      return null;
    }
    return HybridTimestamp(millis, counter, origin);
  }

  @override
  int compareTo(HybridTimestamp other) {
    final byMillis = millis.compareTo(other.millis);
    if (byMillis != 0) return byMillis;
    final byCounter = counter.compareTo(other.counter);
    return byCounter != 0 ? byCounter : origin.compareTo(other.origin);
  }
}

class ChatMessage {
  const ChatMessage({
    required this.id,
    required this.body,
    required this.timestamp,
    required this.mine,
    this.deliveryState,
    this.targetCount,
    this.deliveredCount,
  });
  final String id;
  final String body;
  final HybridTimestamp timestamp;
  final bool mine;

  /// Evidence from the origin-side durable store only. Incoming bubbles never
  /// infer delivery because they are one recipient, not the group authority.
  final String? deliveryState;
  final int? targetCount;
  final int? deliveredCount;
  DateTime get sentAt => DateTime.fromMillisecondsSinceEpoch(timestamp.millis);
  ChatMessage copyWith({
    String? deliveryState,
    int? targetCount,
    int? deliveredCount,
  }) => ChatMessage(
    id: id,
    body: body,
    timestamp: timestamp,
    mine: mine,
    deliveryState: deliveryState ?? this.deliveryState,
    targetCount: targetCount ?? this.targetCount,
    deliveredCount: deliveredCount ?? this.deliveredCount,
  );
  Map<String, Object> toJson() => {
    'id': id,
    'body': body,
    'h': timestamp.toJson(),
    'mine': mine,
    if (deliveryState case final String delivery) 'delivery': delivery,
    if (targetCount case final int targets) 'targets': targets,
    if (deliveredCount case final int delivered) 'delivered': delivered,
  };
  static ChatMessage? fromJson(Object value) {
    if (value is! Map ||
        value['id'] is! String ||
        value['body'] is! String ||
        value['mine'] is! bool) {
      return null;
    }
    // Old local history used `at`; retain it deterministically during migration.
    final timestamp =
        HybridTimestamp.fromJson(value['h']) ??
        (value['at'] is int
            ? HybridTimestamp(
                value['at'] as int,
                0,
                (value['id'] as String).split('-').first,
              )
            : null);
    if (timestamp == null) return null;
    final delivery = value['delivery'];
    final targets = value['targets'];
    final delivered = value['delivered'];
    if (delivery != null &&
        (delivery is! String ||
            !{'queued', 'partial', 'delivered', 'expired'}.contains(delivery) ||
            targets is! int ||
            delivered is! int ||
            targets < 0 ||
            delivered < 0 ||
            delivered > targets ||
            targets > 49)) {
      return null;
    }
    return ChatMessage(
      id: value['id'] as String,
      body: value['body'] as String,
      timestamp: timestamp,
      mine: value['mine'] as bool,
      deliveryState: delivery as String?,
      targetCount: targets as int?,
      deliveredCount: delivered as int?,
    );
  }
}

/// F0 presentation only. All runtime counters and events originate in Rust.
/// A timed-out diagnostic retries the same ID: it cannot double-count a probe.
class LabController extends ChangeNotifier {
  LabController(this.sdk, {this.timeout = const Duration(seconds: 15)});
  final LabSdk sdk;
  final Duration timeout;
  EngineInfo? info;
  IdentityInfo? identity;
  GroupInfo? group;
  String? enrollmentStatus;
  BluetoothInfo? bluetooth;
  AwareInfo? aware;
  VoiceInfo? voice;
  DeliveryInfo? outgoingVoiceDelivery;
  LocationReading? myLocation;
  LocationReading? groupLocation;
  String? lastTextMessage;
  final List<ChatMessage> _chatMessages = [];
  List<ChatMessage> get chatMessages => List.unmodifiable(_chatMessages);
  bool _chatLoaded = false;
  int _chatSequence = 0;
  final Set<String> _pendingDeliveryIds = <String>{};
  String? _pendingVoiceDeliveryId;
  HybridTimestamp? _lastChatTimestamp;
  EngineSnapshot? snapshot;
  LabFailure? failure;
  bool busy = false;
  bool _disposed = false;
  bool stateIsFresh = false;
  bool _foreground = true;
  String activity = 'Cargando motor';
  int? _pendingRequest;
  Timer? _bluetoothRefresh;
  int _observedMessageCount = 0;
  final List<DiagnosticEvent> _events = [];
  List<DiagnosticEvent> get events => List.unmodifiable(_events.reversed);
  bool get ready => info != null && snapshot != null && stateIsFresh;
  bool get retryPending => _pendingRequest != null;

  /// A secure group link may be Bluetooth LE or native Wi‑Fi Aware. The UI
  /// must not disable text, GPS or voice merely because WFA is the active path.
  bool get secureConnected =>
      bluetooth?.authenticated == true || aware?.state == 'connected';
  bool get sessionActive => bluetooth?.active == true || aware?.active == true;
  String? notice;

  Future<void> refresh() {
    _foreground = true;
    return _run('Recuperando estado', () async {
      await _loadChat();
      final nextInfo = await sdk.engineInfo().timeout(timeout);
      if (nextInfo.abiVersion != 1 ||
          nextInfo.apiVersion != 1 ||
          nextInfo.phase != 'F0') {
        throw PlatformException(code: 'MESH_2');
      }
      final next = await sdk.subscribe(snapshot?.cursor ?? 0).timeout(timeout);
      await _recoverIdentityAndGroup();
      await _readRadios();
      _accept(next);
      info = nextInfo;
    });
  }

  Future<void> verify() => _run('Verificando puente', () async {
    if (info == null || snapshot == null) throw StateError('Not bootstrapped');
    final lastId = _events.fold<int>(0, (n, e) => math.max(n, e.requestId));
    _pendingRequest ??= math.max(
      DateTime.now().microsecondsSinceEpoch,
      lastId + 1,
    );
    final next = await sdk.verifyBridge(_pendingRequest!).timeout(timeout);
    _accept(next);
    // Duplicate retry returns the snapshot without a new event; recover replay if needed.
    if (!_events.any((e) => e.requestId == _pendingRequest)) {
      _accept(await sdk.subscribe(0).timeout(timeout));
    }
    _pendingRequest = null;
    notice =
        'Puente verificado. No se ha probado una conexión entre teléfonos.';
  });

  Future<void> prepareIdentity() => _run('Verificando identidad', () async {
    await _recoverIdentityAndGroup();
    await _readRadios();
    final restoredGroup = group!;
    notice = restoredGroup.configured
        ? 'Identidad y grupo existente recuperados. La búsqueda segura se inició automáticamente.'
        : 'Identidad protegida y almacén cifrado local verificados. Ahora puedes crear un grupo o unirte a uno existente.';
  });

  Future<void> createGroup() => _run('Creando grupo', () async {
    if (identity == null || group?.configured == true) {
      throw StateError('Group creation is not available');
    }
    final result = await sdk.createGroup().timeout(timeout);
    if (!result.configured || result.epoch <= 0) {
      throw StateError('Invalid group state');
    }
    group = result;
    await _readRadios();
    notice =
        'Grupo creado. Toca Conectar sesión para descubrir teléfonos cercanos.';
  });

  Future<void> connectSession() => _run('Conectando sesión', () async {
    await sdk.prepareBluetooth().timeout(timeout);
    bluetooth = await sdk.startBluetoothDiscovery().timeout(timeout);
    aware = await sdk.startAwareDiscovery().timeout(timeout);
    voice = await sdk.voiceInfo().timeout(timeout);
    _startRadioPolling();
    notice = 'Sesión activa: Bluetooth descubre y recupera el enlace; Wi‑Fi Aware busca vecinos directos.';
  });

  Future<void> leaveSession() => _run('Saliendo de sesión', () async {
    bluetooth = await sdk.stopBluetoothDiscovery().timeout(timeout);
    aware = await sdk.stopAwareDiscovery().timeout(timeout);
    notice = 'Sesión cerrada. No se reintentará conectar hasta que toques Conectar sesión.';
  });

  Future<bool> sendText(String message) async {
    var sent = false;
    await _run('Enviando mensaje', () async {
      final body = message.trim();
      if (!secureConnected || body.isEmpty) {
        throw StateError('No authenticated link');
      }
      final origin = identity?.fingerprint.substring(0, 12) ?? 'local';
      final logicalId = _nextLogicalMessageId();
      final entry = ChatMessage(
        id: logicalId,
        body: body,
        timestamp: _nextChatTimestamp(origin),
        mine: true,
        deliveryState: 'queued',
        targetCount: 0,
        deliveredCount: 0,
      );
      final envelope = jsonEncode({
        'v': 2,
        'type': 'chat',
        'id': entry.id,
        'body': body,
        'h': entry.timestamp.toJson(),
      });
      if (!await sdk.sendText(envelope, entry.id).timeout(timeout)) {
        throw StateError('Text was not queued');
      }
      _appendChat(entry);
      _pendingDeliveryIds.add(entry.id);
      await _refreshPendingDeliveries();
      notice = 'Mensaje cifrado puesto en la cola segura.';
      sent = true;
    });
    return sent;
  }

  Future<void> sendVoice(
    Uint8List audio,
    int durationMillis,
  ) => _run('Enviando nota de voz', () async {
    if (!secureConnected) {
      throw StateError(
        'Conecta el segundo teléfono antes de enviar una nota de voz.',
      );
    }
    if (audio.isEmpty || durationMillis < 1 || durationMillis > 8000) {
      throw StateError('La nota de voz no es válida. Graba hasta 8 segundos.');
    }
    final logicalId = _nextLogicalMessageId();
    if (!await sdk
        .sendVoice(audio, durationMillis, logicalId)
        .timeout(timeout)) {
      throw StateError('No se pudo poner la nota de voz en la cola segura.');
    }
    _pendingVoiceDeliveryId = logicalId;
    outgoingVoiceDelivery = DeliveryInfo(
      logicalId: logicalId,
      targetCount: 0,
      deliveredCount: 0,
      state: 'queued',
    );
    await _refreshPendingVoiceDelivery();
    voice = await sdk.voiceInfo().timeout(timeout);
    notice = 'Nota de voz cifrada puesta en la cola del enlace autenticado.';
  });

  Future<void> playLastVoice() => _run('Reproduciendo nota de voz', () async {
    if (!await sdk.playLastVoice().timeout(timeout)) {
      throw StateError('No hay una nota de voz lista para reproducir.');
    }
    voice = await sdk.voiceInfo().timeout(timeout);
    notice = 'Reproduciendo la última nota de voz recibida.';
  });

  Future<void> shareCurrentLocation() => _run('Obteniendo ubicación', () async {
    if (!secureConnected) {
      throw StateError(
        'Conecta el segundo teléfono antes de compartir ubicación.',
      );
    }
    if (!await Geolocator.isLocationServiceEnabled()) {
      throw StateError('Activa la ubicación del teléfono para continuar.');
    }
    var permission = await Geolocator.checkPermission();
    if (permission == LocationPermission.denied) {
      permission = await Geolocator.requestPermission();
    }
    if (permission == LocationPermission.deniedForever) {
      throw StateError(
        'La ubicación está bloqueada. Actívala para Mesh Lab en Ajustes.',
      );
    }
    if (permission == LocationPermission.denied) {
      throw StateError(
        'Necesitamos permiso de ubicación para compartir una posición puntual.',
      );
    }
    final position = await Geolocator.getCurrentPosition(
      locationSettings: LocationSettings(
        accuracy: LocationAccuracy.high,
        timeLimit: timeout,
      ),
    );
    if (!position.latitude.isFinite ||
        !position.longitude.isFinite ||
        position.latitude < -90 ||
        position.latitude > 90 ||
        position.longitude < -180 ||
        position.longitude > 180 ||
        !position.accuracy.isFinite ||
        position.accuracy < 0) {
      throw StateError('El teléfono entregó una ubicación no válida.');
    }
    final reading = LocationReading(
      latitude: position.latitude,
      longitude: position.longitude,
      accuracyMeters: position.accuracy,
      capturedAt: DateTime.now(),
    );
    if (!await sdk
        .sendText(reading.encode(), _nextLogicalMessageId())
        .timeout(timeout)) {
      throw StateError('No se pudo enviar la ubicación por el enlace seguro.');
    }
    myLocation = reading;
    bluetooth = await sdk.bluetoothInfo().timeout(timeout);
    _consumeIncomingPayload(bluetooth!);
    notice = 'Ubicación puntual enviada al teléfono del grupo.';
  });

  Future<void> _recoverIdentityAndGroup() async {
    identity = null;
    final result = await sdk.prepareIdentity().timeout(timeout);
    if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(result.fingerprint) ||
        !{'Keychain', 'Android Keystore'}.contains(result.storage)) {
      throw StateError('Invalid public identity');
    }
    identity = result;
    final restoredGroup = await sdk.groupInfo().timeout(timeout);
    if (restoredGroup.configured && restoredGroup.epoch <= 0) {
      throw StateError('Invalid restored group');
    }
    if (!restoredGroup.configured && restoredGroup.epoch != 0) {
      throw StateError('Invalid empty group');
    }
    group = restoredGroup;
  }

  Future<void> _readRadios() async {
    bluetooth = await sdk.bluetoothInfo().timeout(timeout);
    aware = await sdk.awareInfo().timeout(timeout);
    voice = await sdk.voiceInfo().timeout(timeout);
    _consumeIncomingPayload(bluetooth!);
    await _refreshPendingDeliveries();
    await _refreshPendingVoiceDelivery();
  }

  void _startRadioPolling() {
    _bluetoothRefresh ??= Timer.periodic(
      const Duration(seconds: 1),
      (_) => unawaited(_pollBluetooth()),
    );
  }

  Future<void> _pollBluetooth() async {
    // An unconfigured iPhone is precisely the phone that must keep polling:
    // the native BLE enrollment installs its policy asynchronously.  Returning
    // here hid a successful incorporation until the app was reopened.
    if (_disposed || !_foreground || busy) return;
    try {
      final next = await sdk.bluetoothInfo().timeout(timeout);
      var nextAware = await sdk.awareInfo().timeout(timeout);
      final nextVoice = await sdk.voiceInfo().timeout(timeout);
      if (_disposed) return;
      if (group?.configured != true) {
        final recovered = await sdk.groupInfo().timeout(timeout);
        if (recovered.configured && recovered.epoch > 0) {
          group = recovered;
          enrollmentStatus =
              '✓ Grupo incorporado por Bluetooth. Protegiendo la conexión…';
          notice = 'Segundo teléfono incorporado por Bluetooth. La conexión segura se inició automáticamente.';
          // This phone began as an unconfigured scanner, so its first Aware
          // request correctly did nothing. Once BLE installs the verified
          // policy, start Aware in the same automatic flow; no second tap is
          // needed to make both Android radios advertise the shared tag.
          nextAware = await sdk.startAwareDiscovery().timeout(timeout);
        }
      }
      final deliveryChanged = await _refreshPendingDeliveries();
      final voiceDeliveryChanged = await _refreshPendingVoiceDelivery();
      if (next != bluetooth ||
          nextVoice != voice ||
          nextAware != aware ||
          deliveryChanged ||
          voiceDeliveryChanged) {
        bluetooth = next;
        aware = nextAware;
        voice = nextVoice;
        _consumeIncomingPayload(next);
        notifyListeners();
      }
    } catch (_) {
      // A radio-state refresh must not replace an actionable user operation.
    }
  }

  void _consumeIncomingPayload(BluetoothInfo next) {
    if (next.messageCount < _observedMessageCount) {
      _observedMessageCount = 0;
    }
    if (next.messageCount <= _observedMessageCount) return;
    _observedMessageCount = next.messageCount;
    final location = LocationReading.tryDecode(next.lastMessage);
    if (location != null) {
      groupLocation = location;
    } else if (next.lastMessage.isNotEmpty) {
      lastTextMessage = next.lastMessage;
      try {
        final decoded = jsonDecode(next.lastMessage);
        if (decoded is Map &&
            (decoded['v'] == 1 || decoded['v'] == 2) &&
            decoded['type'] == 'chat' &&
            decoded['id'] is String &&
            decoded['body'] is String) {
          final fallback = decoded['at'] is int
              ? HybridTimestamp(
                  decoded['at'] as int,
                  0,
                  (decoded['id'] as String).split('-').first,
                )
              : null;
          final timestamp = HybridTimestamp.fromJson(decoded['h']) ?? fallback;
          if (timestamp == null) return;
          _observeChatTimestamp(timestamp);
          _appendChat(
            ChatMessage(
              id: decoded['id'] as String,
              body: decoded['body'] as String,
              timestamp: timestamp,
              mine: false,
            ),
          );
        } else {
          _appendChat(
            ChatMessage(
              id: 'legacy-${next.messageCount}',
              body: next.lastMessage,
              timestamp: _nextChatTimestamp('legacy'),
              mine: false,
            ),
          );
        }
      } catch (_) {
        _appendChat(
          ChatMessage(
            id: 'legacy-${next.messageCount}',
            body: next.lastMessage,
            timestamp: _nextChatTimestamp('legacy'),
            mine: false,
          ),
        );
      }
    }
  }

  HybridTimestamp _nextChatTimestamp(String origin) {
    final now = DateTime.now().millisecondsSinceEpoch;
    final last = _lastChatTimestamp;
    final next = last == null || now > last.millis
        ? HybridTimestamp(now, 0, origin)
        : HybridTimestamp(last.millis, last.counter + 1, origin);
    _lastChatTimestamp = next;
    return next;
  }

  String _nextLogicalMessageId() {
    final prefix = identity?.fingerprint.substring(0, 8) ?? '00000000';
    final micros = DateTime.now().microsecondsSinceEpoch
        .toRadixString(16)
        .padLeft(16, '0');
    final sequence = (++_chatSequence).toRadixString(16).padLeft(8, '0');
    return '$prefix$micros$sequence';
  }

  /// The native store is authoritative for receipts. We retain only local
  /// queued/partial IDs here, keeping the periodic work bounded and avoiding
  /// any status lookup for received messages.
  Future<bool> _refreshPendingDeliveries() async {
    var changed = false;
    for (final id in List<String>.from(_pendingDeliveryIds)) {
      final progress = await sdk.deliveryInfo(id).timeout(timeout);
      if (progress.logicalId.isEmpty) continue;
      if (progress.logicalId != id ||
          !RegExp(r'^[0-9a-f]{32}$').hasMatch(progress.logicalId) ||
          !{
            'queued',
            'partial',
            'delivered',
            'expired',
          }.contains(progress.state) ||
          progress.targetCount < 0 ||
          progress.targetCount > 49 ||
          progress.deliveredCount < 0 ||
          progress.deliveredCount > progress.targetCount) {
        throw StateError('Invalid delivery evidence');
      }
      final index = _chatMessages.indexWhere(
        (entry) => entry.id == id && entry.mine,
      );
      if (index < 0) {
        _pendingDeliveryIds.remove(id);
        continue;
      }
      final current = _chatMessages[index];
      if (current.deliveryState != progress.state ||
          current.targetCount != progress.targetCount ||
          current.deliveredCount != progress.deliveredCount) {
        _chatMessages[index] = current.copyWith(
          deliveryState: progress.state,
          targetCount: progress.targetCount,
          deliveredCount: progress.deliveredCount,
        );
        changed = true;
      }
      if (progress.state == 'delivered' || progress.state == 'expired') {
        _pendingDeliveryIds.remove(id);
      }
    }
    if (changed) unawaited(_persistChat());
    return changed;
  }

  /// Voice is an independent visible action, but uses the exact same native
  /// receipt summary as text and location. Retain the last terminal result so
  /// the user can still see the outcome after polling stops.
  Future<bool> _refreshPendingVoiceDelivery() async {
    final id = _pendingVoiceDeliveryId;
    if (id == null) return false;
    final progress = await sdk.deliveryInfo(id).timeout(timeout);
    if (progress.logicalId.isEmpty) return false;
    if (progress.logicalId != id ||
        !RegExp(r'^[0-9a-f]{32}$').hasMatch(progress.logicalId) ||
        progress.targetCount < 0 ||
        progress.targetCount > 49 ||
        progress.deliveredCount < 0 ||
        progress.deliveredCount > progress.targetCount ||
        !{
          'queued',
          'partial',
          'delivered',
          'expired',
        }.contains(progress.state)) {
      throw StateError('Invalid voice delivery evidence');
    }
    final previous = outgoingVoiceDelivery;
    outgoingVoiceDelivery = progress;
    if (progress.state == 'delivered' || progress.state == 'expired') {
      _pendingVoiceDeliveryId = null;
    }
    return previous?.logicalId != progress.logicalId ||
        previous?.targetCount != progress.targetCount ||
        previous?.deliveredCount != progress.deliveredCount ||
        previous?.state != progress.state;
  }

  String? get outgoingVoiceDeliveryLabel {
    final delivery = outgoingVoiceDelivery;
    if (delivery == null) return null;
    return switch (delivery.state) {
      'delivered' =>
        delivery.targetCount > 0
            ? 'Entregado a ${delivery.deliveredCount} de ${delivery.targetCount}'
            : 'Entregado',
      'partial' =>
        'Entregado a ${delivery.deliveredCount} de ${delivery.targetCount}',
      'expired' =>
        delivery.deliveredCount > 0
            ? 'Venció: entregado a ${delivery.deliveredCount} de ${delivery.targetCount}'
            : 'Venció sin confirmación',
      _ => 'En cola segura',
    };
  }

  void _observeChatTimestamp(HybridTimestamp remote) {
    final now = DateTime.now().millisecondsSinceEpoch;
    final local = _lastChatTimestamp;
    if (local == null) {
      _lastChatTimestamp = now > remote.millis
          ? HybridTimestamp(now, 0, remote.origin)
          : remote;
      return;
    }
    final maxMillis = math.max(now, math.max(local.millis, remote.millis));
    final counter = maxMillis == local.millis && maxMillis == remote.millis
        ? math.max(local.counter, remote.counter) + 1
        : maxMillis == local.millis
        ? local.counter + 1
        : maxMillis == remote.millis
        ? remote.counter + 1
        : 0;
    _lastChatTimestamp = HybridTimestamp(maxMillis, counter, local.origin);
  }

  Future<void> _loadChat() async {
    if (_chatLoaded) return;
    _chatLoaded = true;
    try {
      final prefs = await SharedPreferences.getInstance();
      _chatSequence = prefs.getInt('mesh.chat.sequence') ?? 0;
      final raw = prefs.getStringList('mesh.chat.history') ?? const [];
      for (final text in raw) {
        try {
          final entry = ChatMessage.fromJson(jsonDecode(text));
          if (entry != null) _chatMessages.add(entry);
        } catch (_) {}
      }
      if (_chatMessages.isNotEmpty) {
        _chatMessages.sort((a, b) {
          final stamp = a.timestamp.compareTo(b.timestamp);
          return stamp != 0 ? stamp : a.id.compareTo(b.id);
        });
        _lastChatTimestamp = _chatMessages.last.timestamp;
        for (final entry in _chatMessages) {
          if (entry.mine &&
              entry.deliveryState != 'delivered' &&
              entry.deliveryState != 'expired' &&
              RegExp(r'^[0-9a-f]{32}$').hasMatch(entry.id)) {
            _pendingDeliveryIds.add(entry.id);
          }
        }
      }
    } catch (_) {
      // Tests and constrained environments can omit the optional local cache.
    }
  }

  void _appendChat(ChatMessage entry) {
    if (_chatMessages.any((item) => item.id == entry.id)) return;
    _chatMessages.add(entry);
    _chatMessages.sort((a, b) {
      final stamp = a.timestamp.compareTo(b.timestamp);
      return stamp != 0 ? stamp : a.id.compareTo(b.id);
    });
    if (_chatMessages.length > 250) {
      _chatMessages.removeRange(0, _chatMessages.length - 250);
    }
    unawaited(_persistChat());
  }

  Future<void> _persistChat() async {
    try {
      final prefs = await SharedPreferences.getInstance();
      await prefs.setInt('mesh.chat.sequence', _chatSequence);
      await prefs.setStringList(
        'mesh.chat.history',
        _chatMessages.map((item) => jsonEncode(item.toJson())).toList(),
      );
    } catch (_) {}
  }

  void markStale() {
    if (_disposed) return;
    stateIsFresh = false;
    _foreground = false;
    notifyListeners();
  }

  void _accept(EngineSnapshot next) {
    if (_disposed) return;
    final differentRuntime =
        snapshot != null && snapshot!.runtimeId != next.runtimeId;
    if (next.runtimeId <= 0 ||
        next.foundationState != 0 ||
        next.cursor < 0 ||
        next.cursor != next.probeCount ||
        next.events.length > 64 ||
        (next.cursorReset && next.events.isNotEmpty)) {
      throw StateError('Invalid snapshot');
    }
    int? previous;
    for (final event in next.events) {
      if (event.kind != 0 ||
          event.sequence <= 0 ||
          event.sequence > next.cursor ||
          event.requestId <= 0 ||
          (previous != null && event.sequence != previous + 1)) {
        throw StateError('Invalid event sequence');
      }
      previous = event.sequence;
    }
    if (!differentRuntime &&
        !next.cursorReset &&
        snapshot != null &&
        next.cursor < snapshot!.cursor) {
      throw StateError('Stale snapshot');
    }
    // Snapshots are complete and authoritative. UI never guesses missing deltas.
    if (differentRuntime || next.cursorReset) {
      _events.clear();
      notice = differentRuntime
          ? 'El proceso nativo cambió. Se cargó su estado actual.'
          : 'Historial incompleto: se recuperó el estado completo del motor.';
    }
    for (final event in next.events) {
      if (!_events.any((e) => e.sequence == event.sequence)) _events.add(event);
    }
    _events.sort((a, b) => a.sequence.compareTo(b.sequence));
    if (_events.length > 64) _events.removeRange(0, _events.length - 64);
    snapshot = next;
    stateIsFresh = _foreground;
  }

  Future<void> _run(String label, Future<void> Function() action) async {
    if (busy || _disposed) return;
    busy = true;
    failure = null;
    notice = null;
    activity = label;
    notifyListeners();
    try {
      await action();
    } catch (error) {
      if (!_disposed) {
        failure = LabFailure.from(error);
        stateIsFresh = false;
        if (error is PlatformException && error.code == 'MESH_6') {
          _pendingRequest = null;
        }
      }
    } finally {
      if (!_disposed) {
        busy = false;
        notifyListeners();
      }
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _bluetoothRefresh?.cancel();
    super.dispose();
  }
}
