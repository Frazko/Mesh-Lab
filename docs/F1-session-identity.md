# F1 — Noise XX e identidad nativa

2026-09-11. **17% global estimado; 100% de este bloque.** Estado detallado en
[progress.md](progress.md). Identidad persistente validada en ambos teléfonos;
Noise XX tiene evidencia local y aún no se ejecuta entre dispositivos por radio.

## Implementado

`mesh-session` usa Noise XX con la suite aprobada, entropía falible del host y un
proveedor restringido. Cada intento obtiene un efímero fresco; rechaza DH X25519
de resultado cero. Tras Noise exige confirmaciones Ed25519 de ambos extremos,
con identidad certificada, rol y hash del transcript. El prologue liga grupo,
epoch, versión y capacidades fijas del perfil. No permite tráfico de aplicación
antes de autenticar al peer. Rechaza firmas falsas, peer inesperado, transcript
diferente, roster cambiado, expiración y replay.

El transporte cifra hasta 4096 bytes por paquete, separa claves por dirección y
mantiene una ventana de 64 números. Admite reordenamiento dentro de la ventana;
un paquete inválido no la desplaza. Limita la vida de cada contador y cierra al
agotarse. Reconexionar usa otro XX y claves nuevas; IK queda pendiente. El contrato
exacto está en `schema/session/lab-v1.md`. No hay radio ni retransmisión automática
oculta: el próximo ejecutor deberá guardar/retransmitir los mismos frames y reabrir
sesiones al perder el enlace.

La prueba de integración transmite por Noise un anuncio firmado y sus chunks,
verifica/descifra y guarda el objeto en SQLCipher, devuelve el receipt por el mismo
canal y retira el outbox solo después de verificarlo y persistirlo en el emisor.
Los secretos de esa prueba son fixtures públicas limitadas al harness.

`SecureIdentity.swift` prepara un bundle de 128 bytes del CSPRNG en Keychain:
cuatro slots independientes de 32 bytes para firma, entrega HPKE, estática Noise y
SQLCipher. Usa `AfterFirstUnlockThisDeviceOnly`, sin sincronización iCloud, y un
marcador protegido/excluido del backup en Application Support. Si el marcador existe
pero la clave desaparece, falla sin regenerar la identidad. iOS puede conservar
Keychain al reinstalar; este bloque no implementa borrado/rotación de identidad.
La protección elegida impide migración de claves a otro dispositivo; la inspección
real de backups/restore y lifecycle sigue pendiente.

`SecureIdentity.kt` usa una clave AES-256-GCM de Android Keystore para envolver
los mismos cuatro slots. Archivo acotado y versionado de 157 bytes en
`noBackupFilesDir`, AAD propio y escritura `AtomicFile` serializada. La clave de
wrapping nunca se exporta. Un archivo existente sin clave, un tag incorrecto o una
clave huérfana sin archivo bloquean la operación, sin crear otra identidad en silencio.
`allowBackup=false` añade una restricción en la aplicación. No se afirma que todas
las claves residan o se utilicen exclusivamente en hardware seguro.

Un método C/JNI separado obtiene solamente la clave pública Ed25519 desde la semilla
nativa; verifica longitud antes de leer/copiar, contiene panics y devuelve un buffer
público con ownership explícito. Se liberan/borran temporales propios en Rust y hosts;
no se ha auditado cada copia interna de Swift/JVM/Snow ni del sistema operativo.
La API no transporta semillas, claves de tráfico o DB hacia Flutter.

Pigeon ahora incluye `prepareIdentity → IdentityInfo(fingerprint, storage)`.
**Diagnóstico → Preparar identidad / Verificar identidad** muestra SHA-256 de la
clave pública en hexadecimal completo, con etiqueta de protección y explicación
clara. Ante fallo retira el éxito anterior. El resto de la app conserva el diagnóstico
F0; el control no abre radios ni inscribe un miembro en un grupo.

Desde este bloque, `prepareIdentity` también abre `state-v1.db` mediante el FFI
SQLCipher. Solo el host toma el cuarto slot: `SecureIdentity` deriva el miembro
Ed25519, entrega ambos buffers temporales al FFI y los borra al terminar. Flutter
no recibe una clave de base de datos. iOS crea `Application Support/mesh-store`
con protección hasta el primer desbloqueo y sin backup; Android usa
`noBackupFilesDir/mesh-store`. SQLCipher integra OpenSSL vendorizado para Android,
y el script de paquetes fija los compiladores NDK y el mínimo iOS 15.

## Verificación y límites

- **69 casos Rust aprobados**: suite anterior + Noise/anti-replay + vector nativo
  de clave pública + integración Noise/SQLCipher/receipts. Clippy sin advertencias.
- Noise comparado byte a byte con vector Cacophony publicado: tres mensajes de
  handshake, hash final y tres mensajes de transporte, usando el resolver productivo.
- **14 tests Flutter** (suite de 13 más un nuevo test de UI), análisis sin errores;
  huella válida, fallo sin éxito residual y controles sin overflow.
- Kotlin **2.4.0**: todas las fuentes del plugin compiladas contra SDK Android 36 y
  Flutter; **3 tests JVM/JNI** aprobados mediante jars fijados ya disponibles en caché.
  Esto no ejecuta Android Keystore. `tools/test_android_host_cached.py` conserva
  la ruta reproducible de bajo uso de disco; no sustituye el build Gradle de CI.
- Swift: typecheck del almacén para **iOS ARM64 / mínimo iOS 15**. Bridge real,
  vector de clave pública y 1000 ciclos de vida nativos aprobados en el host.
- Bibliotecas Rust reconstruidas para los cinco targets Apple/Android. `mesh-session`
  también pasó cargo check de release para iOS ARM64 y Android ARM64. Esto no
  equivale a compilar/empaquetar la app Flutter completa ni ejecutar Noise en teléfonos.
- Prueba física `KEY-01` y NAT-VS0 **aprobadas en Android e iPhone**, dos procesos
  distintos por teléfono conservando la instalación (`--keep-app-running`).
  Android: PID 18924/19246; iOS: PID 4056/4059. Misma huella por teléfono y huellas
  distintas entre plataformas. Evidencia: `artifacts/identity/device-results.json`.
  Esto no ejecuta sesiones Noise ni radio entre dispositivos.
- Tras integrar el store, NAT-VS0 y KEY-01 volvieron a aprobar en iPhone Wi-Fi y
  Android Wi-Fi. KEY-01 abre SQLCipher dos veces desde Keychain/Keystore y solo
  publica la huella. Los logs son `ios-secure-store-retry.log` y
  `android-secure-store.log`; las release normales se restauraron después.

Los logs están en `artifacts/f1-session-native-build.txt`,
`f1-keys-kotlin-host.txt`, `f1-keys-swift-typecheck.txt` y la suite con hashes
`f1-crypto-runtime.json` / `f1-crypto-runtime-tests.txt`.
El intento Gradle falló al descargar su distribución por disco lleno
(`f1-keys-android-tests.txt`). La descarga parcial fue retirada; se conservaron los logs.

El canal y las claves están implementados como componentes. Todavía falta abrir
sesiones desde el host móvil, cargar la política vigente y consumir los slots
protegidos para los efectos de envío/recepción y SQLCipher. Los reducers y el perfil
wire completos, IK, invitaciones/QR, persistencia de autoridad/revocaciones y BLE
no quedan resueltos por este bloque. Se ejecutaron las pruebas y se restauró la app normal en
ambos teléfonos; no se publicó ninguna app.

## Prueba manual disponible en ambas apps instaladas

1. En ambos teléfonos: Diagnóstico → Preparar identidad. Las huellas deben ser distintas.
2. Terminar y reabrir cada app; pulsar Preparar identidad. La huella de cada teléfono
   debe mantenerse. Hot restart de Flutter no reemplaza esta prueba de proceso.
3. Repetir Verificar identidad; verificar que no cambie y que no se creen eventos
   de conexión ni aparezcan destinatarios ficticios.
4. Conservar datos si hay error; no desinstalar para resolver un fallo de claves.

La limpieza de temporales liberó espacio y ambas apps ya compilaron. La autorización
para instalar en Android USB y iPhone Wi-Fi ya consta en esta tarea.

Fuentes: [Noise Framework](https://noiseprotocol.org/noise.html),
[Snow 0.10.0](https://docs.rs/snow/0.10.0/snow/),
[protección de Keychain](https://developer.apple.com/documentation/security/ksecattraccessibleafterfirstunlockthisdeviceonly),
[criptografía Android](https://developer.android.com/privacy-and-security/cryptography),
[AtomicFile](https://developer.android.com/reference/android/util/AtomicFile).
