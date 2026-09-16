import 'package:flutter/material.dart';

import 'core/sdk/lab_controller.dart';
import 'core/theme/lab_theme.dart';
import 'features/laboratory/lab_screen.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(const MeshLabApp());
}

class MeshLabApp extends StatelessWidget {
  const MeshLabApp({super.key, this.sdk});
  final LabSdk? sdk;
  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'Mesh Lab',
    debugShowCheckedModeBanner: false,
    theme: LabTheme.light,
    home: LabScreen(sdk: sdk ?? NativeLabSdk()),
  );
}
