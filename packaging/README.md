# Native packages

These scripts use the root Cargo application. They do not use Node.js, Tauri, or a webview.
The Rust toolchain is pinned in `rust-toolchain.toml`.

## Windows

Required tools: Rust MSVC, Microsoft C++ Build Tools with the Windows SDK, and NSIS 3.
The Windows SDK must include `fxc.exe` for GPUI's release shaders.

```powershell
choco install nsis --yes --no-progress
pwsh -NoProfile -NonInteractive -File packaging/windows/build.ps1
```

The script builds `target/release/mangameeya-rust.exe` and creates
`target/release/bundle/nsis/MangaMeeya-Rust_<version>_windows-setup.exe`.
`build.rs` embeds the original application icon. GPUI supplies the application
manifest with `asInvoker` and PerMonitorV2 DPI support.

The installer:

- Runs without elevation and installs to `%LOCALAPPDATA%\Programs\MangaMeeya Rust` by default.
- Creates a current-user Start menu shortcut and uninstall registration.
- Registers JPG, JPEG, JPE, PNG, GIF, BMP, WebP, ZIP, CBZ, RAR, and CBR under HKCU.
  - Uses `OpenWithProgids`, application capabilities, and a quoted file-open command.
  - Does not change extension defaults or `UserChoice`.
- Removes only its own files, shortcut, and registry entries on uninstall.
  - Keeps the user's preferences and reading history.

Close the reader before installation, upgrade, or uninstall. Both setup and uninstall
support `/S`. To test the installer on a clean Windows CI user profile:

```powershell
pwsh -NoProfile -NonInteractive -File packaging/windows/test-installer.ps1 -InstallerPath <setup.exe>
```

The test checks silent install, shortcuts, all 11 associations, unchanged extension
defaults, and uninstall. It does not start the GPUI window.

## macOS

Required tools: the pinned Rust toolchain and full Xcode with its Metal compiler.
Command Line Tools alone are not sufficient. Packages require macOS 12 or later.

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
bash packaging/macos/build.sh
```

The script builds for the host architecture. For another architecture, install its
Rust target and pass `aarch64-apple-darwin` or `x86_64-apple-darwin` as the first argument.

Output is under `target/release/bundle/macos/<target>/`:

- `MangaMeeya Rust.app`
- `MangaMeeya-Rust_<version>_<target>.zip`
- `MangaMeeya-Rust_<version>_<target>.dmg`

The app includes native document associations for the same 11 extensions, with
`Viewer` role and `Alternate` rank. The script generates `icon.icns` from the original
`assets/icon-256.png`. The DMG includes an Applications shortcut.

Packages have only a local ad-hoc signature. They have no Developer ID signature or
notarization. Gatekeeper can block downloaded packages. The packaging script verifies
bundle metadata, architecture, signature integrity, and the DMG image. It does not
start the packaged application.

## Cross-check Windows code from macOS

This checks Rust code, including Windows-only paths. It does not link or run the
Windows application. Native Windows CI is still required for executable resources,
release shaders, and installer tests. GPUI's manifest must remain enabled.

```sh
brew install llvm
rustup target add x86_64-pc-windows-msvc
PATH="$(brew --prefix llvm)/bin:$PATH" \
CC_x86_64_pc_windows_msvc=clang \
RC_x86_64_pc_windows_msvc="$(brew --prefix llvm)/bin/llvm-rc" \
cargo check --locked --all-targets --target x86_64-pc-windows-msvc
```

## Launch IPC

`src/instance.rs` holds a per-user `fs2` lock and listens only on an ephemeral IPv4
loopback port. A 256-bit OS-generated token authenticates launch requests. The endpoint
file is private on Unix; Windows inherits the user's profile ACL. Tokens are not logged.
Before forwarding on Windows, the second process requests foreground activation
permission for the recorded primary PID. Windows can deny that permission; the
launch paths are still forwarded.

- The UI retains `PrimaryInstance` and polls `try_recv()`.
  - The first message contains the primary launch paths.
  - Every message activates the window; an empty list only activates it.
- Launch paths are made absolute in the sending process. Native path bytes are preserved.
- Messages contain only paths, not executable commands.
- Each launch permits at most 128 paths and 256 KiB of JSON.
- The receive queue permits at most 32 launch messages. Overflow is rejected.
- Each inbound connection has a total 500 ms read deadline. Startup retries for five seconds.
- Drop closes the listener and removes its endpoint before releasing the lock.
  - The empty lock file remains to prevent a race between different lock-file inodes.
  - A new primary replaces stale endpoint data after an unclean exit.

macOS LaunchServices can deliver documents and reopen events to an existing process
without starting another process. The UI handles these native events separately.

## CI

`.github/workflows/windows.yml` checks Rust formatting, tests, and Clippy on Windows
x64, macOS ARM64, and macOS x86_64. It builds native packages and uploads a separate
artifact for each platform. Windows also runs the silent installer test. macOS jobs
set `DEVELOPER_DIR` to full Xcode and check the Metal compiler before Cargo builds.
All platforms run `cargo test --locked --test rendering` against the production UI
with the native text system. macOS also uses the Metal renderer and uploads
`target/ui-smoke/*.png` as separate artifacts, including screenshots available after
a failed test.
