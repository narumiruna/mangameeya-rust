# Production picker gate

## Result

Verified on native Windows with GPUI Kit 0.9.0, GPUI Pre 0.3.11, release profile, and scale factor 1.25. The executable opens two independent Kit Root windows and exits after pointer dispatch and assertions. No manual input is required.

The root package enables `test-support` through a dev-dependency, including for examples. This independent package builds the same fixture without that feature. Its lockfile makes this additional check repeatable. No test-support API is used by either build.

Run from the repository root in WSL:

```sh
cargo.exe build --release --locked --example developer_probe
cargo.exe tree --locked --manifest-path examples/support/developer_probe-production/Cargo.toml -e features > target/developer-probe-production-features.log
cargo.exe build --release --locked --manifest-path examples/support/developer_probe-production/Cargo.toml --target-dir target
rm -f target/developer-probe-runtime.log
timeout 30s target/release/developer-probe-production.exe
```

Expected result:

- Process exit status is zero.
- `target/developer-probe-runtime.log` ends with `PASS all assertions`.
- The feature tree contains no `test-support` feature.
- Each window reports 40 intercepted events, no normal action during picking, and 9 normal action callbacks in the positive control.

The fixture uses production `Window::dispatch_event`, not a test window, test input helper, or Windows message injection. This verifies GPUI event dispatch in native windows. It does not verify the OS conversion of physical mouse input, touch, pen input, or other scale factors.

## Verified cases

- Stable scoped semantic IDs, safe labels, roles, static source hints, per-window identity, view revisions, logical bounds, content masks, and declared semantic paint layers.
- Real `Div` mouse-down, mouse-up, and click handlers remain inactive during selection.
- A later held-button movement handler remains inactive. Capture stops dispatch before drag initiation.
- Mouse movement, down, held movement, and up are dispatched with click counts 1 and 2.
- An `overflow_hidden` parent clips a 129.6 by 100 logical-pixel target to 49.6 by 60 pixels at scale 1.25. Clicking the clipped-away part selects only the registered region.
- A deferred occluding popover wins over a lower control.
- An unregistered higher deferred occluder prevents selection of all underlying targets.
- Removing the popover and drawing a new revision removes stale registrations.
- Both windows use separate registries and event listeners.

## Production implementation rules

1. Put a normal semantic hitbox inside each opt-in occluding control owner. Insert it during prepaint with `Window::insert_hitbox`. A `canvas` child can provide the prepaint and paint callbacks without a custom element. Non-occluding text and icons must stay within this owner. Register containers explicitly as regions. Inside scrolling content, use `block_mouse_except_scroll()` on owners: `occlude()` also blocks ancestor scroll handlers. Keep full occlusion for true overlays.
2. Use `Hitbox::is_hovered_at(position, window)` for candidate eligibility. This public API calls the rendered frame's native hit test. It applies the content mask and real front-to-back hitbox ordering. Do not choose the smallest rectangle. If ownership is ambiguous, do not select.
3. Keep `hitbox.bounds.intersect(&hitbox.content_mask.bounds)` as the visible clipped bounds. Keep the hitbox for native occlusion checks. Declared layer metadata is an application hint; GPUI does not expose its private scene-layer index. The native hit test, not this hint, determines eligibility.
4. Register `Window::on_mouse_event` capture listeners before normal app listeners. The fixture installs these through the first painted canvas in the view. Verify placement relative to dialog and popover roots in the application. Stop propagation for every event in the selection sequence. Do not disable the capture latch or open a dialog immediately on mouse-down; keep capture through release and any pending click sequence before changing UI state. The fixture deliberately keeps picking enabled for its full batch of events.
5. Keep registries per window and per complete painted revision. Clear them when windows close. The fixture does not cache its controls. If application subtrees cache drawing, either rebuild those registrations when the cache is reused or do not cache the instrumented subtree. Do not clear a global registry while silently reusing child hitboxes from an earlier frame.
6. Use the selected window's dialog root and context. `gpui_kit::open_window` returns `(AnyWindowHandle, Entity<V>)`; the window root is Kit's `Root`, not the application view. Update through `AnyWindowHandle::update`.
7. Freeze target metadata, allowed application state, and connection generation on mouse-down. Before opening a dialog after release, revalidate the semantic identity, clipped bounds, scale, native occlusion, and relevant model identity. Ordinary paint revisions can change during a normal click; reject changed targets or state, not every repaint. A disconnected destination requires explicit confirmation before sending an existing draft.

This fixture proves the production API gate. It does not replace application integration checks for virtual lists, actual scrolling, focus restoration, IME, application-specific shortcuts, or macOS.

## Local evidence

- Root build: `target/developer-probe-build.log`.
- Production-only build: `target/developer-probe-production-build.log`.
- Production feature graph: `target/developer-probe-production-features.log`.
- Native assertions and target metadata: `target/developer-probe-runtime.log`.
- Native stderr: `target/developer-probe-production-stderr.log` (empty on success).
- `git diff --check` passed.
- The initial PATH lacked Rust formatter subcommands. The existing native Rust `bin` directory supplies `cargo-fmt.exe` and `rustfmt.exe`; final formatting checks use that directory on PATH.
