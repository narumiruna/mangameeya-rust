## 0.5.0

### Minor Changes

- Open Preferences in one independent, nonmodal window while keeping image and playback tools in the reader. Changes apply immediately and save automatically.

## 0.4.1

### Patch Changes

- Restore reader icons and align desktop controls, panels, typography, and page navigation with the desktop design.

## 0.4.0

### Minor Changes

- Add a grouped toolbar with direct Single/Spread/Continuous selection that preserves fit and custom zoom. Add independently resizable side panels with saved widths and visibility, and Image, Reading, and Playback tool tabs. Keep page navigation, reading progress, and page information at the bottom. Move subfolder inclusion to File → Open options for the next open only.
  
  Before downgrading, back up `settings.json`. For builds with language support but without panel layout support, remove only `preferences.panel_layout` from the working copy. For older builds without language support, also remove `preferences.language`. Keep reading data and all other preferences.

## 0.3.0

### Minor Changes

- Add English, Japanese, and Traditional Chinese reader interfaces with an immediately applied, saved language choice in Preferences. Localize menus, controls, accessibility labels, and reader messages without changing comic files or reading data.

### Patch Changes

- Keep single-page and spread scrolling within the page bounds. Fitted pages no longer scroll into empty space, and zoomed pages remain scrollable.

- Retry macOS DMG verification when a newly created image is temporarily unavailable. Limit verification to three attempts, preserve other failures, and keep package integrity checks required.

- Combine fit selection and manual zoom in one toolbar control and popover. Show the current page's effective percentage in every fit mode. Start manual zoom from the displayed scale instead of jumping from 100%, and keep keyboard shortcuts and input focus behavior consistent.

## 0.2.1

### Patch Changes

- Publish Windows x64 installers and Apple Silicon / Intel macOS packages through verified GitHub Releases. Keep the About dialog version in sync with the Cargo package version. Windows installers are unsigned; macOS packages have only an ad-hoc signature and are not notarized.

