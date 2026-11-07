"""Release checks that do not write to GitHub or change the source checkout."""

import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[2]
UPLOAD_SCRIPT = ROOT / "packaging/upload-release.sh"
PACKAGE = "mangameeya-rust"
VERSION = "0.2.1"
TAG = f"v{VERSION}"
COMMIT = "0123456789abcdef0123456789abcdef01234567"
ASSET_NAMES = (
    f"MangaMeeya-Rust_{VERSION}_windows-setup.exe",
    f"MangaMeeya-Rust_{VERSION}_aarch64-apple-darwin.zip",
    f"MangaMeeya-Rust_{VERSION}_aarch64-apple-darwin.dmg",
    f"MangaMeeya-Rust_{VERSION}_x86_64-apple-darwin.zip",
    f"MangaMeeya-Rust_{VERSION}_x86_64-apple-darwin.dmg",
)

# Both commands are replaced on PATH. Unknown commands fail, never run real tools.
MOCK_COMMAND = r'''
import json
import os
from pathlib import Path
import sys

command, *args = sys.argv[1:]
state_path = Path(os.environ["RELEASE_TEST_STATE"])
state = json.loads(state_path.read_text())
with open(os.environ["RELEASE_TEST_LOG"], "a") as log:
    log.write(json.dumps({"command": command, "args": args,
                          "repo": os.environ.get("GH_REPO")}) + "\n")

def fail(message, code=1):
    print(message, file=sys.stderr)
    sys.exit(code)

if any(arg.split("=", 1)[0] == "--clobber" for arg in args):
    fail("Asset replacement is forbidden")

if command == "git":
    expected = ["rev-parse", "--verify", "refs/tags/" + state["tag"] + "^{commit}"]
    if args != expected:
        fail("Unexpected git command: " + repr(args))
    if state.get("git_error"):
        fail("Tag lookup failed", state["git_error"])
    print(state["commit"])
elif command == "gh" and args[:2] == ["release", "view"]:
    if args[2:] != [state["tag"], "--json", "tagName,isDraft,assets"]:
        fail("Unexpected release view: " + repr(args))
    if state.get("view_error"):
        fail("GitHub API unavailable", state["view_error"])
    print(state.get("view_output", json.dumps(state["release"])))
elif command == "gh" and args[:2] == ["release", "upload"]:
    if len(args) < 4 or args[2] != state["tag"]:
        fail("Unexpected release upload: " + repr(args))
    for argument in args[3:]:
        if argument.startswith("-"):
            fail("Unexpected upload flag: " + argument)
        path = Path(argument)
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            fail("Unsafe upload: " + argument)
        assets = state["release"]["assets"]
        if any(asset["name"] == path.name for asset in assets):
            fail("Attempt to replace existing asset: " + path.name)
        limit = state.get("fail_after")
        if limit is not None and len(state["uploaded"]) >= limit:
            fail("Simulated interrupted upload")
        assets.append({"name": path.name, "id": len(assets) + 100,
                       "size": path.stat().st_size, "state": "uploaded"})
        state["uploaded"].append(path.name)
        state_path.write_text(json.dumps(state))
else:
    fail("Unexpected command: " + command + " " + repr(args))
'''


class UploadReleaseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not UPLOAD_SCRIPT.is_file():
            raise AssertionError(f"Missing upload script: {UPLOAD_SCRIPT}")
        if not shutil.which("bash") or not shutil.which("jq"):
            raise AssertionError("The upload checks require bash and jq on PATH")

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="release-tests-")
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.assets = self.directory / "native assets"
        self.assets.mkdir()
        for name in ASSET_NAMES:
            (self.assets / name).write_bytes(b"test native package\n")
        self.state_path = self.directory / "state.json"
        self.log_path = self.directory / "commands.jsonl"
        self.write_state({
            "tag": TAG,
            "commit": COMMIT,
            "release": {"tagName": TAG, "isDraft": False, "assets": []},
            "uploaded": [],
        })
        mock = self.directory / "mock.py"
        mock.write_text(MOCK_COMMAND)
        bin_directory = self.directory / "bin"
        bin_directory.mkdir()
        for command in ("gh", "git"):
            executable = bin_directory / command
            executable.write_text(
                '#!/bin/sh\nexec "$RELEASE_TEST_PYTHON" "$RELEASE_TEST_MOCK" '
                f'{command} "$@"\n'
            )
            executable.chmod(0o755)
        self.environment = {
            **os.environ,
            "PATH": str(bin_directory) + os.pathsep + os.environ.get("PATH", ""),
            "GH_REPO": "release-tests/mangameeya-rust",
            "GH_TOKEN": "not-a-real-token",
            "RELEASES": json.dumps([{
                "package": PACKAGE, "version": VERSION, "tag": TAG,
            }]),
            "RELEASE_COMMIT": COMMIT,
            "RELEASE_TEST_PYTHON": sys.executable,
            "RELEASE_TEST_MOCK": str(mock),
            "RELEASE_TEST_STATE": str(self.state_path),
            "RELEASE_TEST_LOG": str(self.log_path),
        }

    def read_state(self):
        return json.loads(self.state_path.read_text())

    def write_state(self, state):
        self.state_path.write_text(json.dumps(state))

    def calls(self, command=None):
        if not self.log_path.exists():
            return []
        calls = [json.loads(line) for line in self.log_path.read_text().splitlines()]
        return [call for call in calls if command is None or call["command"] == command]

    def run_upload(self, environment=None, arguments=None):
        env = self.environment.copy()
        for name, value in (environment or {}).items():
            if value is None:
                env.pop(name, None)
            else:
                env[name] = value
        result = subprocess.run(
            ["bash", str(UPLOAD_SCRIPT),
             *([str(self.assets)] if arguments is None else arguments)],
            cwd=self.directory, env=env, text=True, capture_output=True, timeout=30,
        )
        for call in self.calls():
            self.assertFalse(
                any(arg.split("=", 1)[0] == "--clobber" for arg in call["args"]),
                call,
            )
        return result

    def assert_succeeded(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def assert_no_uploads(self, result, *, no_api=False):
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.read_state()["uploaded"], [])
        for call in self.calls("gh"):
            self.assertEqual(call["args"][:2], ["release", "view"], call)
        if no_api:
            self.assertEqual(self.calls("gh"), [])

    def existing_assets(self, names):
        return [{"name": name, "id": index, "size": 101, "state": "uploaded",
                 "label": "keep this"} for index, name in enumerate(names)]

    def test_uploads_all_five_exact_assets(self):
        (self.assets / "MangaMeeya-Rust_0.2.0_windows-setup.exe").write_bytes(b"old")
        (self.assets / "unrelated.txt").write_bytes(b"ignore")
        self.assert_succeeded(self.run_upload())
        self.assertCountEqual(self.read_state()["uploaded"], ASSET_NAMES)
        self.assertEqual(self.calls("git"), [{
            "command": "git",
            "args": ["rev-parse", "--verify", f"refs/tags/{TAG}^{{commit}}"],
            "repo": self.environment["GH_REPO"],
        }])
        api_calls = self.calls("gh")
        self.assertEqual(api_calls[0]["args"],
                         ["release", "view", TAG, "--json", "tagName,isDraft,assets"])
        uploaded_paths = []
        for call in api_calls[1:]:
            self.assertEqual(call["args"][:3], ["release", "upload", TAG])
            uploaded_paths.extend(call["args"][3:])
        self.assertCountEqual(uploaded_paths, [str(self.assets / name) for name in ASSET_NAMES])
        self.assertTrue(all(call["repo"] == self.environment["GH_REPO"] for call in api_calls))

    def test_file_preflight_rejects_missing_empty_symlink_and_directory(self):
        target = self.directory / "nonempty-symlink-target"
        target.write_bytes(b"not a release package")
        for name in ASSET_NAMES:
            for kind in ("missing", "empty", "symlink", "directory"):
                with self.subTest(name=name, kind=kind):
                    self.log_path.unlink(missing_ok=True)
                    path = self.assets / name
                    path.unlink()
                    if kind == "empty":
                        path.touch()
                    elif kind == "symlink":
                        path.symlink_to(target)
                    elif kind == "directory":
                        path.mkdir()
                    try:
                        self.assert_no_uploads(self.run_upload(), no_api=True)
                    finally:
                        if path.is_dir() and not path.is_symlink():
                            path.rmdir()
                        else:
                            path.unlink(missing_ok=True)
                        path.write_bytes(b"test native package\n")

    def test_complete_existing_release_is_not_changed(self):
        state = self.read_state()
        existing = self.existing_assets([*ASSET_NAMES, "unrelated.txt"])
        state["release"]["assets"] = copy.deepcopy(existing)
        self.write_state(state)
        self.assert_succeeded(self.run_upload())
        self.assertEqual(self.read_state()["release"]["assets"], existing)
        self.assertEqual(self.read_state()["uploaded"], [])
        self.assertEqual(len(self.calls("gh")), 1)

    def test_existing_assets_are_preserved_and_only_missing_assets_are_uploaded(self):
        state = self.read_state()
        existing_names = [ASSET_NAMES[0], ASSET_NAMES[3], "unrelated.txt"]
        existing = self.existing_assets(existing_names)
        # An unrelated unfinished asset must not block the five native packages.
        existing[-1].update(state="starter", size=0)
        state["release"]["assets"] = copy.deepcopy(existing)
        self.write_state(state)
        self.assert_succeeded(self.run_upload())
        state = self.read_state()
        self.assertCountEqual(state["uploaded"], set(ASSET_NAMES) - set(existing_names))
        self.assertEqual(state["release"]["assets"][:len(existing)], existing)

    def test_unhealthy_existing_asset_stops_before_any_missing_upload(self):
        healthy = self.existing_assets([ASSET_NAMES[-1]])[0]
        cases = {
            "empty": {**healthy, "size": 0},
            "starter": {**healthy, "state": "starter"},
            "empty starter": {**healthy, "size": 0, "state": "starter"},
            "negative size": {**healthy, "size": -1},
            "invalid size": {**healthy, "size": "101"},
            "missing state": {key: value for key, value in healthy.items() if key != "state"},
            "missing size": {key: value for key, value in healthy.items() if key != "size"},
        }
        for name, asset in cases.items():
            with self.subTest(name=name):
                self.log_path.unlink(missing_ok=True)
                state = self.read_state()
                # Earlier files are missing remotely. Check this last asset before uploads.
                state["release"]["assets"] = [asset]
                self.write_state(state)
                self.assert_no_uploads(self.run_upload())
                self.assertEqual(self.read_state()["release"]["assets"], [asset])

    def test_interrupted_upload_can_retry_without_replacing_completed_assets(self):
        state = self.read_state()
        state["fail_after"] = 2
        self.write_state(state)
        result = self.run_upload()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        state = self.read_state()
        completed = copy.deepcopy(state["release"]["assets"])
        self.assertEqual(len(completed), 2)
        completed_names = {asset["name"] for asset in completed}
        state.pop("fail_after")
        self.write_state(state)
        self.log_path.unlink()
        self.assert_succeeded(self.run_upload())
        state = self.read_state()
        self.assertCountEqual(state["uploaded"], ASSET_NAMES)
        self.assertEqual(state["release"]["assets"][:len(completed)], completed)
        retry_names = [Path(arg).name for call in self.calls("gh")
                       if call["args"][:2] == ["release", "upload"]
                       for arg in call["args"][3:]]
        self.assertCountEqual(retry_names, set(ASSET_NAMES) - completed_names)

    def test_api_failure_stops_before_upload(self):
        state = self.read_state()
        state["view_error"] = 1
        self.write_state(state)
        self.assert_no_uploads(self.run_upload())
        self.assertEqual(len(self.calls("gh")), 1)

    def test_rejects_draft_tag_mismatch_and_invalid_api_response(self):
        release = self.read_state()["release"]
        cases = {
            "draft": {**release, "isDraft": True},
            "wrong tag": {**release, "tagName": "v0.2.0"},
            "missing draft state": {"tagName": TAG, "assets": []},
            "missing assets": {"tagName": TAG, "isDraft": False},
            "invalid assets": {**release, "assets": {}},
        }
        for name, response in cases.items():
            with self.subTest(name=name):
                self.log_path.unlink(missing_ok=True)
                state = self.read_state()
                state["release"] = response
                self.write_state(state)
                self.assert_no_uploads(self.run_upload())
        state = self.read_state()
        state["view_output"] = "not JSON"
        self.write_state(state)
        self.assert_no_uploads(self.run_upload())

    def test_requires_one_assets_directory_argument(self):
        for arguments in ([], [str(self.assets), "extra"],
                          [str(self.assets / ASSET_NAMES[0])],
                          [str(self.directory / "absent")]):
            with self.subTest(arguments=arguments):
                self.assert_no_uploads(self.run_upload(arguments=arguments), no_api=True)
                self.assertEqual(self.calls(), [])

    def test_requires_release_environment(self):
        for name in ("GH_REPO", "RELEASES", "RELEASE_COMMIT"):
            for value in (None, ""):
                with self.subTest(name=name, value=value):
                    self.assert_no_uploads(self.run_upload({name: value}), no_api=True)
                    self.assertEqual(self.calls(), [])

    def test_rejects_invalid_package_version_tag_and_batch_before_tag_lookup(self):
        release = {"package": PACKAGE, "version": VERSION, "tag": TAG}
        cases = {
            "invalid JSON": "not JSON",
            "not an array": json.dumps(release),
            "empty batch": "[]",
            "multiple packages": json.dumps([release, release]),
            "wrong package": json.dumps([{**release, "package": "other"}]),
            "missing package": json.dumps([{"version": VERSION, "tag": TAG}]),
            "mismatched tag": json.dumps([{**release, "tag": "v0.2.2"}]),
            "missing tag": json.dumps([{"package": PACKAGE, "version": VERSION}]),
        }
        for version in (None, 2, "0.2", "v0.2.1", "0.2.1-rc.1", "0.2.1+build.1",
                        "00.2.1", "0.02.1", "0.2.01", "0.2.1 ", "0.2.1\n"):
            cases[f"invalid version {version!r}"] = json.dumps([{
                **release, "version": version, "tag": f"v{version}",
            }])
        for name, releases in cases.items():
            with self.subTest(name=name):
                self.log_path.unlink(missing_ok=True)
                self.assert_no_uploads(self.run_upload({"RELEASES": releases}), no_api=True)
                self.assertEqual(self.calls(), [], "Invalid metadata reached the tag lookup")

    def test_rejects_invalid_commit_before_tag_lookup(self):
        for commit in ("abc123", "a" * 39, "a" * 41, "z" * 40, "a" * 40 + "\n"):
            with self.subTest(commit=commit):
                self.assert_no_uploads(self.run_upload({"RELEASE_COMMIT": commit}), no_api=True)
                self.assertEqual(self.calls(), [])

    def test_local_tag_commit_must_match_tested_commit(self):
        state = self.read_state()
        state["commit"] = "f" * 40
        self.write_state(state)
        self.assert_no_uploads(self.run_upload(), no_api=True)
        self.assertEqual(len(self.calls("git")), 1)

    def test_missing_local_tag_stops_before_api(self):
        state = self.read_state()
        state["git_error"] = 128
        self.write_state(state)
        self.assert_no_uploads(self.run_upload(), no_api=True)
        self.assertEqual(len(self.calls("git")), 1)


@unittest.skipUnless(os.environ.get("CHANGENOTE_BIN"),
                     "Set CHANGENOTE_BIN to the pinned changenote CLI for version integration")
class ChangenoteVersionTests(unittest.TestCase):
    def assert_sources_unchanged(self, sources):
        for path, contents in sources.items():
            self.assertEqual((ROOT / path).read_bytes(), contents, f"Source changed: {path}")

    def test_actual_config_and_pending_notes_synchronize_cargo_release(self):
        executable = shutil.which(os.environ["CHANGENOTE_BIN"])
        self.assertIsNotNone(executable, "CHANGENOTE_BIN must name an executable")
        executable = str(Path(executable).resolve())
        pending = sorted(path for path in (ROOT / ".changenote").glob("*.md")
                         if path.name != "README.md")
        paths = [Path(".changenote/config.json"), Path("Cargo.toml"),
                 Path("Cargo.lock"), Path("rust-toolchain.toml")]
        paths.extend(path.relative_to(ROOT) for path in pending)
        sources = {path: (ROOT / path).read_bytes() for path in paths}
        self.addCleanup(self.assert_sources_unchanged, sources)
        with tempfile.TemporaryDirectory(prefix="changenote-version-") as temporary:
            fixture = Path(temporary)
            for path in paths:
                destination = fixture / path
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / path, destination)
            for name, contents in {"src/lib.rs": "", "src/main.rs": "fn main() {}\n",
                                   "tests/rendering.rs": "fn main() {}\n"}.items():
                path = fixture / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(contents)

            def run_changenote(*arguments):
                result = subprocess.run(
                    [executable, *arguments], cwd=fixture, text=True,
                    capture_output=True, timeout=240,
                )
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                return result.stdout

            def fixture_contents():
                return {path.relative_to(fixture): path.read_bytes()
                        for path in fixture.rglob("*") if path.is_file()}

            def apply_and_check(note_paths, *, require_patch=False):
                before_manifest = tomllib.loads((fixture / "Cargo.toml").read_text())
                before_lock = tomllib.loads((fixture / "Cargo.lock").read_text())
                before_status = fixture_contents()
                plan = json.loads(run_changenote("status"))
                self.assertEqual(fixture_contents(), before_status, "status changed files")
                self.assertEqual(len(plan), 1)
                release = plan[0]
                current = before_manifest["package"]["version"]
                major, minor, patch = map(int, current.split("."))
                next_versions = {"major": f"{major + 1}.0.0",
                                 "minor": f"{major}.{minor + 1}.0",
                                 "patch": f"{major}.{minor}.{patch + 1}"}
                self.assertIn(release["bump"], next_versions)
                if require_patch:
                    self.assertEqual(release["bump"], "patch")
                version = next_versions[release["bump"]]
                expected = {"package": PACKAGE, "old_version": current,
                            "version": version, "bump": release["bump"], "tag": f"v{version}"}
                self.assertEqual({key: release[key] for key in expected}, expected)
                self.assertTrue(release["notes"])

                run_changenote("version")
                expected_manifest = copy.deepcopy(before_manifest)
                expected_manifest["package"]["version"] = version
                self.assertEqual(tomllib.loads((fixture / "Cargo.toml").read_text()),
                                 expected_manifest)
                expected_lock = copy.deepcopy(before_lock)
                root_entries = [package for package in expected_lock["package"]
                                if package["name"] == PACKAGE and "source" not in package]
                self.assertEqual(len(root_entries), 1)
                self.assertEqual(root_entries[0]["version"], current)
                root_entries[0]["version"] = version
                self.assertEqual(tomllib.loads((fixture / "Cargo.lock").read_text()),
                                 expected_lock, "Dependency resolutions changed")
                self.assertEqual(json.loads((fixture / ".changenote/releases.json").read_text()),
                                 [expected])
                changelog = (fixture / "CHANGELOG.md").read_text()
                self.assertIn(version, changelog)
                for note in release["notes"]:
                    self.assertIn(" ".join(note.split()), " ".join(changelog.split()))
                for path in note_paths:
                    self.assertFalse((fixture / path).exists(),
                                     f"Pending note was not consumed: {path}")
                for path in (Path(".changenote/config.json"), Path("rust-toolchain.toml")):
                    self.assertEqual((fixture / path).read_bytes(), sources[path])
                self.assertFalse((fixture / "target").exists(), "The product must not be built")

                after_version = fixture_contents()
                self.assertEqual(json.loads(run_changenote("status")), [])
                run_changenote("version")
                self.assertEqual(fixture_contents(), after_version,
                                 "version without pending notes must not change files")

            def add_fixture_patch():
                path = Path(".changenote/fixture-next-patch.md")
                (fixture / path).write_text(
                    f"---\n{PACKAGE}: patch\n---\n\nCheck a patch after a completed release.\n"
                )
                return path

            self.assertEqual(run_changenote("--version").strip(), "changenote 0.10.0")
            note_paths = [path.relative_to(ROOT) for path in pending]
            if not note_paths:
                # Release-candidate CI runs after the real source notes are consumed.
                note_paths = [add_fixture_patch()]
            apply_and_check(note_paths, require_patch=not pending)
            apply_and_check([add_fixture_patch()], require_patch=True)


if __name__ == "__main__":
    unittest.main()
