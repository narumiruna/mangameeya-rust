# Hardened updater SDK sources

These files build and test altered SDKs. They do not enable application updates.
The reader, normal CI and release packaging remain unchanged. The full native
GPUI/update gate and production signing prerequisites remain open.

## Files

- [`sdk-sources.json`](sdk-sources.json): exact public source URLs, commits,
  archive sizes and SHA-256 values.
- [`prepare_sdk.py`](prepare_sdk.py): bounded archive acquisition, path/type
  validation, isolated patch application, exact output hashes and provenance.
- [`winsparkle/`](winsparkle/README.md): verified-buffer SHA-256 handoff,
  fail-closed Windows SDK, inert notes and native checks.
- [`sparkle/`](sparkle/README.md): explicit session COMMIT, disarm/cleanup,
  bundle/version/architecture policy and native checks.
- [`../../.github/workflows/updater-sdk.yml`](../../.github/workflows/updater-sdk.yml):
  separate source-safety and native SDK jobs. The workflow uses Python 3.13,
  Windows x64, macOS arm64 and macOS x86_64. It does not use signing secrets,
  publish releases, or change the existing package workflow.

## Preparation

Run from the repository root:

```sh
uv run --no-project python packaging/updates/prepare_sdk.py winsparkle \
  --output target/updater-sdk/winsparkle
uv run --no-project python packaging/updates/prepare_sdk.py sparkle \
  --output target/updater-sdk/sparkle
```

Use `--archive <local archive>` to use an existing pinned archive. Archive bytes
must match the committed size and SHA-256 before extraction. Output must be a new
path under `target/`. The prepare tool never replaces an existing directory.

The tool rejects absolute and parent paths, Windows path aliases, links, duplicate
or conflicting members, special file types, excess members and excess expanded
bytes. It initializes its own temporary Git repository without user/system Git
configuration or hooks. Patch check/application cannot use a parent worktree.
Provenance records the source commit, archive and patch hashes, and capability
version. Preparation alone is not a native build or install test.

## Portable checks

```sh
uv run --no-project python -m unittest discover -s tests/updates -p 'test_prepare_sdk.py' -v
uv run --no-project python -m unittest discover -s tests/updates/winsparkle -p 'test_*.py' -v
uv run --no-project python -m unittest discover -s tests/updates/sparkle -p 'test_*.py' -v
```

For native build commands and exact runtime coverage, use each SDK's README.
Native runners require prepared provenance, exact patched-file hashes and a new
build directory under `target/`. Unsupported platforms fail; they do not report
skipped native tests as a pass. macOS checks also require every selected XCTest
method to execute and pass, not only a successful xcodebuild exit.

Build logs, provenance, results and test binaries stay under ignored `target/`.
CI uploads logs/results, not distributable SDK binaries. Do not commit archives,
SDK copies, build products, private keys, or certificate stores.

## Integration boundary

A future host must check the hardened capability before initialization and retain
its SDK and callback storage for the documented lifetime. Windows still needs a
sealed-copy helper, final save/export barrier and NSIS transaction tests. macOS
still needs authenticated framework/helper/process-agent tests and GPUI termination
coordination. Both need real signed A-to-B upgrade, rollback, relaunch and normal
reader behavior evidence before integration.

Public-key review, protected private-key storage, Apple fixture signing and
production Developer ID/notarization remain owner-controlled. Do not add an
unsigned fallback, sample production trust root or XPC authentication bypass.
The acceptance criteria in
[`../../docs/plans/2026-11-07_confirmed-auto-update-plan.md`](../../docs/plans/2026-11-07_confirmed-auto-update-plan.md)
remain unchanged.
