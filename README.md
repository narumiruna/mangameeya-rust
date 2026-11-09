# MangaMeeya Rust

[English](README.md) | [正體中文](README.tw.md) | [日本語](README.ja.md)

An offline comic reader for **Windows and macOS**, rewritten in Rust with [GPUI Kit](https://github.com/longbridge/gpui-kit).
The GPU draws the interface directly. The application does not use Tauri, WebView, TypeScript, or Node.js.

## Features

- Open images, image folders, ZIP/CBZ, and RAR/CBR, with subfolder support and natural sorting.
- English, Japanese, and Traditional Chinese interfaces. Change the language in Preferences; changes apply immediately and save automatically.
- Single-page, two-page spread, and vertical continuous layouts, with right-to-left or left-to-right reading.
- Fit to window, width, or height, original size, and 10%–800% zoom.
- EXIF orientation, 90-degree rotation, brightness, contrast, grayscale, and animated images.
- Split tall images into GPU textures without changing their original dimensions or resolution.
- A virtualized thumbnail strip, file browser, recent files, per-book reading positions, and bookmarks.
- Native open and save dialogs, keyboard controls, mouse wheel, page turns at either edge, and drag and drop across the window.
- Fullscreen, timed slideshows, and saving original images.
- One primary reader window and a separate Preferences window. Subsequent launches send new paths to the existing reader, even when Preferences is active. macOS also supports document-open events from Finder.
- Image and comic file associations in the Windows installer and macOS app bundle.

Source images are not changed. The save operation refuses to overwrite source files in the current book. This protection also covers symbolic links and hard links.
Encrypted RAR files show an error; the application does not request or store passwords.
Each page is limited to 128 MiB of source data and 32 megapixels when decoded. Animations also have limits of 256 frames and 128 MiB of total decoded data.
Rust loads image data directly, without Base64 or WebView IPC transfers.
Images load in the background. Images outside the visible area are released from the CPU cache and GPU atlas. Continuous layouts retain image dimensions to keep the scroll position stable.

## Run

Install [Rust](https://rustup.rs/). `rust-toolchain.toml` pins Rust 1.99.1. `Cargo.lock` pins GPUI Kit and its GPUI snapshot.

### Windows

Requires Windows 10 or 11, a DirectX 11-compatible GPU, and Visual Studio Build Tools with the **Desktop development with C++** workload and Windows SDK.

```powershell
cargo run --locked
cargo run --locked -- "C:\Comics\book.cbz"
```

WebView2 Runtime and Node.js are not required.

### macOS

Supports Apple Silicon and Intel Macs with macOS 12 or later. Building from source requires full Xcode with the Metal compiler, not only Command Line Tools.
If Xcode has not completed its first launch or Metal component installation, complete setup in Xcode first. The commands below do not accept licenses or open installation prompts on your behalf.

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
xcrun --sdk macosx metal --version
cargo run --locked
cargo run --locked -- /path/to/book.cbz
```

## Controls

| Action | Keys |
| --- | --- |
| Open comic / folder | `Ctrl+O` / `Ctrl+Shift+O` |
| Save original image / close comic | `Ctrl+S` / `Ctrl+W` |
| Browse files / go to parent folder | `Backspace` |
| Open selected file or folder | `Enter`, double-click |
| Refresh the file browser | `F5` |
| Next page / previous page | `Space`, `PageDown` / `PageUp` |
| Turn pages in the reading direction | `←`, `→` |
| Scroll the reading area | `↑`, `↓`, mouse wheel |
| First page / last page | `Home` / `End` |
| Single page / spread, continuous pages, reading direction | `S`, `C`, `D` |
| Fit window / width / height / original size | `F` / `W` / `H` / `1` |
| Zoom in / zoom out / 100% | `+` / `-` / `0` |
| Rotate | `[` / `]` |
| Set bookmark / go to bookmark | `B` / `Shift+B` |
| Slideshow / thumbnails / sidebar | `P` / `T` / `Tab` |
| Fullscreen / exit fullscreen | `F11` / `Escape` |

macOS also supports `Command+O`, `Command+Shift+O`, `Command+S`, `Command+W`, `Command+Q`, `Command+,`, and `Control+Command+F`.
The mouse wheel scrolls without directly turning pages. Hold `Ctrl` while using the wheel to zoom. In single-page and two-page layouts, click the left or right edge of the reading area to turn pages.

### Bookmarks

Each book has one bookmark. `B` or the toolbar button sets or updates it. `Shift+B` or **Bookmarks → Go to bookmark** returns to it. **Bookmarks → Delete bookmark** removes it.

New folder bookmarks track the source image, not only its page number. They keep the same image after **Include subfolders** or natural page order changes. If the image is missing or excluded when you reopen the folder, the reader shows an error and keeps the bookmark. It does not select another image. Restore the image or subfolder setting and reopen the folder to use it again. Renamed or moved images need a new bookmark. Archive bookmarks keep their existing page-number behavior.

Old bookmarks remain readable. They keep numeric behavior until you set them again, because old settings do not identify the source image.

### Files sidebar

The **File** tab has an expandable folder tree, independent of the central browser. Click a disclosure arrow to expand or collapse a folder; click its name to navigate the sidebar. A single book click selects it without changing the reader. Double-click a book, press `Enter` on its row, or use **Read selection** to read it. A selection hidden by filtering or collapse cannot be read with that button. **Read this folder** opens the current folder's images using the existing **Include subfolders** preference; it does not combine child archives into one book.

- The tree has one root. **Parent** selects the parent within the tree or changes the root when moving above it. At a filesystem root, Parent is disabled. **Back** restores the previous sidebar location, selection, search, and scroll position, for up to 32 previous locations.
- **Locate current book** clears the search, loads and expands the source's ancestors, and focuses its row. For a source outside the tree, its containing folder becomes the root. If hidden items are needed, Locate shows them and reports the change.
- **Search loaded folders** filters only folders already loaded, including collapsed branches. It does not scan the disk or load unvisited folders. Matching ancestor chains expand temporarily; clearing the search restores normal expansion.
- **Show hidden items** includes dot-prefixed entries and, on Windows, entries with Hidden or System attributes. This toggle affects only the sidebar and lasts only for the session. Expansion, navigation, selection, and search are also session-only; central-browser filtering is unchanged.
- The location field shows an ordinary folder path, including Windows drive and UNC paths without an internal verbatim prefix. Edit the path and press `Enter` to navigate; `Escape` cancels changes. Invalid paths keep the previous location and reader. Long-name tooltips show full paths.

With focus in the Files tree, `↑`/`↓` and `Home`/`End` move between visible rows. `Enter` reads a book or navigates a folder. `→` expands a folder or enters its children; it does nothing on files. `←` collapses a folder or focuses its parent. Search-expanded branches stay expanded until the query is cleared. `Backspace` and `Alt+↑` move to the parent only in non-editable Files controls. Inputs keep normal text editing. These shortcuts do not change reader or central-browser shortcuts. Sidebar navigation does not pause the slideshow or replace the central canvas; the central `Backspace` workflow below is unchanged.

### File browser

Press `Backspace` while reading to replace only the central canvas with a black cover grid. The toolbar, side panels, and status bar stay in place. Select a file, then press `Enter` or double-click it to read. Folders open in the browser; **File → Read this folder** reads the central folder's images. `Escape` closes a popup first, then returns to the current book before it exits fullscreen. Search inputs keep their normal text-editing keys.

The browser keeps the query, sort, selection, and scroll position when you return from reading. Reading retains the page, zoom, filters, and scroll position. Browsing pauses the slideshow and disables reading controls. The **Browser** tools tab controls cover size, file names, and metadata. These browser preferences are not saved between launches.

Sort by natural or literal name, modified or created time, size, type, known page count, known reading progress, recent-open order, or a fixed random order. Folder grouping is independent of sort direction; unknown values stay last. Counts come only from opened books or visible cover reads. Sorting does not scan all archives. Recent-open order is not a last-read timestamp.

The default grid shows covers and two-line file names; metadata is off. Search, sorting, folder grouping, and the main open action share a compact header. Shuffle appears only with random sorting. Use the top reading button or `Escape` to return to the retained book.

Covers distinguish queued, loading, unsupported, and failed sources. Full names, display paths, and failure reasons remain in tooltips. Only visible and nearby rows request covers; visible items take priority, with at most two actual jobs and a 32 MiB retained-pixel cache. This is not a total process-memory limit. Dense high-DPI grids reduce cover resolution to fit that cache. Folder covers use the first image at the top level. Automatic ZIP/CBZ covers support classic single-volume Stored/Deflated archives, including files larger than 64 MiB. Reads use a bounded footer and index, at most 4096 entries and 2 MiB combined metadata, and at most 64 MiB for the selected compressed member. The existing decoded-image and 128 MiB output limits remain. RAR/CBR, ZIP64, unsupported layouts, and over-limit members show a reason instead of a cover. Manual book opening is unchanged. Press `F5` to retry and discard stale covers and counts. Source comics are never changed.

### Toolbar and panels

The toolbar groups file commands, panels, reading modes, zoom, reading direction, bookmarks, reading tools, and fullscreen.
The **Single page / Two pages / Continuous** buttons select a mode directly. Selecting the current mode again does not change it.
Mode changes keep the current fit mode or custom zoom. `S` and `C` keep their existing toggle behavior.
Fullscreen uses the existing commands and shortcuts. This update does not add a focus mode, theme switch, or dependency.
The interface continues to use GPUI Kit, ActiveTheme, and Lucide icons.

You can hide each side panel independently and drag its edge to change its width. Panel visibility and preferred widths are saved and restored.

- The left panel keeps the **File / Thumbnails / Recent** tabs. Thumbnails keep their aspect ratio. Page numbers and file names are separate; tooltips show full file names.
- The right **Reading tools** panel has three tabs:
  - **Browser**: cover size, file-name display and line count, and metadata display.
  - **Image**: brightness, contrast, grayscale, rotation, and **Reset filters**. Reset filters changes only brightness, contrast, and grayscale, not rotation or reading preferences.
  - **Playback**: slideshow playback and a 1–120 second interval.

Changing tool tabs does not reset values or unsubmitted input. The selected tool tab is not saved between launches.
**File → Open options** contains **Include subfolders**, using the existing `recursive_folder` preference.
Changes apply the next time you open a comic; they do not reload the current book.

### Preferences window

Open **Preferences** from the toolbar, **Tools → Preferences**, or the native Preferences entry. On macOS, `Command+,` also opens it.
Only one Preferences window can be open. Repeated requests activate that window.
It is a separate, resizable, nonmodal normal window: you can continue to use the reader while it is open.
The initial size is 560 × 520 logical px, with a 440 × 360 minimum and vertical scrolling. Its size, position, and open state are not saved.
Opening or closing it does not change reader panel visibility, widths, the selected tool tab, or the open book and reading state.

- **Reading**: reading-position memory, reading direction, and Black, Dark gray, or Paper backgrounds. The background applies only to the reading area. Direction changes in either window update both controls; selecting the current direction does not change it.
- **General settings**: language selection and corrupt-settings backup/reset.

Changes apply immediately and use the existing automatic save. There are no Save or Apply buttons. Save errors appear in Preferences.
While Preferences has focus, `Escape` or `Ctrl+W` closes only that window; macOS `Command+W` and Windows `Alt+F4` do the same. The book stays open, and the application keeps running.
Reader navigation and zoom shortcuts do not act through Preferences. **File → Close comic** still closes the book, not Preferences.
Closing the reader or choosing **Quit** closes both windows and exits the application. macOS `Command+Q` also quits.

### Zoom and page navigation

The toolbar keeps one **Zoom** control for the current mode and actual percentage. Click it to open a popover.
Select fit to window, width, height, or original size; enter a percentage; or adjust with `−` / `+`.

- The percentage refers to the current page. Pages of different sizes in a spread can have different percentages. The control shows `—` until image dimensions are available.
- Manual input, zoom in/out, and `Ctrl`+wheel switch to **Custom zoom**. Zoom in/out and the wheel use the current actual percentage; manual input applies the specified percentage. Manual zoom is limited to 10%–800%; automatic fit modes are not limited to this range.
- After you enter a percentage, press `Enter` to apply it and close the popover. Selecting a fit mode also closes it; `−` / `+` allow repeated adjustments. Press `Escape` or click outside to close it without applying unsubmitted input.

Page navigation appears only at the bottom: previous page, page number, total pages, and next page.
The toolbar does not repeat these controls. The bottom bar also shows reading progress, page information, necessary status, and visible errors.
Zoom and panel switches remain in the toolbar.

The page-information popover shows the current page's full file name, dimensions, and available source size.
Missing information shows **Not available**. Page changes update the information; closing or replacing the book closes the popover and clears old data.
Reading progress is the current page number divided by the total, rounded to a percentage.
Without an open comic, progress shows an empty state and page navigation is disabled.

Page numbers apply only when you press `Enter`. The slideshow interval applies when you press `Enter` or leave the field.

## Language

Under **Preferences → General settings → Language**, select
**English**, **日本語**, or **正體中文**.
The choice applies immediately and is saved in `settings.json` as `preferences.language`
(`en`, `ja`, or `zh-Hant`). English is the default, including for older settings without this field.
Both windows update immediately. Menus, controls, tooltips, accessibility labels, and reader messages use the selected language.
File names and technical error details are unchanged. Native file dialog controls use the OS language.
GPUI component text uses the matching locale; component strings not supplied by GPUI, including Japanese text-editing menus, fall back to English.

## Local data

Preferences, reading positions, recent files, and bookmarks are stored in the user's `settings.json`. Saves use a temporary file and atomic replacement.
The independent Preferences window does not change the settings schema. Existing settings need no conversion.

- Windows: `%APPDATA%\MangaMeeya\MangaMeeya Rust\config\settings.json`
- macOS: `~/Library/Application Support/org.MangaMeeya.MangaMeeya-Rust/settings.json`

`directories::ProjectDirs` determines the actual path.
If the settings file is corrupt, the application keeps the original and shows an error.
Under **Preferences → General settings**, select **Back up and reset settings** to save a `settings.json.corrupt-*` backup before resetting.
Local storage from the older WebView version is not read, deleted, or migrated automatically.
The single-instance lock and authenticated loopback endpoint are also in this directory. The endpoint is removed when the application closes.

### Bookmark metadata and downgrade

The `bookmarks` field keeps its existing numeric format. New folder bookmarks also save optional `bookmark_images` metadata. It contains the saved index and the lossless native image path. The field is omitted when empty. Existing settings load without conversion.

Before using an older build, close the application and back up `settings.json`. From a working copy, remove only the top-level `bookmark_images` field if that build does not support it. Keep `bookmarks`, `positions`, `recent`, and all preferences. Numeric bookmarks remain usable, but no longer track image identity. Do not reset settings.

### Panel settings and downgrade

Panel preferences are stored in `preferences.panel_layout`. Widths use integer logical pixels:

| Field | Default | Rule |
| --- | --- | --- |
| `sidebar_width` | `224` | Limited to 216–320 logical px |
| `inspector_width` | `256` | Limited to 240–360 logical px |
| `inspector_visible` | `true` | Right reading-tools panel visibility |

Left-panel visibility still uses `preferences.sidebar`; there is no duplicate field.
Missing `panel_layout` or child fields in older settings use defaults.
A completed drag saves a width only if the visible width changes.
Narrow windows limit the right panel first, then the left panel, and keep at least 280 logical px for reading.
These limits do not overwrite preferred widths. Enlarging the window restores them.

Older builds reject the new fields. To downgrade:

1. Close the application and back up `settings.json`.
2. For builds with language support but without panel-layout support, remove only `preferences.panel_layout` from a working copy. Keep `preferences.language` and other fields.
3. Only for builds that also predate language support, remove `preferences.language` as well.

Keep recent files, reading positions, bookmarks, and other preferences. Do not reset reading data to downgrade.

## Checks

On macOS, set `DEVELOPER_DIR` as shown above before running these commands.

```sh
cargo fmt --all -- --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test rendering
```

The regular tests cover archives, sorting, safety limits, export, settings, EXIF, animation, reader state, launch IPC, and GPUI control interactions.
The `rendering` test uses the production `ReaderView` with native keyboard and mouse events. On macOS, it checks pixels with the real Metal renderer and writes images to `target/ui-smoke/`. Screen recording permission is not required.
On Windows, the same test checks interactions with the native text system. It does not claim Metal or offscreen GPU rendering coverage.

## Packaging

### Windows NSIS

Install NSIS in addition to the build tools, then run:

```powershell
pwsh -NoProfile -NonInteractive -File packaging/windows/build.ps1
```

Output: `target/release/bundle/nsis/*-setup.exe`.
Installation writes only to the current user's directory and HKCU. File associations are added to **Open With** without changing existing defaults. Uninstall keeps reading data.

### macOS app, ZIP, and DMG

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  bash packaging/macos/build.sh
# Install the target before requesting an Intel build:
rustup target add x86_64-apple-darwin
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  bash packaging/macos/build.sh x86_64-apple-darwin
```

Output: `target/release/bundle/macos/<target>/`.
Packages include an `.app`, ZIP, and DMG, with ad-hoc signing to verify local integrity. They do not have Developer ID signing or notarization. The account holder must sign and notarize them before public distribution.

## CI and verification scope

`.github/workflows/ci.yml` checks Rust formatting, tests, production UI interactions, and Clippy on Linux x64, Windows x64, macOS arm64, and macOS x86_64.
Linux uses headless layout and the native text system; it does not test GPU rendering or produce a package.
Windows and macOS also build and upload native packages. macOS verifies Metal rendering.
The Windows job also tests silent install/uninstall, all 11 file associations, and preservation of default associations.
Use actual workflow results to determine installation test status for each platform. A local cross-check is not Windows execution or installation verification.
See the [verification record](docs/verification.md) for full results and platform checks that have not yet run.

Releases use [changenote](https://github.com/narumiruna/changenote) and `.github/workflows/release.yml`.
Add a [release note](.changenote/README.md) before merging changes. After the generated Release PR is merged, the workflow verifies the specified commit, creates a GitHub Release, and attaches native packages for all three platforms.
Before enabling this workflow, allow Actions to create PRs. See the [release workflow](packaging/README.md#release-workflow) for operation, retries, and signing limits.

## Structure

- `src/book.rs`: images, folders, ZIP/RAR, directory browsing, and safe export.
- `src/reader.rs`: reader state, navigation, and layout calculations.
- `src/imaging.rs`: bounded decoding, EXIF, animation, and pixel processing.
- `src/settings.rs`: user settings, atomic saves, and protection for corrupt data.
- `src/i18n.rs`: complete English, Japanese, and Traditional Chinese interface translations with safe parameter substitution.
- `src/instance.rs`: single-instance control and authenticated launch forwarding.
- `src/ui.rs`, `src/ui/tests.rs`: GPUI Kit windows, background loading, texture tiles, and interaction tests.
- `packaging/`: native packages for both operating systems; `tests/`: archive fixtures and native rendering tests.

## License

This project is licensed under the [GNU Affero General Public License v3.0](LICENSE).
GPUI Kit, GPUI, and Lucide icons keep their respective licenses. Rights to the MangaMeeya name and original application belong to their respective owners.
