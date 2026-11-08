# Expandable Files Sidebar

Status: executing on `narumi/feat/expandable-files-tree`, based on `origin/main` at `798ce33`. Baseline formatting, Clippy, and production rendering passed. The baseline full suite had 253 passing tests and three failures in existing IPC tests; logs are under `target/files-tree/baseline-*.log`. Final local evidence: 51 Windows model tests, 26 native Files tests, 32 central-browser tests, all-target Clippy, and production rendering passed. Linux full suite passed 335 tests. Windows full suite has 332 passing tests and the unchanged IPC queue failure; this is not a passing local gate. Release mocks passed 21 tests and HTML checks passed 459 checks. Draft PR #28 is open. Initial head `6b157bc` passed all five jobs in CI run 42520274379. Final review path/focus fixes have local regressions and await their own normal CI evidence before plan retirement.

## Goal

Implement selected design 02, [Expandable folders](../files-sidebar-concepts/02-tree.html), in the native Rust Files sidebar. Provide real folder expansion, readable paths, current-book location, and separate selection and reading actions. Keep the existing reader shell and central cover browser.

## Context

- `src/ui.rs::render_files` and `file_entry` render a flat list with disclosure-like text. Every folder click calls `browse`, which enters central browsing mode.
- `ReaderView` shares `listing`, `listing_revision`, `files`, and `path_input` between the sidebar and central navigation. The path input also calls `browse` directly.
- `open_browser_path` retains central-browser snapshots. Its `from_browser` flag does not distinguish sidebar opens from central-grid opens.
- `src/book.rs::browse_directory` already enumerates one directory and sorts folders first, then names naturally. `DirectoryEntry::entry_path` is the native entry identity; `path` is the canonical read target.
- `src/ui/tests/browser.rs`, `browser_lifecycle.rs`, and `tests/rendering.rs` protect central-browser state, lifecycle, and shell geometry. Reuse these checks rather than replacing their expectations with sidebar behavior.
- The HTML studies passed 459 checks. Those results do not verify Rust behavior. Native Cargo suites were not run while drafting this plan. The available Windows Cargo and Rust compiler report version 1.99.1.

## Decisions and Scope

- Folder names navigate the sidebar only. Disclosure buttons expand or collapse branches without changing the current location or reader. Book clicks select; double-click, Enter, or **Read selection** opens the book.
- Keep central-grid navigation independent. Backspace from the reader or central grid retains its current meaning unless focus is inside the Files sidebar. Sidebar browsing does not pause playback, change reader state, or start cover jobs.
- Load one directory when needed, not an entire drive. Start at the existing initial directory when no book is open. A successful external open can establish the source folder before the user manually navigates the sidebar; later opens must not overwrite a manually chosen location.
- Use one active tree root. Parent navigation within the root selects its parent node; navigation above the root changes the root. At a filesystem root, Parent is disabled. Back restores previously accepted sidebar locations, selections, queries, and scroll anchors.
- **Locate current book** clears the sidebar query, loads and expands the source's ancestor chain, and focuses its row. If the source is outside the tree root, use its containing directory as the new root. Enable the session-only hidden-items view when required to reveal the source, and report that change.
- Search filters **loaded folders only**, including loaded collapsed branches. Label this limit in all locales. Show matching ancestor chains, temporarily expand results, and restore normal expansion after clearing the query. Do not recursively crawl unloaded folders. This is an explicit adaptation of the fully known HTML mock library.
- Hide dot entries by default. On Windows, also classify entries with Hidden or System attributes. The toggle changes only the sidebar, not central-browser filtering or persisted preferences.
- Keep the existing sidebar default and range: 224 logical pixels, resizable from 216 to 320. The prototype's 296 px width is a reference, not a settings change. No new dependencies, preferences, feature flags, recursive search, library database, archive-tree expansion, or drive selector.
- **Read this folder** uses the existing `Book::open` behavior and current recursive-folder preference. Do not combine archive children into one book or pre-scan recursive content only to enable a button.
- Expansion, sidebar navigation, selection, queries, and hidden visibility do not write settings. Existing writes for successful book opens, reading positions, and saved panel preferences retain their current behavior.

## Architecture

- Add a private `src/file_tree.rs` module for tree state, native identities, loaded-directory results, visible-row flattening, filtering, history, and request acceptance. Limit history to 32 metadata snapshots to prevent unbounded growth. Test state transitions without GPUI.
- Add `src/ui/files.rs` for Files controls, virtual rows, focus, and background-directory scheduling. Keep tightly coupled sidebar UI logic together. `ReaderView` owns this state independently of `BrowserUi`.
- Reuse `book::browse_directory` in background work. Classify hidden entries off the UI thread without changing the existing central-browser listing policy. Preserve both native entry paths and canonical targets.
- Use the existing virtual `ListState` over flattened visible rows. Render only viewport rows. Keep disclosure and folder-name targets separate. Do not build an element for every descendant.
- Allow at most two concurrent sidebar directory reads. Track root generation and per-node request identity; obsolete synchronous reads retain their worker slots until completion. Collapse, root replacement, and close invalidate affected pending work. Late results must not reopen a branch or replace a newer location.
- Retain the successful book-open source independently of the consumable `reading_origin`. Distinguish sidebar, central-grid, and external open intent where the current boolean is insufficient. Preserve selected-image identity, aliases, folder-book origin, and central-grid return snapshots.
- Treat a canonical target already present in that branch's ancestor chain as a cycle. Show a non-expandable explanation. Do not merge distinct aliases globally or derive node IDs from lossy display strings.

## Plan

- [x] **Record the native baseline and add isolated fixtures.** Run existing formatting, tests, Clippy, and rendering commands below before implementation. Create temporary nested-folder, image, archive, empty-folder, long-name, alias, and hidden-entry fixtures using the current GPUI test helpers. Record unrelated baseline failures separately; do not waive required checks.
- [x] **Implement and test the tree model in `src/file_tree.rs`.** Add native-path row identities, explicit folder/book selection, expansion, lazy-load states, parent/back history, loaded-only filtering, and scroll anchors. Prove folder-first natural order, separate aliases, branch-local cycle protection, empty results, and restoration after filtering. Acceptance: model tests pass, including 10,000 synthetic siblings with valid flattened indices.
- [x] **Separate sidebar directory loading from central browsing.** Add sidebar-local enumeration, hidden classification, current-path draft, per-node errors, and the two-slot scheduler. Retain the last valid location until a requested navigation succeeds. An invalid path or unreadable branch must not clear the reader, tree, or history. Acceptance: deterministic tests cover out-of-order completions, duplicate requests, collapse/re-expand, root replacement, close, deletion, and retry; unrelated expanded branches stay intact.
- [x] **Replace the flat Files UI with virtual tree controls.** Add fixed icon columns, left-aligned names, depth guides, separate disclosures, loading/error rows with Retry, Back, Parent, Locate, loaded-folder search, hidden-items toggle, and reading actions. Replace the duplicated raw path with one editable location control. Format Windows drive and UNC paths for display without changing native source paths. Show page count in Thumbnails rather than the Files heading. Add all new text and accessibility labels to English, Japanese, and Traditional Chinese catalogs. Acceptance: rendered IDs and states are testable; long names retain full-path tooltips; controls remain reachable at 216 px width.
- [x] **Integrate selection, reading, and source location.** Route sidebar opens through the existing book-loading pipeline without using the central grid's saved location as their origin. Keep current-book and selected-row styles separate; disable Read selection when the selection is filtered out or collapsed. Folder disclosure, name clicks, and single book clicks leave the reader and central-browser state unchanged. Locate supports explicit images, archives, folder books, external opens, and aliases. Acceptance: event tests prove each action, failed-open retention, and correct Backspace return paths; existing central-browser tests still pass.
- [x] **Add scoped keyboard and focus handling.** Use a Files focus context so Up/Down and Home/End move through visible rows, Enter reads or navigates, Right expands or enters folder children, and Left collapses or focuses a parent. Right on files does nothing. Search-expanded branches respect the temporary expansion lock. Backspace and Alt+Up navigate only in non-editable sidebar controls; text inputs retain editing, Enter submission, and Escape cancellation. Restore a stable focus target after navigation, filtering, collapse, tab changes, or sidebar removal. Acceptance: GPUI input-event tests cover these cases without changing reader or central-grid shortcuts.
- [x] **Extend native regression and rendering coverage.** Add `src/ui/tests/files.rs` and register it in `src/ui/tests.rs`. Cover multiple expanded branches, repeated expansion, invalid/Unicode/verbatim/UNC paths, native-name collisions, hidden-current-book reveal, disabled actions, history, and no settings writes during expansion, navigation, selection, query, or hidden-view changes. Test real Windows hidden attributes and directory junctions with temporary fixtures. Extend `tests/rendering.rs` across all three locales, 1280×800, 960×640, and 800×540, minimum/default/maximum sidebar widths, and both central modes. Acceptance: active rows and controls stay reachable; only visible rows render; sibling icons align; reader shell geometry and existing cover budgets remain unchanged.
- [ ] **Run final checks and retain the handoff.** Run every applicable command below; review the changes for lifecycle, source-file safety, focus, and scope defects. Update all three repository READMEs with the new controls and search limit. Record actual results, platform limits, and before/after evidence in `docs/verification.md`; keep captures under `target/`, not tracked image files. Add a minor feature note with the pinned changenote CLI and validate it. Update the prototype README's handoff to record selection of design 02 and native deviations. Delete this plan only after its tasks and completion checks pass and the external handoff is retained.

### Verification Commands

Run Cargo commands with the native Windows toolchain available through PowerShell; do not substitute a Linux build for Windows evidence. Prepare the child process PATH for rustfmt or Clippy if needed. These commands follow `.github/workflows/ci.yml`; new test filters become available after their modules are added.

```text
cargo fmt --all -- --check
cargo test --locked --all-features file_tree::tests::
cargo test --locked --all-features ui::tests::files::
cargo test --locked --all-features ui::tests::browser
cargo test --locked --all-features
cargo test --locked --test rendering
cargo clippy --locked --all-targets --all-features -- -D warnings
changenote status
git diff --check
```

Use changenote v0.10.0 at the revision pinned in `.changenote/README.md`. Do not change product versions, publish packages, dispatch workflows, or alter unrelated plans. A diagnostic serial test run does not replace the normal full-suite result.

## Completion Checklist

- [x] Disclosures expand real folders; folder navigation and single selection preserve the reader and central-browser snapshot.
- [x] Book/folder reading, explicit-image location, alias identity, Locate, Parent, and Back pass native event tests.
- [x] Loaded-only search, hidden filtering, path editing, errors/retry, and keyboard/focus rules pass the new tests.
- [x] Directory work is lazy, limited to two active jobs, cycle-safe, and protected from stale results. The 10,000-entry virtual-row check passes without an eager descendant scan.
- [x] Existing central-browser, reader, desktop, i18n, settings, and lifecycle regressions pass; temporary fixtures leave source data and user settings unchanged.
- [ ] Local formatting, full tests, rendering, Clippy, note validation, and diff checks have recorded results. Any baseline failure or unavailable required check has an explicit disposition, not a silent exemption.
- [x] Native geometry evidence and localized controls cover the specified sizes and widths. Product acceptance and unverified platform behavior are not reported as automated test results.
- [ ] Durable verification and external handoff are in `docs/verification.md`; the completed plan is removed only after the preceding evidence is valid.

## External Handoff

- **Product maintainer:** After native event/geometry results and available before/after captures are ready, review design 02 for hierarchy clarity, reading comfort, and the loaded-only search wording. Expected result: a design assessment. Subjective usability and assistive-technology behavior remain unverified by automated geometry checks. This review is not an additional agent completion gate.
- **Platform maintainer:** With native macOS arm64 and Intel macOS environments, run the existing required CI checks and inspect platform path, keyboard, and filesystem behavior. Expected result: platform-specific test evidence. Local Windows and native WSL Linux checks now exist; they do not establish macOS or physical desktop/assistive-technology behavior. The Windows local IPC baseline failure remains disclosed. Normal required CI evidence is needed before the final gate is checked. Existing merge/release gates are not waived; do not claim unexecuted platform pixels from HTML screenshots.

## Rollback / Recovery

Revert only the tree implementation, its tests/docs, and unreleased feature note. Keep the earlier HTML proposals as design references. Tree expansion, history, search, and hidden visibility remain session-only, so no settings or comic-file migration is required. Do not reset user reading data or change central-browser settings to recover.
