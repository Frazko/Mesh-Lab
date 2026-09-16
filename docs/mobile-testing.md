# Pruebas móviles

## Prueba disponible hoy

1. Construir el paquete nativo de la plataforma y ejecutar Flutter según README.
2. En Red, comprobar “Motor disponible · Radios pendientes”.
3. Pulsar “Verificar puente”. Debe subir una sola vez el contador de Rust.
4. En Diagnóstico, anotar instancia, secuencia e identificador del evento.
5. Pulsar “Recuperar estado”: instancia y secuencia se conservan, sin duplicados.
6. Hacer hot restart de Dart: volver a Diagnóstico y comprobar la misma instancia.
7. Terminar el proceso y abrir la app: empieza otro runtime diagnóstico vacío.
8. Recorrer GPS/Texto/Voz: acciones inactivas con motivo, sin solicitud de permisos.

No evaluar Bluetooth ni recepción de mensajes con esta versión. No hay tráfico
mesh, GPS ni audio. Un resultado positivo del puente no certifica conectividad.

## iOS

Simulador: `flutter run -d <simulator-id>`. El paquete incluye ARM64 y x86_64.
Teléfono físico: conectar/desbloquear, activar Developer Mode y usar firma de
desarrollo válida. Flutter creó el ID provisional `com.frazko.meshLab`; el Apple
Team se toma de la configuración local generada por Flutter. No se publicó en
TestFlight ni se creó un perfil de distribución durante F0.

## Android

APK actual (build 10002): `app/build/app/outputs/flutter-apk/app-release.apk`.
Si se compila con `--split-per-abi`, APK ARM64: `app/build/app/outputs/flutter-apk/app-arm64-v8a-release.apk`.
APK x86_64: `app/build/app/outputs/flutter-apk/app-x86_64-release.apk`.
Son compilaciones optimizadas firmadas con la clave de desarrollo.
Con teléfono ARM64 o emulador x86_64, instalar y ejecutar con `flutter run -d <id>`
o `adb install -r <apk>`. ID provisional `com.frazko.mesh_lab`, mínimo API 24.
Solo firma debug; no es un artefacto para Play Store. No se soporta ARM32 en F0.

Para probar el laboratorio en un teléfono, instalar el paquete Release y usar
los controles visibles de la app. El runner histórico `integration_test` se
mantiene como material de QA, pero no forma parte de las dependencias de la app
Release: no debe instalarse sobre el laboratorio de campo.

## Campaña posterior de radios

Registrar modelos, OS, roles central/peripheral, permisos, posición, build y run ID.
La primera prueba física será iPhone↔Android bidireccional por GATT. Luego:
interrupciones, Bluetooth apagado, permisos revocados, reencuentro y sesión de 60
minutos. Después, tres teléfonos A→B→C y transferencia durable de GPS/texto/voz.
El simulador no sustituye esas pruebas.

## iPhone físico por Wi-Fi

Usar una instalación Release firmada. No sustituirla con un driver Profile o
Debug durante la campaña de radios.


## Identidad persistente — validada en Android e iPhone

La fuente actual incluye **Diagnóstico → Preparar identidad**. No asumir que la
app antigua instalada contiene este control: primero completar el empaquetado y
la instalación. El APK Android build 10002 ya se generó después de liberar disco.
Ambos teléfonos pasaron NAT-VS0 y KEY-01 en dos procesos distintos por teléfono,
conservando la instalación. Las huellas persisten y son distintas entre teléfonos:
`artifacts/identity/device-results.json`. Ambas apps normales quedaron instaladas.
Se conserva Gradle 9.3.1; el wrapper usa ahora la distribución `bin` para reducir
la descarga y el espacio ocupado. Al cambiar entre pruebas y app release, ejecutar
el build normal de Flutter sin `--no-pub` para regenerar el registro de plugins;
un registro anterior de `integration_test` puede impedir compilar release.

El test KEY-01 exporta en `app/build/integration_response_data.json` únicamente
`identity.fingerprint`, `identity.storage` e `identity.processId`. El driver
escribe el informe solamente si todas las pruebas terminan correctamente.

Para probar persistencia real, ejecutar el driver dos veces por teléfono con un
cierre completo del proceso entre ejecuciones (no basta recrear la fachada Dart).
**Usar siempre `--keep-app-running`: Flutter drive desinstala la app al terminar
por defecto, lo que borra Android Keystore y falsea esta prueba.** Cerrar después
solo el proceso con `adb shell am force-stop com.frazko.mesh_lab` en Android o
`devicectl device process signal --pid <pid> --signal SIGTERM` en iOS. Conservar
la instalación y sus datos; `adb install -r` actualiza sin desinstalar.
Antes de cada ejecución retirar el informe temporal anterior y, **solo si el driver
sale con éxito**, copiar el nuevo a un nombre distinto bajo `artifacts/identity/`.
No reutilizar informes viejos. Conservar también logs de ambas ejecuciones y la
compilación utilizada. Un único teléfono valida solo un resultado parcial.

Desde la raíz del proyecto, para comparar los cuatro informes reales:

```sh
python3 tools/check_identity_runs.py   artifacts/identity/android-1.json artifacts/identity/android-2.json   artifacts/identity/ios-1.json artifacts/identity/ios-2.json   --output artifacts/identity/device-results.json
```

El validador rechaza PID repetido, cambio de huella al reiniciar, misma identidad
en ambos teléfonos y una campaña que solo incluya una plataforma. Sus tests usan
fixtures públicas y no crean un informe de aprobación física:

```sh
python3 -m unittest discover -s tools/tests -v
```

Al terminar, restaurar la app normal del laboratorio; el driver instala una
aplicación con entry point instrumental. Esta prueba aún no valida Bluetooth,
GPS, texto ni voz entre dispositivos.

## Android por Wi-Fi

El Samsung SM-A736B quedó vinculado al Mac y conectado por ADB inalámbrico.
Endpoint verificado: `192.168.100.171:36553`; puede cambiar al reactivar depuración
o cambiar de red. Si mDNS no lo descubre, leer «Dirección IP y puerto» en
Opciones de desarrollador → Depuración inalámbrica y ejecutar `adb connect IP:PUERTO`.
Seleccionar explícitamente ese endpoint con `adb -s IP:PUERTO` o `flutter -d IP:PUERTO`
si el mismo teléfono también aparece por USB. Esta conexión es para depurar e
instalar desde el Mac; Mesh Lab todavía no transporta mensajes entre teléfonos.
