import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('identity_runs', Path(__file__).parents[1] / 'check_identity_runs.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def report(pid, fingerprint='ab' * 32, storage='Android Keystore'):
    return {'identity': {'fingerprint': fingerprint, 'storage': storage, 'processId': pid}}


class IdentityRunsTests(unittest.TestCase):
    def test_facade_reopen_does_not_count_as_process_recovery(self):
        with self.assertRaises(ValueError):
            module.validate([report(1), report(1)])

    def test_identity_rotation_does_not_count_as_recovery(self):
        with self.assertRaises(ValueError):
            module.validate([report(1), report(2, 'cd' * 32)])

    def test_single_device_does_not_close_the_two_phone_gate(self):
        result = module.validate([report(1), report(2)])
        self.assertTrue(result['passed'])
        self.assertFalse(result['both_platforms_verified'])

    def test_two_platforms_with_distinct_identities_and_new_processes_pass(self):
        result = module.validate([report(1), report(2), report(3, 'cd' * 32, 'Keychain'), report(4, 'cd' * 32, 'Keychain')])
        self.assertTrue(result['both_platforms_verified'])

    def test_same_identity_on_both_platforms_is_rejected(self):
        with self.assertRaises(ValueError):
            module.validate([report(1), report(2), report(3, storage='Keychain'), report(4, storage='Keychain')])

    def test_two_runs_from_same_platform_do_not_close_full_gate(self):
        with self.assertRaises(ValueError):
            module.validate([report(1), report(2), report(3, 'cd' * 32), report(4, 'cd' * 32)])

    def test_missing_malformed_metadata_and_incomplete_pairs_are_rejected(self):
        for reports in [[report(1)], [report(1), report(True)], [report(1), report(-1)], [report(1), report(2, 'invalid')], [{}, report(1)], [{"identity": None}, report(1)], [{"identity": []}, report(1)]]:
            with self.assertRaises((ValueError, KeyError)):
                module.validate(reports)


if __name__ == '__main__':
    unittest.main()
