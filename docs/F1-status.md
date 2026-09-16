# F1 — primer bloque de persistencia y simulación

Implementado el 2026-09-11. F1 sigue en curso; este bloque prepara VS1 y no cierra
el gate de F1 ni VS1 completo. La app instalada en Android/iPhone conserva F0.
La prueba manual de F0 fue reportada como satisfactoria por el usuario.

El [segundo bloque: criptografía y reducer durable](F1-crypto-runtime.md) ya está
implementado y validado en host. La evidencia de este documento corresponde al
primer bloque sintético.

## Capacidad disponible

- `mesh-object`: manifiesto local inmutable con audiencia, expiración, época,
  namespace, secuencia y hashes SHA-256; CBOR acotado/canónico, fragmentos de
  1024 bytes, máximo 64 KiB, diez destinatarios y dieciséis saltos.
- `mesh-store`: SQLite cifrado mediante SQLCipher embebido, WAL, sincronización
  FULL y migración inicial transaccional. Rechaza clave incorrecta, base ilegible,
  versión desconocida e identidad local distinta. La clave de 256 bits entra
  desde el host y no se guarda; no existe fallback a SQLite sin cifrar.
- Reserva durable de secuencia/idempotencia antes de preparar el objeto. El
  llamador debe hashear **todos** los argumentos semánticos de su comando.
  Una repetición devuelve la misma reserva; cambiar el hash con el mismo ID falla.
- Objeto, fragmentos, resultado de operación y outbox se confirman juntos.
  Reserva sin commit deja un hueco posible, nunca reutiliza una secuencia para
  otro comando. Una repetición después del commit conserva el resultado original.
- Recepción parcial persistente, reanudación por índices faltantes y rechazo de
  alteraciones/equivocación. Solo el commit del último fragmento puede crear la
  marca única de recepción local; repetir fragmentos no duplica esa marca.
- `mesh-replication`: selector puro de fragmentos faltantes con presupuesto de
  bytes, expiración y límite de saltos suministrado por el llamador.
- `mesh-sim`: nodos sintéticos con bases independientes, reloj/semilla reproducibles,
  particiones, pérdida, duplicación, corrupción y cierre/reapertura de stores.

La confirmación `LocalCommit` prueba persistencia en **esa base local**. No es un
receipt firmado ni demuestra que un destinatario remoto autenticado recibió algo.
Por eso el emisor conserva tanto su copia como su outbox; al expirar deja de
ofrecer el objeto. No se implementa todavía retiro por receipt, garbage collection,
retención por namespaces ni cuotas de tamaño físico de DB/WAL. Las cuotas actuales
reservan bytes de payload, cantidad de objetos y operaciones; el almacenamiento
físico incluye metadatos e índices adicionales. La presión devuelve un error, no
borra mensajes. Valores por defecto: 128 objetos, 2 MiB de payload y 256 operaciones.

El formato `schema/store/manifest-v1.cddl` y su ID pertenecen al almacenamiento
local sintético. **No son A003**, no introducen cambios al wire y no sustituyen
firmas, Noise, HPKE, admisión de grupos, créditos o custodia. El simulador usa
claves públicas de fixture y bytes sintéticos; no debe recibir GPS, voz ni texto
personal. `mesh-store`/`mesh-sim` no son dependencias del runtime/FFI móvil.

## Evidencia local

`cargo test --workspace --features mesh-store/fault-injection --locked`: 25 tests
correctos (incluye el entrypoint auxiliar de crashes). Ocho puntos de muerte real
del proceso cubren salida y recepción, antes/después del commit. También se
comprueban reintento, fragmentos fuera de orden, cuotas, expiración, alteración,
clave incorrecta, cifrado de DB/WAL, esquema desconocido y vector CBOR independiente.
Los fallos de proceso no equivalen a un ensayo de corte de energía del dispositivo.

Escenario `scenarios/f1-recovery.json`, semilla 20260911:

| Medida | Resultado |
| --- | ---: |
| Nodos | 10 |
| Mensajes sintéticos originales | 24 |
| Eventos de simulación | 100000 |
| Contactos bloqueados por partición | 1659 |
| Contactos perdidos | 1225 |
| Cierres/reaperturas de stores | 143 |
| Fragmentos duplicados | 129 |
| Confirmaciones locales perdidas | 85 |
| Fragmentos corruptos rechazados | 85 |
| Fragmentos transferidos | 648 |
| Copias durables en destinatarios | 216 / 216 |
| Copias originales conservadas | 24 / 24 |
| Fragmentos necesarios en recuperación final | 0 |

Los 100000 son eventos de reloj/contacto, **no 100000 mensajes ni el gate completo
de operaciones F1**. La topología actual es emisor → destinatarios; no demuestra
multi-hop ni interoperabilidad iOS/Android. El reporte incluye hashes de fuentes
observadas al ejecutar y no constituye provenance de release.

## Repetir

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features mesh-store/fault-injection --locked -- -D warnings
cargo test --workspace --features mesh-store/fault-injection --locked
python3 tools/test_f1.py
python3 tools/check_contracts.py
```

`tools/test_f1.py` crea bases sintéticas en una carpeta temporal propia bajo
`artifacts`, escribe `artifacts/f1-recovery.json` y elimina únicamente esas bases si la prueba pasa. Ante un fallo conserva la
carpeta de fixtures y el detalle en `artifacts/f1-failure.log`.
El CLI acepta `--scenario <json> --output <carpeta-nueva>` para conservarlas; se
niega a reutilizar una carpeta existente. Escenarios: 2–10 nodos, 1–32 mensajes,
100–1000000 eventos, semilla distinta de cero. La inyección de muerte se activa
solo con la feature `mesh-store/fault-injection`, deshabilitada por defecto.

macOS compila SQLCipher con CommonCrypto/Security. Linux necesita compilador C,
`libssl-dev` y `pkg-config`; la lane contracts instala estos paquetes. Clippy,
formato, tests y escenario se ejecutaron localmente. La configuración CI se amplió,
pero todavía no hay ejecución remota observada. El disco lleno interrumpió varias
compilaciones; las validaciones se repitieron después de liberar intermedios y
componentes Rust de simulador instalados por esta tarea. `build_native.py` vuelve
a instalar esos targets cuando se requiere compilarlos.

## Siguiente bloque

1. Crypto y codec A003 con vectores, firmas/verificación de identidad y payloads
   protegidos; ports tipados para admisión/receipts, ACK/créditos y custodia.
2. Integrar el store al reducer/runtime y el simulador con contratos de recuperación
   completos; completar el gate de operaciones y los límites pendientes de F1.
3. Adaptadores nativos de claves (Keychain/Keystore) y persistencia en ambos OS;
   exponer operaciones durables y estados claros en Flutter y reinstalar para QA.
4. F2: enlace BLE real y pruebas bidireccionales entre los teléfonos, antes de
   activar envío real de GPS, texto y voz.

Referencias de implementación: [API SQLCipher](https://www.zetetic.net/sqlcipher/sqlcipher-api/),
[diseño de SQLCipher](https://www.zetetic.net/sqlcipher/design/),
[rusqlite](https://github.com/rusqlite/rusqlite).
