# Confirmed auto-update preflight

## Status

**Native automatic installation is deferred and has not passed the technical gate.**

The user reduced the active scope to
[release notifications](release-notifications.md) on 2026-11-09. Notification
checks do not resolve or replace the installation requirements below.

The approved plan is [2026-11-07_confirmed-auto-update-plan.md](plans/2026-11-07_confirmed-auto-update-plan.md).
Work started from `main` at `527b1de4f2ce3546d6e3cca1a866b9d2dcb614d1`
on branch `narumi/feat/confirmed-auto-update`.
The checks below were made on a Windows x64 host through WSL and native Windows tools.

Plan section 7.1 requires the native SDK handoff and termination gate to pass before
full implementation. The pinned standard integrations do not meet that gate.
Acceptance criteria A1–A15 remain unverified as complete updater requirements.
No reader, dependency, installer, package, release workflow, setting schema, or
version was changed. The existing launch IPC remains paths-only.

## SDK checks

Both official archives match the SHA-256 values in the approved plan:

| SDK | Archive | Result |
| --- | --- | --- |
| WinSparkle 0.9.4 | `WinSparkle-0.9.4.zip` | SHA-256 matched; native Windows DLL and tool smoke check passed |
| Sparkle 2.10.1 | `Sparkle-2.10.1.tar.xz` | SHA-256 matched; source and headers inspected; no native macOS execution |

SDK binaries, source downloads, and local logs are under ignored `target/` paths.
They are not committed or shipped. No updater was initialized and no installer or
remote update payload was executed.

### Reproduce the limited Windows smoke check

Download the official pinned archive from the
[WinSparkle 0.9.4 release](https://github.com/vslavik/winsparkle/releases/tag/v0.9.4),
then use native 64-bit Windows Python:

```powershell
uv run --no-project python tests/updates/sdk_preflight.py C:\path\to\WinSparkle-0.9.4.zip
```

The script verifies the complete archive before reading two fixed members. It loads
the DLL in a child process with its dependency search restricted to the verified
DLL directory and System32. The child exits before temporary-file cleanup.
It resolves 12 C ABI exports and checks acceptance of a valid public test key and
rejection of an invalid key. The official signing tool then checks a temporary,
non-executable text fixture:

1. A valid signature is accepted.
2. A different public key is rejected.
3. Missing and malformed signatures are rejected.
4. Modified and truncated payloads are rejected.

The private fixture key is created in a temporary directory, is never printed, and
is removed after the test. It is not a production key or a product trust anchor.
The script does not initialize WinSparkle, access its registry settings, start its
workers, use the network, or execute an installer. Passing this script does not
prove GPUI integration, callback ordering at runtime, safe handoff, cancellation,
update installation, shutdown, relaunch, or security-gate completion.

An initial version of the smoke script retained a loaded DLL during temporary-file
cleanup. Windows refused to delete that DLL. The final script isolates the DLL probe
in a child process and waits for its exit; cleanup then passed. This is not evidence
that SDK `cleanup()` permits unloading an initialized SDK.

## Source gate findings

These are source/API findings, not demonstrations of an exploitable attack.
No exploit payloads were constructed and no upstream security report was submitted.

### Windows

Reviewed WinSparkle source commit:
`a8986caf620262f7d4581b241436ceaa0cc9370f`.

1. **Verified bytes are not part of the installer callback contract.**
   The downloader verifies a pathname and queues that pathname. The verifier's
   file handle is closed before the user confirms installation. The callback
   receives only `const wchar_t*`, not the verified bytes, digest, signature, or
   an immutable file handle. Hashing that pathname at callback time and checking
   the hash again in a helper cannot prove identity with the earlier verification.
   - [Callback API](https://github.com/vslavik/winsparkle/blob/a8986caf620262f7d4581b241436ceaa0cc9370f/include/winsparkle.h#L565-L580)
   - [Download and verification](https://github.com/vslavik/winsparkle/blob/a8986caf620262f7d4581b241436ceaa0cc9370f/src/updatedownloader.cpp#L176-L198)
   - [Verifier lifetime](https://github.com/vslavik/winsparkle/blob/a8986caf620262f7d4581b241436ceaa0cc9370f/src/signatureverifier.cpp#L330-L369)
   - [Installer callback](https://github.com/vslavik/winsparkle/blob/a8986caf620262f7d4581b241436ceaa0cc9370f/src/ui.cpp#L728-L759)
2. **SDK cleanup is not a worker-unload barrier.**
   The implementation shuts down the UI but explicitly leaves worker shutdown
   unfinished. A future adapter must not unload the library or destroy callback
   storage when a reader view closes. Holding a synchronous callback reply while
   cleanup joins that callback's thread also needs a native deadlock test.
   - [Cleanup implementation](https://github.com/vslavik/winsparkle/blob/a8986caf620262f7d4581b241436ceaa0cc9370f/src/dll_api.cpp#L104-L113)
3. **Other trust requirements remain open.**
   Download-path confinement before verification and inert release-note rendering
   are not established by the inspected public API. Feed-generator escaping is not
   proof of safe handling of a modified feed. These requirements need a reviewed
   SDK-level solution and native tests, not a signature or download bypass.
4. **Redistribution review is incomplete.**
   The inspected source links bundled legacy OpenSSL in addition to the outer MIT
   license. Dependency notices and compatibility with the project's
   `AGPL-3.0-only` license must be reviewed before SDK redistribution. No license
   clearance is claimed. The test script does not redistribute the SDK.

### macOS

Reviewed Sparkle source commit:
`ab7d71291238df6e0492143b120c9b696a35f603`.
GPUI macOS source is `gpui-pre-macos 0.3.11`; the downloaded crate checksum matches
`Cargo.lock`.

1. **A manually prepared update can install on ordinary termination.**
   Sparkle documents that the application can terminate before the final ready
   reply, and that a dismiss reply can leave installation pending. Disabling
   automatic download/installation does not disable this prepared manual update
   behavior. This conflicts with A3 and the plan's confirmation/flush boundary.
   - [Ready-to-install contract](https://github.com/sparkle-project/Sparkle/blob/ab7d71291238df6e0492143b120c9b696a35f603/Sparkle/SPUUserDriver.h#L209-L225)
   - [Host-termination handling](https://github.com/sparkle-project/Sparkle/blob/ab7d71291238df6e0492143b120c9b696a35f603/Autoupdate/AppInstaller.m#L394-L418)
2. **A relaunch postponement is not an installation commit or cancellation API.**
   Retaining the continuation can delay a relaunch, but does not establish that
   the installer will do nothing if the application exits. A normal dismiss is
   different from the user-driver cancellation reply.
3. **GPUI's quit callback cannot veto termination.**
   Its macOS delegate handles `applicationWillTerminate:`, not
   `applicationShouldTerminate:`. The existing `on_app_quit` callback is too late
   for a save-failure veto. Adding a coordinator must preserve GPUI's delegate,
   main-thread rules, and event handlers; this has not been runtime-verified.
4. **The other native guarantees are unproven.**
   The SDK's termination observer is not an application-owned startup/update
   gate. Strict incoming identifier/architecture checks and restoration after
   installation failure also need separate proof. Main-thread coexistence is
   source-feasible, but no native GPUI/Sparkle integration test was run here.

## Trust and platform prerequisites

Read-only GitHub metadata queries returned no repository Actions secrets or
environments. No formal update public key is tracked. This does not prove that
the owner lacks offline keys, Apple credentials, or organization-level resources.
No secret values, private keys, certificate contents, or account credentials were
read. No production keys were generated and no GitHub settings were changed.

After an SDK route passes review, the owner must provide reviewed production
public keys and arrange protected storage and backup of their private keys.
macOS distribution acceptance also requires Developer ID signing and notarization
credentials. Private keys must not be pasted into chat or committed. No paid
account or secret was created by this work.

## Existing application checks

These checks run the unchanged application at `527b1de`; they are not updater tests.
Native Cargo uses Rust 1.99.1 and the Windows MSVC toolchain.

| Check | Result |
| --- | --- |
| Rust formatting | Passed |
| All-target/all-feature Clippy with `-D warnings` | Passed |
| Full Rust suite, default parallelism | Failed: 166 passed; IPC forwarding and bounded-queue tests failed |
| Full Rust suite, serial execution | Passed: 168 passed, no ignored tests |
| Production rendering, first run | Failed at `tests/rendering.rs:120`: expected `100`, got `99.79` |
| Production rendering, separate rerun | Passed; native Windows events and geometry, not GPU pixels |
| Pinned actionlint 1.7.12 | Passed |
| Pinned changenote 0.10.0 status | Passed; no pending note |
| Release/DMG fixtures on Linux | 20 passed; native changenote case skipped here |
| Native Windows changenote version fixture | Passed separately; 21 release cases passed across both environments |

The bounded-queue test reported ConnectionReset. The forwarding test received an
unexpected queued path. The serial and rendering rerun passes do not replace or
hide the first failures. No test was skipped, weakened, or changed to obtain these
results. Their causes remain unresolved.

Local logs are in `target/update-preflight/`: `tests.log`, `tests-serial.log`,
`rendering.log`, `rendering-rerun.log`, `clippy.log`, `fmt.log`, `release-tests.log`,
`version-test.log`, `actionlint.log`, and `sdk-preflight-final.log`.
The reproducible SDK smoke check is a separate, explicit test; normal reader CI
is not presented as an SDK integration pass.

## Required next action

1. Review and approve an SDK-level design change that preserves the original
   acceptance criteria. It must resolve verified-byte handoff on Windows and the
   ordinary-quit/precommit installation policy on macOS. Do not silently substitute
   a custom downloader or lower A3/A4/A6/A7/A9/A13.
2. Prove the revised route with real GPUI A/B fixtures, disposable signing keys,
   and a local signed feed on native Windows and both macOS architectures.
   Cancellation, flush failure, delayed export, crash before confirmation,
   concurrent launch, callback lifetime, and helper termination need real evidence.
   These native tests were already authorized by the plan; only a material design
   change needs new approval.
3. Arrange the owner's production trust/signing prerequisites. Test-key or ad-hoc
   fixture success cannot prove Developer ID/notarization acceptance.
4. Resume the remaining plan only after its gate passes. Until then, keep the PR
   in draft and do not describe automatic updating as delivered.

No application feature or release-package behavior changed, so this blocked
preflight adds no changenote. No package was published, tag created, or release
workflow dispatched.
