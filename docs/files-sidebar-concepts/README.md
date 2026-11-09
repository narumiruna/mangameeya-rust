# Files Sidebar Concepts

Open [index.html](index.html) in a desktop browser. No server, build, or package installation is required to use the prototypes. Keep the HTML, CSS, JavaScript, and SVG files in this directory together.

## Designs

1. [Compact list](01-compact.html): an alternative flat-list design. Use a left-aligned flat list, fixed icon positions, one editable location field, and separate selection and reading actions. Deep folders still require navigation.
2. [Expandable folders](02-tree.html): the selected native design. Use real folder disclosures to inspect several series without leaving the current location. Nested rows have less space for long names.
3. [Reading first](03-reading-first.html): keep the current book, progress, and locate action above nearby volumes. Show the current and following volumes before earlier volumes and subfolders. The book card reduces the height available to the file list.

All three keep the existing desktop window structure: menus, toolbar, left navigation, central reader, right image tools, and bottom status bar. The example library starts at `D:\Comics\夜行列車` with volume 03 selected and being read. The UI uses Traditional Chinese to match the reviewed application.

## Shared interaction rules

- A single click on a book selects it. A double-click, Enter on its row, or **閱讀選取** starts reading.
- A click on a folder name changes the sidebar location. It does not change the central reader.
  - This separates sidebar navigation from the previous Rust behavior, where folder navigation entered the central browser.
  - In design 02, a triangle only expands or collapses a branch. The folder name changes the current location.
  - Search expands matching branches and disables their disclosure buttons until the search is cleared.
- **定位書籍** restores the current book's folder, clears the search, expands its ancestors, and focuses its row.
- **返回上一個瀏覽位置** follows sidebar history. **上一層** follows the directory hierarchy.
  - Backspace or Alt+Up in a non-editable Files control also moves to the parent.
  - Backspace in an input edits text; it does not navigate.
  - Folder history retains the last query, selection, and list scroll position for each folder during the page session.
- The location field displays one readable path. Click it to edit the full path.
  - Enter opens a known mock directory. An invalid path shows an inline error and leaves the location and reader unchanged.
  - Escape or the cancel button cancels editing.
  - The parser accepts ordinary Windows paths, forward slashes, and a Windows verbatim prefix. The UI does not display the internal prefix.
- **隱藏項目** controls the example `.metadata` directory. Hidden entries are off by default.
- **閱讀此資料夾** treats the current folder's direct readable files as one mock book. Empty folders disable both reading actions.
- A current-book marker and the selected-row style are separate. A filtered or collapsed selection cannot be read with the selection button.
- Up/Down and Home/End move focus between visible file rows. Enter reads the focused book.
  - In design 02, Right expands a folder or moves into its children. Left collapses it or focuses its parent.
- Left/Right and Home/End change navigation tabs. The thumbnail tab shows the current book's page count and sample jump targets. Recent books resume their saved mock page.
- Bottom page buttons change the mock page and stop at the first and last page. The toolbar button shows or hides the sidebar.
- Changes remain in page memory only. Reloading a page or switching design pages resets the demo.

## Visual and implementation scope

- At desktop widths, the three sidebars use 264, 296, and 296 CSS pixels. File icons share a fixed column. Long names truncate; their tooltips retain the full path.
- At desktop heights of 950 px or less, design 03 uses a smaller book card and hides duplicate status and quick-place controls. The current and next volume remain visible at the checked 1280 × 850 size.
- At 900 px or less, the right tools are hidden. At 700 px or less, the preview shows the sidebar instead of the central reader. Hide the sidebar to inspect the reader.
  - These rules protect the HTML comparison. They are not a proposal for native mobile support.
- The top menus, static reading-mode labels, and right image tools are visual references only. They are not presented as working HTML controls.
- All books and paths are mock data. All visible images use the original local `reader-art.svg`, including thumbnail samples. There are no external fonts, scripts, packages, images, network requests, or persistent storage in the prototypes.
- These files do not modify Rust code, read real files, decode archives, or measure large-library performance. They are separate from the [previous central-browser concepts](../file-browser-concepts/index.html).

## Verification

The check script tests local links, offline resource loading, navigation, selection versus reading, path editing, hidden entries, empty directories, keyboard controls, and responsive bounds. It also captures desktop, selected-row, narrow, and comparison screenshots in a temporary directory.

```bash
uv run --no-project --with playwright playwright install chromium
uv run --no-project --with playwright python docs/files-sidebar-concepts/verify.py
```

Skip the installation command if the matching Chromium build is already installed. Use `--output-dir <path>` to choose the screenshot and report directory.

The final run passed **459 checks** in **Chromium 156.0.8078.4** with offline mode enabled:

- All three prototypes passed at 1600 × 1050, 1280 × 900, 1280 × 850, 900 × 1000, 700 × 900, and 390 × 950.
- The comparison page passed horizontal bounds checks at 1600, 900, and 390 px widths.
- No prototype page errors, console errors, failed resource loads, or external HTTP requests were recorded.
- Current-book rows remained visible at each checked size. Design 03 also kept the next volume visible. The visibility check allows a 2% tolerance for subpixel scroll rounding.
- Regression checks cover folder-navigation focus, Right Arrow on tree file rows, search-expanded tree keyboard behavior, and folder history restoration.
- Desktop, short-desktop, selected-row, narrow, and comparison screenshots are written outside the repository. The desktop variants and comparison page received visual inspection.

A successful script does not prove native behavior or full accessibility conformance. The report and images are generated evidence for the HTML prototypes only.

## Design handoff

**Design 02, Expandable folders, is selected for native implementation.** Folder names navigate only the sidebar; disclosures expand branches without changing the reader or central-browser location. The existing central Backspace workflow remains independent.

Native adaptations differ from the HTML study:

- Native search filters loaded folders only, including loaded collapsed branches. It does not scan the disk or load unvisited folders. The HTML search uses the complete, already-known mock dataset.
- The native sidebar retains its 224 logical px default and 216–320 range; the 296 CSS px prototype is only a design reference. Native Back history retains up to 32 previous locations.
- Native hidden-item filtering covers dot-prefixed entries and Windows Hidden/System attributes. The session-only toggle does not change central-browser filtering. Locate can reveal hidden items with a notification.
- Native paths retain their original filesystem identity while displaying ordinary drive and UNC paths. Folder reading uses the existing recursive-folder preference, not the mock's direct-files-only rule.

The 459 HTML checks above, including responsive bounds and screenshots, remain HTML-only evidence. They do not establish native test results. The Rust implementation is in `src/file_tree.rs` and `src/ui/files.rs`. Native model, input-event, and geometry checks are recorded separately; physical desktop behavior, full accessibility conformance, and unexecuted platform paths remain outside the HTML evidence. See [the native verification record](../verification.md) for actual results and remaining platform limits. No native release claim is made by these prototypes.
