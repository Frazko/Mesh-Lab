# mesh_host

Plugin Flutter privado de Mesh Lab. Generación: `dart run pigeon --input pigeons/mesh_api.dart`.
Pigeon está fijado a 28.1.0; Dart, Swift y Kotlin se regeneran juntos.

- iOS: SwiftPM con XCFramework estático; podspec como vía alternativa (no validada localmente).
- Android: biblioteca JNI ARM64/x86_64 incluida en AAR; símbolos conservados en las salidas Cargo locales.
- API: engineInfo, subscribe(cursor), verifyBridge(requestId).
- El runtime pertenece al proceso nativo, no al widget ni al isolate Dart.
- La cola serial ejecuta operaciones locales acotadas fuera del hilo UI.
- Sin radios, servicios de fondo, GPS, claves, audio ni acceso a archivos de usuario en F0.

Las consultas snapshot incluyen eventos de diagnóstico acumulados y una señal
explícita de reset si el cursor salió de la ventana. Este slice no mantiene un
stream continuo ni ejecuta timers de protocolo.
