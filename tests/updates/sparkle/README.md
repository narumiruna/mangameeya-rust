# Sparkle hardening checks

These checks test an SDK patch, not a working MangaMeeya updater. No acceptance
criterion in the confirmed-update plan is marked complete by these checks alone.

## Checks that can run without macOS

1. `check_patch.py --source <untouched pinned source>` applies the actual patch in
   a temporary copy. It checks exact input/output hashes and rejects a second apply.
   It does not only search for source strings.
2. `commit_policy_test.c` compiles the same policy header used by `AppInstaller.m`.
   It tests wrong/missing/replayed/expired COMMIT, cancellation, and once-only final
   installation. POSIX child processes test a killed host before COMMIT, a lost
   connection while the host remains alive, acknowledged cancellation after staging
   cleanup, and COMMIT while a host remains alive. The test then uses `waitpid`
   before final installation becomes possible. It does not use sleeps as a gate.

Example on Linux or macOS, after patch application:

```sh
cc -std=c11 -Wall -Wextra -Werror -O2 -UNDEBUG \
  -I target/sparkle-hardening/prepared/source/Autoupdate \
  tests/updates/sparkle/commit_policy_test.c -o target/sparkle-hardening/commit-policy
./target/sparkle-hardening/commit-policy
```

3. The 12 `test_build_sdk.py` cases check unsupported hosts, process and nested
   binary architecture, minimum OS, and XCTest result evidence. Zero selected tests,
   skipped/failed methods and missing required method results fail the helper.
   Binary/result metadata is mocked; these are not native runtime evidence.

```sh
uv run --no-project python -m unittest discover -s tests/updates/sparkle -p 'test_*.py' -v
```

The process harness uses marker files for its side effects. It tests the production
policy, not the Sparkle XPC transport, NSRunningApplication observer, or replacement
engine. Do not present this harness as a native end-to-end install test.

## Native SDK checks supplied for CI

`packaging/updates/sparkle/build_sdk.py` runs these checks separately on arm64 and
x86_64. It fails on non-macOS hosts and requires full Xcode, not Command Line Tools.

1. Build the patched real `Sparkle.xcodeproj` framework and nested tools in Release.
2. Run the new `SUHardenedInstallerTest` XCTest class in the real Sparkle unit target:
   - Production COMMIT policy: reject missing/wrong/expired authorization, disarm on
     precommit host death, reject post-cancel COMMIT, wait before install, install once.
   - Compile isolated real Mach-O app fixtures with Xcode's compiler.
   - Reject a different bundle identifier, the other architecture, a universal
     payload that could switch Rosetta architecture, downgrade, current-version
     replay, missing expected version and feed/payload version mismatch.
   - Cancel a real owned child while extraction is active and join its worker.
     Cancel before launch and verify no child starts. These isolated cases do not
     replace a real host-crash/XPC-loss cleanup test.
   - Run the real `SUPlainInstaller` A → B bundle replacement in a temporary path.
   - Inject file-manager failures in the test build only. Verify restoration of A
     after failed replacement. If restoration also fails, run real cleanup and
     verify that the retained backup still contains A's executable and metadata.
3. Compile and run the POSIX production-policy harness on that native architecture.
4. Load only the exact built framework and call its public capability selector.
5. Inspect every nested Mach-O for the selected architecture and a minimum OS no
   higher than macOS 12. Record Xcode version, logs, an xcresult, and `evidence.json`.

The runner uses Xcode ad-hoc fixture signing (`CODE_SIGN_IDENTITY=-`). It does not
use release scripts, provisioning, production credentials, notarization, or
publication. It does not disable strict XPC authentication or Library Validation.
Its output is not a distributable project package.
A failed native build or test blocks the gate; do not replace it with a source-text
assertion. Current Windows/WSL evidence does not establish which full Xcode version
can build the pinned SDK. The CI run must record the actual version.

## Minimum remaining native runtime gate

The following tests are still required with the actual Sparkle framework, helper,
process agent, signed local feed/payload fixtures, and application coordinator.
They are not implemented or claimed by the policy harness. Authentication tests
need an owner-approved fixture signing identity that meets the strict XPC code
requirement; do not add an ad-hoc authentication bypass to obtain a passing run.

1. Prepare B manually, then quit A normally before the final consent reply.
   Kill A before the reply in a separate case. Both must remove owned staging,
   retain A byte-for-byte, and never enter final replacement.
2. Fail final flush, cancel the SDK dialog, invoke the public cancellation block,
   lose XPC, and expire the prepared session. Verify that no case can commit.
   For explicit cancellation, normal quit must wait for a real disarm ACK. Drop
   the ACK and verify the host receives an error, not false success.
3. Reject a stock helper before installation data is sent. Reject a stock framework
   by capability check. Reject another connection and old-session COMMIT replay.
4. Reply YES only after actual GPUI flush success and export completion. Hold A's
   process alive after COMMIT ACK and verify no bundle writes. Release it, verify
   the process observer fires, and verify exactly one B process and window.
5. Repeat A → B on both architectures. On Apple Silicon, run x86_64 A under Rosetta
   and verify it accepts only x86_64 B and relaunches x86_64, not arm64/universal B.
6. Test signature mismatch, wrong identifier/architecture, downgrade, denied writes,
   failed replacement/restoration, late callbacks, and concurrent application launch.
   Verify the startup/update gate and retained old backup with real files/processes.
7. Preserve the project's GPUI delegate/event loop, document-open/IPC behavior,
   settings, document associations, and single-instance ownership during all cases.

Developer ID, Hardened Runtime, notarization, and production package acceptance
remain separate owner-controlled prerequisites. Do not describe test-key,
unsigned, or ad-hoc fixture results as production verification.
