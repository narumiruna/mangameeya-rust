"""Build and test the pinned hardened Sparkle source on native macOS only."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
COMMIT = "ab7d71291238df6e0492143b120c9b696a35f603"
SOURCE_SHA256 = "d7e5599d56ddb917cddbb4a719fb534ad5d2d7e80a653446d0eea6a38920b302"
MACHO_MAGIC = {b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xce",
               b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca",
               b"\xca\xfe\xba\xbf", b"\xbf\xba\xfe\xca"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def validate_source(source):
    """Require source preparation evidence and the exact reviewed patched files."""
    provenance = json.loads((source.parent / "provenance.json").read_text(encoding="utf-8"))
    require(type(provenance.get("schema")) is int and provenance["schema"] == 1,
            "Wrong source provenance schema")
    require(provenance["sdk"] == "sparkle" and provenance["commit"] == COMMIT,
            "Wrong Sparkle source commit")
    require(provenance["source_sha256"] == SOURCE_SHA256, "Wrong Sparkle source archive")
    patch = Path(__file__).with_name("hardening.patch").read_bytes()
    require(provenance["patch_sha256"] == hashlib.sha256(patch).hexdigest(), "Wrong Sparkle patch")
    require(provenance["hardening_version"] == 1, "Wrong hardening version")
    manifest = json.loads(Path(__file__).with_name("patch-inputs.json").read_text(encoding="utf-8"))
    for name, digest in manifest["outputs"].items():
        require(hashlib.sha256((source / name).read_bytes()).hexdigest() == digest,
                f"Patched source changed: {name}")


def report_failure(message):
    print(message, file=sys.stderr)
    if os.environ.get("GITHUB_ACTIONS") == "true":
        escaped = message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print(f"::error::{escaped}", flush=True)


def run(command, log, environment, timeout=300):
    print("+", " ".join(map(str, command)), flush=True)
    try:
        with log.open("w", encoding="utf-8") as output:
            subprocess.run(list(map(str, command)), check=True, stdin=subprocess.DEVNULL,
                           stdout=output, stderr=subprocess.STDOUT, env=environment, timeout=timeout)
    except subprocess.SubprocessError:
        lines = log.read_text(encoding="utf-8", errors="replace").splitlines()
        failures = [line for line in lines if "error:" in line.lower() or "failed" in line.lower()]
        detail = "\n".join(line[:700] for line in [*failures[-20:], *lines[-15:]])[-16000:]
        report_failure(f"{log.name}:\n{detail}")
        raise


def required_tests(source):
    contents = (source / "Tests/SUInstallerTest.m").read_text(encoding="utf-8")
    implementation = re.search(r"@implementation SUHardenedInstallerTest\b(.*?)@end", contents, re.S)
    require(implementation is not None, "Hardened XCTest class is missing")
    methods = set(re.findall(r"-\s*\(void\)\s*(test\w+)\s*\{", implementation.group(1)))
    require(bool(methods), "Hardened XCTest class has no test methods")
    return methods


def validate_test_run(summary, tests, expected):
    # A successful xcodebuild exit can still mean that no selected tests ran.
    counts = {"totalTestCount": len(expected), "passedTests": len(expected),
              "failedTests": 0, "skippedTests": 0}
    require(all(type(summary.get(key)) is int and summary[key] == value
                for key, value in counts.items()), "XCTest counts do not prove all required tests passed")
    require(summary.get("expectedFailures", 0) == 0, "Expected XCTest failures are not acceptance")
    passed = set()

    def visit(node, in_class=False):
        if isinstance(node, list):
            for child in node:
                visit(child, in_class)
        elif isinstance(node, dict):
            labels = [node.get(key, "") for key in
                      ("name", "nodeIdentifier", "testIdentifier", "testIdentifierString")]
            labels = [value for value in labels if isinstance(value, str)]
            in_class = in_class or any("SUHardenedInstallerTest" in label for label in labels)
            if in_class and node.get("result") == "Passed":
                for label in labels:
                    method = label.rsplit("/", 1)[-1].removesuffix("()")
                    if method in expected:
                        passed.add(method)
            for child in node.values():
                if isinstance(child, (dict, list)):
                    visit(child, in_class)

    visit(tests)
    require(passed == expected, f"Missing required XCTest execution evidence: {sorted(expected - passed)}")


def record_test_run(source, output):
    bundle = output / "hardening.xcresult"
    command = ["/usr/bin/xcrun", "xcresulttool", "get", "test-results"]
    summary = json.loads(subprocess.check_output(
        [*command, "summary", "--path", str(bundle)], text=True, timeout=60))
    tests = json.loads(subprocess.check_output(
        [*command, "tests", "--path", str(bundle)], text=True, timeout=60))
    validate_test_run(summary, tests, required_tests(source))
    (output / "native-test-summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")


def inspect_binaries(framework, architecture):
    inspected = []
    for path in sorted(framework.rglob("*")):
        if path.is_symlink() or not path.is_file():
            continue
        with path.open("rb") as file:
            if file.read(4) not in MACHO_MAGIC:
                continue
        arches = subprocess.check_output(["/usr/bin/xcrun", "lipo", "-archs", str(path)], text=True, timeout=300).split()
        require(arches == [architecture], f"Wrong binary architecture: {path}: {arches}")
        build = subprocess.check_output(["/usr/bin/xcrun", "vtool", "-show-build", str(path)], text=True, timeout=300)
        minimums = re.findall(r"\bminos\s+(\d+(?:\.\d+)*)", build)
        require(minimums and all(tuple(map(int, value.split("."))) <= (12, 0, 0) for value in minimums),
                f"Binary minimum OS is above macOS 12: {path}: {minimums}")
        inspected.append({"path": str(path.relative_to(framework)), "architectures": arches, "minimum_os": minimums})
    require(len(inspected) >= 4, "Expected framework plus installer, progress agent, and XPC executables")
    return inspected


def build(source, output, architecture):
    require(sys.platform == "darwin", "UNSUPPORTED: native macOS and full Xcode are required; no native checks ran")
    require(platform.machine() == architecture, "Run tests on the selected process architecture, not by cross-compilation")
    source = source.resolve(strict=True)
    output = output.resolve()
    require(output.is_relative_to((ROOT / "target").resolve()), "Build output must be under target/")
    require(not output.exists(), "Build output already exists; choose a new directory")
    validate_source(source)
    developer = subprocess.check_output(["/usr/bin/xcode-select", "-p"], text=True, timeout=300).strip()
    require("CommandLineTools" not in developer, "Full Xcode is required, not Command Line Tools")
    output.mkdir(parents=True)
    environment = os.environ.copy()
    # Do not let Sparkle's informational version script read this project's tags.
    environment["GIT_CEILING_DIRECTORIES"] = str(source.parent)
    run(["/usr/bin/xcodebuild", "-version"], output / "xcode-version.log", environment)
    settings = [f"ARCHS={architecture}", "ONLY_ACTIVE_ARCH=NO", "MACOSX_DEPLOYMENT_TARGET=12.0",
                "CODE_SIGN_IDENTITY=-", "CODE_SIGNING_ALLOWED=YES", "CODE_SIGNING_REQUIRED=YES"]
    common = ["/usr/bin/xcodebuild", "-project", source / "Sparkle.xcodeproj", "-scheme", "Sparkle",
              "-derivedDataPath", output / "derived"]
    run([*common, "-configuration", "Release", "-destination", "generic/platform=macOS", *settings, "build"],
        output / "build.log", environment, timeout=1200)
    run([*common, "-configuration", "Debug", "-destination", f"platform=macOS,arch={architecture}",
         "-parallel-testing-enabled", "NO", "-resultBundlePath", output / "hardening.xcresult",
         "-only-testing:Sparkle Unit Tests/SUHardenedInstallerTest", *settings, "test"],
        output / "native-tests.log", environment, timeout=1200)
    record_test_run(source, output)
    framework = output / "derived/Build/Products/Release/Sparkle.framework"
    require(framework.is_dir(), "Built Sparkle.framework is missing")
    inspected = inspect_binaries(framework, architecture)
    tests = ROOT / "tests/updates/sparkle"
    compiler = ["/usr/bin/xcrun", "clang", "-arch", architecture, "-mmacosx-version-min=12.0", "-Wall", "-Wextra", "-Werror"]
    run([*compiler, "-std=c11", "-O2", "-UNDEBUG", "-I", source / "Autoupdate", tests / "commit_policy_test.c", "-o", output / "commit-policy"],
        output / "policy-build.log", environment)
    run([output / "commit-policy"], output / "policy-tests.log", environment)
    run([*compiler, "-fobjc-arc", "-framework", "Foundation", tests / "capability_probe.m", "-o", output / "capability-probe"],
        output / "probe-build.log", environment)
    run([output / "capability-probe", framework], output / "probe.log", environment)
    evidence = {"source_commit": COMMIT, "architecture": architecture, "binaries": inspected,
                "fixture_signing": "Ad-hoc CODE_SIGN_IDENTITY=- only; no production identity or provisioning",
                "limits": "SDK unit/fixture checks only. Authenticated XPC, real process-agent handoff, GPUI gate, Developer ID and notarization remain unverified."}
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(f"SDK checks passed: {output}")
    print(evidence["limits"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--arch", required=True, choices=("arm64", "x86_64"))
    args = parser.parse_args()
    try:
        build(args.source, args.output, args.arch)
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        report_failure(str(error))
        parser.exit(1)


if __name__ == "__main__":
    main()
