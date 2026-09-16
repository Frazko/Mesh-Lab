import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mesh_host/mesh_host.dart';
import 'package:mesh_lab/main.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'fake_sdk.dart';

class ConnectedChatSdk extends FakeLabSdk {
  String? sent;

  @override
  Future<BluetoothInfo> bluetoothInfo() async => BluetoothInfo(
    available: true,
    authorized: true,
    enabled: true,
    active: true,
    peerCount: 1,
    probeCount: 0,
    authenticated: true,
    messageCount: 0,
    lastMessage: '',
    detail: 'Enlace seguro activo.',
  );

  @override
  Future<bool> sendText(String message, String logicalId) async {
    sent = message;
    return true;
  }
}

class GroupSessionSdk extends FakeLabSdk {
  @override
  Future<GroupInfo> groupInfo() async => GroupInfo(configured: true, epoch: 1);
}

void main() {
  testWidgets('session controls and chat remain usable on a phone', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await tester.pumpWidget(MeshLabApp(sdk: FakeLabSdk()));
    await tester.pump();
    await tester.pump();
    for (final tab in ['GPS', 'Texto', 'Voz', 'Diagnóstico', 'Red']) {
      await tester.tap(find.text(tab).last);
      await tester.pump();
      expect(tester.takeException(), isNull);
    }
    await tester.scrollUntilVisible(find.text('Sesión de campo'), 160);
    expect(find.text('Sesión de campo'), findsOneWidget);
    expect(find.text('Wi‑Fi Aware'), findsOneWidget);
  });

  testWidgets('chat draft survives tab changes when no peer is connected', (
    tester,
  ) async {
    await tester.pumpWidget(MeshLabApp(sdk: FakeLabSdk()));
    await tester.pump();
    await tester.pump();
    await tester.tap(find.text('Texto').last);
    await tester.pump();
    await tester.scrollUntilVisible(find.byType(TextField), 160);
    await tester.enterText(find.byType(TextField), 'Punto de encuentro');
    await tester.tap(find.text('GPS').last);
    await tester.pump();
    await tester.tap(find.text('Texto').last);
    await tester.pump();
    expect(find.text('Punto de encuentro'), findsOneWidget);
    expect(find.widgetWithText(FilledButton, 'Enviar'), findsOneWidget);
  });

  testWidgets(
    'connected chat queues text, clears the draft and shows evidence',
    (tester) async {
      final sdk = ConnectedChatSdk();
      SharedPreferences.setMockInitialValues({});
      await tester.pumpWidget(MeshLabApp(sdk: sdk));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
      await tester.tap(find.text('Texto').last);
      await tester.pump();
      await tester.scrollUntilVisible(find.byType(TextField), 160);
      expect(
        tester
            .widget<FilledButton>(find.widgetWithText(FilledButton, 'Enviar'))
            .onPressed,
        isNotNull,
      );
      await tester.enterText(find.byType(TextField), 'Punto de encuentro');
      await tester.tap(find.widgetWithText(FilledButton, 'Enviar'));
      await tester.pump();
      await tester.pump();
      expect(sdk.sent, contains('Punto de encuentro'));
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller?.text,
        isEmpty,
      );
      expect(find.text('Punto de encuentro'), findsOneWidget);
      expect(find.text('En cola segura'), findsOneWidget);
    },
  );

  testWidgets('group session control starts and closes the radios visibly', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    SharedPreferences.setMockInitialValues({});
    await tester.pumpWidget(MeshLabApp(sdk: GroupSessionSdk()));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    final connect = find.widgetWithText(FilledButton, 'Conectar sesión');
    await tester.scrollUntilVisible(connect, 160);
    await tester.pump();
    await tester.tap(connect);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    final leave = find.widgetWithText(OutlinedButton, 'Salir de sesión');
    expect(leave, findsOneWidget);
    expect(find.text('Buscando vecinos autorizados'), findsOneWidget);
    await tester.tap(leave);
    await tester.pump();
    expect(find.text('Conectar sesión'), findsOneWidget);
    expect(find.text('Sesión cerrada'), findsOneWidget);
  });
}
