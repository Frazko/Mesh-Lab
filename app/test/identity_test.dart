import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mesh_host/mesh_host.dart';
import 'package:mesh_lab/core/sdk/lab_controller.dart';

import 'fake_sdk.dart';

class KeysSdk extends FakeLabSdk {
  Object? error;
  String fingerprint = 'ab' * 32;
  @override
  Future<IdentityInfo> prepareIdentity() async {
    if (error != null) throw error!;
    return IdentityInfo(fingerprint: fingerprint, storage: 'Keychain');
  }
}

class GroupSdk extends KeysSdk {
  int createCalls = 0;
  @override
  Future<GroupInfo> createGroup() async {
    createCalls++;
    return GroupInfo(configured: true, epoch: 1);
  }
}

class EnrollmentSdk extends GroupSdk {
  bool configured = false;
  @override
  Future<GroupInfo> groupInfo() async =>
      GroupInfo(configured: configured, epoch: configured ? 2 : 0);
  @override
  Future<GroupInfo> createGroup() async {
    createCalls++;
    configured = true;
    return GroupInfo(configured: true, epoch: 1);
  }
}

void main() {
  test(
    'verifying the same protected identity does not create a bridge event',
    () async {
      final sdk = KeysSdk();
      final controller = LabController(sdk);
      await controller.refresh();
      await controller.prepareIdentity();
      final fingerprint = controller.identity!.fingerprint;
      await controller.prepareIdentity();
      expect(controller.identity!.fingerprint, fingerprint);
      expect(controller.events, isEmpty);
      expect(controller.failure, isNull);
      controller.dispose();
    },
  );
  test(
    'unavailable or malformed identity cannot leave a success visible',
    () async {
      final sdk = KeysSdk();
      final controller = LabController(sdk);
      await controller.prepareIdentity();
      expect(controller.identity, isNotNull);
      sdk.error = PlatformException(code: 'KEY_STORAGE_UNAVAILABLE');
      await controller.prepareIdentity();
      expect(controller.identity, isNull);
      expect(controller.failure!.code, 'KEY_STORAGE_UNAVAILABLE');
      sdk.error = null;
      sdk.fingerprint = 'invalid';
      await controller.prepareIdentity();
      expect(controller.identity, isNull);
      expect(controller.failure, isNotNull);
      controller.dispose();
    },
  );
  test(
    'group creation needs a verified identity and keeps the returned epoch',
    () async {
      final sdk = GroupSdk();
      final controller = LabController(sdk);
      await controller.createGroup();
      expect(sdk.createCalls, 0);
      expect(controller.group, isNull);
      expect(controller.failure, isNotNull);

      await controller.prepareIdentity();
      await controller.createGroup();
      expect(sdk.createCalls, 1);
      expect(controller.group!.configured, true);
      expect(controller.group!.epoch, 1);
      expect(controller.failure, isNull);
      controller.dispose();
    },
  );
  test(
    'preparing identity restores a group already stored by the native host',
    () async {
      final sdk = EnrollmentSdk()..configured = true;
      final controller = LabController(sdk);
      addTearDown(controller.dispose);

      await controller.prepareIdentity();

      expect(controller.group?.configured, true);
      expect(controller.group?.epoch, 2);
      expect(controller.notice, contains('grupo existente recuperados'));
    },
  );
}
