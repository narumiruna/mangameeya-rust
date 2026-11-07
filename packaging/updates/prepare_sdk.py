"""Prepare pinned, patched SDK source under target/. This is a build tool only."""

import argparse
from contextlib import contextmanager
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import tarfile
import tempfile
import time
from urllib.parse import urlsplit
from urllib.request import HTTPSHandler, HTTPRedirectHandler, Request, build_opener
import zipfile


ROOT = Path(__file__).resolve().parents[2]
PIN_FILE = Path(__file__).with_name("sdk-sources.json")
MAX_UNPACKED_BYTES = 1024 * 1024 * 1024
MAX_MEMBERS = 100_000
MAX_DOWNLOAD_SECONDS = 180
SOURCE_FIELDS = {"version", "commit", "url", "sha256", "size", "format", "prefix", "patch"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load_pins():
    pins = json.loads(PIN_FILE.read_text(encoding="utf-8"))
    require(set(pins) == {"schema", "hardening_version", "sdks"}, "Invalid SDK pin fields")
    require(pins["schema"] == 1 and pins["hardening_version"] == 1, "Invalid SDK pin schema")
    require(set(pins["sdks"]) == {"winsparkle", "sparkle"}, "Unexpected SDK set")
    for name, source in pins["sdks"].items():
        require(set(source) == SOURCE_FIELDS, f"Invalid {name} source fields")
        require(re.fullmatch(r"[0-9a-f]{40}", source["commit"]) is not None,
                "Expected a full source commit")
        require(re.fullmatch(r"[0-9a-f]{64}", source["sha256"]) is not None,
                "Expected an archive SHA-256")
        require(type(source["size"]) is int and 0 < source["size"] < MAX_UNPACKED_BYTES,
                "Invalid source archive size")
        require(source["format"] in {"zip", "tar.gz"}, "Unsupported source format")
        require(urlsplit(source["url"]).scheme == "https", "Source URL must use HTTPS")
        require(source["patch"] == f"{name}/hardening.patch", "Unexpected patch path")
        if source["prefix"]:
            member_path(source["prefix"])
    return pins


class HttpsRedirects(HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        require(urlsplit(new_url).scheme == "https", "SDK redirect must use HTTPS")
        return super().redirect_request(request, response, code, message, headers, new_url)


def archive_bytes(source, archive=None):
    size = source["size"]
    if archive is not None:
        with archive.open("rb") as file:
            contents = file.read(size + 1)
    else:
        request = Request(source["url"], headers={"User-Agent": "MangaMeeya-Rust-SDK-Prepare"})
        opener = build_opener(HTTPSHandler(), HttpsRedirects())
        deadline = time.monotonic() + MAX_DOWNLOAD_SECONDS
        with opener.open(request, timeout=30) as response:
            require(response.status == 200, "SDK source request failed")
            chunks = []
            count = 0
            while count <= size:
                require(time.monotonic() < deadline, "SDK source download timed out")
                chunk = response.read1(min(64 * 1024, size + 1 - count))
                if not chunk:
                    break
                chunks.append(chunk)
                count += len(chunk)
            contents = b"".join(chunks)
    require(len(contents) == size, "Source archive size does not match its pin")
    require(hashlib.sha256(contents).hexdigest() == source["sha256"],
            "Source archive SHA-256 does not match its pin")
    return contents


def member_path(name, prefix=""):
    require(name and "\\" not in name and "\0" not in name and ":" not in name,
            "Unsafe SDK member name")
    parts = name.rstrip("/").split("/")
    require(all(part not in {"", ".", "..", ".git"} for part in parts),
            "Unsafe SDK member path")
    require(not PurePosixPath(name).is_absolute(), "Absolute SDK member path")
    require(all(not part.endswith((" ", ".")) for part in parts), "Ambiguous SDK member path")
    reserved = {"con", "prn", "aux", "nul"}
    reserved.update(f"{kind}{i}" for kind in ("com", "lpt") for i in range(1, 10))
    require(all(part.split(".", 1)[0].casefold() not in reserved for part in parts),
            "Reserved SDK member path")
    if prefix:
        require(parts[0] == prefix, "SDK archive has the wrong root directory")
        parts = parts[1:]
    return PurePosixPath(*parts) if parts else None


@contextmanager
def members(contents, archive_format):
    stream = io.BytesIO(contents)
    if archive_format == "zip":
        with zipfile.ZipFile(stream) as archive:
            entries = []
            for member in archive.infolist():
                require(len(entries) < MAX_MEMBERS, "Too many SDK members")
                mode = member.external_attr >> 16
                require(member.orig_filename == member.filename, "Ambiguous SDK member name")
                directory = member.is_dir()
                kind = stat.S_IFMT(mode)
                require(kind in {0, stat.S_IFREG, stat.S_IFDIR}, "SDK archive contains a link or special file")
                require(kind == 0 or (kind == stat.S_IFDIR) == directory, "Invalid SDK member type")
                require(not member.flag_bits & 1, "Encrypted SDK member")
                entries.append((member.filename, directory, member.file_size, mode,
                                lambda member=member: archive.open(member)))
            yield entries
    else:
        with tarfile.open(fileobj=stream, mode="r:gz") as archive:
            entries = []
            for member in archive:
                require(len(entries) < MAX_MEMBERS, "Too many SDK members")
                require(member.isdir() or member.isfile(), "SDK archive contains a link or special file")
                entries.append((member.name, member.isdir(), member.size, member.mode,
                                lambda member=member: archive.extractfile(member)))
            yield entries


def extract_source(contents, archive_format, prefix, destination):
    with members(contents, archive_format) as entries:
        require(len(entries) <= MAX_MEMBERS, "Too many SDK members")
        require(sum(entry[2] for entry in entries) <= MAX_UNPACKED_BYTES, "SDK source is too large")
        checked = []
        seen = {}
        for name, directory, length, mode, reader in entries:
            path = member_path(name, prefix)
            if path is None:
                require(directory, "Invalid SDK archive root")
                continue
            # Reject aliases on Windows and default macOS file systems too.
            identity = str(path).casefold()
            require(identity not in seen, "Duplicate SDK member")
            seen[identity] = directory
            require(length >= 0 and (not directory or length == 0), "Invalid SDK member size")
            checked.append((path, directory, length, mode, reader))
        for identity in seen:
            for parent in PurePosixPath(identity).parents:
                require(seen.get(str(parent), True), "SDK file is used as a directory")
        # Validate all entries before the first source-file write.
        for path, directory, length, mode, reader in checked:
            output = destination.joinpath(*path.parts)
            if directory:
                output.mkdir(parents=True, exist_ok=True, mode=0o700)
                continue
            output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            with reader() as source, output.open("xb") as file:
                copied = 0
                while copied < length:
                    chunk = source.read(min(64 * 1024, length - copied))
                    require(bool(chunk), "Truncated SDK member")
                    file.write(chunk)
                    copied += len(chunk)
                require(not source.read(1), "SDK member exceeds declared size")
            output.chmod(0o700 if mode & 0o111 else 0o600)


def git(source, *arguments):
    environment = os.environ.copy()
    for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR",
                "GIT_OBJECT_DIRECTORY", "GIT_ALTERNATE_OBJECT_DIRECTORIES"):
        environment.pop(key, None)
    for key in list(environment):
        if key.startswith("GIT_CONFIG_"):
            environment.pop(key)
    environment.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull,
                       GIT_TERMINAL_PROMPT="0")
    subprocess.run(["git", "-C", str(source), *arguments], env=environment,
                   stdin=subprocess.DEVNULL, check=True, timeout=60)


def prepare(name, output, archive=None):
    pins = load_pins()
    source_pin = pins["sdks"][name]
    output = output.resolve()
    target = (ROOT / "target").resolve()
    require(output != target and output.is_relative_to(target), "SDK output must be a new directory under target/")
    require(not output.exists(), "SDK output already exists; choose a new output directory")
    patch = (PIN_FILE.parent / source_pin["patch"]).read_bytes()
    require(patch.startswith(b"diff --git "), "Expected a complete source patch")
    contents = archive_bytes(source_pin, archive)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".sdk-prepare-", dir=output.parent) as temporary:
        staging = Path(temporary)
        source = staging / "source"
        source.mkdir(mode=0o700)
        extract_source(contents, source_pin["format"], source_pin["prefix"], source)
        git(source, "init", "--quiet", "--template=", "--initial-branch=narumi/chore/sdk-source")
        git(source, "config", "core.autocrlf", "false")
        patch_file = staging / "hardening.patch"
        patch_file.write_bytes(patch)
        git(source, "apply", "--check", "--whitespace=error", str(patch_file))
        git(source, "apply", "--whitespace=error", str(patch_file))
        provenance = {
            "schema": 1,
            "sdk": name,
            "version": source_pin["version"],
            "commit": source_pin["commit"],
            "source_sha256": source_pin["sha256"],
            "patch_sha256": hashlib.sha256(patch).hexdigest(),
            "hardening_version": pins["hardening_version"],
        }
        # Reserve only a new directory. Never remove an existing build or user file.
        output.mkdir(mode=0o700)
        try:
            os.rename(source, output / "source")
            (output / "provenance.json").write_text(
                json.dumps(provenance, indent=2) + "\n", encoding="utf-8")
        except Exception:
            # Keep any published source for diagnosis; remove only an empty reservation.
            if not any(output.iterdir()):
                output.rmdir()
            raise
    return output / "source"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sdk", choices=("winsparkle", "sparkle"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--archive", type=Path, help="use an offline copy of the pinned source archive")
    arguments = parser.parse_args()
    source = prepare(arguments.sdk, arguments.output, arguments.archive)
    print(f"Prepared SDK source: {source}")
    print("LIMIT: source preparation is not a build, native integration, or update-gate pass")


if __name__ == "__main__":
    main()
