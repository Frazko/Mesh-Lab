# Mesh Lab

**Avance global estimado: 17%** — [seguimiento y pendientes](docs/progress.md).

Aplicación Flutter de laboratorio para iOS y Android. El motor compartido es Rust;
Swift/Kotlin poseen su ciclo de vida y Flutter muestra snapshots mediante Pigeon.

La app móvil implementa **F0 (fundación / VS0)**: el puente
es real. Todavía no hay conexiones entre teléfonos, grupos, envío de GPS, texto ni
voz. Esos controles aparecen deshabilitados con su motivo. No se usan peers o
entregas ficticios en la app.

El motor incorpora además el [primer bloque de F1](docs/F1-status.md): store cifrado,
objetos durables y simulador sintético con recuperación. Se prueba en el host y
todavía no se integra al runtime móvil. Ejecutar `python3 tools/test_f1.py` para
reproducir el escenario de diez nodos. El [segundo bloque de F1](docs/F1-crypto-runtime.md)
añade primitivas criptográficas verificadas y el flujo durable de salida del motor.
El [tercer bloque](docs/F1-authenticated-reception.md) incorpora certificados,
envelopes completos, recepción autenticada y confirmaciones firmadas persistentes.
El [bloque actual](docs/F1-session-identity.md) añade Noise XX y preparación de
identidad nativa con control en Diagnóstico; instalación y QA de claves en teléfonos
pendientes por espacio de disco.

## Ejecutar

Herramientas fijadas: Flutter 3.47.2 / Dart 3.13.2, Rust 1.98.1,
NDK 28.2.13676358, JDK 21, Gradle 9.3.1, AGP 9.1.0 y Kotlin 2.4.0.
Desarrollo local validado con Xcode 26.4.1. Targets iniciales: iOS 15+;
Android API 24+; Android ARM64/x86_64; iOS ARM64 device y ARM64/x86_64 simulator.
Estos mínimos de compilación no certifican la futura compatibilidad de radio.

```sh
export PATH="$HOME/.cargo/bin:$PATH"
python3 tools/build_native.py all
cd app
flutter pub get --enforce-lockfile
flutter devices
flutter run -d <device-id>
```

Para trabajar en una sola plataforma, usar `apple` o `android` en lugar de `all`.
Los binarios nativos son locales e ignorados por Git: un checkout nuevo debe
construirlos antes de ejecutar Flutter. Tras cambios Rust, reconstruir el paquete
y relanzar la app; hot reload no reemplaza bibliotecas nativas.

En **Red → Verificar puente** se prueba Flutter → Pigeon → host → Rust.
**Diagnóstico → Recuperar estado** vuelve a leer la instancia y su secuencia.
Un hot restart de Dart conserva la instancia. Terminar el proceso empieza un
runtime diagnóstico nuevo; F0 no tiene persistencia ni identidad de usuario.

## Validar

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
python3 tools/test_f1_crypto_runtime.py
python3 tools/check_contracts.py
(cd platforms/mesh_host && flutter pub get --enforce-lockfile && flutter analyze)
python3 tools/check_pigeon.py
(cd app && flutter analyze && flutter test)
sh tools/test_apple_host.sh
cargo build --locked -p mesh-ffi-jni
(cd app/android && ./gradlew :mesh_host:testDebugUnitTest :mesh_host:assembleRelease)
```

En macOS configurar `JAVA_HOME` al JDK 21 antes de invocar Gradle directamente;
Flutter usa el JDK configurado en su toolchain. Los tests JVM cargan la biblioteca
Rust **del host**, no ejecutan el binario Android. La validación física actual se
hace siempre con los paquetes Release del laboratorio, nunca con un runner de
integración instalado sobre la app.

## Estructura

- `app/`: pantalla de laboratorio y proyección de estado, sin radio/FFI/PCM en Dart.
- `platforms/mesh_host/`: plugin local, fuentes Pigeon y hosts Swift/Kotlin.
- `crates/mesh-types`, `mesh-codec`, `mesh-runtime`: contratos y reducers puros de diagnóstico/envío/recepción.
- `crates/mesh-object`, `mesh-store`, `mesh-replication`, `mesh-sim`: persistencia/simulador F1.
- `crates/mesh-crypto`, `mesh-protocol`, `mesh-session`: provider criptográfico y perfil autenticado del lab.
- `crates/mesh-ffi-c`, `mesh-ffi-jni`: límites nativos y empaquetado.
- `schema/`, `vectors/`: contrato local CBOR, registros y vectores ejecutables.
- `tools/`: compilación, conformance e inventario de artefactos.
- `.github/workflows/`: CI preparada; no ejecutada remotamente durante este trabajo.

El grafo se ampliará con link, routing y telemetry cuando sus contratos entren a F1. No hay crates vacíos presentados como
funcionalidad implementada.

Ver [estado y evidencia F0](docs/F0-status.md), [contrato del puente](docs/foundation-contract.md)
y [cómo probar en teléfonos](docs/mobile-testing.md).
Las especificaciones HTML originales se conservan como fuente de arquitectura.
No se ha seleccionado licencia de distribución ni publicado ningún release.
