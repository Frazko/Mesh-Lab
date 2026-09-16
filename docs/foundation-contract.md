# Contrato F0 v1

Fuente ejecutable: `schema/api/foundation.cddl`, registro `schema/api/registry.json`,
Pigeon `platforms/mesh_host/pigeons/mesh_api.dart`. Referencias: A005, A008, A009.
La instrucción del usuario del 2026-09-11 autoriza iniciar esta implementación.
No cambia ninguna decisión A001–A010 de transportes, criptografía o durabilidad.

## Semántica

`engineInfo()` devuelve versión de motor, ABI, API, fase y build ID.
ABI/API v1 son estrictas: otra versión se rechaza antes de crear el runtime o
interpretar respuestas. N-1 no existe en v1; se deberá definir antes de v2.

`subscribe(cursor)` devuelve snapshot completo y cursor atómicos. Incluye eventos
posteriores al cursor que aún estén disponibles. Un cursor futuro o anterior a la
ventana devuelve `cursorReset=true`, snapshot actual y lista de eventos vacía.
El consumidor reemplaza su proyección y descarta el historial anterior.

`verifyBridge(requestId)` es una mutación **diagnóstica volátil**: incrementa el
contador y produce `bridgeVerified`. Nunca inicia sesión ni demuestra radio.
IDs positivos, crecientes por runtime; los últimos 64 se deduplican. Repetir un
ID retenido devuelve el estado sin incrementar el contador. Un ID antiguo fuera
de la ventana falla con `STALE_REQUEST`. Esto no es idempotencia durable de F1.
Dart conserva el mismo ID tras timeout, evitando un falso doble éxito.

El identificador diagnóstico visible es `F0-{runtimeId}-{eventSequence}` y solo
distingue eventos dentro del proceso. No identifica una instalación o persona.

## Límites y ownership

- CBOR definido, codificación mínima, sin bytes sobrantes ni anidamiento arbitrario.
- Entrada ≤64 bytes; salida ≤16384; strings ≤128; historial ≤64 eventos.
- Hasta 16 runtimes por proceso y contadores ≤2^53−1, sin wrap.
- Handles monotónicos no reutilizados: release doble es idempotente; handle antiguo falla.
- Inputs C prestados durante la llamada. Cada output Rust se libera exactamente una vez,
  incluso si falla la decodificación. No mezclar allocators ni modificar punteros.
- La API C requiere punteros válidos/alineados del llamador; no puede verificar memoria arbitraria.
- Panics Rust se contienen en la frontera; OOM sigue siendo fallo del proceso.
- Hosts serializan trabajo y mantienen una sola instancia al recrear Dart.

## Estado y errores

Solo existe `foundationOnly=0`. No equivale a READY de store, grupo ACTIVE ni
Bluetooth disponible. No se ha abierto un store, creado una identidad o solicitado
un permiso. La UI recupera estado al volver a foreground y no considera fresca
una respuesta que llegó mientras estaba inactiva.

Errores: invalidArgument, incompatibleVersion, invalidHandle, resourcePressure,
internalInvariant y staleRequest. El host agrega errores de empaquetado/CBOR;
Dart agrega timeout de presentación. No se exponen stacks, payloads ni secretos.

## Qué queda fuera

Wire A003, transport ports físicos, generación de claves, objetos durables,
store cifrado, crypto vectors, audio y reintentos de red son F1+.
Las cuentas de testing globales no aplican: este slice no tiene auth/usuarios,
fixtures de cuentas ni bases de datos.
