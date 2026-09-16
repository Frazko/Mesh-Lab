# Ejecutor durable del host

**Estado: implementación parcial.** Texto, nota de voz y posición puntual ya
entran al relay durable compartido por Mesh Lab y el futuro SDK. El documento
delimita lo completado y las decisiones que aún no deben declararse listas.

## Unidades y fronteras

Cada enlace Bluetooth o Wi‑Fi Aware entrega una trama Noise autenticada. El
host solo acepta un `RoutedRecord` después de esa autenticación. No se acepta
ningún socket, dirección IP, conexión Wi‑Fi local ni payload de Flutter como
sustituto de ese requisito.

`RoutedRecord` contiene exactamente un `RelayFrame` y uno de estos registros:

- `Announcement`: se valida contra el roster y se persiste antes de aceptar
  chunks.
- `Chunk`: se verifica contra el manifest ya autenticado y se escribe en
  SQLCipher. Cuando completa el objeto, su `RelayFrame` y la fila de custody
  deben confirmarse en la misma transacción.
- `Receipt`: se verifica contra el objeto y roster antes de cambiar el progreso
  del outbox del originador.
- `ReceiptAck`: el origen certificado firma el digest de un receipt ya
  registrado; el destinatario detiene sólo ese retry y un relay puede retirar
  la custodia de ese receipt.

Una cola de salida nunca es una promesa de entrega. Los únicos estados visibles
son: `queued`, `custodied`, `delivered`, `expired` y `no_route`. El host no
marcará `delivered` hasta registrar receipts verificables de la audiencia del
objeto.

## Orden en un nodo relay

1. Recibir y autenticar Noise desde un vecino.
2. Decodificar el `RoutedRecord` canónico y rechazar tamaños/versiones inválidos.
3. Al recibir el anuncio, consultar `RelayCache` una sola vez por `RelayId`.
   Un duplicado no se reenvía.
4. Persistir anuncio y chunks autenticados en SQLCipher.
5. Solo tras el commit del último chunk, leer `relay_queue`.
6. Enviar el objeto persistido, con el frame actualizado del relay, a vecinos
   autenticados salvo `received_from`. El frame usa `previous_hop` para nombrar
   este relay hacia el siguiente salto; por eso no puede sustituir al vecino
   real que entregó el objeto.
7. Mantener la fila tras cortes de radio o reinicio; no reenviar desde buffers
   de memoria.
8. Al recibir un receipt válido, actualizar el outbox de origen y retirar el
   objeto solo cuando la audiencia requerida confirme o expire.

Un receipt recibido por un relay que no es el origen entra también en una cola
SQLCipher propia. Conserva el frame transformado y `received_from`; el host lo
reintenta hacia los demás vecinos tras cada recuperación. No se borra por una
escritura de radio: sólo un `ReceiptAck` válido del origen, vinculado al digest
del receipt, puede retirarlo. El ACK entra a su propia cola SQLCipher antes de
reenviarse; por eso una caída del relay no obliga al destinatario a reintentar
hasta el vencimiento. Tras cada enlace autenticado, el host drena las tres
colas: objetos, receipts y ACKs.

La ejecución por objetos evita que un relay tenga que mantener fragmentos de
voz o texto en RAM mientras la radio se reinicia.

## Contratos FFI vigentes y pendientes

El siguiente bloque agrega estas operaciones nativas, sin exponer claves,
filas SQLCipher ni handles a Flutter:

- `enqueueGroupObject`: **implementado para payloads acotados.** El host entrega
  texto, posición puntual o audio AAC/M4A al FFI; éste reserva la operación,
  sella cada audiencia y la persiste en SQLCipher antes de que se consulte un
  `RoutedRecord` del outbox. Voz no conserva un fallback RAM-a-radio.
- `acceptRoutedRecord`: **implementado en C/JNI/Swift.** Ingresa anuncio
  firmado y chunks después de Noise, valida roster y origen, y llama la
  custodia SQLCipher con el `received_from` autenticado. La prueba FFI crea un
  grupo, incorpora un miembro y confirma que un chunk sin anuncio autenticado
  se rechaza.
- `drainRelayQueue`: **ejecutor Android implementado para enlaces BLE/WFA
  autenticados.** Recupera por slot objetos custodiados tras restart, omite el
  vecino de entrada y conserva la fila hasta receipt/expiración. iOS reenvía
  entre sus enlaces BLE; su salida relay WFA sigue pendiente porque el
  adaptador no elige todavía un vecino WFA individual ni puede excluir el
  enlace de entrada con precisión.

La selección física queda en los adaptadores de radio, pero
`mesh_replication::select_transport` ahora impone una política común por
objeto: recibe únicamente enlaces ya autenticados, descarta el vecino de
entrada, ordena por ETA y elige Wi‑Fi Aware en empate. Cuando existe un enlace
actual sano, sólo emite `Switch` si el nuevo ya está sano; el host puede hacer
*make-before-break*. Android e iOS todavía deben consumir esta decisión en sus
ejecutores, por lo que no cierra la prueba física de failover.

## Pruebas obligatorias del bloque

1. **Unitarias:** cada estado y rechazo de `acceptRoutedRecord`, incluidos
   anuncio ausente, chunk alterado, receipt ajeno y TTL agotado.
2. **Integración Rust/FFI:** **cubierta para voz A→B→C.** Objeto sellado →
   SQLCipher en B → reinicio de B → `drainRelayQueue` → commit en C → receipts
   B/C → retiro del outbox A → ACK firmado de A que detiene el retry de C.
   Ningún handle o clave cruza FFI. Falta la misma prueba sobre enlaces
   Noise/radios físicos en la misma topología y una campaña con cortes reales.
3. **Host Android/iOS:** el adaptador no reenvía antes del commit y omite el
   `previous_hop`; los radios BLE/WFA usan el mismo ejecutor.
4. **Física:** A73 → S24+ → iPhone sin A73↔iPhone, apagar/reabrir S24+, enviar
   texto, GPS y voz; cada destino aparece una vez y el estado termina en
   `delivered` o explica `no_route`.
