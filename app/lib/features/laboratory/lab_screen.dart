import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:record/record.dart';

import '../../core/sdk/lab_controller.dart';
import '../../core/theme/lab_theme.dart';

class LabScreen extends StatefulWidget {
  const LabScreen({super.key, required this.sdk});
  final LabSdk sdk;
  @override
  State<LabScreen> createState() => _LabScreenState();
}

class _LabScreenState extends State<LabScreen> with WidgetsBindingObserver {
  late final LabController controller;
  final draft = TextEditingController();
  final recorder = AudioRecorder();
  Timer? _voiceClock;
  DateTime? _voiceStartedAt;
  Duration _voiceElapsed = Duration.zero;
  bool _recordingVoice = false;
  String? _voiceError;
  int selected = 0;
  static const labels = ['Red', 'GPS', 'Texto', 'Voz', 'Diagnóstico'];
  static const icons = [
    Icons.hub_outlined,
    Icons.location_on_outlined,
    Icons.chat_bubble_outline,
    Icons.mic_none,
    Icons.science_outlined,
  ];
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    controller = LabController(widget.sdk);
    unawaited(controller.refresh());
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      unawaited(controller.refresh());
    } else {
      controller.markStale();
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    controller.dispose();
    draft.dispose();
    _voiceClock?.cancel();
    unawaited(recorder.dispose());
    super.dispose();
  }

  Future<void> _beginVoice() async {
    if (_recordingVoice || controller.busy || !controller.secureConnected) {
      return;
    }
    try {
      if (!await recorder.hasPermission()) {
        if (mounted) {
          setState(
            () => _voiceError =
                'Autoriza el micrófono para grabar la nota de voz.',
          );
        }
        return;
      }
      final file = File(
        '${Directory.systemTemp.path}/mesh-lab-${DateTime.now().microsecondsSinceEpoch}.m4a',
      );
      await recorder.start(
        const RecordConfig(
          encoder: AudioEncoder.aacLc,
          // AAC-LC at 24 kb/s keeps short speech intelligible while reducing
          // BLE airtime by roughly one quarter versus the initial 32 kb/s lab
          // profile.  AAC/M4A remains natively decodable on both platforms.
          bitRate: 24000,
          sampleRate: 16000,
          numChannels: 1,
          autoGain: true,
          echoCancel: true,
          noiseSuppress: true,
        ),
        path: file.path,
      );
      if (!mounted) {
        return;
      }
      setState(() {
        _recordingVoice = true;
        _voiceStartedAt = DateTime.now();
        _voiceElapsed = Duration.zero;
        _voiceError = null;
      });
      _voiceClock = Timer.periodic(const Duration(milliseconds: 200), (_) {
        final started = _voiceStartedAt;
        if (!mounted || !_recordingVoice || started == null) return;
        final elapsed = DateTime.now().difference(started);
        if (elapsed >= const Duration(seconds: 8)) {
          unawaited(_finishVoice());
        } else {
          setState(() => _voiceElapsed = elapsed);
        }
      });
    } catch (_) {
      if (mounted) {
        setState(
          () => _voiceError =
              'No se pudo iniciar la grabación. Vuelve a intentarlo.',
        );
      }
    }
  }

  Future<void> _finishVoice() async {
    if (!_recordingVoice) return;
    _voiceClock?.cancel();
    final started = _voiceStartedAt;
    if (mounted) {
      setState(() {
        _recordingVoice = false;
        final elapsed = started == null
            ? Duration.zero
            : DateTime.now().difference(started);
        _voiceElapsed = elapsed > const Duration(seconds: 8)
            ? const Duration(seconds: 8)
            : elapsed;
      });
    }
    try {
      final path = await recorder.stop();
      if (path == null) throw const FileSystemException();
      final file = File(path);
      final audio = await file.readAsBytes();
      final duration = _voiceElapsed.inMilliseconds.clamp(1, 8000);
      await controller.sendVoice(audio, duration);
      try {
        await file.delete();
      } catch (_) {
        // The operating system may already have cleaned this temporary file.
      }
    } catch (_) {
      if (mounted) {
        setState(
          () => _voiceError = 'No se pudo enviar la nota de voz. Revisa la conexión y vuelve a intentarlo.',
        );
      }
    } finally {
      _voiceStartedAt = null;
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: controller,
    builder: (context, _) => Scaffold(
      appBar: AppBar(
        title: const Text('Mesh Lab'),
        actions: const [
          Padding(
            padding: EdgeInsets.only(right: 20),
            child: Text(
              'LAB · F1',
              style: TextStyle(fontWeight: FontWeight.w700),
            ),
          ),
        ],
      ),
      body: GestureDetector(
        behavior: HitTestBehavior.translucent,
        onTap: () => FocusManager.instance.primaryFocus?.unfocus(),
        child: SafeArea(
          child: LayoutBuilder(
            builder: (context, constraints) {
              final wide = constraints.maxWidth >= 840;
              final content = Center(
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 850),
                  child: ListView(
                    key: PageStorageKey(selected),
                    keyboardDismissBehavior:
                        ScrollViewKeyboardDismissBehavior.onDrag,
                    padding: EdgeInsets.fromLTRB(
                      wide ? 28 : 16,
                      wide ? 28 : 16,
                      wide ? 28 : 16,
                      wide ? 28 : 28,
                    ),
                    children: [
                      _status(),
                      const SizedBox(height: 24),
                      Text(
                        labels[selected],
                        style: Theme.of(context).textTheme.headlineMedium
                            ?.copyWith(fontWeight: FontWeight.bold),
                      ),
                      const SizedBox(height: 6),
                      Text(
                        [
                          'Estado del laboratorio y controles de sesión.',
                          'Ubicación puntual, compartida por decisión tuya.',
                          'Mensajes al grupo y estado de entrega.',
                          'Graba un mensaje de hasta 8 segundos.',
                          'Verifica el motor y recupera su estado.',
                        ][selected],
                      ),
                      const SizedBox(height: 20),
                      ...switch (selected) {
                        0 => _network(),
                        1 => _gps(),
                        2 => _text(),
                        3 => _voice(),
                        _ => _diagnostics(),
                      },
                      const SizedBox(height: 24),
                      _linkStatus(),
                    ],
                  ),
                ),
              );
              if (!wide) return content;
              return Row(
                children: [
                  NavigationRail(
                    selectedIndex: selected,
                    labelType: NavigationRailLabelType.all,
                    onDestinationSelected: (i) => setState(() => selected = i),
                    destinations: [
                      for (var i = 0; i < labels.length; i++)
                        NavigationRailDestination(
                          icon: Icon(icons[i]),
                          label: Text(labels[i]),
                        ),
                    ],
                  ),
                  const VerticalDivider(width: 1),
                  Expanded(child: content),
                ],
              );
            },
          ),
        ),
      ),
      bottomNavigationBar: MediaQuery.sizeOf(context).width < 840
          ? NavigationBar(
              selectedIndex: selected,
              labelBehavior:
                  NavigationDestinationLabelBehavior.onlyShowSelected,
              onDestinationSelected: (i) => setState(() => selected = i),
              destinations: [
                for (var i = 0; i < labels.length; i++)
                  NavigationDestination(icon: Icon(icons[i]), label: labels[i]),
              ],
            )
          : null,
    ),
  );

  Widget _status() {
    final error = controller.failure;
    final title = controller.busy
        ? controller.activity
        : error != null
        ? 'Motor sin verificar'
        : controller.secureConnected
        ? 'Enlace seguro activo'
        : controller.ready
        ? 'Motor disponible · Buscando el otro teléfono'
        : 'Estado pendiente de actualizar';
    return Semantics(
      liveRegion: true,
      child: Container(
        padding: const EdgeInsets.all(18),
        decoration: BoxDecoration(
          color: error != null
              ? const Color(0xffffe7df)
              : const Color(0xffe1e8da),
          borderRadius: BorderRadius.circular(14),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Icon(
                  error != null ? Icons.error_outline : Icons.info_outline,
                  color: LabTheme.forest,
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: Text(
                    title,
                    style: const TextStyle(
                      fontWeight: FontWeight.w700,
                      fontSize: 16,
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),
            Text(
              error?.message ?? 'La base del laboratorio está disponible. Tras crear o unirse al grupo, Bluetooth busca y conecta teléfonos del grupo automáticamente.',
            ),
            if (error != null) ...[
              const SizedBox(height: 8),
              SelectableText('Código: ${error.code}'),
              const SizedBox(height: 12),
              FilledButton.icon(
                onPressed: controller.busy ? null : controller.refresh,
                icon: const Icon(Icons.refresh),
                label: const Text('Reintentar conexión al motor'),
              ),
            ],
            if (controller.busy) ...[
              const SizedBox(height: 12),
              const LinearProgressIndicator(),
            ],
          ],
        ),
      ),
    );
  }

  Widget _linkStatus() {
    final bluetooth = controller.bluetooth;
    final authenticated = controller.secureConnected;
    final messageCount = bluetooth?.messageCount ?? 0;
    return Semantics(
      liveRegion: true,
      child: Text(
        authenticated
            ? messageCount > 0
                  ? 'Enlace seguro activo. Hay $messageCount mensaje(s) recibido(s) en este teléfono.'
                  : 'Enlace seguro activo. Ya puedes enviar un mensaje desde la pestaña Texto.'
            : bluetooth?.active == true
            ? 'Buscando automáticamente el otro teléfono del grupo por Bluetooth…'
            : 'Crea o únete al grupo para iniciar la conexión Bluetooth automática.',
        style: TextStyle(
          color: authenticated
              ? const Color(0xff1e704d)
              : const Color(0xff59645b),
          fontWeight: authenticated ? FontWeight.w600 : FontWeight.normal,
        ),
      ),
    );
  }

  List<Widget> _network() => [
    _connectionSignal(),
    const SizedBox(height: 16),
    _panel(
      'Grupo',
      controller.group?.configured == true
          ? 'Grupo seguro listo'
          : 'Preparar grupo',
      [
        Text(
          controller.group?.configured == true
              ? 'Este teléfono pertenece al grupo. Conecta la sesión para detectar vecinos por Bluetooth y Wi‑Fi Aware.'
              : controller.identity == null
              ? 'Primero prepara la identidad protegida.'
              : Platform.isAndroid
              ? 'Crea un grupo nuevo o busca por Bluetooth el grupo de un Android cercano.'
              : 'Mantén este iPhone junto al Android que creó el grupo para incorporarlo por Bluetooth.',
        ),
        const SizedBox(height: 16),
        if (controller.identity == null)
          FilledButton.icon(
            onPressed: controller.busy
                ? null
                : () => setState(() => selected = 4),
            icon: const Icon(Icons.fingerprint),
            label: const Text('Preparar identidad'),
          )
        else if (controller.group?.configured != true && Platform.isAndroid)
          Wrap(
            spacing: 12,
            runSpacing: 12,
            children: [
              FilledButton.icon(
                onPressed: controller.busy ? null : controller.createGroup,
                icon: const Icon(Icons.group_add_outlined),
                label: const Text('Crear grupo nuevo'),
              ),
              OutlinedButton.icon(
                onPressed: controller.busy ? null : controller.connectSession,
                icon: const Icon(Icons.bluetooth_searching),
                label: const Text('Buscar grupo cercano'),
              ),
            ],
          )
        else if (controller.group?.configured != true)
          const LinearProgressIndicator(),
        if (controller.group?.configured != true && !Platform.isAndroid) ...[
          const SizedBox(height: 10),
          Text(
            controller.enrollmentStatus ??
                'Esperando la incorporación Bluetooth…',
          ),
        ],
      ],
    ),
    const SizedBox(height: 16),
    _panel('Sesión de campo', 'Un solo control para todos los radios', [
      Text(
        controller.sessionActive
            ? 'La sesión seguirá buscando y recuperando vecinos hasta que la cierres.'
            : controller.group?.configured == true
            ? 'Conectar activa Bluetooth y Wi‑Fi Aware. No usa router, hotspot, SSID ni dirección IP.'
            : 'Buscar grupo activa Bluetooth para incorporar este Android a un grupo cercano.',
      ),
      const SizedBox(height: 16),
      if (controller.sessionActive)
        OutlinedButton.icon(
          onPressed: controller.busy ? null : controller.leaveSession,
          icon: const Icon(Icons.logout),
          label: const Text('Salir de sesión'),
        )
      else
        FilledButton.icon(
          onPressed: controller.busy || controller.identity == null
              ? null
              : controller.connectSession,
          icon: const Icon(Icons.link),
          label: Text(
            controller.group?.configured == true
                ? 'Conectar sesión'
                : 'Buscar grupo cercano',
          ),
        ),
      const Divider(height: 28),
      _Fact(
        icon: Icons.bluetooth,
        title: 'Bluetooth',
        value: controller.bluetooth?.detail ?? 'Comprobando Bluetooth…',
      ),
      const SizedBox(height: 12),
      _Fact(
        icon: Icons.wifi_tethering,
        title: 'Wi‑Fi Aware',
        value: controller.aware?.detail ?? 'Comprobando Wi‑Fi Aware…',
      ),
      const SizedBox(height: 10),
      _Fact(
        icon: Icons.route_outlined,
        title: 'Estado Wi‑Fi Aware',
        value: _awarePhaseLabel(controller.aware?.state),
      ),
      const SizedBox(height: 10),
      Text(
        'Vecinos del grupo por Wi‑Fi Aware: ${controller.aware?.peerCount ?? 0} · capacidad directa: ${controller.aware?.maxPeers ?? 0}',
      ),
      const SizedBox(height: 12),
      _Fact(
        icon: Icons.people_outline,
        title: 'Dispositivos',
        value: controller.secureConnected
            ? 'Teléfono del grupo conectado de forma segura'
            : controller.sessionActive
            ? 'Buscando vecinos autorizados'
            : 'Sesión cerrada',
      ),
    ]),
    const SizedBox(height: 16),
    _panel('Prueba disponible ahora', 'Flutter ↔ host nativo ↔ Rust', [
      Text(
        'Comprueba que la app recibe una respuesta real del motor en este teléfono.',
      ),
      const SizedBox(height: 16),
      _verifyButton(),
    ]),
    const SizedBox(height: 16),
  ];

  String _awarePhaseLabel(String? state) => switch (state) {
    'unsupported' => 'Este teléfono no soporta Wi‑Fi Aware',
    'unavailable' => 'Radio no disponible',
    'needs_group' => 'Primero incorpora este teléfono al grupo',
    'pairing_required' => 'El iPhone requiere vincular el otro teléfono',
    'paired_waiting' => 'Teléfono vinculado; preparando enlace seguro',
    'connected' => 'Enlace Wi‑Fi Aware seguro activo',
    'direct_only' => 'Este Android no puede emparejar Wi‑Fi Aware con iPhone',
    'peer_detected' => 'Vecino del grupo detectado; preparando enlace seguro',
    'discovering' => 'Buscando teléfonos autorizados del grupo',
    'stopped' => 'Sesión de radio detenida',
    _ => 'Comprobando estado del radio',
  };

  Widget _connectionSignal() {
    final connected = controller.secureConnected;
    final connecting = !connected && controller.sessionActive;
    final color = connected
        ? const Color(0xff1e8a57)
        : connecting
        ? const Color(0xffe0a113)
        : const Color(0xffc73535);
    final label = connected
        ? 'Conectado'
        : connecting
        ? 'Conectando'
        : 'Desconectado';
    final detail = connected
        ? 'Enlace seguro activo con el teléfono del grupo.'
        : connecting
        ? 'Buscando o recuperando vecinos automáticamente…'
        : 'Toca Conectar sesión para iniciar los radios.';
    return Semantics(
      label: 'Estado de conexión: $label. $detail',
      child: Container(
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(
          color: color.withValues(alpha: .12),
          borderRadius: BorderRadius.circular(18),
        ),
        child: Row(
          children: [
            Container(
              width: 16,
              height: 16,
              decoration: BoxDecoration(color: color, shape: BoxShape.circle),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    label,
                    style: Theme.of(context).textTheme.titleLarge
                        ?.copyWith(fontWeight: FontWeight.bold),
                  ),
                  const SizedBox(height: 2),
                  Text(detail),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  List<Widget> _gps() => [
    _panel(
      'Mi ubicación',
      controller.myLocation == null
          ? 'Aún no compartida'
          : 'Ubicación puntual lista',
      [
        _Fact(
          icon: controller.myLocation == null
              ? Icons.location_off_outlined
              : Icons.my_location_outlined,
          title: 'Coordenadas',
          value: controller.myLocation == null
              ? 'Aún no disponibles'
              : _coordinates(controller.myLocation!),
        ),
        _Fact(
          icon: Icons.schedule,
          title: 'Última actualización',
          value: controller.myLocation == null
              ? 'No se ha obtenido una ubicación'
              : _time(controller.myLocation!),
        ),
        const SizedBox(height: 16),
        FilledButton.icon(
          onPressed: controller.busy || !controller.secureConnected
              ? null
              : () => unawaited(controller.shareCurrentLocation()),
          icon: const Icon(Icons.send_outlined),
          label: const Text('Compartir mi ubicación'),
        ),
        const SizedBox(height: 12),
        Text(
          controller.secureConnected
              ? 'Obtiene una sola lectura cuando la pides. Se envía cifrada al teléfono del grupo.'
              : 'El botón se habilita cuando el enlace seguro esté conectado.',
        ),
      ],
    ),
    const SizedBox(height: 16),
    _panel(
      'Ubicación del otro teléfono',
      controller.groupLocation == null
          ? 'El otro teléfono aún no compartió ubicación'
          : 'Recibida del otro teléfono',
      [
        if (controller.groupLocation == null)
          const Text(
            'Aquí aparecerá solo la ubicación que el otro teléfono decida compartir.',
          )
        else ...[
          const Text(
            'Esta no es tu ubicación. Llegó cifrada desde el otro teléfono.',
          ),
          const SizedBox(height: 14),
          _Fact(
            icon: Icons.location_on_outlined,
            title: 'Coordenadas',
            value: _coordinates(controller.groupLocation!),
          ),
          _Fact(
            icon: Icons.schedule,
            title: 'Compartida por el otro teléfono',
            value: _time(controller.groupLocation!),
          ),
        ],
      ],
    ),
  ];

  String _coordinates(LocationReading reading) =>
      '${reading.latitude.toStringAsFixed(5)}, ${reading.longitude.toStringAsFixed(5)} · ±${reading.accuracyMeters.round()} m';

  String _time(LocationReading reading) {
    final age = DateTime.now().difference(reading.capturedAt);
    if (age.inMinutes <= 0) return 'Ahora';
    if (age.inHours <= 0) return 'Hace ${age.inMinutes} min';
    return 'Hace ${age.inHours} h';
  }

  List<Widget> _text() => [
    _panel(
      'Chat del grupo',
      '${controller.chatMessages.length} mensaje(s) guardado(s) en este teléfono',
      [
        if (controller.chatMessages.isEmpty)
          const Padding(
            padding: EdgeInsets.symmetric(vertical: 18),
            child: Text(
              'Aún no hay mensajes. Cuando un teléfono se reconecte, los nuevos mensajes aparecerán aquí en orden.',
            ),
          )
        else
          ...controller.chatMessages.map(
            (message) => Align(
              alignment: message.mine
                  ? Alignment.centerRight
                  : Alignment.centerLeft,
              child: Container(
                margin: const EdgeInsets.only(bottom: 10),
                padding: const EdgeInsets.all(12),
                constraints: const BoxConstraints(maxWidth: 320),
                decoration: BoxDecoration(
                  color: message.mine ? const Color(0xffd8f1e2) : Colors.white,
                  borderRadius: BorderRadius.circular(14),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      message.mine ? 'Tú' : 'Miembro del grupo',
                      style: const TextStyle(fontWeight: FontWeight.w700),
                    ),
                    const SizedBox(height: 4),
                    Text(message.body),
                    const SizedBox(height: 5),
                    Text(
                      _timeLabel(message.sentAt),
                      style: Theme.of(context).textTheme.labelSmall,
                    ),
                    if (message.mine && message.deliveryState != null) ...[
                      const SizedBox(height: 3),
                      Text(
                        _deliveryLabel(message),
                        style: Theme.of(context).textTheme.labelSmall,
                      ),
                    ],
                  ],
                ),
              ),
            ),
          ),
      ],
    ),
    const SizedBox(height: 16),
    _panel('Nuevo mensaje', 'Se guarda en el chat al enviarlo', [
      TextField(
        controller: draft,
        minLines: 2,
        maxLines: 5,
        maxLength: 500,
        textInputAction: TextInputAction.done,
        onEditingComplete: () => FocusManager.instance.primaryFocus?.unfocus(),
        decoration: const InputDecoration(
          labelText: 'Mensaje',
          hintText: 'Escribe al grupo…',
          alignLabelWithHint: true,
        ),
      ),
      const SizedBox(height: 12),
      FilledButton.icon(
        onPressed: controller.busy || !controller.secureConnected
            ? null
            : () async {
                FocusManager.instance.primaryFocus?.unfocus();
                if (await controller.sendText(draft.text)) draft.clear();
              },
        icon: const Icon(Icons.send),
        label: const Text('Enviar'),
      ),
      const SizedBox(height: 10),
      Text(
        controller.secureConnected
            ? 'El mensaje sale cifrado y el borrador se limpia al quedar en la cola.'
            : 'Conecta un teléfono del grupo para enviar.',
      ),
    ]),
  ];

  String _timeLabel(DateTime value) =>
      '${value.hour.toString().padLeft(2, '0')}:${value.minute.toString().padLeft(2, '0')}';
  String _deliveryLabel(ChatMessage message) {
    final targets = message.targetCount ?? 0;
    final delivered = message.deliveredCount ?? 0;
    return switch (message.deliveryState) {
      'delivered' =>
        targets > 0 ? 'Entregado a $delivered de $targets' : 'Entregado',
      'partial' => 'Entregado a $delivered de $targets',
      'expired' =>
        delivered > 0
            ? 'Venció: entregado a $delivered de $targets'
            : 'Venció sin confirmación',
      _ => 'En cola segura',
    };
  }

  List<Widget> _voice() => [
    _panel(
      'Mensaje de voz',
      _recordingVoice ? 'Grabando ahora' : 'Nota de hasta 8 segundos',
      [
        Center(
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 20),
            child: Column(
              children: [
                Text(
                  '${_voiceElapsed.inMinutes}:${(_voiceElapsed.inSeconds % 60).toString().padLeft(2, '0')} / 0:08',
                  style: Theme.of(context).textTheme.headlineLarge,
                ),
                const SizedBox(height: 24),
                SizedBox(
                  width: double.infinity,
                  child: FilledButton.icon(
                    onPressed: controller.busy || !controller.secureConnected
                        ? null
                        : () => unawaited(
                            _recordingVoice ? _finishVoice() : _beginVoice(),
                          ),
                    icon: Icon(
                      _recordingVoice
                          ? Icons.stop_circle_outlined
                          : Icons.mic_none,
                      size: 32,
                    ),
                    label: Padding(
                      padding: const EdgeInsets.symmetric(vertical: 26),
                      child: Text(
                        _recordingVoice
                            ? 'Toca para terminar y enviar'
                            : 'Toca para grabar',
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
        Text(
          controller.secureConnected
              ? 'La nota se codifica en el teléfono y se envía cifrada al otro teléfono. Se detiene sola a los 8 segundos.'
              : 'Conecta el segundo teléfono para habilitar la grabación.',
        ),
        if (controller.outgoingVoiceDeliveryLabel case final label?) ...[
          const SizedBox(height: 8),
          Text('Estado del envío: $label'),
        ],
        if (_voiceError != null) ...[
          const SizedBox(height: 12),
          Text(
            _voiceError!,
            style: TextStyle(color: Theme.of(context).colorScheme.error),
          ),
        ],
      ],
    ),
    const SizedBox(height: 16),
    _panel(
      'Audios del grupo',
      controller.voice?.ready == true
          ? 'Última nota recibida'
          : 'Sin notas recibidas',
      [
        Text(
          controller.voice?.detail ??
              'Esperando una nota de voz del otro teléfono.',
        ),
        if (controller.voice?.ready == true) ...[
          const SizedBox(height: 12),
          Text(
            'Duración: ${(controller.voice!.lastDurationMillis / 1000).toStringAsFixed(1)} segundos',
          ),
          const SizedBox(height: 12),
          OutlinedButton.icon(
            onPressed: controller.busy
                ? null
                : () => unawaited(controller.playLastVoice()),
            icon: const Icon(Icons.play_arrow),
            label: const Text('Reproducir última nota'),
          ),
        ],
      ],
    ),
  ];
  List<Widget> _diagnostics() => [
    _panel('Identidad del teléfono', 'Claves protegidas en este dispositivo', [
      const Text(
        'Prepara la identidad y abre el almacén cifrado local. Al volver a verificar, la huella debe ser la misma. Esto todavía no conecta los teléfonos.',
      ),
      const SizedBox(height: 16),
      if (controller.identity != null) ...[
        Text('Protección: ${controller.identity!.storage}'),
        const SizedBox(height: 8),
        const Text('Huella pública'),
        SelectableText(controller.identity!.fingerprint),
        const SizedBox(height: 16),
      ],
      FilledButton.icon(
        onPressed: controller.busy ? null : controller.prepareIdentity,
        icon: const Icon(Icons.fingerprint),
        label: Text(
          controller.identity == null
              ? 'Preparar identidad'
              : 'Verificar identidad',
        ),
      ),
    ]),
    const SizedBox(height: 16),
    _panel(
      'Puente del motor',
      controller.ready ? 'Respuesta nativa validada' : 'Pendiente de verificar',
      [
        _Fact(
          icon: Icons.memory,
          title: 'Motor Rust',
          value: controller.info?.engineVersion ?? 'Sin respuesta',
        ),
        _Fact(
          icon: Icons.layers_outlined,
          title: 'Contrato ABI / API',
          value: controller.info == null
              ? 'Sin respuesta'
              : '${controller.info!.abiVersion} / ${controller.info!.apiVersion}',
        ),
        _Fact(
          icon: Icons.fingerprint,
          title: 'Instancia del proceso',
          value: controller.snapshot?.runtimeId.toString() ?? 'Sin respuesta',
        ),
        _Fact(
          icon: Icons.format_list_numbered,
          title: 'Secuencia de eventos',
          value: controller.snapshot?.cursor.toString() ?? 'Sin respuesta',
        ),
        _Fact(
          icon: Icons.build_outlined,
          title: 'Compilación',
          value: controller.info?.buildId ?? 'Sin respuesta',
        ),
        const SizedBox(height: 16),
        Wrap(
          spacing: 12,
          runSpacing: 12,
          children: [
            _verifyButton(),
            OutlinedButton.icon(
              onPressed: controller.busy ? null : controller.refresh,
              icon: const Icon(Icons.refresh),
              label: const Text('Recuperar estado'),
            ),
          ],
        ),
        const SizedBox(height: 12),
        const Text(
          'Recuperar estado vuelve a leer la misma instancia nativa; no reinicia el motor ni activa radios.',
        ),
        if (controller.notice != null) ...[
          const SizedBox(height: 12),
          Text(controller.notice!),
        ],
      ],
    ),
    const SizedBox(height: 16),
    _panel('Eventos del motor', 'Últimos 64 eventos · Sin contenido personal', [
      if (controller.events.isEmpty)
        const Text(
          'Aún no hay eventos en este historial. Usa «Verificar puente» para registrar uno.',
        ),
      for (final event in controller.events)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 10),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Evento ${event.sequence} · Puente verificado',
                style: const TextStyle(fontWeight: FontWeight.bold),
              ),
              SelectableText(
                'Diagnóstico: F0-${controller.snapshot!.runtimeId}-${event.sequence}',
              ),
            ],
          ),
        ),
    ]),
  ];
  Widget _verifyButton() => FilledButton.icon(
    onPressed: controller.busy || controller.info == null
        ? null
        : controller.verify,
    icon: const Icon(Icons.check_circle_outline),
    label: Text(
      controller.retryPending ? 'Reintentar verificación' : 'Verificar puente',
    ),
  );
  Widget _panel(String title, String subtitle, List<Widget> children) => Card(
    child: Padding(
      padding: const EdgeInsets.all(20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            title,
            style: Theme.of(context).textTheme.titleLarge
                ?.copyWith(fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 6),
          Text(subtitle, style: const TextStyle(color: Color(0xff566357))),
          const Divider(height: 28),
          ...children,
        ],
      ),
    ),
  );
}

class _Fact extends StatelessWidget {
  const _Fact({required this.icon, required this.title, required this.value});
  final IconData icon;
  final String title;
  final String value;
  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 8),
    child: Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Icon(icon, color: LabTheme.forest),
        const SizedBox(width: 12),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(title, style: const TextStyle(fontWeight: FontWeight.w700)),
              const SizedBox(height: 3),
              Text(value),
            ],
          ),
        ),
      ],
    ),
  );
}
