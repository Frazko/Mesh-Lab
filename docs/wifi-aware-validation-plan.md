# Plan de validación Wi‑Fi Aware sin segundo iPhone

**Estado:** automatización ampliada; validación física iPhone↔iPhone pendiente de
un segundo teléfono compatible.  
**Límite:** ningún simulador, build o test de contrato certifica un canal
Wi‑Fi Aware físico de iPhone a iPhone.

## Objetivo

Mientras sólo exista un iPhone, proteger las propiedades que no dependen de su
radio mediante pruebas deterministas. Cuando llegue el segundo equipo, la
campaña física debe poder concentrarse en radio, permisos y ciclo de vida, sin
descubrir tarde errores de routing, deduplicación o entrega durable.

## Capas de prueba

| Capa | Qué demuestra | Qué no demuestra |
|---|---|---|
| Rust unitario | Límites, TTL, dedupe, presencia, audiencias, orden y rutas deterministas. | Que un dispositivo anuncie o abra un NDP real. |
| Simulador de mesh | Topología de 4–50, partición/reunión, reintentos y presupuesto de saltos. | Potencia, firmware, energía, permisos o APIs de iOS. |
| Contratos host | CBOR, FFI y framing estable entre Rust, Kotlin y Swift. | Descubrimiento y transporte WFA físico. |
| Build de plataforma | Que la app y sus entitlements compilen para cada OS. | Que Apple/Android concedan una conexión cerca de otro teléfono. |
| Física | Enlace, pairing, datos, recuperación y background en equipos concretos. | Generalización automática a otro modelo u OS. |

## Campaña automatizada vigente

| ID | Caso | Evidencia automatizada |
|---|---|---|
| WFA-SIM-01 | Overlay de cada grupo par de 4 a 50 miembros. | 24 configuraciones, dos ejecuciones idénticas de cada una; todos los destinatarios alcanzados. |
| WFA-SIM-02 | Las mismas topologías después de una partición. | 24 configuraciones; las acciones quedan diferidas y convergen tras la reunión. |
| WFA-SIM-03 | Presupuesto de radio. | Máximo dos vecinos WFA y un fallback Bluetooth autenticado; nunca 49 enlaces por teléfono. |
| WFA-SIM-04 | Límite de saltos y bucles. | Ningún mensaje supera 16 saltos; las copias repetidas se descartan. |
| WFA-SIM-05 | Límite de perfil. | Se rechazan grupos impares, fuera de 4–50 y cargas fuera de 1–32 mensajes por ejecución. |
| WFA-CORE-01 | Broadcast de 50. | Cinco audiencias deterministas de 10+10+10+10+9, una acción lógica visible. |
| WFA-CORE-02 | Presencia. | Dos vecinos directos como máximo; una ruta relé puede sustituir a un directo que caduca. |
| WFA-CORE-03 | Durabilidad. | Commit, corrupción, reinicio y receipt no producen una entrega falsa ni duplicada. |
| WFA-HOST-01 | Fronteras nativas. | Kotlin y Swift verifican vectores FFI, CBOR, ruta adjunta y fragmentación BLE. |
| WFA-FACADE-01 | Envío WFA sin Bluetooth autenticado. | La fachada Flutter acepta texto, ubicación con rumbo y voz cuando el único enlace seguro es Wi‑Fi Aware; evita una regresión que exija BLE. |

Los casos `WFA-SIM-01`, `WFA-SIM-02` y `WFA-SIM-05` viven en
`crates/mesh-sim/src/lib.rs`. Cada caso ejecuta varias topologías internamente;
el número de funciones de prueba no debe confundirse con el número de
configuraciones ejercitadas.

## Pruebas que se agregan antes del segundo iPhone

1. **NAT-WFA-01:** extraer la selección de vecinos, reintento y cambio de
   disponibilidad de `WifiAwareAccess` a un controlador puro probado en Kotlin.
2. **IOS-WFA-01:** extraer framing, límite de conexión única, backoff y limpieza
   de enlaces a tipos Swift sin `Network.framework`, cubiertos por XCTest.
3. **XPLAT-WFA-01:** vectors compartidos de frame Noise y records durables que
   Kotlin y Swift decodifican en ambos sentidos.
4. **REC-WFA-01:** simulación de corte a mitad de texto, GPS y voz que exige
   mismo ID lógico, una sola proyección y reintento durable.
5. **HYB-WFA-01:** antes de Convoy, pruebas de que un ACK remoto y un receipt
   mesh para la misma acción no crean dos burbujas ni dos posiciones.

Estas cinco entradas no se marcarán como implementadas hasta que el código y
sus reportes existan. La política de cobertura de 90% para código nuevo Rust y
Dart sigue aplicando; Kotlin y Swift entrarán al mismo gate cuando produzcan
cobertura instrumentada.

## Campaña física pendiente: iPhone↔iPhone

Con dos iPhone compatibles, Wi‑Fi activado y la build firmada con entitlement:

1. **WFA-IOS-01 — pairing:** crear grupo, tocar un solo botón de conexión y
   aprobar el selector de iOS una vez.
2. **WFA-IOS-02 — descubrimiento:** ambos teléfonos se ven sin LAN, hotspot ni
   Bluetooth; registrar el tiempo hasta enlace.
3. **WFA-IOS-03 — datos:** texto, ubicación+rumbo y voz en ambos sentidos.
4. **WFA-IOS-04 — dedupe:** enviar la misma acción dos veces por reintento y
   confirmar una sola burbuja/posición.
5. **WFA-IOS-05 — corte:** apagar Wi‑Fi en un lado, comprobar estado honesto y
   cola durable; restaurar y comprobar reconexión.
6. **WFA-IOS-06 — lifecycle:** bloqueo, background, retorno y relaunch.
7. **WFA-IOS-07 — presión:** mensajes, GPS y voz consecutivos sin starvation.
8. **WFA-IOS-08 — evidencia:** guardar modelo, OS, build, permisos, tiempos,
   logs redaccionados y resultado de cada dirección.

Sólo WFA-IOS-01 a WFA-IOS-08 cierran NET-01 y NET-03 de
[`known-gaps.md`](known-gaps.md). Los resultados deben registrarse por modelo y
versión de iOS: una prueba exitosa en un par no certifica todos los iPhone.
