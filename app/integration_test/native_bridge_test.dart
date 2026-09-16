import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mesh_host/mesh_host.dart';

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('NAT-VS0 real Flutter-host-Rust snapshot and replay', (
    tester,
  ) async {
    final api = MeshHostApi();
    final info = await api.engineInfo();
    expect(info.abiVersion, 1);
    expect(info.apiVersion, 1);
    expect(info.phase, 'F0');
    final before = await api.subscribe(0);
    final id = DateTime.now().microsecondsSinceEpoch;
    final after = await api.verifyBridge(id);
    expect(after.runtimeId, before.runtimeId);
    expect(after.cursor, before.cursor + 1);
    expect(after.events.single.requestId, id);
    final retry = await api.verifyBridge(id);
    expect(retry.cursor, after.cursor);
    final recreatedFacade = MeshHostApi();
    final restored = await recreatedFacade.subscribe(before.cursor);
    expect(restored.runtimeId, before.runtimeId);
    expect(restored.cursor, after.cursor);
    expect(restored.events.single.sequence, after.cursor);
    final ahead = await api.subscribe(after.cursor + 100);
    expect(ahead.cursorReset, true);
    expect(ahead.events, isEmpty);
    for (var i = 1; i <= 65; i++) {
      await api.verifyBridge(id + i);
    }
    final gap = await api.subscribe(before.cursor);
    expect(gap.cursorReset, true);
    expect(gap.cursor, after.cursor + 65);
  });
  testWidgets('KEY-01 native identity survives reopening the facade', (
    tester,
  ) async {
    final first = await MeshHostApi().prepareIdentity();
    expect(first.fingerprint, matches(RegExp(r'^[0-9a-f]{64}$')));
    final second = await MeshHostApi().prepareIdentity();
    expect(second.fingerprint, first.fingerprint);
    expect({'Keychain', 'Android Keystore'}, contains(second.storage));
    // Public metadata only. The host compares separate launches to verify recovery.
    binding.reportData = {
      'identity': {
        'fingerprint': second.fingerprint,
        'storage': second.storage,
        'processId': pid,
      },
    };
  });
}
