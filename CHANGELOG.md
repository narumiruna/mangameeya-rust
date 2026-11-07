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

