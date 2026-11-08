"""Fail-closed tests for the native build helper. These do not run native SDK code."""

import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("sparkle_build_sdk", ROOT / "packaging/updates/sparkle/build_sdk.py")
SDK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SDK)


class BuildHelperTests(unittest.TestCase):
    def test_non_mac_host_never_starts_xcode(self):
        with mock.patch.object(SDK.sys, "platform", "win32"), mock.patch.object(SDK.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "UNSUPPORTED"):
                SDK.build(Path("unused"), Path("unused"), "x86_64")
            run.assert_not_called()

    def test_wrong_process_architecture_rejected(self):
        with mock.patch.object(SDK.sys, "platform", "darwin"), mock.patch.object(SDK.platform, "machine", return_value="arm64"):
            with self.assertRaisesRegex(ValueError, "not by cross-compilation"):
                SDK.build(Path("unused"), Path("unused"), "x86_64")

    def inspect(self, architectures="x86_64\n", minimum="12.0"):
        with tempfile.TemporaryDirectory() as temporary:
            framework = Path(temporary)
            for name in ("Sparkle", "Autoupdate", "Updater", "Installer"):
                (framework / name).write_bytes(b"\xcf\xfa\xed\xfe" + b"fixture")

            def output(command, **kwargs):
                if "lipo" in command:
                    return architectures
                return "" if minimum is None else f"    minos {minimum}\n"

            with mock.patch.object(SDK.subprocess, "check_output", side_effect=output):
                return SDK.inspect_binaries(framework, "x86_64")

    def test_binary_metadata_at_minimum_is_accepted(self):
        self.assertEqual(len(self.inspect()), 4)

    def test_universal_binary_metadata_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Wrong binary architecture"):
            self.inspect(architectures="x86_64 arm64\n")

    def test_newer_minimum_os_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "above macOS 12"):
            self.inspect(minimum="13.0")

    def test_missing_minimum_os_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "above macOS 12"):
            self.inspect(minimum=None)

    def results(self):
        summary = {"totalTestCount": 2, "passedTests": 2, "failedTests": 0,
                   "skippedTests": 0, "expectedFailures": 0}
        tree = {"testNodes": [{"name": "SUHardenedInstallerTest", "children": [
            {"name": "testOne()", "result": "Passed"},
            {"nodeIdentifier": "SUHardenedInstallerTest/testTwo()", "result": "Passed"},
        ]}]}
        return summary, tree, {"testOne", "testTwo"}

    def test_required_executed_methods_are_accepted(self):
        SDK.validate_test_run(*self.results())

    def test_zero_selected_tests_is_not_a_pass(self):
        summary, tree, expected = self.results()
        summary.update(totalTestCount=0, passedTests=0)
        with self.assertRaisesRegex(ValueError, "counts"):
            SDK.validate_test_run(summary, tree, expected)

    def test_skipped_failed_or_missing_counters_are_not_a_pass(self):
        for field in ("skippedTests", "failedTests", "expectedFailures"):
            with self.subTest(field=field):
                summary, tree, expected = self.results()
                summary[field] = 1
                with self.assertRaises(ValueError):
                    SDK.validate_test_run(summary, tree, expected)
        summary, tree, expected = self.results()
        del summary["skippedTests"]
        with self.assertRaises(ValueError):
            SDK.validate_test_run(summary, tree, expected)

    def test_counts_alone_do_not_prove_expected_methods(self):
        summary, tree, expected = self.results()
        tree["testNodes"][0]["children"].pop()
        with self.assertRaisesRegex(ValueError, "execution evidence"):
            SDK.validate_test_run(summary, tree, expected)

    def test_failed_method_with_inconsistent_summary_is_rejected(self):
        summary, tree, expected = self.results()
        tree["testNodes"][0]["children"][0]["result"] = "Failed"
        with self.assertRaisesRegex(ValueError, "execution evidence"):
            SDK.validate_test_run(summary, tree, expected)

    def test_expected_methods_come_from_the_validated_source_class(self):
        with tempfile.TemporaryDirectory() as temporary:
            source = Path(temporary)
            (source / "Tests").mkdir()
            (source / "Tests/SUInstallerTest.m").write_text(
                "@implementation Other\n- (void)testOther {}\n@end\n"
                "@implementation SUHardenedInstallerTest\n"
                "- (void)testOne { }\n- (void)helper { }\n"
                "- (void)testTwo { }\n@end\n", encoding="utf-8")
            self.assertEqual(SDK.required_tests(source), {"testOne", "testTwo"})


if __name__ == "__main__":
    unittest.main()
