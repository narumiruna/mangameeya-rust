"""Bounded DMG verification checks with no native tools or disk mounts."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
VERIFY_SCRIPT = ROOT / "packaging/macos/verify-dmg.sh"

MOCK_COMMAND = r'''
import json
import os
from pathlib import Path
import sys

command, *args = sys.argv[1:]
state_path = Path(os.environ["DMG_TEST_STATE"])
state = json.loads(state_path.read_text())
call = {"command": command, "args": args}
if command == "hdiutil":
    call.update(locale=os.environ.get("LC_ALL"), stdin=sys.stdin.read())
with open(os.environ["DMG_TEST_LOG"], "a") as log:
    log.write(json.dumps(call) + "\n")

if command == "hdiutil":
    if args != ["verify", state["image"]]:
        sys.exit("Unexpected hdiutil command")
    index = state["attempts"]
    state["attempts"] += 1
    state_path.write_text(json.dumps(state))
    response = state["responses"][min(index, len(state["responses"]) - 1)]
    print(response["output"], file=sys.stderr if response["status"] else sys.stdout)
    sys.exit(response["status"])
elif command == "sleep" and args == ["2"]:
    pass
else:
    sys.exit("Unexpected command: " + command + " " + repr(args))
'''


class MacosDmgVerificationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="dmg-verification-")
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.image = self.directory / "image with spaces; [日本語].dmg"
        self.image.write_bytes(b"mock disk image")
        self.state_path = self.directory / "state.json"
        self.log_path = self.directory / "commands.jsonl"
        mock = self.directory / "mock.py"
        mock.write_text(MOCK_COMMAND)
        bin_directory = self.directory / "bin"
        bin_directory.mkdir()
        for command in ("hdiutil", "sleep"):
            executable = bin_directory / command
            executable.write_text(
                '#!/bin/sh\nexec "$DMG_TEST_PYTHON" "$DMG_TEST_MOCK" '
                f'{command} "$@"\n'
            )
            executable.chmod(0o755)
        self.environment = {
            **os.environ,
            "PATH": str(bin_directory) + os.pathsep + os.environ.get("PATH", ""),
            "DMG_TEST_PYTHON": sys.executable,
            "DMG_TEST_MOCK": str(mock),
            "DMG_TEST_STATE": str(self.state_path),
            "DMG_TEST_LOG": str(self.log_path),
        }

    def verify(self, responses, arguments=None):
        self.log_path.unlink(missing_ok=True)
        self.state_path.write_text(json.dumps({
            "image": str(self.image), "responses": responses, "attempts": 0,
        }))
        return subprocess.run(
            ["bash", str(VERIFY_SCRIPT),
             *([str(self.image)] if arguments is None else arguments)],
            env=self.environment, cwd=self.directory, input="do not pass to hdiutil\n",
            text=True, capture_output=True, timeout=10,
        )

    def calls(self, command):
        calls = ([json.loads(line) for line in self.log_path.read_text().splitlines()]
                 if self.log_path.exists() else [])
        return [call for call in calls if call["command"] == command]

    def assert_attempts(self, expected, sleeps):
        calls = self.calls("hdiutil")
        self.assertEqual(len(calls), expected)
        for call in calls:
            self.assertEqual(call["args"], ["verify", str(self.image)])
            self.assertEqual(call["locale"], "C")
            self.assertEqual(call["stdin"], "")
        self.assertEqual(len(self.calls("sleep")), sleeps)

    def test_success_does_not_retry(self):
        result = self.verify([{"status": 0, "output": "checksum is VALID"}])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("checksum is VALID", result.stdout)
        self.assertEqual(result.stderr, "")
        self.assert_attempts(1, 0)

    def test_transient_failures_retry_the_same_image_then_succeed(self):
        transient = {"status": 1, "output": "Resource temporarily unavailable"}
        for failures in (1, 2):
            with self.subTest(failures=failures):
                result = self.verify(
                    [transient] * failures + [{"status": 0, "output": "verified"}]
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("verified", result.stdout)
                self.assertEqual(result.stderr.count("Resource temporarily unavailable"), failures)
                self.assertEqual(result.stderr.count("retrying"), failures)
                self.assert_attempts(failures + 1, failures)

    def test_persistent_transient_failure_stops_after_three_attempts(self):
        result = self.verify([{"status": 35, "output": "Resource temporarily unavailable"}])
        self.assertEqual(result.returncode, 35)
        self.assertNotIn("verified", result.stdout)
        self.assertIn("Resource temporarily unavailable", result.stderr)
        self.assertEqual(result.stderr.count("retrying"), 2)
        self.assert_attempts(3, 2)

    def test_other_verification_errors_fail_without_retry(self):
        for error in ("checksum is INVALID", "image not recognized", "Permission denied"):
            with self.subTest(error=error):
                result = self.verify([{"status": 12, "output": error}])
                self.assertEqual(result.returncode, 12)
                self.assertIn(error, result.stderr)
                self.assertNotIn("retrying", result.stderr)
                self.assert_attempts(1, 0)

    def test_permanent_error_after_transient_failure_stops_immediately(self):
        result = self.verify([
            {"status": 35, "output": "Resource temporarily unavailable"},
            {"status": 13, "output": "checksum is INVALID"},
        ])
        self.assertEqual(result.returncode, 13)
        self.assertIn("checksum is INVALID", result.stderr)
        self.assert_attempts(2, 1)

    def test_invalid_arguments_never_call_native_tools(self):
        for arguments in ([], [str(self.image), "extra"], [str(self.image) + ".missing"]):
            with self.subTest(arguments=arguments):
                result = self.verify([], arguments)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Usage:", result.stderr)
                self.assert_attempts(0, 0)


if __name__ == "__main__":
    unittest.main()
