# App Flutter

Consulta `../README.md` para preparar los paquetes nativos y ejecutar la app.

Cinco secciones: Red, GPS, Texto, Voz y Diagnóstico. Solo la verificación del
puente y la recuperación del estado están activas en F0. Los fakes viven en
`test/`; el entry point de la app siempre usa `NativeLabSdk`.

`LabController` es una proyección de presentación con ChangeNotifier. No decide
conectividad, rutas ni entrega. La separación mediante `LabSdk` permite incorporar
Riverpod en F5 sin cambiar la autoridad del motor ni el contrato del plugin.
