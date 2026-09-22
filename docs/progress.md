# Avance del plan

Estado vigente: 2026-09-22. **81% global estimado · 90% Wi‑Fi Aware · 78% comunicación.**

La tabla inicial conserva la línea base del plan; las secciones fechadas posteriores y el [registro de huecos](known-gaps.md) describen el estado vigente y la evidencia pendiente.

## Enlace BLE de Convoy y apertura del mapa

**Actualizado: 2026-09-21. Plan completo: 80%; bloque físico y UI: 96%.**

La campaña Android↔iPhone encontró y corrigió una colisión real del protocolo:
la incorporación pública se reconocía por un solo byte y un registro Noise
opaco de 32 bytes comenzó casualmente por `0xf3`. Android lo clasificó como una
política y dejó la sesión en “Conectando”. Android e iOS ahora anteponen el
mismo dominio versionado de 128 bits a los cuatro registros de incorporación;
un paquete Noise ya no puede entrar por esa ruta por coincidencia de un byte.
La nueva prueba Kotlin reproduce el registro de 39 bytes observado y exige que
no sea inscripción, además de cubrir round-trip, corrupción y límites.

Se compilaron e instalaron builds Release nuevos en el SM A736B y el iPhone. La
evidencia física registró `Noise step 1`, `Noise step 2` y
`Secure link authenticated`; Convoy mostró **Malla: En red** con un vecino BLE.
La admisión de Convoy refresca el roster mientras hay servidor y el enlace sigue
pendiente, de modo que el líder incorpora el binding del segundo teléfono sin
reiniciar la sesión. La cápsula de Malla quedó centrada debajo del encabezado.

El mapa abre con Centroamérica (`13.2, -85.7`, zoom `5.2`) y consume solamente
la primera fijación local válida para animar al marcador propio. La prueba
física sin señal GPS conservó correctamente la vista regional; las pruebas
unitarias demuestran que una fijación nula o `(0,0)` no consume el latch y que
las fijaciones posteriores no vuelven a mover el mapa. Aprobaron 39 pruebas
cercanas de Convoy, la unidad nativa del protocolo y ambos builds Release. Falta
validar visualmente la animación con señal GPS disponible y ampliar la campaña
a corte/reconexión y multihop; por eso el bloque no se declara cerrado.

## Puente seguro Internet–Malla

**Actualizado: 2026-09-21. Implementación local: 60%; gate físico y de servidor: pendiente.**
El núcleo añade un dominio de firma exclusivo para relevo a nube. Un host sólo
puede producir una prueba si su política Field vigente lo certifica como
miembro: devuelve el identificador público de grupo de 32 bytes, la época y 64
bytes de firma sobre el sobre canónico. No expone la semilla de identidad, las
llaves de entrega ni el roster. El grupo se incluye porque un servidor no puede
verificar de forma segura una firma sin saber qué política Field la emitió. Las
pruebas Rust verifican que la prueba falla sin política o con otra identidad,
que el grupo y época corresponden a la política activa y que la firma deja de
ser válida si cambia el contenido; además queda separada de certificados,
objetos, recibos y handoffs. Pigeon, JNI, Swift y la fachada
`FieldMeshCloudRelaySigner` lo trasladan como capacidad opcional. La suite del
SDK tiene 34 pruebas y valida límites, grupo público y ausencia de llaves en
Flutter.

La identidad que se entrega al producto ahora es la clave pública Ed25519
codificada en hexadecimal, exactamente la identidad certificada por la radio.
Antes se entregaba un hash de presentación: no era secreto pero tampoco podía
ser comparado por el host ni verificado por un servidor. Al volver a entrar a
un convoy con cobertura, Convoy sustituye idempotentemente el binding antiguo
por esta clave pública real. Convoy ya adjunta el sobre a texto y ubicación antes de sellarlos por Field;
la prueba funcional atraviesa adaptador, codec y host simulado con un texto de
500 caracteres y firma completa. El transporte interno admite 2 KiB y mide el
sobre sellado, mientras Convoy limita su sobre interno a 1.3 KiB para que el
texto visible de 500 caracteres siga cabiendo. Falta registrar en backend el
grupo Field público por convoy, una función de servidor que verifique autor,
binding, participación, grupo, época e idempotencia, y una cola durable que
permita a un vecino con Internet publicar la acción de un miembro sin cobertura.

## C3 — admisión automática gobernada por Convoy

**Actualizado: 2026-09-20. C3 local: 84%; gate operativo: pendiente.** La
autoridad de producto aprobada es el **líder actual del convoy**: el creador
comienza como líder y crea la autoridad inicial, pero esa propiedad no queda
atada de forma permanente a su teléfono. Antes de que el host emita una
política de incorporación, el núcleo Rust ahora valida la solicitud pública y
extrae únicamente la huella certificada del solicitante. Android e iOS comparan
esa huella con un roster explícito que el adaptador de producto instala; una
solicitud que no figura en el roster se rechaza antes de tocar la llave privada
de la autoridad. El nuevo control opcional `FieldMeshEnrollmentAccessController`
mantiene al SDK reutilizable: un producto aporta sólo huellas públicas
autorizadas y puede consultar únicamente un bit de capacidad: si la identidad
protegida local coincide con la autoridad certificada de la época activa. Mesh
Lab conserva su modo experimental de incorporación abierta. Un exlíder queda
bloqueado por el host aunque conserve la llave local de una época anterior.

Las pruebas unitarias validan que la huella procede de una solicitud firmada,
que una alterada, sin instante válido o sobredimensionada falla, que la FFI limpia su salida
antes de devolver un error y que sólo la semilla de la autoridad activa recibe capacidad de
incorporación. El puente JNI compila con el motor. La fachada SDK
suma 30 pruebas y análisis limpio: acepta sólo 50 huellas minúsculas, rechaza
hosts antiguos que no pueden imponer la política y conserva una limpieza
compatible. En Convoy, 116 pruebas cercanas cubren el flujo integrado en
memoria roster→binding→adaptador→SDK→host, sólo una creación de grupo por
líder, miembro sin autoridad, salida, el líder promovido sin autoridad de la época y los fallos de cada frontera. Pigeon se
regeneró para Dart/Kotlin/Swift y el host pasó análisis Dart. El framework Apple
se recompiló y el build iOS de dispositivo pasó, junto con el host Swift. El
host Android actualizado también compiló; falta ejecutar la campaña física. El
núcleo Rust ahora también codifica un `AuthorityHandoff` canónico y acotado:
la autoridad anterior firma su scope, digest de roster, sucesor, época inmediata
y vencimiento. SQLCipher acepta el cambio sólo si la firma coincide con la
política activa y el sucesor instala exactamente la época siguiente del mismo
grupo; una clave nueva sin ese handoff se rechaza. Las pruebas cubren la firma,
el digest, expiración, manipulación y el rechazo del salto sin autorización.
La frontera C/JNI y ambos hosts nativos preparan el handoff, rotan desde el
almacén local del sucesor y permiten a miembros existentes instalar la nueva
política solamente con su autorización adjunta. La prueba funcional de FFI abre
dos bases SQLCipher, incorpora al sucesor, rechaza una clave no miembro y un
almacén distinto, rota por las funciones C y hace converger al líder anterior.
Pigeon y `mesh_field_sdk` ya exponen las tres operaciones con límites de bytes,
IDs hex y vencimiento; sus 31 pruebas cubren delegación y host legado. Android
e iOS refrescan WFA al cambiar la época: iOS retira las sesiones Noise de la
política previa y rediscovery usa el pairing ya existente. Compilan Rust/JNI,
Kotlin y Swift. Aún falta que Convoy persista el relevo, autorice al líder,
distribuya el bundle con ACKs y pruebe el flujo físico. Por eso la admisión de
un líder promovido sigue bloqueada de forma segura y C3 no se cuenta como
cerrada hasta que esa migración recorra producto, backend y prueba física.

### SEC-05 — separación de grupos de producto

**Actualizado: 2026-09-20. Implementación local: 84%; gate operativo: pendiente.**
Cada `convoyId` UUID se normaliza a un scope de 128 bits, privado del host y
nunca anunciado por Bluetooth ni Wi-Fi Aware. Al configurar el roster, el SDK
lo cruza con ese scope y Android/iOS abren exclusivamente
`mesh-store/<scope>/state-v1.db`, una base SQLCipher distinta por convoy. Al
pasar de A a B, Convoy detiene la sesión cercana y limpia la política A antes
de solicitar B; los hosts cierran además enlaces Bluetooth y sockets directos
Aware antes de liberar el almacén cifrado. El almacenamiento A permanece
cifrado para auditoría y una futura decisión de reingreso, pero no puede ser
el grupo radio activo de B. La prueba de integración Convoy A→B valida la
secuencia salida→limpieza→nuevo grupo y que ambos scopes generan grupos
independientes; las 29 pruebas del SDK validan propagación/forma del scope y
su rechazo si no es hexadecimal de 32 caracteres. Android e iOS ya compilaron con el host de scope; falta una campaña física que cambie
de convoy con Bluetooth y Aware conectados; hasta entonces SEC-05 no se marca como cierre operativo.

## Entrega agregada por conversación

### C2 — eventos entrantes certificados para adaptadores de producto

**Actualizado: 2026-09-16. C2 local: 86%; gate operativo: pendiente.** El completion packet Rust cambió a v2 y contiene el ID de objeto, el **origen firmado** del manifest —no el receptor que firmó el receipt— y el instante de verificación. Sólo se serializa tras validar roster, firma, todos los chunks y confirmar el receipt local. Android e iOS validan el encuadre y guardan texto en una FIFO de hasta 64 entradas; Pigeon la drena como `VerifiedIncomingText`, de modo que una ráfaga no se pierda detrás de `lastMessage`. `mesh_field_sdk` 0.2.2 expone la capacidad opcional `FieldMeshVerifiedIncomingSource`; vuelve a comprobar hexadecimal, hora y cuerpo antes de entregar cada evento. Rust aprobó 28 pruebas de `mesh-ffi-c` y `mesh-protocol`; el SDK aprobó 22 y análisis limpio; el laboratorio aprobó 26 y análisis limpio. El SDK también sella su ID lógico en el payload durable cifrado y lo valida al recibir, por lo que texto/ubicación pueden correlacionarse sin confiar en metadatos de radio. En la rama de Convoy, el adaptador transforma la cola en `NearbyMeshIncomingAction`, recupera el UUID v4 exacto y rechaza SDKs sin la capacidad certificada. Antes de exponer nada a la app, `CertifiedActionReducer` mantiene una ventana acotada de 512 acciones: deduplica reintentos por ID de objeto o UUID, y rechaza una colisión de origen o cuerpo sin sustituir la primera acción. Las 43 pruebas del módulo Convoy validan esa proyección, los casos de replay/conflicto, el sobre de alcance y el despacho con el UUID exacto hacia la SDK; el archivo nuevo del reducer alcanzó 100% de cobertura de líneas en esa campaña. Un proyector sólo acepta el sobre si el convoy activo, el origen certificado y un vínculo de miembro confiable coinciden; el outbox ya refleja el primer texto en una sesión mesh autenticada sin bloquear Internet. El comprobante agregado de esa acción ahora se persiste en Hive bajo el mismo UUID y se consulta de nuevo cada 20 segundos hasta entrega o vencimiento, sin sustituir el ACK Internet como criterio durable. Las 22 pruebas del bloque cercano cubren la conversión del receipt y su recuperación tras reabrir Hive. El controlador de ubicación de Convoy también refleja fijaciones GPS válidas sobre el mesh con latitud, longitud, precisión e instante UTC; la vía se coalescea a una acción cada cinco segundos y conserva el broadcast Internet independiente. Convoy no serializa rumbo ni inicia la brújula: sus marcadores son fotos o iniciales de participantes y permanecen verticales. Sus payloads, UUID y cadencia están cubiertos por la suite cercana, ahora de 59 pruebas. El receptor de Convoy ya transforma texto certificado en su historial y ubicación certificada en el participante inscrito; evita duplicados de historial, requiere que el miembro aún esté activo y conserva la hora original de la fijación para no presentar un relay tardío como GPS actual. Conserva hasta 64 acciones certificadas mientras el roster local carga y las descarta al resolver un miembro ausente. Convoy ya define una inscripción persistente por convoy: la migración pendiente `20260916120000_convoy_mesh_member_bindings.sql` permite leer el roster sólo a participantes activos, elimina escrituras directas del cliente y expone un RPC que fija el usuario desde `auth.uid()`, valida membresía activa y rechaza una huella ya inscrita por otra persona. El servicio de inscripción prepara únicamente la huella pública de Field y el carril que abre la sesión de Convoy lo invoca sólo después de que el roster protegido confirme que falta el usuario actual; al confirmarse, refresca el roster para que el receptor comience a aceptar contenido sólo de ese origen certificado. El contrato HTTP/RPC, huellas inválidas, inscripción sin user ID provisto, inscripción tras roster, recuperación del carril entrante, continuidad limitada del roster y autorización de salida suman **79 pruebas cercanas**. Convoy ahora conserva sólo un roster que el backend autorizó, separado por usuario y convoy; exige que el usuario local figure en él, borra datos corruptos o vencidos y caduca a las seis horas. Ante un fallo del backend puede usar esa copia, pero no incorpora miembros nuevos y falla cerrado si no hay una copia válida. Una lectura posterior del servidor que ya no incluye al usuario la elimina. Sin Internet no puede observar una revocación nueva; el vencimiento breve acota ese riesgo y la primera lectura disponible vuelve a validar el roster. Antes de reflejar texto o GPS al mesh, Convoy exige que usuario autenticado, huella pública local y miembro del roster sean el mismo vínculo; si no lo son, suprime sólo mesh y conserva la vía Internet. Las nuevas unidades de caché y proveedor miden **96.0%** y **92.5%** de cobertura de líneas. La migración no está aplicada ni probada con dos cuentas; también faltan pruebas físicas Android/iOS y extender la cola certificada a voz. El gate no cambia; el avance local permite sostener el global en 64%.

## Guardrail WFA de la fachada SDK

**Actualizado: 2026-09-15.** La prueba `WFA-FACADE-01` modela un teléfono con Wi‑Fi Aware autenticado y Bluetooth no autenticado. Comprueba que `mesh_field_sdk` permite texto, ubicación puntual y voz sobre ese único enlace seguro. `flutter test` aprobó 17 pruebas y la cobertura de `field_mesh_client.dart` permanece en **93.0%** (185/199 líneas); `flutter analyze` y las 9 pruebas de `mesh-sim` también aprobaron. Esta evidencia protege el contrato Flutter contra una regresión de selección de radio, pero no sustituye la prueba física WFA iPhone↔iPhone ni cierra S2.

## S3 — recibos durables para voz

**Actualizado: 2026-09-15. S3 local: 55%; gate operativo: pendiente.** La voz ya usa el mismo ID lógico de 128 bits y el mismo outbox durable que texto y ubicación: Pigeon transporta el ID; Android y iOS lo validan antes de entregarlo a `secureStoreEnqueueTextWithLogicalId`; y `mesh_field_sdk` 0.2.0 devuelve `FieldDelivery` en vez de un booleano. Mesh Lab muestra `En cola segura`, `Entregado a n de m` o vencimiento para la nota enviada. Las pruebas de fachada cubren voz en cola, parcial y entregada; el controlador cubre esas transiciones hasta el texto visible. Quedan abiertos el reinicio físico durante una transferencia de voz, la recepción de receipts entre teléfonos y la medición bajo carga; por ello S3 no está cerrado ni modifica el 60% global.

## S4 — selección de transporte por objeto

**Actualizado: 2026-09-15. S4 local: 30%; gate operativo: pendiente.** `mesh_replication::select_transport` agrega una decisión pura y determinista para cada intento durable: toma enlaces que el adaptador ya autenticó, descarta el salto de entrada, filtra enlaces sin salud, ordena por ETA, prefiere Wi‑Fi Aware en empate y devuelve `NoRoute`, `Send`, `Keep` o `Switch`. `Switch` sólo es posible si el enlace anterior y el siguiente permanecen sanos, de modo que los hosts puedan hacer *make-before-break*; si el anterior ya cayó, el resultado honesto es `Send`. Tres pruebas cubren exclusión de salto, prioridad, cambio seguro, caída, duplicados y datos inválidos; junto con ellas aprobaron 17 pruebas de `mesh-replication` y las 9 del simulador. Android/iOS todavía no consumen el selector y faltan corte/reinicio físicos, por eso S4 no se considera cerrado.

## Integración Convoy — C1: sesión cercana

**Actualizado: 2026-09-15.** La rama de Convoy `codex/mesh-field-sdk-integration` contiene la primera entrega de integración: `NearbyMeshRepository` adapta la API pública de `mesh_field_sdk` a una proyección segura para Convoy; el controlador Riverpod observa el estado y el encabezado incorpora el control con semáforo. Convoy no importa `mesh_host`, no decide radios y no recibe claves, IP, rutas ni identidades de vecinos.

La entrega se validó con 15 pruebas de repositorio, controlador y widget, más `flutter analyze` limpio. Se registró como commit `87d7c39` en Convoy. Esta es sólo la integración de sesión: chat, GPS y voz permanecen fuera del flujo funcional hasta que el SDK exponga inscripción firmada e identidad/ID/hora verificables de los eventos entrantes, y se cierren S2–S6. No modifica los porcentajes del cierre operativo.


### Estrategia híbrida Internet + Mesh

**Actualizado: 2026-09-14.** El modelo híbrido quedó incorporado en el plan
maestro HTML, sección “Internet + mesh”, y conserva su especificación completa
en [`hybrid-internet-mesh-plan.md`](hybrid-internet-mesh-plan.md). Define una
acción lógica con ID estable, outbox durable, selección por ACK real, dedupe,
semántica diferente para chat/voz/ubicación puntual, confirmations de alcance y
los paquetes HYB-01 a HYB-05. Es diseño documentado: no está implementado y no
altera los porcentajes.

### Guardrails automatizados para Wi‑Fi Aware pendiente en iPhone

**Actualizado: 2026-09-14.** Se añadieron tres pruebas de simulación que
ejecutan **48 topologías**: cada roster par de 4 a 50 se prueba con entrega
normal y tras una partición, además de rechazar tamaños y cargas fuera del
perfil. Las pruebas verifican grado WFA máximo de dos, fallback Bluetooth
acotado, límite de 16 saltos, deduplicación, convergencia y determinismo. Son
protección de overlay, no evidencia de radio iPhone↔iPhone. La matriz completa
y los ocho pasos físicos pendientes están en
[`wifi-aware-validation-plan.md`](wifi-aware-validation-plan.md). Esto mantiene
NET-01 y NET-03 abiertos y no cambia los porcentajes.

### SDK: contrato de producto cerrado antes de Convoy

**Actualizado: 2026-09-14. Fachada SDK: 100% · cierre operativo SDK: 0% planificado · global: 60% · Wi-Fi Aware: 90%.**
La fachada del SDK queda cerrada como contrato reutilizable y validado localmente. El cierre operativo del SDK se planificó antes de continuar la integración funcional con Convoy. Expone identidad, grupo,
estado de radios, conexión, texto durable, evidencia de entrega, voz,
ubicación puntual y eventos entrantes tipados. `FieldLocation` conserva
coordenadas, precisión, instante UTC y orientación opcional genérica para productos que la necesiten. Convoy usa
solamente posición, precisión e instante UTC en sus marcadores de participantes.

La ubicación viaja como una única acción durable cifrada y se valida tanto al
enviar como al decodificar; no activa seguimiento continuo ni cambia la
política de background. `watchIncoming` proyecta solamente texto y ubicación
nuevos y no filtra rutas, IP, claves, objetos ni receipts. La suite del paquete
cuenta con 15 pruebas y **93.0%** de cobertura en `field_mesh_client.dart`,
100% en sus modelos, además de `flutter analyze` limpio. Se comprobó también
que la fachada ampliada sigue compilando con las pruebas y el análisis de
Convoy. Las campañas físicas WFA, multihop y de background son gates del cierre
operativo del SDK, no de la fachada; el plan completo está en
[`sdk-closure-and-product-integration-plan.md`](sdk-closure-and-product-integration-plan.md).

### Cobertura de código nuevo protegida en CI

**Actualizado: 2026-09-14.** Cada pull request ahora falla si las líneas
ejecutables nuevas de Rust o Flutter de producto bajan de **90%** de cobertura.
El verificador compara el diff real de Git contra ambos LCOV y también rechaza
un archivo de producción modificado que no haya emitido ninguna línea de
cobertura. La política está en
[`testing-coverage.md`](testing-coverage.md); Kotlin y Swift quedan fuera de
este gate hasta que sus reportes instrumentados estén disponibles.

La línea base medida localmente es Rust **75.2%** (7,544/10,038) y Flutter
**69.6%** (663/952). Es una tendencia transparente, no un incumplimiento
retroactivo: la regla de 90% protege cada incremento nuevo. La comprobación
end-to-end del gate crea un repositorio temporal, genera un diff real y prueba
las dos salidas: aceptación al 100% y rechazo sin datos LCOV.

### SDK: entrega durable para productos

**Actualizado: 2026-09-14.** `mesh_field_sdk` se alineó con el contrato durable
del host. `sendText` genera un ID lógico criptográficamente aleatorio, acepta
la acción solo después de que el host la guarda y devuelve `FieldDelivery`.
El producto puede actualizar ese ID mediante `delivery(id)` y obtiene
únicamente `queued`, `partial`, `delivered` o `expired` con sus contadores
agregados; nunca recibe IDs de objetos, recibos, destinatarios, claves o rutas.

El SDK ejecuta sus propias pruebas de análisis y cobertura en CI, junto con
Mesh Lab. Las pruebas cubren envío seguro, rechazo sin enlace, evidencia
corrupta o ajena, cola sin radio, estados de sesión, observación y el mapeo
Pigeon sin filtrar DTOs nativos. GPS tipado sigue fuera de esta fachada hasta
que exista su política durable de campo.

**Actualizado: 2026-09-14.** Cada acción visible de texto, voz o GPS ya crea
un identificador lógico local y persiste su relación con cada objeto de
audiencia cifrada en SQLCipher. Una conversación de 50 miembros puede producir
cinco objetos (10+10+10+10+9), pero el origen obtiene una sola evidencia:
`queued`, `partially delivered` o `delivered`. El último estado exige que las
cinco audiencias se hayan comprometido y que existan los 49 receipts firmados;
un solo receipt nunca promociona la burbuja a entregada.

La migración `010-logical-delivery.sql` conserva esta relación a través de
reinicios. La prueba funcional A→B→C cubre el paso de pendiente a parcial tras
el receipt de B y a entregado solo después del receipt de C, incluido un reinicio
del relay; la prueba de roster de 50 comprueba cinco audiencias, 49 destinos y
una sola acción lógica pendiente. El motor expone este resumen al host por el ID lógico exacto, sin filtrar IDs
de objetos ni receipts hacia Flutter. Flutter conserva los IDs pendientes y
actualiza únicamente su burbuja al recibir evidencia del motor.

Preferencia del usuario: incluir porcentaje del plan completo en cada actualización
y entrega; mostrar también el porcentaje del bloque activo cuando corresponda.
El avance se actualiza por entregables implementados y evidencia, nunca por tiempo
transcurrido ni cantidad de líneas. Por ahora usamos una referencia simple: cada
fase F0–F8 pesa 1/9. Es avance técnico aproximado, no porcentaje de horas ni
certificación de todos los gates. Evitar contar preparativos de una fase dos veces.

## Meta de producto — SDK de comunicación cercana

**Actualizado: 2026-09-13. Avance global: 36%; Wi‑Fi Aware: 88%.**

La meta aprobada es convertir el trabajo actual en un SDK reutilizable para
teléfonos cercanos sin internet. Mesh Lab se conserva como laboratorio,
simulador y prueba física de radios. El SDK separará el núcleo Rust, bridges
nativos iOS/Android y una fachada Flutter de las interfaces de cada producto.

Convoy será la primera integración: recibirá miembros, conexión, posiciones,
texto y voz como eventos tipados, y enviará acciones de sesión por la misma
fachada. Convoy no contendrá lógica de Bluetooth, Wi‑Fi Aware, IP, Noise,
topología o saltos. La siguiente integración de cualquier otra app reutilizará
el mismo SDK y solo aportará su adaptador de interfaz.

La primera base está en `packages/mesh_field_sdk`: `FieldMeshClient` usa el
bridge existente a través de `MeshHostGateway`, y expone identidad, grupo,
estado de radios, sesión, texto y voz con DTOs inmutables de producto. Su
gateway puede sustituirse por un fake en pruebas; Convoy no necesitará importar
`mesh_host`. GPS tipado, relay durable, receipts y presencia global se añaden
solo cuando estén conectados de extremo a extremo, sin métodos simulados.

| Fase | Avance estimado | Evidencia / pendiente |
|---|---:|---|
| F0 Fundación | 100% local | Puente, UI base y pruebas previas en ambos teléfonos. CI preparada; ejecución remota no observada. |
| F1 Core y simulador | 70% | Store, crypto, perfil autenticado, receipts, ACK firmado de receipt, reducers, Noise XX, commit durable, política durable, creación local de grupo, inscripción QR firmada y apertura nativa del store SQLCipher. Faltan IK, wire completo, credits e inventarios/custodia y gate completo. |
| F2 BLE | 18% | Permisos explícitos, estado real del adaptador, descubrimiento BLE, superficie GATT común y sonda física de un byte. Falta validarla en teléfonos, además de tramas autenticadas y prueba de una hora. |
| F3 Multihop | 0% | Validar A↔B↔C sin contacto A↔C. |
| F4 Wi-Fi y failover | 0% | Aware, arbitraje y continuidad. |
| F5 App completa | 0% | Grupos/QR, GPS/texto/voz reales, PTT y controles finales. UI de preparación contabilizada en F0/F1. |
| F6 Laboratorio mixto | 0% | Campaña 2–10 teléfonos y SLOs. |
| F7 Campo | 0% | Recorridos trazables con vehículos. |
| F8 Hardening | 0% | Fuzz/certificación, auditoría, privacidad e integración Convoy. |

(100 + 69 + 18) / 9 = 20,8%; se comunica redondeado como **21%**.
Estimación anterior comunicada al iniciar este seguimiento: 15%.

Bloque completado: **sesión Noise XX + preparación de identidad nativa: 100%**.
Noise XX está probado localmente; la identidad nativa quedó validada en ambos
teléfonos físicos. Esto no cierra F1 ni valida sesiones Noise o radio entre teléfonos.
El porcentaje global sigue redondeándose a 17% con la misma base de cálculo.

## Evidencia física del bloque

Campaña del 2026-09-11, build 10002, driver profile:

- Samsung Galaxy A73 (SM-A736B) por USB: NAT-VS0 y KEY-01 pasaron en los procesos
  18924 y 19246. Misma huella Android Keystore; misma instalación (UID y fecha),
  con `am force-stop` y ausencia de PID comprobada entre ejecuciones.
- iPhone 17 Pro Max, iOS 26.6.2, por Wi-Fi: NAT-VS0 y KEY-01 pasaron en los procesos
  4056 y 4059. Misma huella Keychain. El primer PID ya no existía al comprobar su
  terminación; la instalación se conservó.
- Las huellas Android/iOS son distintas. `tools/check_identity_runs.py` aprobó los
  cuatro informes con `both_platforms_verified=true`.
- App normal release reinstalada y abierta en ambos teléfonos. Android mostró en
  Diagnóstico la misma huella de las pruebas después de `adb install -r`.

Evidencia: `artifacts/identity/device-results.json`, `android-{1,2}.json`,
`ios-{1,2}.json`, logs de cada ejecución, `android-{1,2}-installation.txt`,
`android-1-stopped.txt`, `ios-1-stop.json`, `android-identity-ui.xml` y los informes
`*-normal-install.*` / `*-normal-launch.*` en el mismo directorio.

## Corrección del procedimiento

Flutter drive desinstala la app al terminar por defecto. Los primeros dos informes
Android cambiaron de huella porque eran instalaciones distintas: quedaron
excluidos en `artifacts/identity/invalid-uninstalled-runs/`. Los informes iOS
anteriores también se sustituyeron por una campaña sin desinstalación; se
conservan en `ios-previous-driver/` como evidencia histórica.

Se exige **`--keep-app-running`** y cierre explícito del proceso. No desinstalar
para probar persistencia. El primer reintento iOS no descubrió el servicio de VM
por Wi-Fi; se interrumpió sin contar como prueba. Dos reintentos con puerto local
59550 completaron la conexión y pasaron. Log: `ios-wifi-timeout.log`.

## Compilación y espacio

Gradle 9.3.1 usa la distribución `bin` con SHA-256 oficial fijado. Los fallos
anteriores de empaquetado fueron ENOSPC; se limpiaron cachés regenerables y 43
temporales abandonados de Git, recuperando 22,4 GiB. Las cuatro bibliotecas
nativas conservaron sus hashes. Logs: `artifacts/disk-cleanup.json`,
`native-packages-before-cache-clean.json` y `gradle-bin-bootstrap.txt`.

APK Android release 10002: 17,6 MB. App iOS release 10002: 16,4 MB. El registro
obsoleto de `integration_test` en release se corrigió ejecutando el build normal
sin `--no-pub`, para que Flutter regenerara los plugins. Evidencia adicional:
`artifacts/identity-android-build-after-refresh.txt`, `identity-android-apk.json`
y `identity-ios-build.txt`.

Siguiente bloque: crear y transferir invitaciones firmadas, conectar la política
al ejecutor de mensajería y después integrar BLE. GPS, texto y voz todavía no
están activos. Las instalaciones de prueba ya están autorizadas.

## Commit autenticado completo

`DurableSender` dejó de aceptar `PreparedObject` para terminar protección. Exige
un `SealedMessage` y mantiene el envelope cifrado junto con su anuncio firmado
hasta el commit. La integración contra SQLCipher usa `commit_sealed`, certificados
y roster real; tras perder el callback y reabrir, el destinatario vuelve a
autenticar y descifrar los bytes persistidos. También cubre presión del store y
reanudar con la misma secuencia sin reutilizar el ciphertext fallido.

Validación: `cargo test --workspace --all-targets --all-features --locked` aprobó
**69 pruebas** y `cargo clippy -p mesh-runtime -p mesh-sim --all-targets
--all-features --locked -- -D warnings` pasó. Este subbloque está completo; el
avance global se mantiene en **17%** porque la fase F1 pasa de 50% a 55%.
Todavía no expone una operación de mensaje en Flutter ni conecta iPhone y Android.

## Android por Wi-Fi

Conexión ADB inalámbrica verificada en `192.168.100.171:36553`, modelo SM-A736B.
La depuración inalámbrica ya estaba activada y el Mac vinculado; mDNS no
anunciaba el servicio. Se leyó el puerto en Ajustes y se conectó explícitamente.
Se confirmó el build 10002 y se llevó Mesh Lab al frente usando exclusivamente
ese transporte. USB seguía conectado durante esta comprobación. Evidencia:
`artifacts/identity/android-wifi.json`. Esto habilita desarrollo Mac→Android
sin cable; no equivale a una conexión mesh iPhone↔Android. El puerto puede cambiar.

## Store protegido desde los hosts nativos

`prepareIdentity` conserva su contrato público, pero ahora también abre
`state-v1.db` mediante SQLCipher. El slot de base de datos (bytes 96–127) se toma
en el host desde Keychain o Android Keystore, se copia a memoria temporal
zeroizada y se entrega directamente al FFI; Flutter recibe únicamente la huella
pública. El identificador de miembro se deriva nativamente de la clave Ed25519.

Los directorios de store quedan fuera de backup: `Application Support/mesh-store`
en iOS con `completeUntilFirstUserAuthentication`, y `noBackupFilesDir/mesh-store`
en Android. El empaquetado usa SQLCipher con OpenSSL vendorizado para que Android
no dependa de una `libcrypto` del sistema. `tools/build_native.py` fija iOS 15 en
Rust/C y configura compilador, ar y ranlib del NDK para los dos ABI Android.

Validación: prueba FFI de store, suite Rust completa, Clippy de FFI/JNI,
Kotlin/JNI host, contrato Swift, análisis y 14 pruebas Flutter. NAT-VS0 y KEY-01
pasaron en iPhone por Wi-Fi y Samsung Android por ADB Wi-Fi; KEY-01 invoca dos
veces `prepareIdentity`, que ahora abre SQLCipher con el slot protegido. El primer
driver iOS quedó detenido al reanudar porque coexistía una sesión `flutter run`
release; el reintento sin esa sesión pasó. Las apps release normales quedaron
instaladas y abiertas: Android `versionCode=10004`; iOS con firma de desarrollo.
Logs: `artifacts/identity/ios-secure-store-retry.log` y
`android-secure-store.log`. Esta integración no habilita aún mensajes, GPS, voz
ni radio.

## Ejecutor de store persistente

El host ya no abre el store solo para comprobarlo: conserva un handle opaco del
ejecutor Rust durante la vida del plugin y lo libera de forma idempotente al
desacoplarse. El handle, las filas y la clave no cruzan Pigeon/Flutter. La prueba
FFI verifica apertura, opacidad y doble liberación; Clippy, Kotlin/JNI, Swift y
los builds release pasaron. Releases actualizadas: Android `10004` e iOS abierta
por Wi-Fi. Aún falta conectar la política/roster durable y las operaciones de
envío/recepción a este ejecutor.

La prueba Android del ejecutor aprobó NAT-VS0 y KEY-01, pero Flutter intentó
instalar un profile `10002` sobre la release `10004`; al rechazar el downgrade,
la herramienta desinstaló y reinstaló el profile. Por eso ese resultado confirma
la apertura del ejecutor, pero no la persistencia de la identidad anterior. La
release `10004` fue restaurada inmediatamente y no se deben repetir drivers con
un número menor que el instalado.

## Política durable y estado de grupo

La migración SQLCipher `003-policy.sql` guarda una única política activa para el
laboratorio: autoridad, grupo, época, digest del roster, certificados y
revocaciones. Antes de persistirla, Rust verifica cada certificado; después de la
primera instalación fija autoridad y grupo. Una época puede repetirse únicamente
si el digest es idéntico; una sustitución, retroceso o cambio de autoridad queda
rechazado dentro de la transacción.

El host consulta solo la época activa mediante un handle opaco. Flutter no recibe
claves, certificados, identificador de grupo ni filas de SQLCipher. En Red aparece
«Sin grupo» hasta que exista una invitación firmada y se instale una política
válida. La prueba de store cubre reapertura y una sustitución firmada de la misma
época, que se rechaza sin cambiar el snapshot. Suite completa: Rust, Kotlin/JNI,
Swift, Pigeon y 15 pruebas Flutter aprobadas. La release `10006` está instalada
en el iPhone por Wi-Fi; el APK `10006` está construido y pendiente de que Android
vuelva a anunciar su puerto ADB.

## Creación local del grupo

El botón **Red → Crear grupo** se habilita solo después de preparar la identidad.
El motor obtiene de Keychain/Keystore los slots de firma y entrega, genera un
identificador de grupo con el CSPRNG del sistema, emite el primer certificado y
persiste la política en SQLCipher. Flutter recibe solamente el resultado
«configurado» y la época 1. Una segunda creación se rechaza y no reemplaza el
grupo existente.

Esto configura un único teléfono, no una red entre teléfonos: todavía falta el
intercambio de una solicitud de inscripción y la respuesta firmada del teléfono
que creó el grupo. La release `10006` se compiló para iOS y Android; se instaló y
abrió en el iPhone por Wi-Fi. Android no anunció un endpoint ADB al intentar la
instalación, aunque el APK ARM64 `10006` está listo.

## Base de inscripción del segundo teléfono

El núcleo ya codifica una solicitud pública y acotada de inscripción. Incluye
grupo, época, las dos claves públicas del teléfono que solicita entrar, nonce y
vigencia corta; el propio teléfono la firma en el dominio criptográfico de
inscripción. Alterarla o usarla caducada se rechaza. Esa solicitud no instala una
política ni concede acceso: falta que la autoridad la reciba, emita certificados
en una época nueva y entregue el roster firmado al solicitante. La prueba del
protocolo cubre firma, límite, caducidad y manipulación.

El roster de respuesta usa un paquete canónico y acotado con autoridad, grupo,
época, certificados y revocaciones. No es una credencial por sí solo: al
instalarse, SQLCipher vuelve a verificar todos los certificados y exige que el
teléfono receptor figure en el roster. Las pruebas de protocolo cubren su
codificación, datos sobrantes y límites de tamaño.

## Wi‑Fi Aware mesh — WA‑0 iniciado

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 52%; avance global: 22%.**

Se retiraron de la interfaz y del contrato Pigeon los campos de IP, puerto y
Wi‑Fi local. Red ahora ofrece un único control **Conectar sesión / Salir de
sesión**: inicia Bluetooth y el descubrimiento Wi‑Fi Aware; al salir ambos se
detienen. En ese registro inicial, el acelerador TCP histórico estaba aislado, privado y sin API o UI. Posteriormente se retiró del host y de la app; no cuenta ni contó como canal de campo.

Android declara `NEARBY_WIFI_DEVICES`, comprueba `FEATURE_WIFI_AWARE`, reserva
una sesión NAN, publica y se suscribe a `comfrazkomeshlab`, y reporta vecinos y
capacidad directa de dos enlaces. iOS consulta `WACapabilities`, la lista de
dispositivos Wi‑Fi Aware vinculados y declara el servicio `_meshlab._tcp`.
También se añadieron el entitlement Publish/Subscribe y la configuración de
firma correspondiente.

El chat ya conserva hasta 250 entradas locales, identifica mensajes, usa HLC (reloj híbrido, contador lógico y origen) para un orden determinista, evita duplicados y limpia el borrador tras aceptar la cola. El formato de conversación no sustituye aún el log durable/replícado del motor, que pertenece a WA‑3.

Validación: `flutter test` (15 pruebas), `flutter analyze --no-fatal-infos`,
`tools/check_pigeon.py`, `git diff --check`, build iOS Release sin firma y APK
Android Release aprobaron. El build iOS firmado queda bloqueado por el perfil
Apple actual: no contiene `com.apple.developer.wifi-aware`. WA‑1 no puede
probarse ni instalarse en el iPhone hasta habilitar dicha capacidad para
`com.frazko.meshLab` y regenerar el provisioning profile.


### Reconexión del radio

Android mantiene un único reintento Wi‑Fi Aware cada dos segundos si el sistema
pierde recursos NAN, la sesión termina o el radio queda temporalmente no
disponible. iOS refresca periódicamente sus dispositivos Wi‑Fi Aware vinculados.
Ambos ciclos se cancelan de forma explícita con **Salir de sesión**. No se
presenta un estado “conectado” por esta búsqueda: WA‑1 sigue requiriendo un
canal de datos auténtico iPhone↔Android y la capacidad Apple firmada.

Validación posterior: APK Android Release e iOS Release sin firma compilaron;
`flutter test` aprobó 15 pruebas, Pigeon y `git diff --check` aprobaron.

## Wi‑Fi Aware mesh — capacidad de grupo y presencia acotada

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 57%; avance global: 23%.**

El núcleo de grupo ya no arrastra el límite histórico de diez certificados:
`MAX_GROUP_MEMBERS` es 50, mientras `MAX_TARGETS` sigue en 10 para limitar el
fan-out de claves de cada objeto. La autoridad emitió y validó por prueba un
roster exacto de 50 miembros y rechazó el 51. El bundle de política aumenta a
64 KiB y los límites equivalentes de C/JNI se mantienen alineados.

Las bases SQLCipher existentes migran de schema v3 a v4 mediante
`004-group-capacity.sql`. La migración recrea únicamente la tabla de
certificados con índice 0–49, conserva sus filas y actualiza su hash de schema.
Una prueba abre una base v3, confirma la preservación del certificado previo y
verifica que acepta el índice 49 y rechaza el 50.

`mesh-replication` incorpora una tabla de presencia determinista y acotada:
dos vecinos directos autenticados por teléfono, rutas relé de hasta 16 saltos,
versiones origen `(incarnation, sequence)` para negar replays y expiración por
reloj lógico. La tabla no acepta descubrimientos radio sin autenticar ni hace
declaraciones de conectividad desde Flutter/Kotlin/Swift. Falta conectar estos
frames firmados a los enlaces Wi‑Fi Aware reales y propagar los cambios, parte
del gate WA‑3.

Validación: `cargo test -p mesh-protocol -p mesh-replication -p mesh-ffi-c -p
mesh-store` aprobó 44 pruebas; `cargo clippy -p mesh-replication --all-targets
-- -D warnings` y `git diff --check` aprobaron.

## Wi‑Fi Aware mesh — frame de presencia de origen

La presencia ya tiene un formato canónico pequeño, firmado por la clave de
firma certificada de su miembro y separado criptográficamente en el dominio
`Presence`. Liga grupo, época, miembro, `incarnation`, secuencia y vencimiento.
Un relay conserva los bytes de origen; no puede crear una secuencia nueva de
otro teléfono. La verificación rechaza scope, clave, firma o vencimiento
incorrectos antes de entregar el claim a la tabla de presencia.

Validación: `cargo test -p mesh-crypto -p mesh-protocol -p mesh-replication`
aprobó 20 pruebas y Clippy con warnings como errores aprobó. Aún falta la capa
de sobre de relay, la entrega de frames por enlaces autenticados y la campaña
física WA‑1/WA‑3.

## Wi‑Fi Aware mesh — preparación de emparejamiento físico

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 59%; avance global: 24%.**

El único control **Conectar sesión** abre en iPhone el selector de
`DeviceDiscoveryUI` del sistema cuando aún no existe una vinculación Wi‑Fi
Aware. La autorización queda en el sistema con acceso permanente; Mesh Lab no
presenta una dirección IP, QR, SSID, hotspot ni un segundo flujo propio. Android
y iOS ahora anuncian el mismo identificador de servicio `_meshlab._tcp`, que es
requisito para que los radios estén buscando el mismo servicio en vez de dos
anuncios incompatibles.

El semáforo no usa el estado del selector como prueba de enlace: sigue verde
solo con una sesión autenticada. Falta el frame autenticado entre Android e iOS
por una ruta de datos Wi‑Fi Aware, por lo que WA‑1 todavía no se declara
completo. La compilación iOS Release sin firma aprobó con DeviceDiscoveryUI y la
compilación nativa de iOS/Android aprobó. La instalación firmada en iPhone
sigue bloqueada hasta que el perfil Apple incluya
`com.apple.developer.wifi-aware`.

Android también escucha `ACTION_WIFI_AWARE_STATE_CHANGED`: si el sistema pierde
o recupera NAN mientras la sesión sigue solicitada, cierra los handles antiguos
y reinicia descubrimiento sin pedir que la persona pulse otro botón. El APK
Release 10007 y el build iOS Release sin firma volvieron a aprobar después de
esta recuperación.

## Wi‑Fi Aware mesh — estado observable y empaquetado Release

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 59%; avance global: 24%.**

`AwareInfo` incorpora un estado estable que la pantalla traduce en lenguaje
claro: radio no disponible, grupo pendiente, emparejamiento requerido,
buscando, vecino detectado o teléfono vinculado a la espera del enlace
protegido. Un radio activo o una pareja guardada nunca pone el semáforo verde:
solo una sesión autenticada puede hacerlo.

El paquete Release ya no arrastra el runner de `integration_test`, que Flutter
registraba como un plugin Android de producción y hacía el APK no reproducible
después de una prueba. Las pruebas unitarias continúan independientes del
runner. Validación actual: 15 pruebas Flutter, Pigeon reproducible, APK Android
Release 10007 e iOS Release sin firma aprobados.

## Wi‑Fi Aware mesh — descubrimiento limitado al grupo

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 60%; avance global: 24%.**

El descubrimiento Android ya no cuenta cualquier anuncio que comparta el nombre
público `_meshlab._tcp`. El store SQLCipher deriva dentro del FFI una huella de
16 bytes a partir de la política de grupo ya verificada, con separación de
dominio y sin entregar a Flutter el ID de grupo, certificados ni claves. La
publicación y suscripción NAN incluyen esa huella; Android la compara en tiempo
constante antes de agregar un vecino al estado observable. La autenticación Noise
sigue siendo obligatoria antes de declarar una conexión segura.

Cuando Android recibe `onServiceLost`, elimina ese vecino de inmediato de su
contador. Si cambia la disponibilidad de NAN, conserva la sesión solicitada y
reconstruye el descubrimiento. El estado visible usa “vecinos del grupo” y
“teléfonos autorizados”, evitando presentar otros radios cercanos como miembros
del laboratorio.

Validación local: la prueba FFI confirma que dos miembros inscritos derivan la
misma huella no vacía; `cargo test -p mesh-ffi-c -p mesh-ffi-jni` aprobó 11
pruebas, `tools/build_native.py all`, 15 pruebas Flutter, análisis sin errores,
APK Android Release e iOS Release sin firma aprobaron. No se instaló ningún
paquete ni se hizo prueba entre teléfonos en este bloque. WA‑1 continúa
pendiente del entitlement Apple firmado y de una ruta de datos Wi‑Fi Aware real.

El host Android además libera el receptor de disponibilidad NAN al desacoplarse
el plugin. Eso evita conservar callbacks o una sesión de radio de un engine
Flutter anterior después de recrear la app.

## Wi‑Fi Aware mesh — renovación por membresía

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 61%; avance global: 24%.**

Android conserva la huella con la que abrió NAN. En cada consulta de estado
compara esa huella con la derivada de la política actual. Si una incorporación
avanzó el roster, cierra solo los handles NAN antiguos y reinicia publicación y
suscripción con la huella nueva; Bluetooth y la sesión de campo solicitada no se
cierran. Esto evita que miembros con versiones distintas del roster se presenten
como vecinos válidos y evita pedir al usuario que reinicie la sesión.

Validación local: APK Android Release y 15 pruebas Flutter aprobaron. Las
pruebas físicas Wi‑Fi Aware se mantienen pendientes, conforme a la decisión de
posponerlas.

La especificación de WA‑1 ya separa descubrimiento, solicitud de red directa,
puerto de transporte y autenticación Noise. No se contarán `PeerHandle`,
emparejamiento de iOS ni callbacks `onAvailable` como conexión segura: solo una
trama autenticada bidireccional podrá cambiar ese estado.

## Wi‑Fi Aware mesh — firma Apple habilitada

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 63%; avance global: 25%.**

La cuenta Apple del equipo `5R3BWDNRX8` quedó iniciada en Xcode. La capacidad
Wi‑Fi Aware conserva Publish y Subscribe y Xcode regeneró el perfil de
`com.frazko.meshLab` sin los errores anteriores. El build iOS Release firmado
completó; su perfil integrado y la firma de la app contienen
`com.apple.developer.wifi-aware` con ambas opciones. No se instaló esta versión
ni se inició una prueba física.

## Corrección iOS — cierre al consultar Wi‑Fi Aware

**Actualizado: 2026-09-13. Avance global: 25%; Wi‑Fi Aware: 63%.**

El informe físico `Runner-2026-09-13-111731.ips` identificó un
`EXC_BREAKPOINT` en `NativeRuntime.policyEpoch()`: los métodos Pigeon de Wi‑Fi
Aware consultaban el store desde un executor cooperativo, aunque el runtime
exige su propia cola serial. `MeshHostPlugin` ahora obtiene la membresía con
`execute` en la cola nativa y toca `WifiAwareAccess` solo en la cola principal.

El build iOS Release firmado aprobó, se reinstaló en el iPhone y el proceso
`Runner` permaneció activo tras abrirse. Se conserva el crash report en
`app/artifacts/ios-crash-20260913/Runner-2026-09-13-111731.ips` como regresión.

## Wi‑Fi Aware mesh — NDP Android físico y relay controlado

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 82%; avance global: 33%.**

El A73 y el S24+ forman ahora un NDP Wi‑Fi Aware real mediante
`WifiAwareNetworkSpecifier` y `ConnectivityManager`: cada lado recibe la red
`aware_data0`, abre un socket IPv6 de enlace local y completa Noise. La evidencia
del sistema confirma NDP cifrado, socket aceptado y autenticación en ambos
Android. La prueba se repitió con Bluetooth apagado en los dos equipos; ambos
semáforos permanecieron verdes y reportaron un vecino Wi‑Fi Aware directo.

Estos modelos reportan `isNanPairingSupported=false`, así que no pueden iniciar
el pairing de plataforma requerido por el iPhone. En esta campaña el iPhone
queda como borde Bluetooth autenticado y el A73 reenvía las cargas verificadas
entre Wi‑Fi Aware y Bluetooth. La entrega S24→A73→iPhone validó el primer salto
multirradio, sin router, SSID, hotspot ni IP introducida.

`mesh-replication::RelayCache` añade el gate puro de WA‑3: ID compuesto por
origen y operación, previous-hop, vencimiento lógico, máximo de 16 saltos y
deduplicación acotada a 512 entradas con expulsión determinista. Su `RelayFrame`
se codifica canónicamente en 91 bytes y rechaza versión, tamaño, límite, saltos
y expiración inválidos antes de entrar al host.

El simulador determinista ya ejercita 50 miembros: cada uno conserva exactamente
dos aristas Wi‑Fi Aware y una arista Bluetooth autenticada de respaldo. Un anillo
solo tendría diámetro 25 y violaría el máximo de 16 hops; el matching Bluetooth
reduce el máximo observado a 13 sin abrir una tercera sesión Aware. Para 32
objetos se entregaron 1,568 copias lógicas, con deduplicación y sin sobrepasar el
budget. Durante una partición de las aristas entre ambas mitades, las tareas se
mantienen pendientes y se reintentan al reencuentro sin exceder dos sesiones
Aware. `cargo test -p mesh-sim -p mesh-replication` aprobó 11 pruebas y Clippy
para ambos crates pasó como error estricto.

La migración SQLCipher `005-relay-custody.sql` añade `relay_outbox` y
`relay_metadata`. Cuando un teléfono recibe de otro miembro todos los chunks
válidos de un objeto, registra en la misma transacción el objeto completo, su
custodia y el `RelayFrame` canónico. La entrada persiste tras reiniciar; no crea
un `local_delivery`, no inventa un receipt y no altera el `outbox` exclusivo del
originador. Un frame conflictivo posterior se rechaza y no modifica la ruta ya
guardada. `cargo test -p mesh-store -p mesh-sim -p mesh-replication` aprobó 36
pruebas y Clippy de los tres crates pasó en modo estricto. Falta emitir el
receipt de custodia y llevar el scheduler a Android/iOS.

`mesh-replication::neighbor_plan` transforma el roster certificado en el mismo
objetivo de overlay para cada teléfono: predecesor y sucesor Wi‑Fi Aware, más un
chord Bluetooth al miembro opuesto cuando el roster es par. La prueba de 50
miembros verifica que cada teléfono conserva exactamente dos candidatos Aware,
que el resultado no depende del orden recibido y que todos los pares quedan a
16 hops o menos. El host aún debe negociar los enlaces reales, reemplazar de
forma temporal un candidato fuera de alcance y volver al plan al reencontrarlo.

El host Android ahora aplica ese presupuesto antes de solicitar la red: cuenta
enlaces NDP activos y en negociación, y nunca reserva más de dos. Los demás
anuncios válidos siguen visibles como candidatos, sin disparar solicitudes de
red ni desplazar un enlace existente. `flutter test` aprobó 15 pruebas y el APK
Android **Release** compiló; no se instaló ningún paquete en los teléfonos.

## Wi‑Fi Aware mesh — selector certificado conectado al radio

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 86%; avance global: 35%.**

El selector ya no es solo una simulación: el puente Rust/JNI entrega al host
Android, desde el roster verificado de SQLCipher, las huellas públicas cortas de
sus dos vecinos Wi‑Fi Aware. La publicación sigue mostrando otros miembros
cercanos, pero el host rechaza abrir NDP con cualquiera que no pertenezca a ese
par. La autenticación Noise conserva la verificación completa, por lo que una
coincidencia accidental de una huella corta no concede un enlace.

Al emitir una inscripción, la autoridad conserva el bundle firmado nuevo y lo
propaga en chunks cifrados por los enlaces autenticados existentes. Cada teléfono
vuelve a verificarlo dentro de SQLCipher antes de aplicarlo y reiniciar su
descubrimiento con el nuevo selector. Texto, voz y controles de roster se envían
ahora por todos los enlaces directos vivos del grupo; los filtros de duplicados
evitan que un ciclo se multiplique visualmente. Aún falta el scheduler durable
de ACK/custody que confirme la convergencia completa antes de retirar una
configuración anterior.

Validación: `cargo test -p mesh-ffi-c -p mesh-ffi-jni -p mesh-replication`
aprobó 21 pruebas; se recompilaron ambos ABI JNI y el APK Android **Release**
compiló correctamente. Flutter mantiene 15 pruebas aprobadas y análisis sin
errores, con seis avisos de estilo ya existentes. No se instaló ningún paquete.

## Ruta prevista iPhone↔iPhone

Wi‑Fi Aware de Apple soporta conexiones entre iPhones compatibles, no solo
entre iPhone y accesorios. Mesh Lab ya incluye el entitlement `Publish` y
`Subscribe`, declara `_meshlab._tcp` y abre `DeviceDiscoveryUI` para el pairing
controlado por el sistema. El siguiente incremento de iOS añade el canal real
con `NetworkListener`, `NetworkBrowser` y `NetworkConnection` sobre los iPhones
vinculados, seguido por el mismo handshake Noise y relay que usa Android.

No existe una simulación válida con la Mac mini: hacen falta dos iPhone físicos
compatibles para validar ese enlace. Apple requiere pairing explícito por sus
propias interfaces; la app no puede emparejar dos iPhones silenciosamente. Esta
ruta forma parte del plan Wi‑Fi Aware y no cambia el porcentaje hasta que haya
un canal autenticado físico entre dos iPhones.

## Registro vivo de huecos técnicos

**Actualizado: 2026-09-13. Avance global: 36%; Wi‑Fi Aware: 88%.**

Los huecos descubiertos ya no se dejan solo en notas de implementación. El
registro canónico está en [known-gaps.md](known-gaps.md): separa evidencia
física, código pendiente, simulación y bloqueos de plataforma. Los más
importantes son el canal WFA iPhone↔iPhone y su prueba con dos iPhones, relay
durable con custody/ACK/inventario en hosts, reemplazo de vecinos, presencia
propagada, background y la campaña física de 5/10/50. El transporte LAN
histórico ya fue retirado del host y de la app; `NET-06` conserva solo una
regresión física WFA pendiente y no lo considera parte del perfil de campo.


## Decisión de producto — sin historial retroactivo

**Actualizado: 2026-09-13.**

No se implementará sincronización de historial para miembros nuevos ni recuperación
de mensajes que ocurrieron antes de su incorporación. Cada teléfono conserva un
historial local limitado para su propio reinicio; al ingresar al grupo, una persona
ve los mensajes recibidos desde ese momento. Esto elimina inventarios de chat y
transferencias históricas del alcance actual.

## Wi‑Fi Aware mesh — canal iPhone↔iPhone

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 88%; avance global: 36%.**

El host iOS ya abre su canal directo moderno: publica `NetworkListener`, busca
con `NetworkBrowser`, crea `NetworkConnection<TLS>` hacia dispositivos WFA
vinculados y encuadra cada frame con longitud explícita. Un escritor serial
impide intercalar encabezados y cuerpo de texto, GPS, receipt o voz. Cada enlace
crea una sesión Noise separada y solo después de autenticarse puede atravesar la
capa compartida de texto/voz; Bluetooth conserva su propio enlace y el dedupe
por ID evita que un fallback duplique el chat.

Validación local: `flutter build ios --release --no-codesign` aprobó. No se
instaló nada. La prueba física continúa bloqueada por falta de un segundo iPhone
compatible; el estado y su criterio de cierre están en `NET-01` y `NET-03` del
registro de huecos.

## Wi‑Fi Aware mesh — habilitación de contenido directo

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 88%; avance global: 36%.**

El estado seguro de la interfaz ya acepta tanto Bluetooth autenticado como un
enlace Wi‑Fi Aware autenticado. Así, texto, GPS y voz no quedan bloqueados si
dos iPhones usan WFA con Bluetooth apagado. La prueba de controlador cubre ese
caso y los builds Android/iOS Release aprobaron sin instalar paquetes. La
confirmación física entre dos iPhones sigue pendiente.

## Wi‑Fi Aware mesh — recuperación automática iOS

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 88%; avance global: 36%.**

Al perder un datapath iPhone↔iPhone, el host conserva el pairing de iOS, cancela
el listener y browser anteriores y los recrea después de una espera breve. No
requiere un segundo toque de `Conectar sesión`; el semáforo permanece en
reconectando hasta que Noise vuelve a autenticar el enlace. `flutter test`
(16 pruebas) y el build iOS Release sin firma aprobaron. Falta evidencia física
con dos iPhones.

## Operación de campo Android — base de segundo plano

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 88%; avance global: 36%.**

El control único **Conectar sesión** ahora promueve la sesión Android a un
servicio de primer plano de tipo `connectedDevice`, con una notificación
persistente que explica que Bluetooth y Wi‑Fi Aware siguen buscando enlaces
seguros. **Salir de sesión** la retira. El servicio no crea otro stack de
radios: protege el mismo proceso que posee los enlaces autenticados y evita
presentar una sesión ficticia tras una terminación del proceso.

Esto es una base operativa, no una promesa de funcionamiento ilimitado. Un
cierre forzado termina la sesión por diseño de Android; iOS aún necesita su
estrategia específica de background y ambos sistemas requieren una campaña con
pantalla bloqueada, retorno a foreground y consumo medido. `flutter test`
(16 pruebas) y `flutter build apk --release` aprobaron. No se instaló ningún
paquete.

## Mesh durable — límite de audiencia confirmado

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 88%; avance global: 36%.**

La revisión conjunta de `MAX_GROUP_MEMBERS=50` y `MAX_TARGETS=10` confirmó que
un broadcast durable no puede ser un sobre único para todos los miembros. El
núcleo Rust ya particiona un grupo de 50 de forma determinista en hasta cinco
audiencias de diez o menos, excluyendo al emisor. El siguiente incremento añade
un ID lógico de chat y crea un objeto cifrado por audiencia. La interfaz
deduplicará esos objetos en una sola burbuja; los receipts se agregarán por
audiencia y nunca se mostrarán como entrega completa hasta terminar las cinco.

Este hallazgo se registró como `MESH-07` en `known-gaps.md`. Evita que el
trabajo de relay durable convierta por accidente un diseño de 10 destinatarios
en una promesa falsa para 50. `cargo test -p mesh-replication` aprobó 12
pruebas, incluidas las dos que fijan esta partición.

## Operación de campo iOS — modos Bluetooth de fondo

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 88%; avance global: 36%.**

El paquete iOS ahora declara `bluetooth-central` y `bluetooth-peripheral`.
Corresponden a los dos roles CoreBluetooth que el host ya usa para descubrir,
conectar y atender enlaces BLE autenticados. Esto permite que iOS mantenga o
despierte trabajo Bluetooth que el sistema autorice mientras la interfaz está
en segundo plano; no es una promesa de ejecución ilimitada ni reemplaza la
restauración tras terminación, que sigue pendiente.

No se habilitó `location` en background: el GPS de Mesh Lab continúa siendo una
lectura puntual solicitada por la persona hasta definir consentimiento,
frecuencia, expiración y consumo. `flutter build ios --release --no-codesign`
y `flutter test` (16 pruebas) aprobaron. No se instaló ningún paquete.

## Retiro de incorporación por IP

**Actualizado: 2026-09-13. Bloque Wi‑Fi Aware: 88%; avance global: 36%.**

La app ya no abre un servidor HTTP para inscribir teléfonos, no conserva una
Dirección IP, no acepta unión por IP y no mantiene el servicio Android
`dataSync` que servía ese flujo. También se retiraron sus pruebas. La
incorporación disponible sigue siendo la nativa por Bluetooth; Wi‑Fi Aware es
el transporte de campo directo cuando el hardware lo permite.

El host Android ahora nombra y conserva esos sockets como canal Wi‑Fi Aware: solo `WifiAwareAccess` le entrega sockets después de un NDP y valida que un socket aceptado pertenece a esa red antes de iniciar Noise. Se retiraron el `ServerSocket` del host Bluetooth, la conexión a una IP/puerto y el cliente TCP iOS. `NET-06` queda pendiente únicamente de una regresión física WFA después de esta separación. `flutter test`, `flutter analyze`, APK Release, iOS Release y las pruebas/análisis del nuevo SDK aprobaron. No se instaló ningún paquete.

## Contrato de registros durables entre saltos

**Actualizado: 2026-09-13. Avance global: 37%; Wi‑Fi Aware: 88%.**

`mesh-protocol::DurableRecord` fija el límite de wire de anuncio firmado,
fragmento identificado y receipt. `mesh-replication::RoutedRecord` lo une ahora
a un `RelayFrame` canónico en cada transmisión: un fragmento o receipt no puede
avanzar como payload sin su ruta, TTL y presupuesto de saltos. Ambos rechazan
versiones, tamaños, truncamientos y extensiones antes de tocar SQLCipher.

La prueba funcional de Noise envía anuncio, todos los fragmentos y receipt por
un canal autenticado y confirma la entrega durable. La simulación A→B→C valida
que B acepta el anuncio una vez, toma custodia transaccional, se reinicia y
reenvía el mismo objeto enmarcado a C, que lo verifica y confirma una única
entrega. Una ruta sintética sin anuncio autenticado se rechaza antes de crear
custodia. `mesh-protocol` (11), `mesh-replication` (13), `mesh-store` (23),
`flutter test` (14) y `flutter analyze` aprobaron. El siguiente incremento
conecta esta cola al FFI y a los enlaces Android/iOS; la UI no mostrará
“entregado” hasta que exista un receipt auténtico de cada audiencia.

## Contrato durable en los puentes móviles

**Actualizado: 2026-09-13. Avance global: 38%; Wi‑Fi Aware: 88%.**

El límite de anuncio se ajustó a 3,998 bytes: con el encabezado de
`RoutedRecord` de 93 bytes, todo registro válido ocupa como máximo los 4,096
bytes que admite una sesión Noise de vecino. Esto evita aceptar una unidad que
no podría transmitirse. El puente C compartido ya valida y empaqueta la ruta y
el registro; JNI y Swift exponen la misma operación sin revelar handles de
SQLCipher ni claves. Android Release y los artefactos Rust de ambos ABI se
recompilaron; el contrato C/Swift aprobó su vector de ruta, payload y
truncamiento. La prueba Kotlin/JNI está incluida, pero no pudo ejecutarse en
este Mac porque su runner offline no encuentra `junit:junit:4.13.2` en la caché
local.

Las pruebas de Rust ahora suman `mesh-protocol` (11), `mesh-replication` (14),
`mesh-store` (23) y `mesh-ffi-c` (12). Falta que `BluetoothAccess` y
`WifiAwareAccess` conviertan los envíos de texto, voz y GPS a objetos durables,
ejecuten la cola de custodia y procesen los receipts en equipos reales.

## Ingreso durable autenticado

**Actualizado: 2026-09-14. Avance global: 39%; Wi‑Fi Aware: 88%.**

`secure_store_accept_routed` es la primera operación del ejecutor disponible
por C/FFI. Solo se invoca después de Noise: verifica el roster local, vincula
el origen del `RelayFrame` al manifest firmado y persiste anuncio/chunks
mediante la ruta autenticada de SQLCipher. Un chunk sin anuncio autenticado se
rechaza antes de crear custody. La prueba funcional construye un grupo,
incorpora un segundo miembro, sella un objeto, ingiere todos los
`RoutedRecord` y comprueba el rechazo del mismo chunk en un store sin anuncio.
`cargo test` de protocolo, relay, store y FFI aprobó 61 pruebas. Falta la
exposición JNI/Swift, cache de dedupe, drain de cola y receipts.

## Entrada durable disponible en Android e iOS

**Actualizado: 2026-09-14. Avance global: 40%; Wi‑Fi Aware: 88%.**

`secure_store_accept_routed` ya cruza C, JNI y Swift. Devuelve únicamente un
resultado compacto de scheduling tras guardar un anuncio o fragmento
verificado; no expone filas, handles ni claves a Flutter. Los artefactos Rust
de Android y el XCFramework iOS se recompilaron. `mesh-ffi-c` (13 pruebas),
`mesh-ffi-jni`, el contrato C/Swift y `flutter analyze` aprobaron. El método
no está conectado aún a los payloads de radio: el siguiente bloque une
`RelayCache` y `relay_queue` para que ninguna recepción se reenvíe antes del
commit y para que un reinicio pueda drenar su custodia.

## Custodia durable con vecino de entrada

**Actualizado: 2026-09-14. Avance global: 45%; Wi‑Fi Aware: 88%.**

La cola de relé ahora está disponible para el host nativo por C, JNI y Swift:
un slot devuelve un `RoutedRecord` canónico o vacío, sin exponer secretos,
filas ni handles a Flutter. Al completar un objeto entrante, SQLCipher conserva
por separado el `received_from` autenticado y el frame que deberá salir del
relay. La distinción sigue existiendo tras un reinicio y permite al scheduler
omitir el vecino que acababa de entregar el objeto, en lugar de generar un
rebote inmediato.

El esquema subió a la versión 6; al actualizar desde la versión 5 conserva las
filas anteriores y marca su origen histórico con el `previous_hop` ya guardado.
La prueba de integración reinicia el store y confirma que conserva el frame y
el vecino de entrada. Aprobaron `mesh-store` (23), `mesh-ffi-c` (14), el check
JNI, contrato Swift, `flutter analyze`, `flutter test` (14), APK Release e iOS
Release sin firma. No se instaló ningún paquete ni aplicación.

## Primer ejecutor durable de radio Android

**Actualizado: 2026-09-14. Avance global: 46%; Wi‑Fi Aware: 88%.**

Android ya ejecuta una cola de custody completa sólo después de que el último
chunk se confirmó en SQLCipher. Recupera anuncio y chunks mediante el bridge,
omite el miembro `received_from` persistido y cifra cada registro para los
otros enlaces autenticados BLE o Wi‑Fi Aware. El límite de 65 slots coincide
con el objeto máximo del contrato (un anuncio y hasta 64 chunks), por lo que
un callback corrupto no puede provocar un drain sin límite. La fila no se
borra al transmitir: falta un receipt auténtico para hacerlo.

APK Release, `mesh-ffi-c` (14), `mesh-store` (23), `mesh-protocol` (11),
`mesh-replication` (14), check JNI, contrato Swift, análisis Flutter y 14
pruebas Flutter aprobaron. No se instaló el APK. El siguiente paso conecta la
producción y consumo durable de texto, GPS y voz; el chat directo visible no
se cambió ni se presenta como entrega durable.

## Ingreso y relay durable iOS

**Actualizado: 2026-09-14. Avance global: 47%; Wi‑Fi Aware: 88%.**

iOS ahora pasa tanto los payloads BLE como los de Wi‑Fi Aware por la misma
admisión durable posterior a Noise. Conserva el vecino autenticado, persiste
anuncio/chunks y, tras el commit, programa el reenvío a sus otros enlaces BLE,
excluyendo al de entrada. El canal Wi‑Fi Aware de iOS queda como ingreso
durable y puente hacia BLE; su reenvío WFA se mantiene desactivado hasta que
el adaptador seleccione un vecino WFA individual y pueda excluir el enlace de
entrada con la misma precisión.

`flutter build ios --release --no-codesign` aprobó con el XCFramework recién
compilado. Rust/FFI y el contrato Swift permanecen verdes. No se instaló la
aplicación. El siguiente incremento crea los objetos durables desde texto,
GPS y voz y los consume al completar su verificación.

## Origen durable de texto para grupos de hasta 50

**Actualizado: 2026-09-14. Avance global: 48%; Wi‑Fi Aware: 88%.**

El host nativo ya puede sellar un texto antes de tocar una radio. Lee sólo el
roster certificado, separa hasta 49 destinatarios en audiencias de diez,
cifra y firma un objeto por audiencia, confirma cada objeto y sus chunks en
SQLCipher y devuelve los `RoutedRecord` del outbox tras una reconexión o
reinicio. El `RelayId` sale del ID opaco y ya salado del objeto, de modo que no
depende de memoria del proceso. C, JNI y Swift exponen esta operación sin
entregar claves a Flutter.

La prueba funcional crea un roster de 50 miembros y demuestra cinco objetos,
cinco anuncios y diez chunks: 15 registros en total. Las envolturas de claves
para diez destinatarios explican que un texto corto ocupe dos chunks; esa
medición queda fijada en la prueba, no como una suposición. `mesh-ffi-c` (14),
check JNI, APK Release, iOS Release, `flutter analyze` y 14 pruebas Flutter
aprobaron. No se instaló nada.

En ese punto quedaban pendientes el descifrado posterior al commit en el
receptor, los receipts y la proyección de una burbuja lógica. El bloque
siguiente cerró recepción y commit de texto; el enrutamiento de receipts y el
agregador de las cinco audiencias siguen pendientes.

## Texto durable: recepción, commit y proyección nativa

**Actualizado: 2026-09-14. Avance global: 49%; Wi‑Fi Aware: 88%.**

El flujo de texto ya recorre el siguiente tramo completo dentro de los hosts
nativos: al tocar Enviar, el texto se sella y entra primero al outbox SQLCipher;
el scheduler inicia los registros persistidos por cada enlace Noise sano. El
receptor conserva anuncio y chunks como custodia, descifra y verifica cada
fragmento sólo al completarse el objeto, y en la misma transacción registra su
receipt firmado. Sólo después de ese commit entrega el texto UTF-8 al reducer
de chat. Un corte de radio después de enviar no pierde el objeto origen; el
outbox conserva el mismo ID para un nuevo drain.

Se corrigió una separación crítica: completar chunks de un objeto de relay ya
no lo marcaba como entrega local. Custodia y entrega autenticada son estados
distintos; un ciphertext con clave errónea sigue pendiente y no expone texto ni
recibo. La prueba funcional crea un grupo real de dos miembros, pasa todos los
registros por la admisión enrutada, intenta descifrar con clave incorrecta,
reintenta con la correcta y comprueba una sola proyección tras el commit.

El bridge C, JNI y Swift ofrecen el mismo paquete interno de finalización, que
no cruza Flutter ni contiene claves. Android Release e iOS Release sin firma
compilaron; `cargo test` de protocolo/store/FFI, check JNI, análisis Flutter y
14 pruebas Flutter aprobaron. No se instaló nada. Falta encaminar el receipt
por saltos y retirar el outbox origen sólo cuando todos los destinatarios
certificados lo hayan confirmado; por eso la interfaz aún no afirma
“entregado a todo el grupo”.

## Receipt durable de regreso al origen

**Actualizado: 2026-09-14. Avance global: 51%; Wi‑Fi Aware: 88%.**

El receipt firmado que nace al confirmar el texto ya es un `RoutedRecord`
persistido y reintentable. El receptor lo recrea desde SQLCipher después de
cualquier reconexión; cada relay lo mueve por el frame de hop autenticado sin
reenviarlo al vecino de entrada; el origen encuentra el objeto, verifica firma,
actor, grupo, época, contexto y vigencia antes de registrar la confirmación.
Cuando la audiencia de un objeto queda completa, SQLCipher retira sólo ese
objeto del outbox. Repetir un receipt no aumenta el progreso ni recrea texto.

La prueba funcional de dos miembros cubre la cadena completa: origen → records
Noise → custodia del receptor → fallo con clave errónea → commit con clave
correcta → receipt enrutado → outbox del origen vacío. `cargo test --workspace`
aprobó el núcleo, simulador, store y FFI; también aprobaron XCFramework iOS,
APK Release, iOS Release sin firma, `flutter analyze` y 14 pruebas Flutter. No
se instaló ningún paquete. Falta probar esa misma cadena sobre radios físicos,
agregar una confirmación de receipt para dejar de reintentarlo antes de vencer,
y agrupar las cinco audiencias bajo una sola burbuja de chat para 11–50 miembros.

## Voz durable y ubicación por el mismo outbox

**Actualizado: 2026-09-14. Avance global: 53%; Wi‑Fi Aware: 88%.**

Las notas de voz de hasta ocho segundos ahora entran primero al mismo outbox
SQLCipher que el texto: la duración y los bytes AAC/M4A forman un payload
acotado, se sellan para cada audiencia certificada y sólo se entregan a la
interfaz después del commit criptográfico del receptor. Android e iOS guardan
el audio recibido en su caché privada, actualizan la ficha de voz y devuelven
el receipt durable. Se retiró la salida heredada en memoria: si el objeto no
puede reservarse en el outbox, la app rechaza el envío en vez de escribir audio
directamente a una radio. La lectura GPS bajo demanda ya se representa como payload
estructurado de este canal, por lo que conserva la misma custodia y reintento.

La prueba funcional `durable_voice_chunks_commit_before_the_receiver_exposes_audio`
envía una nota de voz de 4 KiB con encabezado de duración: verifica que sus
registros se admiten cifrados, que el receptor no expone nada hasta completar y
validar el objeto, que los bytes recuperados son exactos y que el receipt
firmado vacía el outbox del emisor. Es evidencia local de almacenamiento y
protocolo; aún falta probar audio y GPS mediante radios físicos, cortes y tres
saltos. La prueba Flutter también entrega el mismo mensaje conversacional por
dos rutas simuladas y conserva una única burbuja por su ID lógico. `cargo test
--workspace`, el contrato Swift, `flutter analyze` y 15 pruebas Flutter
aprobaron. No se instaló ningún paquete durante este incremento.

La segunda prueba funcional crea A→B→C con un roster real de tres miembros:
B guarda anuncio y chunks, se libera por completo y abre de nuevo su SQLCipher;
después reconstruye su cola, reenvía a C y C recupera exactamente la misma nota
de voz. Los receipts firmados de B y C regresan por la ruta y A conserva su
outbox tras el primero, retirándolo sólo tras el segundo. Esto valida la cadena
de persistencia y entrega en el bridge Rust/FFI. El salto de vuelta del receipt
se prueba mientras B está vivo; la custodia persistente de un receipt
intermedio y la campaña de radios físicos siguen siendo huecos explícitos.

## Custodia durable de receipts en relay

**Actualizado: 2026-09-14. Avance global: 55%; Wi‑Fi Aware: 88%.**

La migración SQLCipher 007 introduce una cola específica para receipts que
atraviesan un relay. Guarda el receipt firmado, su `RelayFrame`, el vecino de
entrada, TTL y momento de custodia; deduplica por SHA‑256 y elimina sólo los
vencidos. No necesita los chunks del objeto original, por lo que puede retener
una confirmación de C aunque B no sea el origen. C/JNI/Swift exponen una lectura
host-only y Android/iOS la reintentan únicamente hacia vecinos autenticados que
no sean el de entrada.

La prueba A→B→C ahora mata B una segunda vez, justo después de que recibe el
receipt de C. Al reabrir SQLCipher, B reconstruye ese receipt y A retira su
outbox sólo al recibirlo. La prueba no representa aún un radio físico ni añade
un ACK de receipt para detener su reintento antes de expiración. Los paquetes
release Android/iOS, el XCFramework, 107 pruebas Rust, contrato Swift,
`flutter analyze` y 15 pruebas Flutter aprobaron; no se instaló ningún paquete.

## ACK firmado para detener reintentos de receipt

**Actualizado: 2026-09-14. Avance global: 56%; Wi‑Fi Aware: 89%.**

El origen ahora firma un `ReceiptAck` separado del receipt, con dominio
criptográfico propio, grupo/época, objeto, actor y SHA‑256 del receipt que ya
confirmó en SQLCipher. El ACK no puede reutilizarse como evidencia de entrega.
Al recibirlo, el destinatario comprueba la firma certificada del origen y el
digest contra su receipt local antes de registrar `receipt_acknowledgements`.
Conserva evidencia de la entrega, pero el scheduler deja de reintentarla. Un
relay valida el mismo ACK y retira sólo la custodia pública del receipt
coincidente.

La prueba funcional A→B→C cubre: voz cifrada que sobrevive el reinicio de B,
receipt de C custodiado por B, ACK de A que regresa por B, rechazo de un ACK
alterado y cese de la cola de receipts de C. Pasaron las 107 pruebas Rust y la
compilación de los binarios nativos iOS/Android. En ese corte aún faltaba la
custodia durable del ACK; quedó incorporada en el bloque siguiente. Sigue
pendiente únicamente la prueba física multihop.

## Custodia durable del ACK en relay

**Actualizado: 2026-09-14. Avance global: 57%; Wi‑Fi Aware: 90%.**

La migración SQLCipher 009 añade una cola de custody para `ReceiptAck`. Un
relay guarda el ACK verificado, el frame transformado, el vecino de entrada y
su TTL antes de enviarlo a otros enlaces. Al autenticarse un vecino después de
un reinicio, Android e iOS drenan también las colas de objetos, receipts y ACKs
sin reenviar al ingreso original.

La prueba funcional A→B→C ahora reinicia B una tercera vez: después de guardar
el ACK de A para C y antes de entregarlo. B lo reconstruye desde SQLCipher, C
rechaza una versión alterada, acepta la original y deja de reintentar el
receipt. Pasaron 107 pruebas Rust, los contratos Swift, `flutter analyze`, 15
pruebas Flutter y los builds release Android/iOS. No se instaló ningún paquete;
la validación física multihop sigue pendiente.

## Resumen durable de entrega por acción de grupo

**Actualizado: 2026-09-14. Avance global: 58%; Wi‑Fi Aware: 90%.**

La migración SQLCipher 010 asocia cada objeto de audiencia con una sola acción
lógica local. En un roster de 50, las cinco audiencias permanecen cifradas y
acotadas, mientras el origen mantiene un único resumen: en cola, parcial o
entregado. El último estado sólo ocurre cuando los 49 destinos han emitido sus
receipts verificados; un receipt aislado no cambia la burbuja a entregada.

La integración FFI prueba una conversación A→B→C: B confirma y el resumen queda
parcial; después de que C confirma, pasa a entregado. La misma suite verifica
la partición 10+10+10+10+9 para 50 miembros y una sola acción pendiente. Pasaron
107 pruebas Rust, contratos Swift, análisis Flutter, 15 pruebas Flutter y builds
release Android/iOS. No se instaló nada. El siguiente paso es proyectar ese
resumen de evidencia en la burbuja del chat sin inventar estados de entrega.


## Evidencia de entrega dentro del chat

**Actualizado: 2026-09-14. Avance global: 59%; Wi‑Fi Aware: 90%.**

Cada mensaje local nuevo obtiene un ID lógico hexadecimal de 16 bytes antes de
entrar al outbox. El bridge Android/iOS consulta SQLCipher usando ese mismo ID,
nunca el “último mensaje” global. La burbuja conserva “En cola segura”,
“Entregado a X de Y” o “Entregado” solamente cuando el resumen de receipts
firmados cambia. Los mensajes recibidos no muestran estado de entrega porque
ese teléfono no es la autoridad que conoce al grupo completo.

La prueba Flutter `origin receipt updates only its matching chat bubble` envía
dos mensajes y confirma que la evidencia parcial del primero no altera el
segundo. La prueba FFI para el roster de 50 consulta el ID exacto, verifica los
49 destinos y prueba que un ID distinto no devuelve el estado más reciente.
Pasaron 107 pruebas Rust, el contrato Swift, `flutter analyze`, 16 pruebas
Flutter y builds release Android/iOS. No se instaló nada en dispositivos. Falta
la campaña física multihop y acordar los estados de expiración/sin ruta.


## Caducidad explícita de entrega

**Actualizado: 2026-09-14. Avance global: 60%; Wi‑Fi Aware: 90%.**

Una consulta de entrega por ID lógico ahora conserva la evidencia de caducidad
en SQLCipher. Al vencer la ventana durable de 15 minutos, el chat deja de
consultar esa acción y muestra “Venció sin confirmación” o el conteo parcial
que sí alcanzó receipts firmados. No cambia un mensaje vencido a entregado ni
lo confunde con un ID inexistente.

La prueba FFI de 50 miembros avanza el reloj a la caducidad y exige el estado
explícito `expired`; la prueba del controlador proyecta ese resultado y deja
de hacer polling de la burbuja. Siguen pendientes la campaña física multihop y
la política de producto para volver a enviar una acción vencida.

## Integración Convoy — C2: política híbrida por acción

**Actualizado: 2026-09-15. Avance global: 60%; Wi‑Fi Aware: 90%; C2 Convoy: 25%.**

En el branch `codex/mesh-field-sdk-integration` de Convoy se añadió una
política pura que toma una acción con ID lógico persistido, el último ACK real
del backend y el estado de una sesión mesh autenticada. Decide de forma
determinista entre Internet, mesh, ambos canales o cola. Un ACK reciente usa
Internet; un ACK ausente, vencido o futuro inicia ambos canales con el mismo ID
para que una reconciliación posterior no duplique la acción.

No envía contenido todavía ni conecta Supabase con el SDK. Esa ejecución queda
deliberadamente pendiente hasta que el SDK exponga inscripción firmada y
contenido entrante con autor, ID lógico y hora verificables, y hasta cerrar los
gates S2–S6. Pasaron seis pruebas directas de la política y las 21 pruebas del
módulo nearby-mesh de Convoy; `flutter analyze` no reportó problemas.

## SDK — ID lógico aportado por el producto

**Actualizado: 2026-09-15. Avance global: 60%; contrato SDK–Convoy: 30%.**

`mesh_field_sdk` 0.2.1 incorpora la capacidad opcional
`FieldMeshActionSender`. Permite que una aplicación aporte el ID lógico
persistido para texto, ubicación y voz sin romper a quien sólo implementa
`FieldMeshSdk`. El ID debe ser el formato canónico de 128 bits en hexadecimal;
si no lo es, el SDK rechaza la acción y nunca crea un ID paralelo. Este
contrato prepara la reconciliación Internet+mesh por acción, pero no habilita
todavía el despacho funcional de Convoy ni modifica los gates S2–S6.

La prueba de fachada exige que texto, ubicación puntual y voz preserven el
mismo ID hasta el host. Pasaron 18 pruebas y `flutter analyze` del SDK, además
de las 26 pruebas y el análisis limpio de Mesh Lab. No se instaló ningún
paquete.

Convoy también transforma únicamente su UUID v4 persistido a la representación
sin guiones que requiere el SDK, manteniendo ambas como la misma acción. Las
pruebas rechazan UUIDs que no sean canónicos en minúscula, de una versión o
variante distinta, y texto arbitrario; por tanto no existe un fallback que
invente otro ID de mesh.

## SDK — voz entrante certificada por objeto

**Actualizado: 2026-09-16. Avance global: 64%; bloque SDK–Convoy: 88%; Wi‑Fi Aware: 90%.**

El host ya no reduce cada recepción de voz durable a “la última nota”. El
completion packet v2 se interpreta en Android e iOS sólo después del commit de
receipt; una voz v2 lleva su ID lógico dentro del payload cifrado y se almacena
privadamente bajo el ID de objeto certificado. Los hosts drenan una FIFO de
`VerifiedIncomingVoice` por Pigeon con autor, objeto, ID lógico, instante y
duración. El SDK añade `FieldMeshVerifiedIncomingVoiceSource`, vuelve a validar
cada identificador, duración y hora, y sólo permite reproducir por el ID de
objeto devuelto. Ningún byte de audio, ruta de archivo, frame, receipt o clave
entra a una aplicación consumidora.

Pasaron 25 pruebas del SDK, `flutter analyze` limpio en `mesh_field_sdk` y
`mesh_host`, la regeneración Pigeon reproducible y cobertura de las unidades
nuevas/modificadas: `field_mesh_client.dart` 293/312 (93.9%) y
`field_mesh_models.dart` 17/17 (100%). La compilación Release Android terminó correctamente y no se instaló ningún
paquete. Sigue pendiente la prueba física Android↔iOS con dos voces recibidas
fuera de orden y la adaptación de la nota certificada a la conversación de
Convoy; por ello no se aumenta el global.

## SDK — contexto cifrado de voz para productos

**Actualizado: 2026-09-16. Avance global: 64%; bloque SDK–Convoy: 89%; Wi‑Fi Aware: 90%.**

Una voz certificada ahora puede transportar un contexto de producto de hasta
512 bytes dentro del mismo objeto durable cifrado y firmado. El formato v3
preserva autor, objeto, ID lógico, instante, duración y contexto tras el commit
local de receipt; el formato v2 previo sigue siendo legible sin contexto. El
host no interpreta ese contenido para rutas, radio o autorización. Android e
iOS rechazan contexto vacío, UTF‑8 inválido, tamaños fuera de límite y objetos
que excedan el techo SQLCipher antes de crear la cola o escribir audio.

`FieldMeshVoiceContextSender` permite que Convoy adjunte su sobre de alcance a
una voz y `FieldMeshVerifiedIncomingVoiceSource` lo devuelve sólo tras la
verificación nativa. Pasaron 26 pruebas del SDK, análisis Flutter limpio,
reproducibilidad Pigeon y cobertura de `field_mesh_client.dart` 302/323
(93.5%) y `field_mesh_models.dart` 17/17 (100%). Las compilaciones Release
Android e iOS aprobaron sin instalar ni abrir paquetes. El siguiente bloque
adapta la nota a la conversación/lector de Convoy preservando el reproductor
privado por ID de objeto; sigue pendiente la prueba física de dos voces fuera
de orden.

## Integración Convoy — voz certificada privada

**Actualizado: 2026-09-16. Avance global: 64%; bloque SDK–Convoy: 90%; Wi‑Fi Aware: 90%.**

Convoy ya adapta una nota de voz certificada al historial existente sólo cuando
el sobre cifrado declara el convoy activo, el autor Field coincide con la
inscripción confiable y el usuario aún pertenece a la sesión. La burbuja
conserva el UUID lógico de Convoy, duración y el ID de objeto certificado; no
recibe bytes, ruta local ni URL. Al tocar reproducir, solicita al SDK que el
host nativo privado reproduzca exclusivamente ese ID. Texto y GPS conservan
su flujo actual, y un sobre que diga `voice` sin objeto certificado —o un
objeto de voz con otro tipo de sobre— se descarta.

La salida toma los bytes mientras el grabador aún posee el temporal y, si la
nota dura de uno a ocho segundos, la refleja como segundo canal usando el
mismo UUID que el outbox de Internet. El host recibe el audio y el sobre
sellado juntos; la subida a Supabase continúa siendo independiente y no se
bloquea si la radio está ausente. Se añadió control de ciclo de vida de los dos
flujos certificados para que una fuente de voz o texto terminada no deje una
suscripción de producto abierta.

Pasaron las **87** pruebas de `test/features/nearby_mesh` y la burbuja de
voz de Convoy, incluidas pruebas funcionales de recepción de voz, sellado,
reproducción por ID privado, proyección al historial y un segundo toque que no
simula detener el reproductor nativo. El nuevo servicio puro
`ConvoyMeshVoiceMirror` separa del outbox el límite de duración, los bytes y la
identidad inmutable: sus pruebas exigen que una nota conserve el UUID, convoy,
autor y duración, que audio vacío o mayor de ocho segundos no inicie radio y
que un error de la radio devuelva control al outbox de Internet sin propagar un
fallo de entrega.
Las líneas nuevas del widget mesh están cubiertas; su cobertura total incluye
ramas heredadas de URL y almacenamiento. La cobertura medida de los módulos
modificados fue: repositorio 130/134 (**97.0%**), codec 40/41 (**97.6%**),
proyector 22/22 (**100%**), controlador 121/126 (**96.0%**) y mirror 6/6
(**100%**). `flutter analyze --no-fatal-infos` no reportó errores. Tres
invocaciones Release Android de Gradle se cancelaron sin artefacto ni
instalación; la última quedó esperando un daemon Gradle compartido que ya estaba
ocupado, así que se detuvo sin afectar ese proceso. La compilación se repetirá
cuando el daemon esté libre. Sigue pendiente la prueba física de dos notas de
voz fuera de orden y de su reproducción individual por ID.


### Pasarela certificada Convoy → Internet

**Actualizado: 2026-09-21. Implementación local: 60%; gates remoto y físico: pendientes.**

El SDK ya firma una prueba de relevo ligada al grupo Field, época, autor, UUID y contenido canónico. Convoy registra la línea pública del grupo cuando el líder prepara o recupera su política y un vecino con cobertura conserva texto ajeno certificado en una cola durable. El servicio remoto sólo acepta ese texto si verifica la firma Ed25519 original, miembro, convoy, participación de puente y autor, ventana de cinco días, grupo/época vigentes e idempotencia. El relé no conoce ni recibe secretos Field y nunca puede sustituir al autor.

Las pruebas Rust existentes protegen la prueba del SDK; Convoy añade contratos Deno de firma y registro, pruebas unitarias de codec, proyector, adaptadores HTTP, compatibilidad, persistencia y recuperación de cola, y dos E2E locales que recorren recepción certificada→proyección→chat→cola durable→frontera de nube y rechazo por autor distinto. La campaña local suma 178 pruebas cercanas aprobadas con 90.6% de cobertura del bloque. Quedan aplicar la migración y dos funciones Edge en el remoto, ejecutar el E2E contra la base desplegada —la reconstrucción local está bloqueada por migraciones históricas duplicadas—, probar tres teléfonos y extender el relevo de servidor a voz y GPS.

## Contrato de radios para aplicaciones consumidoras

**Actualizado: 2026-09-21. Plan completo: 80%; bloque de prueba física:
96%.**

El plugin Android ahora exporta desde su propio manifiesto todos los permisos y
features opcionales que necesita el host: BLE scan/connect/advertise, Nearby
Wi-Fi Devices, estado y cambio de Wi-Fi/red, compatibilidad anterior a Android
12 y el servicio foreground de dispositivo conectado. Esto evita que una app
consumidora compile correctamente pero quede silenciosamente sin radio por
haber omitido declaraciones duplicadas.

El APK release de Convoy confirmó esas entradas en el manifiesto fusionado. En
el SM A736B se instalaron y concedieron los permisos, y el registro nativo
confirmó `Mesh Lab GATT service is ready`, `Mesh Lab advertising started` y,
con el iPhone en el mismo convoy, las dos etapas Noise y
`Secure link authenticated`. Wi-Fi Aware en la distribución iOS continúa como
gate separado: el código y el entitlement están declarados, pero el perfil de
provisión actual de Convoy todavía no incluye la capacidad concedida por Apple.

## Entrega inmediata de texto, voz y GPS en Convoy

**Actualizado: 2026-09-21. Plan completo: 81%; bloque de contenido por Malla:
92%.**

La observación física de que un mensaje aparecía únicamente al restablecer el
Wi-Fi confirmó que el backend estaba recuperando la acción, pero la recepción
cercana se perdía en la interfaz. El host Android había registrado
`Certified durable text committed locally`; Convoy drenaba ese evento único
mientras la lista autorizada seguía cargando, lo proyectaba contra un roster
vacío y lo descartaba. El adaptador ahora conserva hasta 64 acciones
certificadas durante esa ventana, vuelve a leer el roster actual y entrega
texto, voz y GPS solamente después de resolver autorización y participación.
No relaja la vinculación de identidad Field con el usuario y convoy activos.

La voz saliente también excedía el presupuesto del objeto cifrado: el grabador
producía WAV PCM de aproximadamente 320 KiB para diez segundos frente al techo
nativo de 48 KiB. Convoy ahora captura AAC-LC mono a 16 kHz y 24 kbps, permite
hasta diez segundos y reserva margen para el encabezado y contexto autenticado.
SDK, Android e iOS comparten los mismos límites: diez segundos y 47 KiB de
audio codificado antes del sobre durable.

La revisión de GPS confirma su recorrido funcional sin Internet: valida la
lectura, limita emisiones cercanas a una cada cinco segundos, cifra convoy,
autor, UUID y hora de captura, y actualiza el marcador del participante tras la
misma autorización diferida. La suite conjunta aprobó 59 pruebas enfocadas,
incluida una que recibe texto, voz y GPS antes de resolver el roster y exige su
entrega posterior en orden; el SDK aprobó 34 pruebas y análisis limpio. Los
builds release Android e iOS finalizaron; iOS quedó instalado y abierto. El
entitlement Wi-Fi Aware permanece en el código fuente, aunque se retiró sólo
del artefacto iOS local porque el perfil de provisión todavía no lo concede.
Android no estaba visible por ADB y su APK actualizado quedó pendiente de
instalación. No se aumenta el porcentaje global hasta repetir texto/voz en dos
teléfonos sin Internet y ejecutar la validación física posterior de GPS.

## Presencia visual híbrida en el mapa de Convoy

**Actualizado: 2026-09-21. Plan completo: 81%; bloque de presencia visual:
70%.**

La pantalla aplicaba blanco y negro a todos los participantes cuando fallaba el
probe de Internet, incluso con una sesión Malla autenticada. El mapa ahora
considera utilizables ambos caminos: conserva las fotos a color si existe ruta
por Internet o por Malla y fuerza el tratamiento desconectado sólo cuando
faltan las dos. El punto del marcador mantiene una función distinta y útil:
verde, amarillo o gris según la antigüedad de la última ubicación recibida; no
pretende identificar el transporte.

Pasaron 14 pruebas de cámara y marcadores, incluida la tabla de verdad para
Internet, Malla, ambos y ninguna ruta. Sigue pendiente exponer al producto la
presencia firmada por miembro. El estado Malla actual es agregado para este
teléfono y no permite afirmar cuál de hasta 50 participantes es alcanzable de
forma directa o mediante saltos; esa distinción requiere un contrato SDK por
participante y su TTL antes de convertirla en UI individual.

## Transición de cámara entre participante y grupo

**Actualizado: 2026-09-21. Plan completo: 81%; bloque de transición de cámara:
90%.**

Los controles de “mi ubicación” y “grupo” comparten ahora una transición de
500 ms con curva `easeInOutCubic`. El movimiento interpola centro, zoom y
rotación desde la pose visible, y una nueva pulsación cancela la animación
anterior para que no se acumulen recorridos. El encuadre del grupo calcula
primero la cámara que contiene los marcadores y su padding, y después se anima
hasta ella; antes aplicaba `fitCamera` de inmediato y producía un salto frente
al centrado individual.

Pasaron 16 pruebas de cámara, controles y HUD. La prueba del contrato exige
medio segundo, extremos exactos y progresión suave de la curva. También se
generaron correctamente el APK Android release de 147,1 MB y la app iOS release
de 83,6 MB. El entitlement Wi-Fi Aware quedó intacto en el código fuente; se
retiró únicamente durante la firma del artefacto iOS local porque el perfil de
provisión todavía no lo concede. Queda instalar en dispositivos y validar la
sensación del movimiento a 60/120 Hz: no había Android visible por ADB y el
iPhone aparecía no disponible al cerrar esta revisión física.

## Frescura de la última ubicación GPS en Convoy

**Actualizado: 2026-09-21. Plan completo: 81%; bloque de frescura GPS: 75%.**

El punto del marcador usa ahora intervalos de cinco minutos y una sola regla
compartida por el dominio y la pantalla en vivo: verde hasta cinco minutos,
amarillo después de cinco y hasta diez minutos, y gris después de diez
minutos. Los límites exactos conservan el tramo anterior: a los 5:00 sigue
verde y a los 10:00 sigue amarillo; el cambio ocurre al superar cada límite.
La antigüedad describe únicamente la última ubicación GPS y no el transporte
Internet/Malla disponible para comunicarse con el participante.

Pasaron 21 pruebas de frescura, tratamiento visual, colores del punto, cámara y
marcadores. Se cubren los límites de cinco y diez minutos y el mapeo real a
verde, amarillo y gris. Queda generar los artefactos release e instalar en los
teléfonos para comprobar el cambio temporal en una sesión real.

## Canal de contenido listo y recuperación GPS de Android

**Actualizado: 2026-09-21. Plan completo: 81%; bloque de entrega física por
Malla: 90%.**

La investigación en el SM-A736B confirmó que BLE terminaba las dos etapas Noise
y autenticaba el enlace, pero Convoy no invocaba la cola durable del SDK. El
emisor repetía una autorización dependiente del servidor justo antes de cada
GPS, texto o audio; al quedar desactualizada durante una pérdida de Internet,
bloqueaba contenido ya autorizado por el grupo nativo y por la validación del
receptor. Se retiró ese bloqueo redundante. Las comprobaciones estrictas de
convoy, autor Field, participación activa, vigencia y deduplicación permanecen
en el host cifrado y en cada receptor. Un fallo de la firma opcional de relevo a
Internet tampoco cancela ahora el envío cercano.

El semáforo verde exige desde este cambio un enlace nativo autenticado, al menos
un vecino de radio y un roster certificado vigente que cubra a los
participantes activos. Mientras la radio ya está enlazada pero el canal de
contenido aún prepara su autorización, la interfaz permanece amarilla y muestra
“Preparando el canal seguro del convoy”. Así el producto no promete una entrega
que todavía no puede aceptar.

Android recupera además el GPS después de un reinicio frío: el manifiesto final
conserva `ACCESS_FINE_LOCATION` en versiones modernas, la entrada al convoy
vuelve a solicitar precisión cuando una instalación anterior sólo obtuvo una
ubicación aproximada, y el servicio detiene tanto el isolate actual como uno
heredado antes de arrancar una sesión nueva. El APK release quedó instalado con
ubicación precisa concedida; la pantalla física volvió a mostrar “GPS activo” y
el marcador propio.

Pasaron **71 pruebas enfocadas**. Incluyen el flujo funcional completo de
mensaje rápido, reacción, GPS y voz por un enlace certificado; dos carriles
Internet/Malla independientes; firma opcional fallida; roster incompleto o
vencido; semáforo amarillo hasta que contenido esté listo; permiso preciso y
recuperación del servicio de ubicación. Los builds release Android e iOS
finalizaron y quedaron instalados. El artefacto local de iOS usa BLE porque el
perfil de provisión aún no concede Wi‑Fi Aware; el entitlement WFA permanece en
el código. Falta la confirmación física final, con ambos teléfonos dentro de
`Testing` y sin Internet, de recepción en iPhone del marcador Android, mensaje
rápido, reacción y audio.

## Prioridad de contenido durante ráfagas de reconexión

**Actualizado: 2026-09-21. Plan completo: 81%; bloque de entrega física por
Malla: 98%.**

La prueba física confirmó el primer extremo completo: el marcador GPS del
Android ya aparece en el mapa del iPhone. También aisló la causa de la latencia
de mensajes rápidos. Durante una interrupción, Convoy creaba una acción durable
de ubicación cada cinco segundos; al reconectar, el almacén las recorría por ID
de objeto y los hosts sólo conservaban 64 entregas certificadas para Flutter.
Una ráfaga extensa podía retrasar el contenido interactivo o expulsarlo antes
de que el producto lo drenara.

Convoy mantiene ahora una sola ubicación durable pendiente y combina en ella
los ticks posteriores hasta tener entrega o vencimiento. El almacén sirve
primero las acciones lógicas más recientes y deja al final únicamente objetos
heredados sin vínculo lógico. Android e iOS conservan hasta 4096 entregas
certificadas de texto y voz, el mismo orden de magnitud que el límite durable,
para que una descarga de reconexión no desplace mensajes, reacciones o audio.

Pasaron **45 pruebas Rust**, **35 pruebas del cliente Flutter del SDK** y **52
pruebas enfocadas de Convoy**. La suite del SDK incluye una ráfaga de 256
acciones certificadas y exige que todas lleguen una sola vez, incluido el
mensaje interactivo situado entre ubicaciones. Las versiones release quedaron
instaladas en Android e iPhone. En Android, `Espérenme` fue aceptado en la cola
a las 21:52:29.144 y comenzó su notificación BLE a las 21:52:29.573: 429 ms.
Queda observarlo en la interfaz receptora y repetir reacción y voz en ambos
sentidos. El iPhone quedó fuera de la consola de desarrollo al apagar Wi-Fi y
no estar conectado por USB; este gate físico no aumenta el porcentaje global.

## Corrección de colas BLE, voz fragmentada y cadencia GPS

**Actualizado: 2026-09-22. Plan completo: 81%; bloque de entrega por Malla:
90% estimado, reabierto tras el fallo físico.** El 98% anterior no acreditaba
recepción ni latencia extremo a extremo. Los 429 ms anteriores medían la
aceptación de una notificación para envío; no demostraban entrega completa.

La inspección encontró escrituras ATT concurrentes al añadir nuevas acciones,
reinyección de todo el outbox en cada envío y una bomba periférica iOS que
enviaba sólo un fragmento aunque CoreBluetooth hubiera aceptado más. Además,
el origen iOS sólo seleccionaba el primer enlace cliente BLE y omitía enlaces
en los que actuaba como periférico. Esto explica mecanismos de bloqueo y
congestión; su resolución física permanece pendiente de medir.

Los dos hosts usan ahora una cola con un único fragmento ATT en vuelo. iOS
periférico continúa hasta que CoreBluetooth indica saturación, conserva el
fragmento rechazado y lo retoma cuando recibe disponibilidad. Android cierra
un transporte fallido para recuperar desde el almacén durable en una conexión
nueva, en lugar de descartar silenciosamente sus fragmentos. La cola por
sesión evita reinsertar registros durables ya programados, permite el replay
tras reconectar y pone acciones nuevas delante de la cola pendiente antes de
asignar contadores Noise. No reordena fragmentos ni ciphertext ya generado.

Convoy limita GPS por malla a una emisión cada **seis segundos**. Cuando una
ubicación ya fue recibida por algún participante, permite publicar la actual
sin esperar a todos los miembros desconectados; conserva la contrapresión si
todavía no hay ninguna entrega. Internet conserva su cadencia independiente.

Validación: **45 pruebas Rust**, con voz durable ampliada a **47 KiB**;
**13 pruebas Kotlin/JNI**; suite Swift nativa con fragmentación de 48 KiB,
backpressure, nuevas acciones durante escritura en vuelo, cola de 520 registros,
prioridad frente a 100 pendientes y reconstrucción de sesión; **53 pruebas de
Convoy** de contenido, recepción, outbox y GPS. Cobertura medida de líneas:
**100%** en las clases nuevas `BleWriteQueue`/`DurableRecordHistory` Kotlin
(JaCoCo) y Swift (llvm-cov), y **96,2%** en el mirror GPS (Flutter). No es la
cobertura de todo el adaptador nativo ni sustituye las pruebas con radio.

Ambos builds release compilaron y se instalaron; Android se recuperó por mDNS
en `192.168.100.171:42833` y abrió `Testing` con GPS activo. El iPhone se instaló
y abrió correctamente antes de perder acceso de consola. El entitlement WFA
del fuente permanece intacto; el artefacto iOS local sigue usando BLE por el
perfil de provisión disponible. Pendiente: verificar ambos sentidos sin salida
a Internet, medir aceptación → recepción certificada → UI para mensajes cortos,
reproducir audio recibido, confirmar GPS renovado y repetir con reconexión.
No se aprueba el gate físico ni se eleva el avance global por estas pruebas locales.

### Seguimiento físico del 22 de septiembre

**Plan completo: 81%; bloque de comunicación: 70% estimado.** Se reabre la
validación del bloque después del reporte de Android en «Conectando» sin
entrega. Posteriormente el usuario confirmó: «ahora están llegando» y sugirió
que aún no estaban instalados los últimos cambios. Esa posible causa no está
confirmada: el registro de trabajo anterior sí acredita instalaciones release,
pero no identifica qué versión estaba abierta durante cada intento del usuario.

La recepción reportada es evidencia parcial, sin especificación del tipo de
contenido, sentido, latencia ni aislamiento de Internet. Los registros nativos
también muestran registros BLE recibidos y texto durable confirmado localmente;
eso por sí solo no acredita su presentación en la UI. Queda probar mensajes
rápidos, reacciones, audio reproducible y GPS actualizado en ambos sentidos sin
Internet, y repetir después de desconectar/reconectar Bluetooth. No se añaden
cambios de transporte basados únicamente en el fallo previo a esta confirmación.

### Audio de Convoy, confirmaciones repetidas y estado GPS

**2026-09-22 — Plan completo: 81%; bloque de comunicación: 75% estimado.**
El usuario confirmó mensajes cortos funcionando bien y GPS aparentemente
actualizándose, pero audio sin funcionar. La revisión encontró dos defectos
independientes: los eventos certificados de voz de Malla se agregaban al
historial sin pasar por el receptor de radio, y éste sólo reproducía URLs de
Internet. Ahora usa el audio local certificado, respeta silencio, serializa
notas y comparte la deduplicación con la llegada por Internet.

Los registros de Android muestran voces aceptadas en 15–20 registros durables y
ráfagas extensas de confirmaciones. Cada consulta del outbox volvía a firmar
ACKs con la hora actual, generando bytes e identidades de ruta nuevas. El SDK
persiste una confirmación por recibo antes de transmitirla; la repite idéntica
tras nuevos drenajes y reinicios. La migración local cifrada 10→11 conserva
operaciones existentes y no cambia Supabase ni la app pública. La expiración
elimina también el ACK mediante su relación con el recibo. La prueba de voz
de 47 KiB ahora verifica 20 consultas posteriores, reinicio, recepción del ACK
y retiro del recibo pendiente.

El marcador de foto calculaba amarillo/gris tanto por GPS como por estado del
servidor. Ahora su punto y foto usan la ubicación recibida por cualquier ruta:
verde hasta cinco minutos, amarillo después de cinco, gris después de diez;
la salida explícita del participante sigue respetándose. GPS en Malla mantiene
la cadencia de seis segundos.

Validación: **46 pruebas Rust** (incluida migración 10→11), **13 Kotlin/JNI**,
suite Swift de contrato/fragmentación y **28 pruebas Flutter** de radio,
autoplay local, duplicados, silencio, serialización, recepción autorizada y GPS.
Analizador Flutter sin incidencias en los cinco archivos de producción
modificados. Cobertura de líneas nuevas medidas: **91,7%** en la función de
ACK persistente y **97,2%** en los cambios Dart cubiertos por LCOV. Los logs
nativos ahora identifican explícitamente voz reconstruida y guardada, sin
imprimir el contenido. Ambos builds release compilaron y se instalaron el
22 de septiembre: Android confirmó instalación y arranque de su proceso nuevo;
CoreDevice confirmó instalación y lanzamiento de iPhone. El entitlement WFA
del fuente quedó restaurado; el build local iOS usa BLE con el perfil disponible.
Pendiente: recepción/reproducción física de audio en ambos sentidos sin
Internet; las pruebas automáticas no cierran ese gate.

### Límite de voz en iOS, notas largas y GPS fuera de orden

**2026-09-22 — Plan completo: 81%; comunicación: 78% estimado.** El usuario
confirmó audio iPhone→Android, con fallo aparente por encima de ocho segundos;
Android→iPhone seguía sin audio y el punto GPS de Android aparecía amarillo.
Android registró voces recibidas de 31–47 KiB y notas propias aceptadas de
9–19 KiB. Esto permite separar recepción real de mera aceptación en cola.

Causa confirmada en el host iOS: `finalizeNextDurableText` utilizaba el límite
de 4163 bytes de un fragmento también para el objeto de voz reconstruido.
Rust ya había confirmado la entrega cuando Swift descartaba el resultado.
Ahora esa llamada acepta 48 KiB más el encabezado y recibo, manteniendo
estricto el límite de cada trama de radio. Las pruebas de la frontera Swift
incluyen 4163/4164 bytes, 19 KiB y 48 KiB más metadatos, búferes inválidos y
estado de error. Cobertura del nuevo `NativeOutput`: 100% de líneas/regiones.

Se reprodujo localmente el relleno de un contenedor AAC de Apple: un tono
sintético de diez segundos pasó de 31 441 a 8 428 bytes al retirar `free` y
reubicar `stco`/`co64`. `afconvert` produjo WAV idénticos antes y después
(SHA-256 idéntico): no se recodifica ni se reduce calidad. Convoy compacta sólo
contenedores ordinarios reconocidos; diseños desconocidos o malformados quedan
intactos. También se corrige el redondeo del temporizador: una parada a
10 001 ms ya no etiqueta la nota como once segundos y provoca su rechazo.

La actualización de GPS descarta muestras anteriores a la ya mostrada, para
que reenvíos de Malla o recuperación por Internet no hagan retroceder el punto.
No convierte una ubicación realmente antigua en fresca. Se conserva amarillo
después de cinco minutos y gris después de diez; queda comprobar físicamente
la renovación continua de GPS del Android en el iPhone.

Validación: 24 pruebas Flutter de compactación, grabación, marcador y recepción
autorizada; la prueba integrada de GPS también verifica replay fuera de orden.
Compactador: 100% de líneas medidas. Suite Swift nativa aprobada, con nueva
frontera de entrega y límites de radio conservados. Analizador sin incidencias
en los cinco archivos revisados. Builds release compilados e instalados en
ambos teléfonos; Android se recuperó por mDNS en el puerto 46217 e iPhone
confirmó instalación a las 07:53. Pendiente repetir audios NUEVOS de 3 y
9–10 segundos en ambas direcciones. Las notas que el iPhone
ya descartó tras confirmar en Rust no validan una retransmisión de la versión
corregida.


### Feedback de radio, audios en cola y pantalla activa

**2026-09-22 — Plan completo: 81%; comunicación: 78% estimado.** El usuario
confirma que los audios llegan bien sin Internet usando sólo Malla. Esta
confirmación se registra como evidencia física de entrega; no certifica por
sí sola todas las duraciones, tres emisores físicos, multihop ni background.
Se conservan los porcentajes hasta cerrar esos escenarios y la revisión visual.

El indicador previo dependía de `isRecording`, difundido por Internet. La
reproducción nativa de una voz certificada de Malla no activaba ese estado.
Ahora el receptor publica el audio que está reproduciendo por convoy, después
de que el host confirma el inicio. La vista superpone el micrófono al marcador
y muestra el nombre del remitente; el aviso permanece visible hasta terminar.
El estado es local de presentación, no modifica el roster ni la frescura GPS.
Internet utiliza el mismo feedback después de cargar la voz y el sonido previo.
Fallos de carga no anuncian un hablante y la finalización limpia el aviso.

La cola común ya serializaba recepción Internet/Malla. Una nueva prueba
funcional de widgets inyecta tres remitentes simultáneos y un duplicado:
comprueba el orden de reproducción, un nombre a la vez, micrófonos, duración
visible y desaparición al finalizar. Se mantiene el orden local de recepción;
no se promete un orden idéntico de audios entre teléfonos con latencias distintas.
Malla conserva la espera por duración certificada porque el host actual
confirma inicio, no expone un evento de finalización al receptor Flutter.

La vista Convoy activa `wakelock_plus` al entrar, lo reafirma al volver a primer
plano y libera su solicitud al salir. Un servicio compartido conserva las
solicitudes de grabación de rutas: terminar una función no desactiva el bloqueo
requerido por la otra. Las llamadas nativas se serializan para entradas/salidas
rápidas. Esto evita el apagado automático con Convoy abierto; no impide bloquear
manualmente el teléfono ni constituye soporte nuevo de ejecución en background.

Validación: 30 pruebas Flutter aprobadas (recepción, cola, feedback funcional,
fallos/mute/duplicados, wake lock compartido y controlador real de grabación
de rutas). Cobertura diferencial de líneas instrumentadas: 48/53 (90,57%);
quedan sin cubrir automáticamente cuatro puntos de conexión en la pantalla
completa y una línea preexistente reformateada del reproductor. LCOV guardado
en `/tmp/convoy-radio-feedback-lcov.info`. Cobertura de líneas del aviso
42/42 y del servicio de pantalla 11/11; las nuevas líneas de estado de
reproducción están cubiertas. Análisis de los archivos revisados sin errores
ni warnings, con recomendaciones de estilo. iOS release compiló (52 s) y se instaló con éxito por CoreDevice a las 08:26,
con el perfil local BLE habitual; el entitlement WFA del fuente quedó restaurado.
Android release compiló (40,3 s) y ADB confirmó instalación USB en el A73.
Ambas versiones release están instaladas; queda revisión física del feedback
y del bloqueo de pantalla. Código de Convoy: `d943cd9`. La primera compilación Android encontró
metadata.bin ausente en la caché Gradle. El directorio de caché ya no existía,
pero el daemon seguía referenciándolo; se reinició el daemon inactivo para
regenerarla sin cambiar las dependencias del proyecto.

La reconstrucción Android también detectó que el registro generado de plugins
había incorporado dependencias de pruebas mientras Gradle compilaba release.
Se finalizaron todas las pruebas y se inició una nueva compilación release
secuencial, regenerando el registro sin esos plugins de desarrollo. No se
modificaron dependencias ni versiones para resolverlo.


## 2026-09-22 — Recuperación de BLE tras reiniciar Convoy

**81% global estimado · 78% comunicación; porcentajes sin cambio.**
El usuario precisó que hay sólo dos teléfonos: iPhone mostraba «En red» y
Android «Conectando». Los registros del A73 después de reinstalar mostraron
conexiones GATT heredadas, sin escrituras de descriptor ni negociación Noise.
La autenticación antigua del iPhone no tenía comprobación periódica de vida.
Además, al desacoplar un motor Flutter Android no liberaba su propietario BLE,
y una desconexión central podía dejar el vecino en la lista que impide reintentar.

Cambios en el host del SDK, consumidos por la dependencia local de Convoy:

- Ambos hosts comprueban respuestas autenticadas con reloj monótono: sondeo
  cada 3 segundos y vencimiento tras 12 segundos sin prueba válida. También
  vence una conexión que no termina de autenticar. Escribir correctamente al
  sistema operativo no renueva la vida del vecino.
- Los sondeos y sus respuestas pasan por la cola de registros en claro antes
  del cifrado. Respetan el fragmento ATT en vuelo y adelantan la cola pendiente
  de audio; no reordenan contadores Noise ni eliminan contenido pendiente.
- iOS reinicia negociación ante invalidación del servicio y errores de
  descubrimiento/suscripción. Android elimina vecinos estancados y cierra
  GATT, publicidad y receiver al destruir su host, ignorando callbacks de
  inicio que llegan después del cierre.
- Desacoplar un motor Flutter deja de liberar el almacén nativo global que
  otro motor puede seguir utilizando. La salida explícita del ámbito de
  producto mantiene su propia limpieza.

Validación automática: 17 pruebas Kotlin/JNI aprobadas y contrato Swift/FFI
aprobado. Se prueban plazos, rechazo de progreso no autenticado, pares
independientes, sesión nueva tras reinicio, sondeos durante una sesión larga y
prioridad del sondeo entre registros de voz sin pérdida de fragmentos.
La nueva política pura BleLinkHealth tiene cobertura de líneas 9/9 en Kotlin
(JaCoCo, ramas 12/12) y 14/14 en Swift (LLVM). Esto NO equivale a cobertura del
90% de todo el cambio: los callbacks de Android/CoreBluetooth requieren
instrumentación física; el gate global de cobertura nativa sigue pendiente.
Evidencia temporal: /tmp/mesh-ble-health-kotlin-coverage.log,
/tmp/mesh-ble-health-swift.log y /tmp/mesh-ble-health-coverage/.

Android Release compiló en 51,6 s y se instaló por USB en el A73. iOS Release
compiló en 42,5 s y CoreDevice confirmó instalación en el iPhone a las 08:49.
Se usó el perfil local BLE, sin entitlement Wi-Fi Aware en el artefacto iOS;
el archivo fuente de entitlements quedó restaurado. No se instaló Debug.
Android quedó abierto dentro de Testing. La evidencia física de las 08:48:49
confirma descarte de los enlaces antiguos por falta de respuesta. Aún no se
registra una negociación autenticada nueva tras ambas instalaciones: se pidió
al usuario volver a Testing en iPhone. Pendientes: ambos semáforos En red,
mensajes bidireccionales y reinicio de una app conservando la otra abierta.
No se declara resuelta la prueba de reconexión física hasta observarla.
