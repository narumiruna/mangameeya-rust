"""Offline WinSparkle SDK smoke check, not an application updater test.

Run on native Windows with the official pinned archive. This does not initialize
WinSparkle, access the registry, use the network, or execute an installer.
"""

import argparse
import base64
import ctypes
import hashlib
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile


ARCHIVE_SHA256 = "6037df37fc263bd1650a1c4949681a9d40ffe991d01f35892a406cb5d103c976"
ARCHIVE_SIZE = 27_366_440
ARCHIVE_ROOT = "WinSparkle-0.9.4"
# RFC 8032 public test vector, not an application trust anchor.
TEST_PUBLIC_KEY = bytes.fromhex(
    "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
)
EXPORTS = (
    "win_sparkle_init",
    "win_sparkle_cleanup",
    "win_sparkle_set_app_details",
    "win_sparkle_set_appcast_url",
    "win_sparkle_set_eddsa_public_key",
    "win_sparkle_set_automatic_check_for_updates",
    "win_sparkle_get_automatic_check_for_updates",
    "win_sparkle_set_config_methods",
    "win_sparkle_set_can_shutdown_callback",
    "win_sparkle_set_shutdown_request_callback",
    "win_sparkle_set_user_run_installer_callback",
    "win_sparkle_set_update_cancelled_callback",
)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def command(tool, *args):
    return subprocess.run(
        [str(tool), *map(str, args)],
        cwd=tool.parent,
        stdin=subprocess.DEVNULL,
        text=True,
        encoding="utf-8",
        capture_output=True,
        timeout=30,
        check=False,
    )


def check_dll(dll):
    # Run only in a child process. Its exit unloads the DLL before parent cleanup.
    # No init call: the DLL never starts a worker or UI callback.
    # Search only the verified DLL directory and System32 for dependencies.
    sdk = ctypes.CDLL(str(dll), winmode=0x00000100 | 0x00000800)
    for name in EXPORTS:
        getattr(sdk, name)
    print(f"PASS: native x64 DLL load and {len(EXPORTS)} required exports")

    set_key = sdk.win_sparkle_set_eddsa_public_key
    set_key.argtypes = [ctypes.c_char_p]
    set_key.restype = ctypes.c_int
    require(set_key(base64.b64encode(TEST_PUBLIC_KEY)) == 1, "Valid public key rejected")
    require(set_key(b"invalid-public-key") == 0, "Invalid public key accepted")
    print("PASS: C ABI public-key validation")


def check_sdk(archive, work):
    # Hash and extract the same immutable bytes, not a file that can change.
    with archive.open("rb") as source:
        contents = source.read(ARCHIVE_SIZE + 1)
    require(
        len(contents) == ARCHIVE_SIZE
        and hashlib.sha256(contents).hexdigest() == ARCHIVE_SHA256,
        "The archive does not match the approved WinSparkle 0.9.4 SHA-256",
    )
    with zipfile.ZipFile(io.BytesIO(contents)) as package:
        dll = work / "WinSparkle.dll"
        tool = work / "winsparkle-tool.exe"
        dll.write_bytes(package.read(f"{ARCHIVE_ROOT}/x64/Release/WinSparkle.dll"))
        tool.write_bytes(package.read(f"{ARCHIVE_ROOT}/bin/winsparkle-tool.exe"))

    probe = subprocess.run(
        [sys.executable, "-I", "-c",
         "import importlib.util, sys; "
         "s = importlib.util.spec_from_file_location('sdk_probe', sys.argv[1]); "
         "m = importlib.util.module_from_spec(s); s.loader.exec_module(m); "
         "m.check_dll(sys.argv[2])",
         str(Path(__file__).resolve()), str(dll)],
        cwd=work,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=30,
        check=False,
    )
    require(probe.returncode == 0, "Native DLL probe failed: " + probe.stderr)
    print(probe.stdout, end="")

    # The private fixture key is temporary and is never printed or retained.
    key = work / "fixture-key.pem"
    payload = work / "non-executable-fixture.txt"
    original = b"MangaMeeya updater SDK preflight. Never execute this fixture.\n"
    payload.write_bytes(original)
    require(command(tool, "generate-key", "--file", key).returncode == 0,
            "Fixture key generation failed")
    public_result = command(tool, "public-key", "--private-key-file", key)
    require(public_result.returncode == 0, "Fixture public-key extraction failed")
    public_key = next(
        line.removeprefix("Public key: ")
        for line in public_result.stdout.splitlines()
        if line.startswith("Public key: ")
    )
    require(len(base64.b64decode(public_key, validate=True)) == 32,
            "Unexpected fixture public-key format")
    signed = command(tool, "sign", "--private-key-file", key, payload)
    require(signed.returncode == 0, "Fixture signing failed")
    signature = signed.stdout.strip()
    require(len(base64.b64decode(signature, validate=True)) == 64,
            "Unexpected fixture signature format")

    def verify(public_key=public_key, signature=signature):
        return command(tool, "verify", "--public-key", public_key,
                       "--signature", signature, payload).returncode

    require(verify() == 0, "Valid signed fixture rejected")
    print("PASS: official native tool verifies a signed text fixture")
    require(verify(public_key=base64.b64encode(TEST_PUBLIC_KEY).decode("ascii")) != 0,
            "Wrong public key accepted")
    require(verify(signature="invalid") != 0, "Malformed signature accepted")
    require(verify(signature="") != 0, "Missing signature accepted")
    print("PASS: wrong key, malformed signature, and missing signature rejected")
    payload.write_bytes(original + b"modified\n")
    require(verify() != 0, "Modified payload accepted")
    payload.write_bytes(original[:8])
    require(verify() != 0, "Truncated payload accepted")
    print("PASS: modified and truncated payloads rejected")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path, help="official WinSparkle-0.9.4.zip")
    args = parser.parse_args()
    if sys.platform != "win32" or ctypes.sizeof(ctypes.c_void_p) != 8:
        parser.error("Run with native 64-bit Windows Python through uv run python")
    with tempfile.TemporaryDirectory(prefix="mangameeya-sdk-preflight-") as directory:
        check_sdk(args.archive, Path(directory))
    print("LIMIT: GPUI integration, SDK callbacks, installation, shutdown, and relaunch "
          "are not tested; this is not a security-gate pass")


if __name__ == "__main__":
    main()
