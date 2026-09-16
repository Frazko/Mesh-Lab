# F0 · Fundación implementada / VS0

Fecha: 2026-09-11. Alcance autorizado: comenzar la primera fase, app Flutter para
iOS y Android, controles claros y prioridad a estabilidad.

## Entregado

- Monorepo con cinco crates funcionales, app Flutter y plugin Swift/Kotlin.
- Puente real Flutter → Pigeon → Swift/C o Kotlin/JNI → Rust.
- engineInfo, snapshot/cursor, replay acotado, reset e idempotencia diagnóstica.
- Runtime nativo que sobrevive a hot restart de Dart; ejecución serial fuera de UI.
- CBOR validado, límites, rechazo de versión, handles obsoletos y panic containment.
- UI Red/GPS/Texto/Voz/Diagnóstico. Verificación disponible al inicio de Red;
  controles futuros deshabilitados con explicación visible.
- XCFramework iOS device/simulator, AAR ARM64/x86_64 y APKs de laboratorio.
- Pruebas Rust, Swift, Kotlin/JNI, Flutter e integración en simulador iPhone.
- CI preparada, vectores/registro y herramientas de inventario/checksums/SBOM Rust.

## Evidencia local

| Verificación | Resultado |
|---|---|
| Rust unit/conformance/recovery/FFI | 9 tests pasaron |
| Carga diagnóstica | 100.000 verificaciones, historial máximo 64 |
| Stress C ABI | 10.000 ciclos de creación/petición/liberación |
| Swift contra Rust real del host | Vectores/entradas truncadas + 1.000 ciclos pasaron |
| Kotlin/JNI contra Rust real del host | 2 tests / 1.000 ciclos pasaron |
| Flutter analyze, app y plugin | Sin incidencias |
| Flutter unit/widget | 11 tests pasaron: timeout, dedupe, reset, lifecycle y layouts |
| Pigeon | Tres fuentes regeneradas sin diferencias |
| Vectores JSON + ABI real | Pasaron; versiones e inputs inválidos rechazados |
| iPhone 17 simulado / iOS 26.4 | Build, ejecución e integración VS0 pasaron |
| Hot restart manual de Dart | El contador de Rust permaneció en 1 |
| Android APK debug | Compiló; anterior al último ajuste visual |
| Android APK release ARM64/x86_64 | Compiló con código final y firma de desarrollo |
| iOS device release | Compiló y posteriormente se firmó para desarrollo y se instaló en el iPhone |
| Android físico: Galaxy A73 / Android 16 | Instalación, ejecución y prueba integrada VS0 pasaron |
| iPhone físico: 17 Pro Max / iOS 26.6.2 | Instalación por Wi-Fi y prueba integrada VS0 pasaron |
| Radios entre teléfonos | No implementados ni probados |
| CI en GitHub | Configurada; no ejecutada remotamente en esta sesión |

## Artefactos

- `app/build/app/outputs/flutter-apk/app-arm64-v8a-release.apk`: teléfono Android ARM64.
- `app/build/app/outputs/flutter-apk/app-x86_64-release.apk`: emulador Android x86_64.
- `app/build/mesh_host/outputs/aar/mesh_host-release.aar`.
- `platforms/mesh_host/ios/mesh_host/MeshEngine.xcframework`.
- `artifacts/build-manifest.json`: hashes, versiones e inventario de fuentes al reportar.
- `artifacts/rust-sbom.cdx.json`: dependencias Rust; no es un SBOM móvil completo.
- `artifacts/flutter-dependencies.json`: inventario Dart/Flutter separado.
- `artifacts/flutter-tests.jsonl`: resultados de las pruebas Flutter finales.
- `artifacts/physical-vs0.json`: evidencia del puente en ambos teléfonos físicos.

Son builds locales ignoradas por Git y reproducibles con los comandos del README.
No tienen firma de distribución ni provenance de CI. El engine informa `local-dev`;
el identificador verificable de cada binario es su SHA-256.

## Límites y cierre formal

**VS0 está implementado; el gate completo F0 del plan maestro no se declara cerrado.**
Pendientes: validación del CI remoto,
contratos wire y vectores criptográficos previos a F1, SBOM móvil completo,
provenance/símbolos de release y revisión independiente de los gates.
SwiftPM está compilado; la alternativa CocoaPods queda sin validar.

No hay grupos, permisos de radio, GPS, grabación, cifrado, store, mensajes durables
ni servicios de fondo activos. No hay estados ficticios de conexión/entrega. El
borrador de texto es local y volátil.

Se conservan las especificaciones originales. Este slice implementa únicamente
el contrato diagnóstico local; no modifica el wire ni la suite criptográfica.
Los demás crates se incorporarán con sus contratos de F1.

## Incidencias resueltas

- Integración ajustada a async/suspend de Pigeon 28.1.0.
- Variante x86_64 incorporada al XCFramework de simulador.
- Filtros ABI compatibles con APK universal soportado y APKs separados.
- Tests de UI desplazan la lista antes de verificar controles fuera de pantalla.
- Una compilación optimizada falló por disco lleno. Se retiraron intermedios,
  caché nativa y el simulador temporal de esta tarea; Android recompiló correctamente.
  Se redujo memoria y concurrencia de Gradle para este proyecto.

El simulador temporal se retiró después de validar la UI. El simulador iPad
preexistente y los datos de los otros proyectos se conservaron.

## Validación física posterior — 2026-09-11

Galaxy A73 (USB, Android 16) e iPhone 17 Pro Max (Wi-Fi, iOS 26.6.2):
`flutter drive --profile` ejecutó el test NAT-VS0 completo con resultado positivo
en ambos. El iPhone utilizó `--publish-port` y firma de desarrollo existente.
Se comprobó ABI/API, petición repetida sin duplicado, snapshot/replay, recreación
de fachada y reset de cursor fuera de la ventana. Esto no prueba tráfico mesh.
Después de la instrumentación se restauró la app normal de laboratorio.
