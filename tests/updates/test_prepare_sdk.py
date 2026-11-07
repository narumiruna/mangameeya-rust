"""Source-preparation safety checks. No SDK initialization or remote requests."""

import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import tarfile
import tempfile
import unittest
from unittest.mock import patch
from urllib.request import Request
import zipfile


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "prepare_sdk", ROOT / "packaging/updates/prepare_sdk.py")
PREPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARE)

PATCH = b"""diff --git a/hello.txt b/hello.txt
--- a/hello.txt
+++ b/hello.txt
@@ -1 +1 @@
-before
+after
"""


def zip_bytes(entries):
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w") as archive:
        for name, data, mode in entries:
            member = zipfile.ZipInfo(name)
            # Keep malformed raw names; ZipInfo otherwise normalizes them on Windows.
            member.filename = name
            member.orig_filename = name
            member.create_system = 3
            member.external_attr = mode << 16
            archive.writestr(member, data)
    return output.getvalue()


def regular(name, data=b"before\n"):
    return name, data, stat.S_IFREG | 0o644


def tar_bytes(member):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as archive:
        archive.addfile(member, io.BytesIO(b"x" * member.size) if member.isfile() else None)
    return output.getvalue()


class SourcePreparationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="sdk-prepare-tests-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.destination = self.root / "extracted"
        self.destination.mkdir()

    def assert_rejected_without_writes(self, data, archive_format="zip", prefix=""):
        with self.assertRaises(ValueError):
            PREPARE.extract_source(data, archive_format, prefix, self.destination)
        self.assertEqual(list(self.destination.iterdir()), [])

    def fixture(self, data=None, source_patch=PATCH):
        data = data if data is not None else zip_bytes([regular("hello.txt")])
        archive = self.root / "source.zip"
        archive.write_bytes(data)
        folder = self.root / "pins"
        folder.mkdir()
        source = {
            "version": "1.0.0",
            "commit": "a" * 40,
            "url": "https://example.invalid/source.zip",
            "sha256": hashlib.sha256(data).hexdigest(),
            "size": len(data),
            "format": "zip",
            "prefix": "",
            "patch": "winsparkle/hardening.patch",
        }
        other = dict(source, patch="sparkle/hardening.patch")
        pins = {"schema": 1, "hardening_version": 1,
                "sdks": {"winsparkle": source, "sparkle": other}}
        pin_file = folder / "sdk-sources.json"
        pin_file.write_text(json.dumps(pins), encoding="utf-8")
        (folder / "winsparkle").mkdir()
        (folder / "winsparkle/hardening.patch").write_bytes(source_patch)
        return archive, pin_file, source

    def test_checked_in_pins(self):
        pins = PREPARE.load_pins()
        self.assertEqual(set(pins["sdks"]), {"winsparkle", "sparkle"})
        self.assertEqual(pins["hardening_version"], 1)
        for source in pins["sdks"].values():
            self.assertTrue(source["url"].startswith("https://"))
            self.assertEqual(len(source["sha256"]), 64)

    def test_pin_size_hash_and_immutable_snapshot(self):
        archive, _, source = self.fixture()
        snapshot = PREPARE.archive_bytes(source, archive)
        archive.write_bytes(b"x" * len(snapshot))
        self.assertEqual(hashlib.sha256(snapshot).hexdigest(), source["sha256"])
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            PREPARE.archive_bytes(source, archive)
        archive.write_bytes(b"x")
        with self.assertRaisesRegex(ValueError, "size"):
            PREPARE.archive_bytes(source, archive)
        archive.write_bytes(snapshot + b"x")
        with self.assertRaisesRegex(ValueError, "size"):
            PREPARE.archive_bytes(source, archive)

    def test_no_insecure_source_redirect(self):
        redirect = PREPARE.HttpsRedirects()
        request = Request("https://example.invalid/source")
        with self.assertRaisesRegex(ValueError, "HTTPS"):
            redirect.redirect_request(request, None, 302, "Found", {}, "http://example.invalid/source")

    def test_zip_prefix_and_file_mode(self):
        data = zip_bytes([("root/", b"", stat.S_IFDIR | 0o755),
                          regular("root/folder/readme.txt"),
                          ("root/build.sh", b"#!/bin/sh\n", stat.S_IFREG | 0o755)])
        PREPARE.extract_source(data, "zip", "root", self.destination)
        self.assertEqual((self.destination / "folder/readme.txt").read_bytes(), b"before\n")
        if os.name != "nt":
            self.assertTrue((self.destination / "build.sh").stat().st_mode & stat.S_IXUSR)
            self.assertFalse((self.destination / "folder/readme.txt").stat().st_mode & stat.S_IXUSR)

    def test_tar_prefix(self):
        member = tarfile.TarInfo("root/hello.txt")
        member.mode = 0o644
        member.size = 3
        PREPARE.extract_source(tar_bytes(member), "tar.gz", "root", self.destination)
        self.assertEqual((self.destination / "hello.txt").read_bytes(), b"xxx")

    def test_unsafe_paths_fail_before_any_source_write(self):
        for name in ("../escape", "/absolute", "C:/drive", "folder\\escape", "folder//alias",
                     "folder/./alias", "folder/../escape", "hello:stream", ".git/config",
                     "folder/CON.txt", "folder/name.", "folder/name ",
                     "evil\0suffix"):
            with self.subTest(name=name):
                self.assert_rejected_without_writes(zip_bytes([regular("valid.txt"), regular(name)]))

    def test_wrong_prefix(self):
        self.assert_rejected_without_writes(zip_bytes([regular("other/file")]), prefix="root")

    def test_zip_links_and_special_files(self):
        for kind in (stat.S_IFLNK, stat.S_IFIFO, stat.S_IFCHR, stat.S_IFSOCK):
            with self.subTest(kind=kind):
                self.assert_rejected_without_writes(zip_bytes([("unsafe", b"target", kind | 0o777)]))

    def test_zip_type_mismatch(self):
        self.assert_rejected_without_writes(zip_bytes([("directory", b"", stat.S_IFDIR | 0o755)]))

    def test_tar_links_and_special_files(self):
        for kind in (tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.FIFOTYPE, tarfile.CHRTYPE):
            with self.subTest(kind=kind):
                member = tarfile.TarInfo("unsafe")
                member.type = kind
                member.linkname = "../outside"
                self.assert_rejected_without_writes(tar_bytes(member), "tar.gz")

    def test_duplicates_case_aliases_and_file_parents(self):
        for names in (("same", "same"), ("Same", "same"), ("parent", "parent/child")):
            with self.subTest(names=names):
                self.assert_rejected_without_writes(zip_bytes([regular(name) for name in names]))

    def test_member_and_expansion_limits(self):
        data = zip_bytes([regular("one"), regular("two")])
        with patch.object(PREPARE, "MAX_MEMBERS", 1):
            self.assert_rejected_without_writes(data)
        with patch.object(PREPARE, "MAX_UNPACKED_BYTES", 1):
            self.assert_rejected_without_writes(data)

    def test_existing_output_preserved(self):
        archive, pin_file, _ = self.fixture()
        output = self.root / "target/sdk"
        output.mkdir(parents=True)
        (output / "keep.txt").write_text("user data", encoding="utf-8")
        with patch.object(PREPARE, "ROOT", self.root), patch.object(PREPARE, "PIN_FILE", pin_file):
            with self.assertRaisesRegex(ValueError, "already exists"):
                PREPARE.prepare("winsparkle", output, archive)
        self.assertEqual((output / "keep.txt").read_text(encoding="utf-8"), "user data")

    def test_output_outside_target_rejected(self):
        archive, pin_file, _ = self.fixture()
        with patch.object(PREPARE, "ROOT", self.root), patch.object(PREPARE, "PIN_FILE", pin_file):
            with self.assertRaisesRegex(ValueError, "under target"):
                PREPARE.prepare("winsparkle", self.root / "src", archive)
        self.assertFalse((self.root / "src").exists())

    def test_patch_preflight_and_provenance(self):
        archive, pin_file, source = self.fixture()
        output = self.root / "target/sdk"
        with patch.object(PREPARE, "ROOT", self.root), patch.object(PREPARE, "PIN_FILE", pin_file):
            result = PREPARE.prepare("winsparkle", output, archive)
        self.assertEqual((result / "hello.txt").read_bytes(), b"after\n")
        provenance = json.loads((output / "provenance.json").read_text(encoding="utf-8"))
        self.assertEqual(provenance["source_sha256"], source["sha256"])
        self.assertEqual(provenance["patch_sha256"], hashlib.sha256(PATCH).hexdigest())
        self.assertEqual(provenance["hardening_version"], 1)
        self.assertTrue((result / ".git").is_dir())

    def test_patch_mismatch_never_publishes_partial_source(self):
        archive, pin_file, _ = self.fixture(source_patch=PATCH.replace(b"-before", b"-wrong"))
        output = self.root / "target/sdk"
        with patch.object(PREPARE, "ROOT", self.root), patch.object(PREPARE, "PIN_FILE", pin_file):
            with self.assertRaises(Exception) as caught:
                PREPARE.prepare("winsparkle", output, archive)
            self.assertIsInstance(caught.exception, PREPARE.subprocess.CalledProcessError)
        self.assertFalse(output.exists())
        self.assertEqual(list(output.parent.glob(".sdk-prepare-*")), [])

    def test_git_environment_does_not_redirect_patch_writes(self):
        archive, pin_file, _ = self.fixture()
        output = self.root / "target/sdk"
        with patch.object(PREPARE, "ROOT", self.root), patch.object(PREPARE, "PIN_FILE", pin_file), \
                patch.dict(os.environ, {"GIT_DIR": str(self.root / "not-a-repo"),
                                        "GIT_WORK_TREE": str(self.root / "outside"),
                                        "GIT_CONFIG_COUNT": "1", "GIT_CONFIG_KEY_0": "core.worktree",
                                        "GIT_CONFIG_VALUE_0": str(self.root / "outside")}):
            result = PREPARE.prepare("winsparkle", output, archive)
        self.assertEqual((result / "hello.txt").read_bytes(), b"after\n")
        self.assertFalse((self.root / "outside").exists())


if __name__ == "__main__":
    unittest.main()
