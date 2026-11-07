---
mangameeya-rust: minor
---

Add a grouped toolbar with direct Single/Spread/Continuous selection that preserves fit and custom zoom. Add independently resizable side panels with saved widths and visibility, and Image, Reading, and Playback tool tabs. Keep page navigation, reading progress, and page information at the bottom. Move subfolder inclusion to File → Open options for the next open only.

Before downgrading, back up `settings.json`. For builds with language support but without panel layout support, remove only `preferences.panel_layout` from the working copy. For older builds without language support, also remove `preferences.language`. Keep reading data and all other preferences.
