# Release notifications

## Scope

This feature checks for a newer stable release and shows a non-modal notice.
Download and installation remain manual. It does not load WinSparkle or Sparkle,
fetch release assets, execute an installer, replace the app, or restart it.

The user reduced the requested scope to notifications on 2026-11-09. The native
installation plan in `plans/2026-11-07_confirmed-auto-update-plan.md` is deferred.
Its SDK research and security requirements remain unchanged for any future
installation work. Notification checks are not evidence that those gates pass.

## Behavior

- The primary application checks once on startup. Checks are enabled by default.
  Disable **Preferences → New version notifications → Check for updates on
  startup** to keep future launches offline.
- **Check for updates** in Preferences, the Help menu, or the native app menu
  starts a manual check. It works when startup checks are disabled.
- Checks run on a background worker. Concurrent requests are not started.
  There is no periodic polling or automatic retry.
- A newer stable version shows a notice without moving focus or interrupting
  reading. Dismiss hides the notice until the next completed check.
- Only an explicit **Open download page** action opens the fixed official page:
  `https://github.com/narumiruna/mangameeya-rust/releases`.
- Check failures appear in Preferences. They do not close a book, replace reader
  messages, or block reading.

## Network and storage

Checks use the fixed HTTPS GitHub API endpoint
`https://api.github.com/repos/narumiruna/mangameeya-rust/releases/latest`.
TLS certificate validation remains enabled. Requests send an application/version
User-Agent and no application credentials or reading data. GitHub receives normal
connection data, including the source IP address. The request has a total timeout
and a bounded JSON response. Draft and prerelease versions are not notified.

The startup preference is stored in `release-notifications.json`, next to
`settings.json`. Its only field is `check_on_startup`. Saving it does not change
reader settings, bookmarks, recent books, or reading positions. If this file
cannot be read, startup checks stay disabled and Preferences shows the error.
An explicit checkbox change writes a new notification preference file.

## Verification and release limits

Deterministic backend and GPUI tests do not contact GitHub. Construction of a
`ReaderView` alone does not start a network check; only the real application's
primary launch does so automatically.

Verified locally on macOS arm64 on 2026-11-09:

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` and `git diff --check` | Passed |
| `cargo test --locked --all-features` | 412 passed; none failed or ignored |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --locked --test rendering` | Passed with the native Metal renderer; includes new Preferences controls at both supported sizes in all three languages |
| `changenote status` with CLI 0.10.0 | Passed; notification feature is a pending minor note, not a published version |
| `CHANGENOTE_BIN=<local CLI> uv run --no-project python -m unittest discover -s tests/release -v` | 21 passed; no skips |

The Rust suite includes 20 backend checks and six GPUI notification checks.
HTTP fixtures use loopback servers; UI checks use deterministic results rather
than GitHub. A live GitHub check was not used as test evidence.

Local macOS arm64 checks do not establish native Windows or Intel macOS behavior.
The maintainer must obtain the existing CI results on those platforms before a
release. This change does not alter package signing, publish a release, or claim
automatic-installation readiness.
