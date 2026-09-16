# F1 — recepción autenticada y confirmaciones durables

Estado histórico: continuación en [sesiones e identidad](F1-session-identity.md).

Tercer bloque, 2026-09-11. Implementado y probado en el host Rust/SQLCipher.
La app instalada conserva el diagnóstico F0: aún no intercambia texto, GPS o voz.

## Flujo implementado

`mesh-protocol` introduce un perfil de laboratorio **v1**, subconjunto de A002/A003:
certificados de miembros, lista verificada de miembros, envelope cifrado completo,
anuncio firmado y receipt firmado. Los campos y límites están en
`schema/protocol/lab-v1.cddl`. No es una declaración de interoperabilidad con todo
A003 ni implementa Noise, invitaciones o admisión por radio.

El host entrega autoridad anclada, grupo/epoch vigente, certificados y revocaciones
conocidas. `VerifiedRoster` comprueba firmas, vigencia, pertenencia al grupo/epoch,
seriales revocados y duplicados de identidad/claves/seriales. Nunca se toma una raíz
de confianza del anuncio entrante. Los certificados del perfil conceden únicamente
el rol fijo de enviar/recibir mensajes; capacidades, roles y recuperación quedan pendientes.

`seal_message` recibe una secuencia global ya reservada, crea salt, clave y nonce
nuevos mediante el puerto de aleatoriedad y almacena en el envelope el header,
wraps HPKE de los destinatarios y todos los chunks AEAD. El header liga grupo,
secuencia/origen, audiencia y sus claves, namespace, schema, tiempos y tamaño.
Máximo: 48 KiB de plaintext, 10 destinatarios y 64 KiB del objeto almacenado.
El anuncio firmado liga el grupo y el manifiesto local con hashes de todo el envelope.

El receptor valida el anuncio **antes de reservar espacio**. Puede guardar y reanudar
fragmentos sin generar una entrega. Para finalizar, reconstruye desde SQLCipher el
anuncio y los bytes completos, verifica firmas, metadata, hashes, destinatario y
**cada** tag AEAD. Solo entonces obtiene `VerifiedDelivery`, cuyo constructor no
está expuesto. Los plaintexts temporales se borran al liberarse; aún no se proyectan
a Flutter ni se conservan descifrados para lectura/reproducción.

`Store::finalize_received` vuelve a comprobar el objeto, el tiempo y la versión de
la lista de miembros, y guarda el marcador de entrega y su receipt firmado en **una
transacción**. Devuelve el receipt después del commit. Un reinicio o reintento
recupera exactamente la misma confirmación. Una lista de certificados/revocaciones
cambiada entre verificación y commit invalida la prueba y exige verificar otra vez.

El emisor verifica grupo, objeto, secuencia, destinatario, tiempo y firma del receipt
contra el envelope original. Persiste cada confirmación una sola vez; retira el
objeto del outbox cuando **todos** los destinatarios han confirmado. Conserva los
bytes originales. Un receipt generado antes de expirar puede llegar después de la
expiración del objeto, mientras los certificados actuales sigan siendo válidos.
La confirmación acredita recepción/descifrado por el peer, no lectura humana.

`mesh-runtime::reception::DurableReceiver` es un reducer sin I/O:

```
Inspect → [receipt persistido: Received]
        → Verify → Commit → [callback posterior al commit: Received]
```

Limita ocho operaciones simultáneas y correlaciona etapa y encarnación del runtime.
Rechaza callbacks antiguos, duplicados, fuera de orden o de otro objeto/miembro.
Un error ambiguo produce `NeedsRetry`; el reintento consulta primero el receipt
persistido, incluso si el mensaje ya expiró. El host es de confianza: jamás debe
alimentar `inspected`/`committed` desde un ACK de transporte. La función de bajo
nivel `receipt_for_commit` solo prepara bytes para la transacción del store;
llamarla y publicar esos bytes directamente no demuestra persistencia.

## Almacenamiento y compatibilidad

Migración transaccional de schema 1 a 2: añade anuncios, receipts locales y receipts
por destinatario. Conserva reservas, secuencias y objetos existentes. Comprueba
hash de schema e identidad; un schema inesperado o alterado falla sin migración
parcial. El schema 1 original permanece intacto como entrada de migración.

Las APIs sintéticas anteriores se conservan para el simulador base y están aisladas
por objeto: no pueden insertar chunks ni marcar entregas de objetos protegidos,
ni convertir una entrega sintética preexistente en una entrega autenticada.
`LocalCommit` por sí solo **no** es un receipt de red. La futura integración móvil
usará exclusivamente `announce_authenticated`, `stage_authenticated_chunk`,
`verify_received`, `finalize_received` y `commit_sealed` para este perfil.

`DurableSender` ahora solo acepta un `SealedMessage` completo al terminar
`Protect`. El efecto `Commit` conserva, en un contenedor diferido, el objeto
cifrado y su anuncio firmado; el host autenticado debe llamar `commit_sealed` con
ese mismo valor y el roster vigente. Ya no hay una ruta del reducer que acepte un
`PreparedObject` desnudo y pueda perder el anuncio antes del commit.

La integración `mesh-sim/tests/durable_runtime.rs` usa certificados, roster,
envelope real y SQLCipher. Comprueba que un reinicio después de un commit sin
callback recupera los mismos bytes, incluido el anuncio, y que el destinatario
puede autenticar y descifrar desde el store reabierto. El ejecutor móvil y el
roster persistido siguen pendientes; estos tests no activan radio.

## Contratos y límites del perfil

- CBOR de arrays con longitudes definidas y representación mínima. Rechaza campos
  inesperados, versiones desconocidas, datos sobrantes, tamaños abusivos y truncamientos.
- `context_id = SHA256("MeshLab/EnvelopeContext/v1" + NUL + header_canónico)`.
  Este ID precede al cifrado y evita depender circularmente del ciphertext.
  El ID local sigue siendo el hash del manifiesto de almacenamiento; ambos IDs
  quedan ligados en cada receipt. No sustituye al object ID definitivo de A003.
- Clave de entrega: SHA256 del prefijo `MeshLab/DeliveryKeyId/v1` + NUL + clave pública.
  Audiencia y versión de roster también tienen prefijos hash distintos.
- Suite fija Ed25519 + HPKE X25519/HKDF-SHA-256/ChaCha20-Poly1305 y chunks
  ChaCha20-Poly1305. Firmas separadas por dominio certificado/objeto/receipt.
- Tiempos enteros suministrados por el host; no hay reloj de pared interno ni HLC
  completo. Prioridad y payload schema fijos en 1. El host serializa efectos y
  debe proveer tiempo y política vigentes, sin reusar encarnaciones del runtime.
- La política/raíz/epoch/revocaciones activas aún no se persisten ni distribuyen:
  son entradas de confianza del host. Pasar una lista antigua como si fuera actual
  queda fuera de la protección de este bloque.

## Evidencia reproducible

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
python3 tools/test_f1_crypto_runtime.py
python3 tools/test_f1.py
python3 tools/check_contracts.py
python3 tools/build_native.py all
sh tools/test_apple_host.sh
```

La suite incluye vectores CBOR/hash generados con Python independiente del codec
Rust; certificados inválidos/revocados/fuera de grupo o epoch; límites y truncamientos;
receipts con actor, objeto, contexto, etapa o dominio alterados; objeto firmado por
un miembro legítimo cuyo último chunk no descifra; migración; reanudación;
recuperación del reducer y muerte real de procesos antes/después de ambos commits.
Los tests criptográficos RFC del bloque anterior siguen incluidos.

Validación local completada: **58 casos aprobados** (56 tests, incluidos dos
auxiliares de procesos de crash, y dos comprobaciones compile-fail), Clippy sin
advertencias, bridge CBOR/Swift y 1000 ciclos de vida nativos. El escenario
sintético anterior conserva 216/216 copias destinatarias tras 143 reinicios;
ese escenario todavía usa ACKs locales sintéticos. Se reconstruyeron los cinco
targets nativos: iOS dispositivo y dos simuladores, Android ARM64/x86_64.

Resultado y hashes del código: `artifacts/f1-crypto-runtime.json`, con log completo
en `artifacts/f1-crypto-runtime-tests.txt`. Compilación nativa:
`artifacts/f1-auth-native-build.txt`. Son evidencias locales, no ejecución remota
de CI ni pruebas del nuevo flujo en teléfonos.

## Siguiente bloque

Sesiones seguras y persistencia nativa de claves/política en Keychain/Keystore;
ejecutor autenticado de envío y recepción y contrato FFI/Pigeon para la UI.
Después, transporte BLE y prueba directa Android ↔ iPhone. La verificación móvil
del bridge F0 no equivale a ejecutar SQLCipher/crypto/radio del flujo nuevo. F1
completo, pruebas de particiones autenticadas multihop y revisión criptográfica
siguen pendientes.
