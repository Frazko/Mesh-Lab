import 'package:integration_test/integration_test_driver.dart';

/// Supports physical iOS over Wi-Fi, where flutter test disables port publication.
Future<void> main() => integrationDriver();
