"""Apply the actual Sparkle patch to a temporary source copy and verify its bytes."""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
PACKAGE = ROOT / "packaging/updates/sparkle"
SPEC = importlib.util.spec_from_file_location("sdk_prepare", PACKAGE.parent / "prepare_sdk.py")
PREPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PREPARE)


def require(condition, message):
    if not condition:
        raise ValueError(message)


def verify(source):
    manifest = json.loads((PACKAGE / "patch-inputs.json").read_text(encoding="utf-8"))
    patch = PACKAGE / "hardening.patch"
    require(manifest["commit"] == "ab7d71291238df6e0492143b120c9b696a35f603", "Wrong source pin")
    require(hashlib.sha256(patch.read_bytes()).hexdigest() == manifest["patch_sha256"], "Patch digest mismatch")
    for name, digest in manifest["inputs"].items():
        require(hashlib.sha256((source / name).read_bytes()).hexdigest() == digest,
                f"Original source does not match the pin: {name}")
    for name in manifest["outputs"].keys() - manifest["inputs"].keys():
        require(not (source / name).exists(), f"New patch file already exists: {name}")
    with tempfile.TemporaryDirectory(prefix="sparkle-patch-check-") as temporary:
        private = Path(temporary) / "source"
        shutil.copytree(source, private, ignore=shutil.ignore_patterns(".git"))
        PREPARE.git(private, "init", "--quiet", "--template=", "--initial-branch=narumi/chore/sdk-source")
        PREPARE.git(private, "config", "core.autocrlf", "false")
        for args in (("--check", "--whitespace=error"), ("--whitespace=error",)):
            PREPARE.git(private, "apply", *args, str(patch))
        for name, digest in manifest["outputs"].items():
            require(hashlib.sha256((private / name).read_bytes()).hexdigest() == digest,
                    f"Patched bytes differ: {name}")
        # Applying twice must fail, not silently add another gate or change offsets.
        try:
            PREPARE.git(private, "apply", "--check", str(patch))
        except subprocess.CalledProcessError:
            pass
        else:
            raise ValueError("Patch unexpectedly applied twice")
    print(f"PASS: {len(manifest['outputs'])} exact patched files; clean apply; double-apply rejected")
    print(f"Patch SHA-256: {manifest['patch_sha256']}")
    print("LIMIT: patch reproduction is not a macOS build or native integration check")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    args = parser.parse_args()
    verify(args.source.resolve(strict=True))


if __name__ == "__main__":
    main()
