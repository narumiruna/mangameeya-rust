# Native rewrite verification

## Scope

The application uses Rust and GPUI Kit 0.9.0 on Windows and macOS. The former Tauri and TypeScript application, Node dependencies, frontend tests, and WebView configuration are removed.

```mermaid
flowchart LR
    Desktop[Native menus, dialogs, keyboard, mouse, file drops] --> UI[GPUI Kit ReaderView]
    Launch[CLI and macOS document events] --> UI
    Instance[Authenticated local launch forwarding] --> UI
    UI --> Reader[Reader state and settings]
    UI --> Worker[Bounded background image jobs]
    Worker --> Book[Image folders and ZIP / RAR]
    Worker --> Imaging[EXIF, animation, rotation, filters]
    Imaging --> Tiles[GPU texture tiles]
    Tiles --> UI
    Reader --> JSON[Atomic per-user JSON]
```

## Regression coverage

- Book tests cover natural order, selected images, recursive folders, directory links, canonical paths, directory browsing, missing/corrupt/empty archives, supported ZIP compression methods, RAR generations and solid archives, encryption errors, bounded reads, and source-safe export. RAR regressions include a large preceding non-image attachment and the RAR5/RAR7 dictionary memory limit.
- Reader tests cover navigation, reading direction, spreads, mode changes, bookmarks, position restoration, slideshow, fit, rotation, zoom, and continuous scroll anchors.
- Image tests cover actual encoded formats, EXIF orientations, animation timing/compositing, transforms, thumbnails, and allocation limits.
- Settings tests cover defaults, JSON validation, atomic replacement, backup/reset, lossless native paths, legacy UTF-8 JSON, and preservation of corrupt or unsupported data.
- Instance tests use real local sockets and child processes. They cover authentication, queue and message bounds, concurrent primary ownership, stale endpoints, focus messages, native path encoding, and cleanup.
- GPUI tests use the production view and event path. They cover native open/save dialogs, file browsing, history, fullscreen, slideshow, stale book results, launch ordering, image atlas cleanup, continuous scroll remeasurement, mixed page widths, visible-range loading, visible animated thumbnails above the cache budget, cache and atlas cleanup after hiding the sidebar, tiled long pages, native-path settings after book close, safe settings reset, and save ordering.
- The standalone rendering test opens the production view with native text shaping. On macOS it uses Metal, checks decoded page pixels, and saves captures under `target/ui-smoke/`. It does not need screen recording permission.

## Settings path format

UTF-8 paths remain JSON strings. Other paths use a single `{"unix":[u8]}` or `{"windows":[u16]}` object with the original native units. Recent history remains an array. A position or bookmark map stays a JSON object when all keys are UTF-8; otherwise it uses an array of `[path, page]` pairs. Loading accepts both map formats.

Native objects are platform-specific. Unknown tags, invalid units, extra fields, and foreign-platform encodings are errors; loading and saving leave the file unchanged. Older builds cannot read native objects or pair maps. Keep a backup before a downgrade. UTF-8-only saves retain the legacy format.

## Local checks

The verification host is macOS arm64 with Rust 1.99.1 and full Xcode selected through `DEVELOPER_DIR`.

Native rewrite baseline, verified on 2026-11-07 before the unified zoom change:

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo test --locked --all-features` | 113 passed; no failures or ignored tests |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --locked --test rendering` | Passed with real Metal pixels and 10 captures |
| Windows MSVC all-target, all-feature Clippy with `-D warnings` | Passed using LLVM resource tools; no manifest bypass |
| Intel macOS `cargo check --locked --all-targets --target x86_64-apple-darwin` | Passed |
| `bash packaging/macos/build.sh aarch64-apple-darwin` | Release app, ZIP, and DMG created; plist, architecture, ad-hoc signature, and DMG checksum checks passed |
| Packaged macOS app launch | One visible native window; the initial image loaded |
| Native macOS document-open event | Selected the second project icon in the existing window |
| Second CLI process | Forwarded an image to the same primary PID without creating another window |
| `git diff --check` and dependency audit | Passed; no Tauri/WebView application dependencies remain |

The rendering workflow verifies actual CBZ extraction and natural order, decoded image colors, spread order, keyboard and pointer input, input focus, page jump, zoom, ordinary and Control-wheel input, continuous navigation, bookmarks, settings, persistence, window-wide OS file drops, errors, close, lossless GPU tiles for long pages, and animated source pixels. Captures are in `target/ui-smoke/01-empty.png` through `10-cbz-natural-order.png`.

Native launch checks used the project's icon files, not user comics. The app was stopped after the check. The tests do not modify comic sources.

macOS output is under `target/release/bundle/macos/aarch64-apple-darwin/`. Windows installer compilation and macOS packaging mechanics were also checked with temporary test payloads, but those checks are not substitutes for the real macOS release build above or for Windows execution.

## Unified zoom control verification

Validated on 2026-11-07 on macOS arm64 with Rust 1.99.1 and `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`.
The implementation is based on `7425e2a`. The unchanged baseline passed 115 tests and the native rendering workflow before implementation.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo test --locked --all-features` | 132 passed; no failures or ignored tests |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --locked --test rendering` | Passed with real Metal pixels, the original 10 captures, and 6 zoom captures |
| Pinned `changenote 0.10.0` `status` | Passed; pending notes produce one patch release plan |
| `CHANGENOTE_BIN="$PWD/target/changenote-cli/bin/changenote" uv run --no-project python -m unittest discover -s tests/release -v` | 15 passed; GitHub upload commands are mocked |
| `git diff --check` | Passed |

The added tests cover the controlled Popover lifecycle and 16 zoom regressions. They check the rendered page bounds against source dimensions, rather than only comparing two scale helpers. Coverage includes:

- Effective scale for single pages, unequal-size RTL/LTR spreads, the last unpaired page, continuous active pages, retained geometry, rotation, missing geometry, and errors.
- Fitted percentages below 1% and outside the manual 10–800% range. The first manual step uses the unrounded fitted scale.
- Fit choices, their accessible pressed state, custom versus original-size 100%, repeated stepper clicks, and existing reader/menu command paths.
- Keyboard activation, Tab navigation, reader-shortcut isolation, fit/zoom shortcuts outside text editing, global close, and Escape before fullscreen exit.
- Draft retention during resize, sidebar/inspector changes, page changes, rotation, and asynchronous geometry updates, including temporarily missing geometry.
- Invalid/non-finite input, repeated `%` suffixes, finite-value clamping, disabled pending controls, and delayed submissions from a closed panel.
- No Control-wheel zoom through an open panel, no zoom from horizontal-only or zero wheel input, and panel dismissal on book close or asynchronous replacement.
- Display-only input synchronization without settings-dirty changes or settings-file writes. Existing paged scroll-bound regressions also pass.

The production rendering workflow checks popup bounds and visible controls at both 1280×800 and 960×640. Captures remain local or in CI artifacts, not tracked source assets:

- `target/ui-smoke/zoom-before.png`: retained baseline toolbar.
- `target/ui-smoke/zoom-closed-{1280x800,960x640}.png`: current fit mode and percentage.
- `target/ui-smoke/zoom-open-{1280x800,960x640}.png`: the fit choices and manual zoom controls.
- `target/ui-smoke/zoom-custom-{1280x800,960x640}.png`: custom 200% after input submission.

The change adds no dependencies, settings migration, persisted zoom value, or image assets. Source comics are not modified. Recovery is a revert of the UI change and its unreleased note; existing settings files need no conversion.
No package was published, version tag created, or release workflow dispatched for this work.
The complete change was reviewed for correctness, security, lifecycle safety, compatibility, regressions, and scope. No material defects remain.
The completed execution plan was `docs/plans/2026-11-07_unified-zoom-control-plan.md`; its external handoff is retained below.

### Retained zoom handoff

- **Product maintainer:** With the before/after captures and the selected proposal, review visual clarity and reading comfort. The expected result is an assessment of discoverability and usability. These subjective properties are not verified by pixel or geometry tests. This review is optional and is not an agent coding gate.
- **Platform maintainer:** With an interactive Windows or Intel macOS desktop at the intended display scaling, inspect text, popup placement, focus restoration, and input behavior. Automated platform CI is separate evidence. These interactive desktop paths are not verified by local macOS arm64 checks. No new release gate is added; do not claim platform-specific visual or runtime verification without the corresponding evidence.

## Internationalization verification

Validated on 2026-11-07 on macOS arm64 with Rust 1.99.1 and `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`.

- `cargo test --locked --all-features`: 144 passed; no failures or ignored tests.
- `cargo clippy --locked --all-targets --all-features -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo test --locked --test rendering`: passed with real Metal pixels and all three locales at 1280×800, 960×640, and 800×540.
- `cargo check --locked --all-targets --target x86_64-apple-darwin`: passed; this is compilation evidence, not Intel runtime evidence.
- Pinned `changenote 0.10.0 status`: passed; pending notes select one minor release.
- `CHANGENOTE_BIN="$PWD/target/changenote-cli/bin/changenote" uv run --no-project python -m unittest discover -s tests/release -v`: 15 passed; GitHub operations are mocked.

The catalog requires English, Japanese, and Traditional Chinese for each typed message key. Tests check nonempty translations, equal named placeholders, stable locale codes, and one-pass substitution that preserves Unicode and braces in paths and error details. Settings tests cover old-file English defaults, all three language round trips, unchanged reading data, and preservation of files with unsupported language values.

Seven GPUI regressions check default English, preference-button selection in each language, translated reader and native menus, accessible controls, input placeholders, native file/folder prompt wording, current formatted status and diagnostic details, saved selection after reopening, and reader/input state preservation. Existing Traditional Chinese test baselines are explicit. Native dialog controls still use the OS language. Tests inspect native menu definitions; they do not prove physical native menu activation or IME input.

Native rendering checks the translated reader menus, component locale, accessible zoom controls, and sidebar tab bounds at all three sizes. Japanese tabs wrap inside the fixed-width sidebar rather than overlap the reader. Language captures are `target/ui-smoke/i18n-{en,ja,zh-Hant}.png`, with additional `-960x640` and `-800x540` captures. These are local/CI outputs, not tracked image assets.

English is the default when `preferences.language` is absent. Saved values are `en`, `ja`, and `zh-Hant`. Unknown values remain errors and are not overwritten. Before a downgrade, back up `settings.json` and remove only `preferences.language` from the working copy; older builds reject unknown fields. Reading history and comic files need no conversion. No dependencies or image assets were added, and no package or release was published.

### Known component limits

GPUI uses `zh-TW` for its Traditional Chinese component locale. GPUI Kit 0.9.0 does not supply Japanese component translations, so built-in text-editing menus fall back to English. Application-owned reader strings use the complete Japanese catalog. File names, technical diagnostic details, units, and shortcuts are unchanged.

The pinned macOS GPUI implementation retains 24 action entries each time the application rebuilds native menus. A language change can therefore add a small process-lifetime allocation. No stale command dispatch was found. Quitting releases the process memory; this upstream issue does not change settings or comic data.

### Retained internationalization handoff

- **Platform maintainer:** With a Windows or Intel macOS desktop, native toolchain, and intended display scaling, run the native suite and inspect translated text, native menu activation, and IME/focus behavior. The expected result is readable text and working commands in each language. Local arm64 checks do not verify these runtime paths. Do not describe those platforms or input methods as runtime-verified without the corresponding evidence.
- **Product maintainer:** With the language captures under `target/ui-smoke/`, review Japanese and Traditional Chinese wording, glyph forms, and reading comfort. The expected result is an assessment of linguistic and visual quality. These subjective properties are not established by automated geometry or pixel checks. This review is optional and is not an agent coding gate.

## macOS DMG verification retry

The arm64 job in [run 42362258497](https://github.com/narumiruna/mangameeya-rust/actions/runs/42362258497) passed Rust tests, native rendering, and Clippy, then failed during packaging. Immediately after image creation, `hdiutil verify` reported `Resource temporarily unavailable`. The Windows, Intel macOS, and release-check jobs passed.

`packaging/macos/verify-dmg.sh` now retries only that diagnostic. It uses at most three verification attempts with two seconds between attempts, keeps standard input closed, and retains the final failure status. Other verification errors fail immediately. Packaging still requires successful checksum verification and does not recreate or bypass the image during retries.

Local checks on 2026-11-07, macOS arm64:

- Six new mocked DMG tests passed. They cover immediate success, recovery after one or two transient errors, exhausted retries, immediate failure for corruption and permission errors, a permanent error after a transient error, invalid arguments, quoted Unicode paths, fixed diagnostic locale, and closed standard input.
- All 21 release/packaging tests passed with the pinned changenote CLI; native tools and GitHub writes are mocked in those tests.
- A real `bash packaging/macos/build.sh aarch64-apple-darwin` build passed bundle metadata, architecture, ad-hoc signature, and DMG checksum checks, and produced the app, ZIP, and DMG under `target/release/bundle/macos/aarch64-apple-darwin/`.
- All 144 Rust tests, all-target/all-feature Clippy, native Metal rendering, formatting, shell syntax, changenote validation, and diff checks passed.

The transient lock was simulated deterministically in regression tests; it did not occur during the successful local package build. No release workflow was dispatched, no package was published, and the unrelated rust-i18n migration plan was not changed.

## Desktop efficiency workspace verification

Implementation work started on 2026-11-07 from `d41f28b` on Windows x64.
The approved plan is `docs/plans/2026-11-07_desktop-efficiency-plan.md`.
The branch is `narumi/feat/desktop-efficiency-ui`.
**This work is not fully verified. PR #13 is open; required platform checks still need CI evidence.**

The change uses GPUI Kit 0.9.0 without new dependencies. It adds a grouped toolbar, explicit reading modes, resizable side panels, three reading-tool tabs, and a page-information Popover. Reading, decoding, archive, IPC, and export implementations are unchanged.

### Initial implementation evidence

These results describe the original implementation before the main-branch merge. Merge checks are recorded below.
Rust is 1.99.1. Cargo commands used the native Windows MSVC toolchain.
The shell is WSL; the Cargo process and test executables are Windows programs, not Linux builds.
The native Rust bin directory was added only to the child process PATH so Cargo could find rustfmt and Clippy.

| Check | Result |
| --- | --- |
| Baseline `d41f28b` fmt and all-target/all-feature Clippy | Passed in a temporary detached worktree; the worktree was removed |
| Baseline full Rust suite | 139 passed, 3 failed: two IPC ConnectionReset failures and one settings AccessDenied failure |
| Baseline settings backup test, isolated rerun | Passed |
| Baseline production rendering | Passed native Windows events and geometry |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | Passed |
| `cargo test --locked --all-features ui::tests::` | 60 passed; no failures or ignored tests |
| Settings tests | 24 passed; no failures or ignored tests |
| `cargo test --locked --test rendering` | Passed native Windows events and geometry, including the desktop matrix |
| Full Rust suite, default parallelism | **Failed:** 166 passed, 2 failed in unchanged IPC tests |
| Full Rust suite, `-- --test-threads=1` | **Passed:** 168 passed; no failures or ignored tests |
| Pinned changenote 0.10.0 `status` | Passed; pending notes select a minor release; no product version files were changed |
| Release/DMG mock tests on WSL Linux | 20 passed; the native changenote integration test was skipped here and run separately below |
| Native changenote version integration on Windows | 1 passed, with `PYTHONUTF8=1` and the pinned CLI; changes were limited to temporary fixtures |

The complete full-suite failures are retained, not suppressed:

- The final default run failed `instance::tests::queue_is_bounded_and_rejects_overflow` and `instance::tests::rejects_oversized_payloads_and_path_counts` with ConnectionReset. An earlier parallel run also failed `instance::tests::forwards_paths_and_empty_focus_requests` with an unexpected queued path.
- An earlier serial run failed the queue test. The final serial run passed all 168 tests, including the IPC concurrency tests; no test was ignored or removed.
- The queue failure was also reproduced before implementation. Its cause is not established. No IPC code or test was changed to hide it.
- Normal parallel CI results are still required to assess this baseline instability. The serial pass is not reported as a parallel pass.

The shell mock tests require a POSIX shell and jq. A first attempt under Windows Python selected WSL bash and failed on Windows paths. The shell cases were rerun successfully on Linux. The separate native CLI integration needed UTF-8 mode for subprocess output. These environment corrections did not change test assertions or release scripts.

Local logs are under `target/`: `desktop-baseline.log`, `desktop-baseline-lint.log`, `desktop-baseline-rendering.log`, `desktop-ui-final.log`, `desktop-settings.log`, `desktop-clippy-final.log`, `desktop-rendering-final.log`, `desktop-tests-final.log`, `desktop-tests-serial-final.log`, `desktop-linux-release-tests.log`, and `desktop-version-test2.log`.
These are local outputs, not tracked source assets.

### Regression coverage and review fixes

The desktop tests use real pointer, keyboard, wheel, menu, and input events. They cover three locales at 1280×800, 960×640, and 800×540; explicit mode selection; both panel states and width persistence; page and progress boundaries; image/read/playback separation; field drafts; reset scope; recursion on the next open; information lifecycle; and shortcut isolation.

Review added targeted protection for:

- Two consecutive sidebar drags in a constrained viewport without overwriting the untouched inspector preference.
- A no-op constrained inspector overdrag. The callback compares visible, clamped widths and leaves the stored preference and file bytes unchanged.
- A right-panel overdrag followed by a sidebar drag. Completed drags reconcile GPUI's raw slot sizes with visible width limits.
- Pointer-move remeasurement without settings writes; only a changed, completed handle drag commits a width.
- A jump to an unmeasured virtual thumbnail. It uses an explicit item anchor when no measured bounds exist.
- Fit and zoom keys inside Information Popover, without changing the existing Zoom Popover shortcut rules.
- Native Preferences while the playback input is focused. It commits the existing Blur value and moves focus before the input is removed.

The production rendering test defines 84 additional `desktop-*` scenarios. On macOS, captures go under `target/ui-smoke/`.
Windows runs the interaction and geometry checks but the existing test does not create GPU pixel captures. HTML concept images are not native verification evidence.

### Settings compatibility

`preferences.panel_layout` stores integer logical-pixel widths and inspector visibility. Missing fields use defaults. Invalid types, unknown fields, duplicate fields, and non-object layouts are rejected without overwriting the original file. Existing reading data and native path formats are retained.

Older builds reject the new key. Before a downgrade, back up `settings.json` and remove only `preferences.panel_layout` from a working copy. Builds that also predate language support require the documented removal of `preferences.language`. Do not reset reading data to downgrade.

### Merge-conflict resolution

On 2026-11-07, the branch was updated with `main` at `986512f`.
Upstream split the README into English, Traditional Chinese, and Japanese and released version 0.3.0.
The merge keeps that document structure and release metadata.
The desktop UI controls, panel settings, and safe downgrade instructions are present in all three README files.

No Rust source, test, dependency resolution, build script, asset, or workflow was changed by this conflict resolution.
The completed upstream release notes stay removed. The desktop feature note remains pending.
The pinned changenote CLI reports the next minor version as 0.4.0; this check did not change product version files.

Checks on the combined tree:

| Check | Result |
| --- | --- |
| Formatting and all-target/all-feature Clippy | Passed on Windows MSVC |
| Full Rust suite, default parallelism | 167 passed, 1 failed in the existing IPC queue test with ConnectionReset |
| Full Rust suite, serial execution | 168 passed; no failures or ignored tests |
| Production rendering | Passed native Windows events and geometry |
| Pinned changenote status and native version integration | Passed; version integration used temporary fixtures |
| Release/DMG mock checks | 20 passed on Linux; the skipped native CLI case passed separately on Windows |
| Multilingual README checks | Upstream sections, local links, settings fields, code fences, and conflict-marker checks passed |

The new logs are `target/desktop-merge-clippy.log`, `desktop-merge-tests.log`, `desktop-merge-tests-serial.log`, `desktop-merge-rendering.log`, `desktop-merge-changenote.json`, and `desktop-merge-version-test.log`.
This merge does not establish macOS or parallel CI success. The existing IPC failure is not hidden or removed.
No PR was merged, package published, version tag created, or release workflow dispatched by this conflict resolution.

### Remaining required evidence

- Normal parallel CI confirmation for this implementation revision. The full serial Windows suite passed; the local parallel baseline IPC failures remain unresolved.
- macOS arm64 tests, Clippy, Metal rendering, and actual before/after captures.
- Intel macOS all-target compile/Clippy evidence.
- Normal CI results from [PR #13](https://github.com/narumiruna/mangameeya-rust/pull/13). WSL GitHub CLI authentication is available; the initial Windows CLI login blocker is resolved. No release workflow was dispatched for this work.

Physical DPI, IME, Intel desktop interaction, and subjective reading comfort remain extra platform review paths. No package was published and no version tag was created.


## External platform and release checks

These checks are not represented as local passes:

- **Windows runtime and installer:** A Windows runner or maintainer with the MSVC toolchain, Windows SDK, and NSIS must run the repository workflow or the documented Cargo and installer test commands. The workflow checks silent installation, all 11 Open With associations, unchanged extension defaults, and uninstall. Local Windows cross-checks prove compilation, not native execution. Do not describe a Windows release as runtime-verified until the native checks pass.
- **Intel macOS runtime:** A maintainer with a supported Intel Mac, full Xcode, and Metal-capable GPU, or a suitable Intel macOS runner, must run the native suite and package checks. An arm64-host cross-check does not prove Intel runtime behavior. Do not describe the Intel build as runtime-tested before these checks pass.
- **Signed distribution:** The release account owner must provide Developer ID signing and notarization for a public macOS release, and an appropriate Windows signing certificate if signed Windows distribution is required. Local macOS packages are ad-hoc signed only; do not describe them as notarized or trusted downloads.
- **Subjective visual acceptance:** A maintainer can review the generated captures and native reading workflow on the intended display and input hardware. Automated geometry and pixel checks do not establish personal usability acceptance. This optional product review is not an agent coding gate.

The project does not require credentials, paid services, or changes to comic sources to build or test locally.
