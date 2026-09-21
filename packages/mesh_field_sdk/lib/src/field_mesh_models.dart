import 'dart:typed_data';

enum FieldConnectionState { disconnected, connecting, connected, unavailable }

/// Origin-side evidence for one action queued to the nearby group.
///
/// The logical ID is safe for a product UI to retain and later query. Object
/// IDs, recipient identities, receipts, encrypted frames, and radio routes
/// remain inside the native host.
enum FieldDeliveryState { queued, partial, delivered, expired }

final class FieldDelivery {
  const FieldDelivery({
    required this.logicalId,
    required this.targets,
    required this.delivered,
    required this.state,
  });

  final String logicalId;
  final int targets;
  final int delivered;
  final FieldDeliveryState state;

  bool get complete => state == FieldDeliveryState.delivered;
  bool get retrying =>
      state == FieldDeliveryState.queued || state == FieldDeliveryState.partial;
}

final class FieldIdentity {
  const FieldIdentity({required this.fingerprint, required this.storage});

  final String fingerprint;
  final String storage;
}

final class FieldGroup {
  const FieldGroup({required this.configured, required this.epoch});

  final bool configured;
  final int epoch;
}

final class FieldBluetoothStatus {
  const FieldBluetoothStatus({
    required this.available,
    required this.authorized,
    required this.enabled,
    required this.active,
    required this.detectedPeers,
    required this.authenticated,
    required this.receivedMessages,
    required this.lastMessage,
    required this.detail,
  });

  final bool available;
  final bool authorized;
  final bool enabled;
  final bool active;
  final int detectedPeers;
  final bool authenticated;
  final int receivedMessages;
  final String lastMessage;
  final String detail;
}

/// A one-shot position shared deliberately with the encrypted field group.
final class FieldLocation {
  const FieldLocation({
    required this.latitude,
    required this.longitude,
    required this.accuracyMeters,
    required this.capturedAt,
    this.headingDegrees,
  });

  final double latitude;
  final double longitude;
  final double accuracyMeters;
  final DateTime capturedAt;

  /// Clockwise vehicle heading in degrees from true north, when available.
  /// Null means the source could not determine a reliable heading.
  final double? headingDegrees;
}

sealed class FieldIncomingEvent {
  const FieldIncomingEvent();
}

final class FieldIncomingText extends FieldIncomingEvent {
  const FieldIncomingText({required this.body});

  final String body;
}

/// Native-certified durable text. These fields are emitted only after the
/// host commits a receipt for the signed object; product code must use this
/// stream, rather than a UI “last message”, for reconciliation.
final class FieldVerifiedIncomingText {
  const FieldVerifiedIncomingText({
    required this.authorId,
    required this.objectId,
    required this.logicalId,
    required this.verifiedAt,
    required this.body,
  });

  /// Certified group member, lowercase hexadecimal (32 bytes).
  final String authorId;

  /// Certified durable object ID, lowercase hexadecimal (32 bytes).
  final String objectId;

  /// Product action ID, carried inside the durable encrypted payload.
  final String logicalId;

  /// The native verification time, captured by the signed-delivery verifier.
  final DateTime verifiedAt;
  final String body;
}

/// Native-certified durable voice. Its audio never crosses the SDK boundary;
/// replay is addressed by [objectId] and performed in the private native host.
final class FieldVerifiedIncomingVoice {
  const FieldVerifiedIncomingVoice({
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

  /// Product-defined encrypted context, bounded to 512 UTF-8 bytes by host.
  final String context;
}

final class FieldIncomingLocation extends FieldIncomingEvent {
  const FieldIncomingLocation({required this.location});

  final FieldLocation location;
}

final class FieldAwareStatus {
  const FieldAwareStatus({
    required this.available,
    required this.enabled,
    required this.active,
    required this.detectedPeers,
    required this.maxDirectPeers,
    required this.state,
    required this.detail,
  });

  final bool available;
  final bool enabled;
  final bool active;
  final int detectedPeers;
  final int maxDirectPeers;
  final String state;
  final String detail;
}

final class FieldVoiceStatus {
  const FieldVoiceStatus({
    required this.receivedCount,
    required this.lastDuration,
    required this.ready,
    required this.detail,
  });

  final int receivedCount;
  final Duration lastDuration;
  final bool ready;
  final String detail;
}

final class FieldSessionStatus {
  const FieldSessionStatus({
    required this.connection,
    required this.bluetooth,
    required this.aware,
    required this.voice,
  });

  final FieldConnectionState connection;
  final FieldBluetoothStatus bluetooth;
  final FieldAwareStatus aware;
  final FieldVoiceStatus voice;

  /// A product may enable its composer only after this condition is true.
  /// It deliberately accepts a verified Wi-Fi Aware link without requiring
  /// Bluetooth to stay on.
  bool get secure => connection == FieldConnectionState.connected;
}

/// The public product contract for a nearby, encrypted field group.
abstract interface class FieldMeshSdk {
  Future<FieldIdentity> prepareIdentity();
  Future<FieldGroup> groupInfo();
  Future<FieldGroup> createGroup();
  Future<FieldSessionStatus> status();
  Future<FieldSessionStatus> connect();
  Future<FieldSessionStatus> leave();

  /// Queues a durable group message and returns its local delivery evidence.
  /// Returns null when the session cannot accept the action.
  Future<FieldDelivery?> sendText(String message);

  /// Queues a one-shot location. It does not enable background tracking.
  Future<FieldDelivery?> sendLocation(FieldLocation location);

  /// Refreshes source-side evidence for an action returned by [sendText].
  /// It returns null for an unknown, malformed, or unavailable action.
  Future<FieldDelivery?> delivery(String logicalId);

  /// Queues a durable voice note and returns receipt-backed delivery evidence.
  ///
  /// Voice uses the same logical action and outbox semantics as text and
  /// location, so a product never has to infer delivery from a boolean.
  Future<FieldDelivery?> sendVoice(Uint8List audio, Duration duration);
  Future<bool> playLastVoice();
  Stream<FieldSessionStatus> watch({Duration interval});

  /// Emits new text or location events received after this subscription starts.
  Stream<FieldIncomingEvent> watchIncoming({Duration interval});
}

/// Optional capability for products that keep a local field group available
/// while their cloud route remains healthy.
///
/// This does not share, proxy, or tether Internet. It keeps only the signed
/// nearby radio custody alive so an authenticated neighbour that loses
/// coverage can still exchange its own group actions. Products retain their
/// normal server route in parallel and use the same logical action ID on both
/// transports when they choose dual delivery.
///
/// It is separate from [FieldMeshSdk] to preserve source compatibility for
/// integrations that intentionally activate nearby radios only on demand.
abstract interface class FieldMeshPresenceController {
  Future<FieldSessionStatus> maintainPresence();
}

/// Product-owned admission scope for automatic group enrollment.
///
/// Member IDs are public, lowercase 32-byte fingerprints. The separate scope
/// is an opaque product identifier. The host verifies each signed enrollment
/// request independently before it compares the request with this roster, so
/// callers cannot authorize a request by naming a radio address.
final class FieldEnrollmentAccessPolicy {
  FieldEnrollmentAccessPolicy({
    required this.scopeId,
    required Iterable<String> authorizedMemberIds,
    required this.authorityEnabled,
  }) : authorizedMemberIds = Set.unmodifiable(authorizedMemberIds);

  final Set<String> authorizedMemberIds;

  /// Opaque lowercase 128-bit product group scope. The value is local to the
  /// host and is never advertised over Bluetooth or Wi-Fi Aware.
  final String scopeId;

  /// Only the product's current authority may issue new membership policies.
  /// A former leader retains no admission power after the product disables it.
  final bool authorityEnabled;
}

/// Optional product control for automatic enrollment. A Convoy adapter sets
/// this from its authenticated membership roster before it starts radios. The
/// Mesh Lab can intentionally omit it to retain its explicit open experiment.
abstract interface class FieldMeshEnrollmentAccessController {
  Future<void> configureEnrollmentAccess(FieldEnrollmentAccessPolicy policy);
  Future<void> clearEnrollmentAccess();

  /// True only if this phone's protected identity is the authority certified
  /// in the active Field policy. A product must fail closed after a leader
  /// change until it completes a signed authority rotation.
  Future<bool> canIssueEnrollment();
}

/// Optional authority-rotation capability. Its byte payloads are public signed
/// transport data; private authority material never leaves the native host.
abstract interface class FieldMeshAuthorityHandoffController {
  Future<Uint8List> prepareAuthorityHandoff(
    String successorMemberId,
    DateTime validUntil,
  );
  Future<Uint8List> rotateAuthority(Uint8List handoff);

  /// Re-exports the current, already verified public policy from the protected
  /// host. This lets a promoted authority recover publication after a network
  /// interruption without re-running the irreversible rotation.
  Future<Uint8List> exportCurrentPolicy();
  Future<FieldGroup> installRotatedPolicy(Uint8List policy, Uint8List handoff);
}

/// Optional capability for products that already persist their own action ID.
///
/// The base [FieldMeshSdk] remains source-compatible for existing products.
/// Implementations of this capability preserve one canonical 128-bit,
/// lowercase-hex ID across Internet and mesh, or reject the action.
abstract interface class FieldMeshActionSender {
  Future<FieldDelivery?> sendTextWithLogicalId(
    String message,
    String logicalId,
  );
  Future<FieldDelivery?> sendLocationWithLogicalId(
    FieldLocation location,
    String logicalId,
  );
  Future<FieldDelivery?> sendVoiceWithLogicalId(
    Uint8List audio,
    Duration duration,
    String logicalId,
  );
}

/// Optional stream for adapters that need authenticated incoming actions.
///
/// Existing product integrations can continue to use [FieldMeshSdk]; Convoy
/// uses this capability and rejects non-certified incoming text.
abstract interface class FieldMeshVerifiedIncomingSource {
  Stream<FieldVerifiedIncomingText> watchVerifiedIncomingText({
    Duration interval,
  });
}

/// Optional voice capability. A voice is exposed only after native durable
/// verification and receipt commit; products cannot access a file path or raw
/// audio bytes for incoming notes.
/// Optional sender for applications that need an authenticated product envelope
/// attached to voice. The context travels within the same encrypted durable
/// object as the audio and is returned only after certified receipt commit.
abstract interface class FieldMeshVoiceContextSender {
  Future<FieldDelivery?> sendVoiceWithLogicalIdAndContext(
    Uint8List audio,
    Duration duration,
    String logicalId,
    String context,
  );
}

abstract interface class FieldMeshVerifiedIncomingVoiceSource {
  Stream<FieldVerifiedIncomingVoice> watchVerifiedIncomingVoice({
    Duration interval,
  });

  Future<bool> playVerifiedVoice(String objectId);
}
