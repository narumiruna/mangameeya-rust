---
mangameeya-rust: minor
---

Add notifications for newer stable releases, with one check per application launch, a dismissible reader banner, and a separate Preferences section for the installed version, check status, and manual checks. Help and native app-menu checks open Preferences. Startup checks are enabled by default and can be disabled without disabling manual checks. Network errors do not interrupt reading.

Store the startup preference separately in `release-notifications.json`; book data and the `settings.json` schema are unchanged. Open the fixed GitHub Releases page only at the user's request for manual download and installation. The application never downloads or installs release payloads; the first version with notifications also requires manual installation. Package signing is unchanged, and updater SDK research remains deferred.
