import sys
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_new_code_coverage as coverage


class NewCodeCoverageTests(unittest.TestCase):
    def run_gate(self, flutter_lcov: str) -> subprocess.CompletedProcess[str]:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        tools = root / "tools"
        tools.mkdir()
        shutil.copy(Path(coverage.__file__), tools / "check_new_code_coverage.py")
        source = root / "app/lib/feature.dart"
        source.parent.mkdir(parents=True)
        source.write_text("void existing() {}\n", encoding="utf-8")
        subprocess.run(["git", "init", "-q"], cwd=root, check=True)
        subprocess.run(["git", "add", "."], cwd=root, check=True)
        subprocess.run(
            ["git", "-c", "user.name=Coverage", "-c", "user.email=coverage@test", "commit", "-qm", "base"],
            cwd=root,
            check=True,
        )
        source.write_text("void existing() {}\nvoid added() {}\n", encoding="utf-8")
        subprocess.run(["git", "add", "."], cwd=root, check=True)
        subprocess.run(
            ["git", "-c", "user.name=Coverage", "-c", "user.email=coverage@test", "commit", "-qm", "change"],
            cwd=root,
            check=True,
        )
        rust_lcov = root / "rust.lcov"
        rust_lcov.write_text("", encoding="utf-8")
        flutter_report = root / "flutter.lcov"
        flutter_report.write_text(flutter_lcov, encoding="utf-8")
        return subprocess.run(
            [
                sys.executable,
                str(tools / "check_new_code_coverage.py"),
                "--base",
                "HEAD~1",
                "--rust-lcov",
                str(rust_lcov),
                "--flutter-lcov",
                str(flutter_report),
            ],
            cwd=root,
            text=True,
            capture_output=True,
        )

    def test_only_new_instrumented_lines_count_toward_gate(self):
        changed = {"app/lib/example.dart": {1, 2, 3}, "crates/example/src/lib.rs": {4, 5}}
        measured = {"app/lib/example.dart": {2: 1, 3: 0}, "crates/example/src/lib.rs": {4: 4}}
        self.assertEqual(coverage.report(changed, measured), (2, 3))

    def test_only_product_rust_and_dart_sources_are_subject_to_gate(self):
        self.assertTrue(coverage.source_path("crates/mesh-store/src/lib.rs"))
        self.assertTrue(coverage.source_path("app/lib/main.dart"))
        self.assertTrue(
            coverage.source_path("packages/mesh_field_sdk/lib/mesh_field_sdk.dart")
        )
        self.assertFalse(coverage.source_path("crates/mesh-store/tests/store.rs"))
        self.assertFalse(
            coverage.source_path("packages/mesh_field_sdk/test/client_test.dart")
        )
        self.assertFalse(coverage.source_path("platforms/mesh_host/android/Main.kt"))

    def test_flutter_lcov_paths_map_to_repository_paths(self):
        self.assertEqual(
            coverage.normalized("lib/core/sdk/lab_controller.dart"),
            "app/lib/core/sdk/lab_controller.dart",
        )
        self.assertEqual(
            coverage.normalized(
                "lib/src/field_mesh_client.dart",
                coverage.ROOT / "packages/mesh_field_sdk",
            ),
            "packages/mesh_field_sdk/lib/src/field_mesh_client.dart",
        )

    def test_new_product_file_without_lcov_data_is_rejected(self):
        changed = {
            "app/lib/new_feature.dart": {1, 2},
            "crates/mesh-new/src/lib.rs": {1},
        }
        measured = {"crates/mesh-new/src/lib.rs": {1: 1}}
        self.assertEqual(
            coverage.unmeasured_sources(changed, measured),
            ["app/lib/new_feature.dart"],
        )

    def test_cli_enforces_coverage_from_a_real_git_diff(self):
        result = self.run_gate("SF:lib/feature.dart\nDA:1,1\nDA:2,1\nend_of_record\n")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("New executable code coverage: 1/1 (100.00%)", result.stdout)

    def test_cli_rejects_changed_product_file_absent_from_lcov(self):
        result = self.run_gate("")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("coverage emitted no executable-line data", result.stdout)


if __name__ == "__main__":
    unittest.main()
