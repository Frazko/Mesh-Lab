# F1 — criptografía base y envío durable en el motor

Estado histórico del segundo bloque. Continuación: [recepción autenticada](F1-authenticated-reception.md).

Segundo bloque, 2026-09-11. El flujo de salida ahora se ejecuta mediante el reducer
y se ha probado junto con criptografía real y SQLCipher en el host. La integración
móvil sigue pendiente: la API Pigeon/FFI pública continúa siendo el diagnóstico F0.
No se ha activado tráfico de texto/GPS/voz ni Bluetooth.

## Implementado

`mesh-crypto` ofrece Ed25519 con verificación estricta, ChaCha20-Poly1305 y HPKE
X25519/HKDF-SHA-256/ChaCha20-Poly1305. La API solo permite esta suite. Las claves
Ed25519 y X25519 tienen tipos distintos; los secretos no implementan Clone, Debug
ni Serialize y sus wrappers se borran al liberarse. Los plaintexts descifrados se
entregan en buffers con zeroization. Esto no es una auditoría del uso de memoria
de todas las dependencias o del sistema operativo.

`RandomSource` es un puerto falible. `OsRandom`, bajo la feature `os-rng`, usa
`getrandom`; la aleatoriedad sintética solo aparece en tests. El adaptador hacia
HPKE registra fallos del RNG, limpia el buffer y rechaza el resultado antes de
aceptar un contexto/ciphertext. Nunca convierte un error del RNG en éxito.

Cada `ContentSealer` genera una clave exclusiva de contenido y un nonce base.
Consume índices en orden, limita cantidad/tamaño, comprueba overflow y no expone
una operación para rebobinar/restaurar el sealer. Reintentar después del commit
reutiliza **los mismos bytes cifrados guardados**. Los wraps HPKE ligan destinatario,
identificador de clave, clave pública, grupo, epoch y contexto del objeto.

Las firmas usan dominios tipados para certificado, objeto, receipt, transición y
recuperación. El AAD de cada chunk liga grupo/epoch, origen/secuencia, audiencia,
namespace/schema, índice/cantidad, lifetime y prioridad. `Scope` y los contextos
son parámetros suministrados por la futura capa que valida membresía; una clave
pública por sí sola no demuestra pertenencia al grupo.

`mesh-runtime::durable::DurableSender` añade el flujo:

```
start → Reserve → reserved → Protect → protected → Commit → committed → Accepted
```

El core produce efectos y recibe resultados; no abre bases, obtiene aleatoriedad
ni hace radio. Ocho operaciones simultáneas como máximo, payloads de hasta 48 KiB,
IDs de efecto por encarnación del runtime, rechazo de resultados fuera de etapa,
comprobación de metadata/ID y deduplicación de solicitudes en curso. La huella del
comando incluye payload, origen y todos los campos de policy. Cambiar argumentos
con el mismo ID falla. El host debe asignar encarnaciones que no se reutilicen
mientras puedan llegar callbacks de la instancia anterior.

`Accepted` significa commit **local**; jamás entrega remota. Ante fallo se emite
`NeedsRetry` con el mismo operation ID porque un error de I/O podría ser ambiguo.
Reiniciar y consultar la operación persistida recupera un commit exitoso sin volver
a cifrar. La nueva consulta `Store::lookup_operation` permite recuperar un resultado
ya confirmado después de expirar, sin reservar secuencias para nuevas solicitudes
vencidas. El ejecutor del efecto usa lookup cuando `create_if_missing` es falso.

## Contratos de bytes internos

Estos contextos fijan la API del provider para pruebas; no son todavía el codec
A003 ni un envelope interoperable. Prefijo: ASCII `MeshLab/Crypto/v1` + NUL + suite
u16 BE (=1) + purpose u8 + group[32] + epoch u64 BE. Firmas: purpose 1–5 según
Domain, longitud u32 BE y bytes canónicos originales. Wrap HPKE: purpose 16,
object_context_id[32], member[32], delivery_key_id[32], public_key[32]; el conjunto
se usa como `info` y AAD. Contenido: purpose 17, object_context_id[32], origin[32],
origin_sequence u64 BE, audience_digest[32], longitud de namespace u8 + ASCII,
schema u32 BE, índice u32 BE, count u32 BE, lifetime u64 BE y prioridad u8.
Nonce de chunk: conservar primeros cuatro bytes; sumar índice a los últimos ocho
bytes interpretados como u64 BE, comprobando overflow.

`object_context_id` debe venir del header inmutable anterior al cifrado. No puede
ser un hash que dependa del propio ciphertext, pues produciría una dependencia
circular en el AAD. El ID local de `mesh-object` sigue siendo el hash del manifiesto
de almacenamiento. El mapeo definitivo hacia object ID de A003 queda pendiente.

La integración de `mesh-sim/tests/durable_runtime.rs` prueba un envelope de
`mesh-protocol` con certificados y roster, persistido mediante `commit_sealed`.
Tras perder el callback y reabrir el store, el destinatario vuelve a autenticar y
descifrar los bytes y el anuncio guardados. `Protect` sigue siendo un efecto de
confianza del host: la futura capa móvil debe suministrar solo roster vigente,
aleatoriedad de sistema y claves protegidas. El test no es un decoder de radio ni
activa recepción desde un peer real.

## Validación

- RFC 8032 (Ed25519), RFC 8439 (AEAD) y RFC 9180 A.2 (HPKE).
- Firma/contexto/payload/clave alterados, claves débiles, public key X25519 de orden
  bajo, errores de RNG, límites, índices incorrectos y overflow del nonce.
- Comprobaciones de compilación que impiden clonar o registrar `ContentKey`.
- Reducer: callbacks atrasados, resultados fuera de orden, ID equivocado, cambios
  de comandos, presión de capacidad y expiración sin falso éxito.
- Integración host: perder callback después del commit, reabrir store y motor,
  repetir incluso después de expirar, recuperar exactamente el mismo ciphertext,
  verificar firma y descifrar con el destinatario correcto.
- Store sin espacio lógico: rollback, ausencia de Accepted/outbox/objeto, reintento
  con la misma reserva y clave de contenido nueva antes de un nuevo commit.

Comandos reproducibles:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
python3 tools/test_f1_crypto_runtime.py
python3 tools/check_contracts.py
python3 tools/test_f1.py
python3 tools/build_native.py all
```

La suite completa pasó 41 casos (39 tests, incluido el auxiliar de crash, y dos
comprobaciones compile-fail). El script conserva el resultado y hashes de fuentes
en `artifacts/f1-crypto-runtime.json`; no es provenance de release.

También se regeneraron los paquetes Rust para Android ARM64/x86_64 y el
XCFramework Apple (dispositivo y simuladores). El provider criptográfico pasó
`cargo check --features os-rng` para iOS ARM64 y Android ARM64; esto es compilación,
no una prueba de ejecución criptográfica en teléfonos. El contrato Swift del host
y los vectores del puente F0 continuaron pasando.

Se conservaron logs locales bajo `artifacts/f1-crypto-runtime-tests.txt` y
`artifacts/f1-native-build.txt`. La lane contracts de CI ahora prueba todas las
features para incluir el RNG del sistema y la inyección de fallos. No se ha observado
una ejecución remota de CI. Las versiones principales se fijaron explícitamente
por compatibilidad de sus APIs; las actualizaciones requieren volver a ejecutar
los vectores. HPKE upstream declara que esa implementación no está auditada;
la revisión criptográfica del sistema sigue siendo un gate pendiente.

## Próximo paso hacia los dos teléfonos

Completar el codec/envelope A003, validación de certificados/admisión, firma y
validación de receipts, finalización de recepción solo tras verificar/descifrar,
y sesiones Noise. Integrar los ejecutores nativos de efectos y sus claves en
Keychain/Keystore, exponer el flujo en la app y conectar BLE para VS2. Este bloque
no cierra F1 ni habilita aún una prueba de comunicación entre teléfonos.

## Fuentes

- [Ed25519 / RFC 8032](https://www.rfc-editor.org/rfc/rfc8032).
- [ChaCha20-Poly1305 / RFC 8439](https://www.rfc-editor.org/rfc/rfc8439.html).
- [HPKE / RFC 9180](https://www.rfc-editor.org/rfc/rfc9180.html).
- [Vectores HPKE oficiales](https://github.com/cfrg/draft-irtf-cfrg-hpke/blob/master/test-vectors.json).
- [API ed25519-dalek 2.2](https://docs.rs/ed25519-dalek/2.2.0/ed25519_dalek/).
- [API HPKE 0.13](https://docs.rs/hpke/0.13.0/hpke/).
- [RUSTSEC-2022-0093](https://rustsec.org/advisories/RUSTSEC-2022-0093.html): resuelto desde Ed25519-dalek 2.
- [RUSTSEC-2024-0344](https://rustsec.org/advisories/RUSTSEC-2024-0344.html): resuelto desde curve25519-dalek 4.1.3, versión del lockfile.

Esta consulta puntual de advisories no sustituye cargo-deny, la revisión completa
del lockfile, la auditoría ni la certificación de F8.
