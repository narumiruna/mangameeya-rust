"""Portable fail-closed checks for the Windows SDK build helper."""

import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("winsparkle_build_sdk", ROOT / "packaging/updates/winsparkle/build_sdk.py")
SDK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SDK)


class BuildHelperTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="winsparkle-build-tests-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "prepared/source"
        self.source.mkdir(parents=True)
        self.output = self.root / "target/native"
        pins = json.loads((SDK.HERE.parent / "sdk-sources.json").read_text(encoding="utf-8"))
        expected = pins["sdks"]["winsparkle"]
        self.provenance = {
            "schema": 1, "sdk": "winsparkle", "commit": expected["commit"],
            "source_sha256": expected["sha256"], "hardening_version": 1,
            "patch_sha256": hashlib.sha256((SDK.HERE / "hardening.patch").read_bytes()).hexdigest(),
        }
        self.provenance_file = self.source.parent / "provenance.json"
        self.write_provenance()

    def write_provenance(self):
        self.provenance_file.write_text(json.dumps(self.provenance), encoding="utf-8")

    def test_wrong_platform_never_starts_tools(self):
        with mock.patch.object(SDK.sys, "platform", "darwin"), \
                mock.patch.object(SDK, "toolchain") as tools, \
                mock.patch.object(SDK.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "UNSUPPORTED"):
                SDK.build(self.source, self.output)
            tools.assert_not_called()
            run.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_32_bit_python_rejected(self):
        with mock.patch.object(SDK.sys, "platform", "win32"), \
                mock.patch.object(SDK.ctypes, "sizeof", return_value=4), \
                mock.patch.object(SDK, "toolchain") as tools:
            with self.assertRaisesRegex(ValueError, "64-bit"):
                SDK.build(self.source, self.output)
            tools.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_valid_provenance(self):
        SDK.validate_source(self.source)

    def test_wrong_provenance_rejected(self):
        for key in ("schema", "sdk", "commit", "source_sha256", "patch_sha256", "hardening_version"):
            with self.subTest(key=key):
                original = self.provenance[key]
                self.provenance[key] = "wrong"
                self.write_provenance()
                with self.assertRaises(ValueError):
                    SDK.validate_source(self.source)
                self.provenance[key] = original
        self.write_provenance()

    def test_existing_output_preserved(self):
        self.output.mkdir(parents=True)
        keep = self.output / "keep.txt"
        keep.write_text("user file", encoding="utf-8")
        with mock.patch.object(SDK.sys, "platform", "win32"), \
                mock.patch.object(SDK, "ROOT", self.root), \
                mock.patch.object(SDK, "toolchain") as tools:
            with self.assertRaisesRegex(ValueError, "already exists"):
                SDK.build(self.source, self.output)
            tools.assert_not_called()
        self.assertEqual(keep.read_text(encoding="utf-8"), "user file")

    def test_output_outside_target_rejected(self):
        outside = self.root / "src"
        with mock.patch.object(SDK.sys, "platform", "win32"), \
                mock.patch.object(SDK, "ROOT", self.root), \
                mock.patch.object(SDK, "toolchain") as tools:
            with self.assertRaisesRegex(ValueError, "under target"):
                SDK.build(self.source, outside)
            tools.assert_not_called()
        self.assertFalse(outside.exists())

    def test_bad_source_stops_before_output_and_toolchain(self):
        self.provenance["patch_sha256"] = "0" * 64
        self.write_provenance()
        with mock.patch.object(SDK.sys, "platform", "win32"), \
                mock.patch.object(SDK, "ROOT", self.root), \
                mock.patch.object(SDK, "toolchain") as tools:
            with self.assertRaisesRegex(ValueError, "patch"):
                SDK.build(self.source, self.output)
            tools.assert_not_called()
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
