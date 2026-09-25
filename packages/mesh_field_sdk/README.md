# Mesh Field SDK

Reusable Flutter interface for a nearby Mesh session without Internet. It hides
the native bridge, Bluetooth LE, Wi-Fi Aware, IP, credentials, and encrypted
frames. An application consumes only session state and product operations.

```dart
final mesh = FieldMeshClient();

await mesh.prepareIdentity();
final session = await mesh.connect();

if (session.secure) {
  final delivery = await mesh.sendText('Convoy ready');
  // Keep delivery?.logicalId to refresh the state later.
  if (delivery?.complete == true) {
    // All certified recipients confirmed delivery.
  }
}

mesh.watch().listen((state) {
  // Project state.connection and radio state into the product UI.
});
```

`MeshHostGateway` is the current iOS/Android adapter over `mesh_host`.
Products can replace `FieldMeshGateway` with a test fake without initializing
Flutter channels or physical radios.

## Current scope

The initial version exposes identity, group state, Bluetooth, Wi-Fi Aware,
voice, connection, durable text, one-shot location, and aggregate delivery
evidence. `sendText` and `sendLocation` return a logical ID, and `delivery(id)`
refreshes `queued`, `partial`, `delivered`, or `expired`. `FieldLocation` carries
coordinates, accuracy, time, and an optional vehicle heading in degrees; a null
heading means the phone could not obtain it reliably. `watchIncoming` provides
received text or location to a product interface.

For an integration that must attribute and reconcile actions, the client
implements the optional `FieldMeshVerifiedIncomingSource` and
`FieldMeshVerifiedIncomingVoiceSource` capabilities. Their native FIFO queues
contain only durable objects verified after all chunks and the local receipt.
Voice includes roster origin, object ID, logical ID, timestamp, duration, and
up to 512 bytes of product context; `FieldMeshVoiceContextSender` seals it with
the audio. `playVerifiedVoice(objectId)` plays the corresponding private file
without giving the product audio bytes or a path. The SDK validates the format
again and discards malformed evidence. An application's logical ID travels
inside the encrypted payload before the application correlates an incoming
action with its own outbox.
It does not expose encrypted objects, receipts, recipient identities, paths,
IP addresses, or keys. Global presence and continuous background tracking
remain outside the SDK because they require product policy and physical
validation.

Mesh Lab is the reference consumer and physical harness. Convoy will be the
first product integration: it maps members to vehicles, positions to the map,
and messages to its chat without knowing the radios or topology.
