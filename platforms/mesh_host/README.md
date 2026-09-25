# mesh_host

Private Flutter plugin for Mesh Lab. Generate it with:
`dart run pigeon --input pigeons/mesh_api.dart`.
Pigeon is pinned to 28.1.0; Dart, Swift, and Kotlin are regenerated together.

- iOS: SwiftPM with a static XCFramework; the podspec is an alternative path (not validated locally).
- Android: ARM64/x86_64 JNI library included in the AAR; symbols are retained in local Cargo outputs.
- API: `engineInfo`, `subscribe(cursor)`, `verifyBridge(requestId)`.
- The runtime belongs to the native process, not the widget or Dart isolate.
- A serial queue runs bounded local operations outside the UI thread.
- F0 has no radios, background services, GPS, keys, audio, or user-file access.

Snapshot queries include accumulated diagnostic events and an explicit reset
signal when the cursor has moved outside the window. This slice does not keep a
continuous stream or run protocol timers.
