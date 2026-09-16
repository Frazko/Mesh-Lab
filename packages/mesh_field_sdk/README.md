# Mesh Field SDK

Fachada Flutter reutilizable para una sesión Mesh cercana sin internet. Oculta
el bridge nativo, Bluetooth LE, Wi‑Fi Aware, IP, credenciales y frames
cifrados. Una aplicación consume únicamente estado de sesión y operaciones de
producto.

```dart
final mesh = FieldMeshClient();

await mesh.prepareIdentity();
final session = await mesh.connect();

if (session.secure) {
  final delivery = await mesh.sendText('Convoy listo');
  // Conserva delivery?.logicalId para refrescar el estado después.
  if (delivery?.complete == true) {
    // Todos los destinatarios certificados confirmaron la entrega.
  }
}

mesh.watch().listen((state) {
  // Proyectar state.connection y radios en la interfaz del producto.
});
```

`MeshHostGateway` es el adaptador actual iOS/Android sobre `mesh_host`.
Productos pueden sustituir `FieldMeshGateway` por un fake de pruebas sin
inicializar Flutter channels ni radios físicos.

## Alcance actual

La versión inicial expone identidad, estado del grupo, Bluetooth, Wi‑Fi Aware,
voz, conexión, texto durable, ubicación puntual y evidencia agregada de su
entrega. `sendText` y `sendLocation` devuelven un ID lógico y `delivery(id)`
refresca `queued`, `partial`, `delivered` o `expired`. `FieldLocation` lleva
coordenadas, precisión, hora y orientación opcional del vehículo en grados;
una orientación nula significa que el teléfono no pudo obtenerla con confianza.
`watchIncoming` entrega texto o ubicación recibida para una interfaz de producto.

Para una integración que debe atribuir y reconciliar acciones, el cliente
implementa la capacidad opcional `FieldMeshVerifiedIncomingSource`.
`watchVerifiedIncomingText` drena una cola FIFO nativa de texto durable que
incluye origen de roster, ID de objeto y momento de verificación. Cada entrada
se publica únicamente después de verificar todos los chunks y confirmar el
receipt local; el SDK vuelve a validar formato y descarta evidencia malformada.
El ID lógico de una aplicación todavía debe viajar en un sobre de producto
cifrado antes de que una aplicación pueda correlacionar una acción entrante con
su propio outbox.
No expone objetos cifrados, receipts, identidades de destinatarios, rutas, IP
ni claves. La presencia global y el rastreo continuo en segundo plano siguen
fuera del SDK porque requieren una política de producto y validación física.

Mesh Lab es el consumidor de referencia y el harness físico. Convoy será la
primera integración de producto: adapta miembros a vehículos, posiciones al
mapa y mensajes a su chat, sin conocer radios ni la topología.
