"""Build the patched x64 SDK and run focused native unit checks on Windows."""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
GETTEXT_SHA256 = "8e2ff0a055778ad72c0077e09a30028011816c9eac5168aa2c88101a4ddde1a0"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def report_failure(message: str) -> None:
    print(message, file=sys.stderr)
    if os.environ.get("GITHUB_ACTIONS") == "true":
        escaped = message.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
        print(f"::error::{escaped}", flush=True)


def run(args: list[str | Path], *, cwd: Path, env: dict[str, str], timeout: int = 300,
        log: Path | None = None) -> None:
    command = [str(a) for a in args]
    search = next(value for name, value in env.items() if name.lower() == "path")
    command[0] = shutil.which(command[0], path=search) or command[0]
    print("+", " ".join(command), flush=True)
    options = dict(cwd=cwd, env=env, stdin=subprocess.DEVNULL, check=True, timeout=timeout)
    if log is None:
        subprocess.run(command, **options)
    else:
        try:
            with log.open("wb") as output:
                subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, **options)
        except subprocess.SubprocessError:
            lines = log.read_text(encoding="utf-8", errors="replace").splitlines()
            failures = [line for line in lines if "error" in line.lower() or "fail" in line.lower()]
            detail = "\n".join(line[:700] for line in [*failures[-20:], *lines[-15:]])[-16000:]
            report_failure(f"{log.name}:\n{detail}")
            raise


def toolchain() -> tuple[Path, dict[str, str]]:
    vswhere = Path(os.environ["ProgramFiles(x86)"]) / "Microsoft Visual Studio/Installer/vswhere.exe"
    result = subprocess.run(
        [str(vswhere), "-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"],
        stdin=subprocess.DEVNULL, capture_output=True, check=True, timeout=30,
    )
    vs = Path(result.stdout.decode("utf-8").strip())
    devcmd = vs / "Common7/Tools/VsDevCmd.bat"
    if not devcmd.is_file():
        raise RuntimeError("Visual Studio C++ tools were not found")
    # Capture only compiler variables, not all user environment values. Pass a
    # command line directly: cmd.exe does not use the MSVCRT quote rules.
    command = f'cmd.exe /d /s /c ""{devcmd}" -no_logo -arch=x64 -host_arch=x64 >nul && set INCLUDE && set LIB && set LIBPATH && set VCToolsInstallDir && set VisualStudioVersion && set Path"'
    result = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True, check=True, timeout=60)
    allowed = {"include", "lib", "libpath", "vctoolsinstalldir", "visualstudioversion", "path"}
    env = dict(os.environ)
    for line in result.stdout.decode("utf-8", errors="replace").splitlines():
        name, sep, value = line.partition("=")
        if sep and name.lower() in allowed:
            for old in list(env):
                if old.lower() == name.lower():
                    del env[old]
            env[name] = value
    # UTF-8 release sources must not depend on the host ANSI code page.
    env["CL"] = env.get("CL", "") + " /utf-8"
    return vs / "MSBuild/Current/Bin/MSBuild.exe", env


def probe(dll: Path) -> None:
    # The child process keeps the DLL loaded until process exit. It does not
    # initialize the updater, start workers, call cleanup or execute a payload.
    with os.add_dll_directory(str(dll.parent)):
        library = ctypes.CDLL(str(dll), winmode=0x00000100 | 0x00000800)
        version = library.win_sparkle_get_hardening_version
        version.argtypes = []
        version.restype = ctypes.c_uint
        if version() != 1:
            raise RuntimeError("Wrong hardened SDK ABI version")
        library.win_sparkle_set_verified_installer_callback.argtypes = [ctypes.c_void_p]
        library.win_sparkle_set_verified_installer_callback.restype = None
        library.win_sparkle_set_verified_installer_callback(None)
        library.win_sparkle_set_eddsa_public_key.argtypes = [ctypes.c_char_p]
        library.win_sparkle_set_eddsa_public_key.restype = ctypes.c_int
        if library.win_sparkle_set_eddsa_public_key(b"bad") != 0:
            raise RuntimeError("Malformed public key was accepted")
    print("PASS: built DLL hardened C ABI probe")


def validate_source(source: Path) -> None:
    pins = json.loads((HERE.parent / "sdk-sources.json").read_text(encoding="utf-8"))
    expected = pins["sdks"]["winsparkle"]
    provenance = json.loads((source.parent / "provenance.json").read_text(encoding="utf-8"))
    require(type(provenance.get("schema")) is int and provenance["schema"] == 1,
            "Wrong source provenance schema")
    require(provenance["sdk"] == "winsparkle" and provenance["commit"] == expected["commit"],
            "Wrong WinSparkle source commit")
    require(provenance["source_sha256"] == expected["sha256"], "Wrong WinSparkle source archive")
    require(provenance["patch_sha256"] == hashlib.sha256((HERE / "hardening.patch").read_bytes()).hexdigest(),
            "Wrong WinSparkle patch")
    require(provenance["hardening_version"] == 1, "Wrong hardening version")


def build(source: Path, output: Path) -> None:
    require(sys.platform == "win32" and ctypes.sizeof(ctypes.c_void_p) == 8,
            "UNSUPPORTED: native 64-bit Windows Python is required")
    source = source.resolve(strict=True)
    output = output.resolve()
    target = (ROOT / "target").resolve()
    require(output != target and output.is_relative_to(target), "Build output must be under target/")
    require(not output.exists(), "Build output already exists; choose a new directory")
    validate_source(source)
    msbuild, env = toolchain()
    output.mkdir(parents=True)
    run([sys.executable, ROOT / "tests/updates/winsparkle/check_patch.py", "--source", source],
        cwd=source, env=env, timeout=60, log=output / "source-checks.log")
    solution = source / "WinSparkle.sln"
    dll_out = output / "dll"
    dll_out.mkdir(exist_ok=True)
    common = ["/p:Configuration=Release", "/p:Platform=x64", "/nologo"]
    run([msbuild, solution, "/t:Restore", "/p:RestorePackagesConfig=true", "/p:NuGetInteractive=false", "/p:RestoreSources=https://api.nuget.org/v3/index.json", *common], cwd=source, env=env, timeout=1200, log=output / "restore.log")
    package = source / "packages/Gettext.Tools.0.26.0/Gettext.Tools.0.26.0.nupkg"
    if hashlib.sha256(package.read_bytes()).hexdigest() != GETTEXT_SHA256:
        raise RuntimeError("Gettext.Tools 0.26.0 SHA-256 mismatch")
    run([msbuild, solution, "/t:WinSparkle", "/m:2", f"/p:OutDir={dll_out}\\", *common],
        cwd=source, env=env, timeout=1200, log=output / "build.log")
    test_out = output / "tests"
    test_out.mkdir(exist_ok=True)
    include = [source / p for p in ["src", "include", "3rdparty/config", "3rdparty/wxWidgets/include", "3rdparty/ed25519/src", "3rdparty/expat/expat/lib"]]
    sources = [source / "src" / p for p in ["appcast.cpp", "appcontroller.cpp", "dll_api.cpp", "download.cpp", "error.cpp", "settings.cpp", "signatureverifier.cpp", "threads.cpp", "updatechecker.cpp"]]
    libs = ["WinSparkle_wx.lib", "WinSparkle_expat.lib", "WinSparkle_ed25519.lib", "comctl32.lib", "kernel32.lib", "user32.lib", "rpcrt4.lib", "version.lib", "wininet.lib", "shlwapi.lib", "crypt32.lib", "bcrypt.lib", "gdi32.lib", "advapi32.lib", "shell32.lib", "ole32.lib", "oleaut32.lib", "uuid.lib"]
    exe = test_out / "native.exe"
    run(["cl.exe", "/nologo", "/EHsc", "/std:c++17", "/MT", "/W3", "/DUNICODE", "/D_UNICODE", "/DXML_STATIC", "/DBUILDING_WIN_SPARKLE", "/D_CRT_SECURE_NO_WARNINGS", *[f"/I{p}" for p in include], ROOT / "tests/updates/winsparkle/native.cpp", *sources, f"/Fe{exe}", f"/Fo{test_out}\\", f"/Fd{test_out / 'native.pdb'}", "/link", "/LTCG", source / "x64/Release/WinSparkle/winsparkle.res", source / "x64/Release/WinSparkle/translations.res", f"/LIBPATH:{dll_out}", *libs], cwd=source, env=env, timeout=1200, log=output / "test-build.log")
    run([exe], cwd=test_out, env=env, timeout=60, log=output / "native-tests.log")
    dll = dll_out / "WinSparkle.dll"
    for kind in ["imports", "exports"]:
        search = next(value for name, value in env.items() if name.lower() == "path")
        dumpbin = shutil.which("dumpbin.exe", path=search)
        if not dumpbin:
            raise RuntimeError("dumpbin was not found")
        result = subprocess.run([dumpbin, f"/{kind}", str(dll)], cwd=source, env=env, stdin=subprocess.DEVNULL, capture_output=True, check=True, timeout=30)
        (output / f"dll-{kind}.txt").write_bytes(result.stdout)
        if kind == "imports" and any(name in result.stdout.lower() for name in [b"libcrypto", b"libssl", b"webview2", b"ieframe"]):
            raise RuntimeError("Unexpected legacy/browser DLL dependency")
    run([sys.executable, Path(__file__).resolve(), "--probe-dll", dll],
        cwd=test_out, env=env, timeout=30, log=output / "probe.log")
    evidence = {
        "scope": "focused native SDK unit checks only; not application update acceptance",
        "dll_sha256": hashlib.sha256(dll.read_bytes()).hexdigest(),
        "patch_sha256": hashlib.sha256((HERE / "hardening.patch").read_bytes()).hexdigest(),
        "gettext_nupkg_sha256": GETTEXT_SHA256,
    }
    (output / "evidence.json").write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print("PASS: focused native SDK checks. Full application update gate remains open.")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--probe-dll", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if sys.platform != "win32":
        parser.error("Run with native Windows Python")
    try:
        if args.probe_dll:
            probe(args.probe_dll.resolve(strict=True))
        elif args.source and args.output:
            build(args.source, args.output)
        else:
            parser.error("--source and --output are required")
    except (ValueError, RuntimeError, OSError, subprocess.SubprocessError) as error:
        report_failure(str(error))
        parser.exit(1)


if __name__ == "__main__":
    main()
