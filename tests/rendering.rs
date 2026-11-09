//! Production-window integration checks. macOS uses the real Metal renderer.
//! Run explicitly with `cargo test --test rendering`.

#[path = "support/browser_visual.rs"]
mod browser_visual;

use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, HeadlessAppContext, Modifiers, OwnedMenuItem,
    PlatformInput, ScrollDelta, ScrollEvent, Window, WindowBounds, WindowOptions, point, px, size,
    test::TestWindowExt,
};
use mangameeya_rust::{
    i18n::{Language, Text},
    settings::AppSettings,
    ui::{self, ReaderView},
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::Duration,
};

fn settle(cx: &mut HeadlessAppContext, window: AnyWindowHandle) {
    settle_windows(cx, &[window]);
}

fn settle_windows(cx: &mut HeadlessAppContext, windows: &[AnyWindowHandle]) {
    for _ in 0..8 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        for window in windows {
            cx.update_window(*window, |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
    }
    cx.run_until_parked();
}

fn open_preferences(
    cx: &mut HeadlessAppContext,
    reader: AnyWindowHandle,
    view: &Entity<ReaderView>,
) -> AnyWindowHandle {
    let geometry = |window: &mut Window| {
        (
            window.find("viewer").bounds(),
            window.try_find("sidebar").map(|node| node.bounds()),
            window.try_find("inspector").map(|node| node.bounds()),
            window
                .try_find("inspector-image")
                .map(|node| node.checked()),
            window
                .try_find("inspector-playback")
                .map(|node| node.checked()),
            window.find("page-input").value().unwrap().to_owned(),
        )
    };
    let before = cx
        .update_window(reader, |_, window, _| geometry(window))
        .unwrap();
    cx.update_window(reader, |_, window, cx| window.click("preferences", cx))
        .unwrap();
    cx.run_until_parked();
    let preferences = cx.update(|cx| view.read(cx).preferences_window().unwrap());
    assert_ne!(reader, preferences);
    settle_windows(cx, &[reader, preferences]);
    let after = cx
        .update_window(reader, |_, window, _| geometry(window))
        .unwrap();
    assert_eq!(
        before, after,
        "Preferences must not change reader geometry or state"
    );
    cx.update_window(reader, |_, window, cx| window.click("preferences", cx))
        .unwrap();
    settle_windows(cx, &[reader, preferences]);
    assert_eq!(
        cx.update(|cx| view.read(cx).preferences_window()),
        Some(preferences)
    );
    preferences
}

fn contains_bounds(outer: Bounds<gpui_kit::Pixels>, inner: Bounds<gpui_kit::Pixels>) -> bool {
    inner.size.width > px(0.)
        && inner.size.height > px(0.)
        && inner.left() >= outer.left() - px(0.5)
        && inner.top() >= outer.top() - px(0.5)
        && inner.right() <= outer.right() + px(0.5)
        && inner.bottom() <= outer.bottom() + px(0.5)
}

fn preference_in_view(window: &Window, id: &str) -> bool {
    let control = window.find(id.to_owned());
    control.visible()
        && contains_bounds(
            window.find("preferences-content").bounds(),
            control.bounds(),
        )
}

fn scroll_preferences(
    cx: &mut HeadlessAppContext,
    reader: AnyWindowHandle,
    preferences: AnyWindowHandle,
    delta: f32,
) {
    cx.update_window(preferences, |_, window, cx| {
        window.scroll(
            "preferences-content",
            ScrollDelta::Pixels(point(px(0.), px(delta))),
            cx,
        );
    })
    .unwrap();
    settle_windows(cx, &[reader, preferences]);
}

fn reach_preference(
    cx: &mut HeadlessAppContext,
    reader: AnyWindowHandle,
    preferences: AnyWindowHandle,
    id: &str,
) {
    scroll_preferences(cx, reader, preferences, 10_000.);
    for _ in 0..24 {
        if cx
            .update_window(preferences, |_, window, _| preference_in_view(window, id))
            .unwrap()
        {
            return;
        }
        scroll_preferences(cx, reader, preferences, -64.);
    }
    panic!("Preferences control must be scroll-reachable: {id}");
}

fn assert_preferences_hierarchy(window: &mut Window, language: Language, error: bool) {
    let viewport = Bounds::new(Default::default(), window.viewport_size());
    let root = window.find("preferences-window");
    assert_eq!(root.label(), Some(language.text(Text::Preferences)));
    assert!(contains_bounds(viewport, root.bounds()));
    let content = window
        .within("preferences-window")
        .find("preferences-content");
    let tip = window
        .within("preferences-window")
        .find("settings-save-tip");
    assert_eq!(tip.label(), Some(language.text(Text::PreferencesAutosave)));
    assert!(tip.visible() && contains_bounds(viewport, tip.bounds()));
    assert!(content.bounds().bottom() <= tip.bounds().top() + px(0.5));
    let reading = window
        .within("preferences-content")
        .find("reading-preferences");
    let general = window
        .within("preferences-content")
        .find("general-preferences");
    assert_eq!(reading.label(), Some(language.text(Text::ReadingTab)));
    assert_eq!(general.label(), Some(language.text(Text::GeneralSettings)));
    assert!(reading.bounds().bottom() < general.bounds().top());
    let releases = window
        .within("preferences-content")
        .find("release-preferences");
    assert_eq!(
        releases.label(),
        Some(language.text(Text::ReleaseNotifications))
    );
    assert!(general.bounds().bottom() < releases.bounds().top());
    for (id, text) in [
        ("release-check-on-startup", Text::ReleaseCheckOnStartup),
        ("check-for-updates", Text::CheckForUpdates),
    ] {
        let control = window.within("release-preferences").find(id);
        assert_eq!(control.label(), Some(language.text(text)));
        assert!(contains_bounds(releases.bounds(), control.bounds()));
    }
    for id in [
        "remember-position",
        "reading-direction-rtl",
        "reading-direction-ltr",
        "黑色",
        "深灰",
        "紙色",
    ] {
        let control = window.within("reading-preferences").find(id);
        assert!(
            contains_bounds(reading.bounds(), control.bounds()),
            "Reading child: {id}"
        );
    }
    for choice in Language::ALL {
        let id = format!("language-{}", choice.code());
        let control = window.within("general-preferences").find(id);
        assert_eq!(control.label(), Some(choice.native_name()));
        assert_eq!(control.checked(), Some(choice == language));
        assert!(contains_bounds(general.bounds(), control.bounds()));
    }
    for (id, text) in [
        ("remember-position", Text::RememberPosition),
        ("reading-direction-rtl", Text::DirectionRtl),
        ("reading-direction-ltr", Text::DirectionLtr),
        ("黑色", Text::Black),
        ("深灰", Text::Gray),
        ("紙色", Text::Paper),
    ] {
        assert_eq!(window.find(id).label(), Some(language.text(text)));
    }
    assert_eq!(window.try_find("reset-corrupt-settings").is_some(), error);
    if error {
        let warning = window.find("settings-save-error");
        assert_eq!(warning.role(), Some(gpui_kit::Role::Alert));
        assert!(warning.label().unwrap().contains("settings.json"));
        assert_eq!(
            window.find("reset-corrupt-settings").label(),
            Some(language.text(Text::BackupReset))
        );
    }
    assert!(window.try_find("inspector-image").is_none());
    assert!(window.try_find("page-input").is_none());
}

fn capture_preferences_sizes(
    cx: &mut HeadlessAppContext,
    reader: AnyWindowHandle,
    preferences: AnyWindowHandle,
    language: Language,
    error: bool,
) {
    let mut controls = vec![
        "remember-position",
        "reading-direction-rtl",
        "reading-direction-ltr",
        "黑色",
        "深灰",
        "紙色",
        "language-en",
        "language-ja",
        "language-zh-Hant",
        "release-check-on-startup",
        "check-for-updates",
    ];
    if error {
        controls.extend(["settings-save-error", "reset-corrupt-settings"]);
    }
    for (width, height, suffix) in [(560., 520., "560x520"), (440., 360., "440x360")] {
        cx.update_window(preferences, |_, window, cx| {
            window.resize(size(px(width), px(height)));
            window.bounds_changed(cx);
        })
        .unwrap();
        scroll_preferences(cx, reader, preferences, 10_000.);
        let name = format!(
            "preferences-{}-{suffix}-{}",
            language.code(),
            if error { "error" } else { "healthy" }
        );
        cx.update_window(preferences, |_, window, _| {
            assert_eq!(window.viewport_size(), size(px(width), px(height)));
            assert_preferences_hierarchy(window, language, error);
        })
        .unwrap();
        snapshot(cx, preferences, &name);
        let mut reached = vec![false; controls.len()];
        for _ in 0..24 {
            cx.update_window(preferences, |_, window, _| {
                for (index, id) in controls.iter().enumerate() {
                    reached[index] |= preference_in_view(window, id);
                    let a = window.find(*id).bounds();
                    for other in &controls[index + 1..] {
                        let b = window.find(*other).bounds();
                        assert!(
                            a.right() <= b.left() + px(0.5)
                                || b.right() <= a.left() + px(0.5)
                                || a.bottom() <= b.top() + px(0.5)
                                || b.bottom() <= a.top() + px(0.5),
                            "Preferences controls overlap: {id}, {other}"
                        );
                    }
                }
            })
            .unwrap();
            if reached.iter().all(|reached| *reached) {
                break;
            }
            scroll_preferences(cx, reader, preferences, -64.);
        }
        for (id, reached) in controls.iter().zip(reached) {
            assert!(
                reached,
                "{language:?} {suffix}: control must be fully visible or scroll-reachable: {id}"
            );
        }
        scroll_preferences(cx, reader, preferences, -10_000.);
        snapshot(cx, preferences, &format!("{name}-scrolled"));
    }
}

fn capture_preferences_states(
    cx: &mut HeadlessAppContext,
    reader: AnyWindowHandle,
    view: &Entity<ReaderView>,
) {
    let preferences = open_preferences(cx, reader, view);
    // Exercise selected states through production callbacks, not direct model mutation.
    for (id, selected) in [
        ("remember-position", false),
        ("remember-position", true),
        ("reading-direction-ltr", true),
        ("reading-direction-ltr", true),
        ("reading-direction-rtl", true),
        ("深灰", true),
        ("紙色", true),
        ("黑色", true),
    ] {
        reach_preference(cx, reader, preferences, id);
        cx.update_window(preferences, |_, window, cx| window.click(id, cx))
            .unwrap();
        settle_windows(cx, &[reader, preferences]);
        cx.update_window(preferences, |_, window, _| {
            assert_eq!(window.find(id).checked(), Some(selected))
        })
        .unwrap();
    }
    for language in Language::ALL {
        let id = format!("language-{}", language.code());
        reach_preference(cx, reader, preferences, &id);
        cx.update_window(preferences, |_, window, cx| window.click(id, cx))
            .unwrap();
        settle_windows(cx, &[reader, preferences]);
        cx.update_window(preferences, |_, window, _| {
            assert_eq!(window.find("remember-position").checked(), Some(true));
            assert_eq!(window.find("reading-direction-rtl").checked(), Some(true));
            assert_eq!(window.find("reading-direction-ltr").checked(), Some(false));
            for id in ["黑色", "深灰", "紙色"] {
                assert_eq!(window.find(id).checked(), Some(id == "黑色"));
            }
        })
        .unwrap();
        capture_preferences_sizes(cx, reader, preferences, language, false);
    }
    // Reader input remains live while Preferences is open, and updates both views.
    for rtl in [false, true] {
        cx.update_window(reader, |_, window, cx| window.click("direction", cx))
            .unwrap();
        settle_windows(cx, &[reader, preferences]);
        cx.update_window(preferences, |_, window, _| {
            assert_eq!(window.find("reading-direction-rtl").checked(), Some(rtl));
            assert_eq!(window.find("reading-direction-ltr").checked(), Some(!rtl));
        })
        .unwrap();
    }
    let before = page(cx, reader);
    cx.update_window(preferences, |_, window, cx| {
        for key in ["space", "right", "left", "+", "b"] {
            window.press(key, cx);
        }
    })
    .unwrap();
    settle_windows(cx, &[reader, preferences]);
    assert_eq!(
        page(cx, reader),
        before,
        "Preferences keys must not navigate the reader"
    );
    // Native Preferences close must leave the reader geometry and book intact.
    let viewport = cx
        .update_window(reader, |_, window, _| window.find("viewer").bounds())
        .unwrap();
    cx.update_window(preferences, |_, window, cx| window.press("escape", cx))
        .unwrap();
    settle(cx, reader);
    assert_eq!(page(cx, reader), before);
    assert_eq!(
        cx.update_window(reader, |_, window, _| window.find("viewer").bounds())
            .unwrap(),
        viewport
    );
    assert!(cx.update_window(preferences, |_, _, _| ()).is_err());
    let reopened = open_preferences(cx, reader, view);
    assert_ne!(reopened, preferences);
}

fn page(cx: &mut HeadlessAppContext, window: AnyWindowHandle) -> String {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.find("page-input").value().unwrap().to_string()
    })
    .unwrap()
}

fn snapshot(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    name: &str,
) -> Option<image::RgbaImage> {
    #[cfg(target_os = "macos")]
    {
        let screenshot = cx
            .capture_screenshot(window)
            .expect("Metal offscreen rendering is required");
        std::fs::create_dir_all("target/ui-smoke").unwrap();
        screenshot
            .save(format!("target/ui-smoke/{name}.png"))
            .unwrap();
        Some(screenshot)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (cx, window, name);
        None
    }
}

fn assert_zoom_closed(window: &Window, label: &str) {
    assert_eq!(window.find_all("zoom-trigger").len(), 1);
    assert_eq!(window.find("zoom-trigger").label(), Some(label));
    assert!(window.try_find("zoom-panel").is_none());
    assert!(window.try_find("zoom-input").is_none());
}

fn assert_zoom_panel_in_viewport(window: &Window) {
    assert_eq!(window.find_all("zoom-panel").len(), 1);
    let panel = window.find("zoom-panel");
    assert!(panel.visible());
    let bounds = panel.bounds();
    let viewport = window.viewport_size();
    assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
    assert!(
        bounds.left() >= px(0.)
            && bounds.top() >= px(0.)
            && bounds.right() <= viewport.width
            && bounds.bottom() <= viewport.height,
        "zoom popup must stay inside the window: {bounds:?}, viewport: {viewport:?}"
    );
    for id in [
        "zoom-input",
        "zoom-in",
        "zoom-out",
        "zoom-fit-window",
        "zoom-fit-width",
        "zoom-fit-height",
        "zoom-original",
    ] {
        assert!(
            window.find(id).visible(),
            "popup control must be visible: {id}"
        );
    }
}

fn capture_zoom_states(cx: &mut HeadlessAppContext, window: AnyWindowHandle) {
    for (width, height, suffix) in [(1280., 800., "1280x800"), (960., 640., "960x640")] {
        cx.update_window(window, |_, window, cx| {
            window.resize(size(px(width), px(height)));
            // TestPlatform changes its bounds without a native resize callback.
            window.bounds_changed(cx);
        })
        .unwrap();
        settle(cx, window);
        cx.update_window(window, |_, window, _| {
            assert_eq!(window.viewport_size(), size(px(width), px(height)));
            assert_zoom_closed(window, "縮放設定，適合視窗，100%");
        })
        .unwrap();
        snapshot(cx, window, &format!("zoom-closed-{suffix}"));

        cx.update_window(window, |_, window, cx| window.click("zoom-trigger", cx))
            .unwrap();
        settle(cx, window);
        cx.update_window(window, |_, window, _| {
            assert_zoom_panel_in_viewport(window);
            let viewer = window.find("viewer").bounds().size;
            let scale = 1.0_f64
                .min(f64::from(f32::from(viewer.width) - 24.) / 320.)
                .min(f64::from(f32::from(viewer.height) - 24.) / 480.);
            let input: f64 = window.find("zoom-input").value().unwrap().parse().unwrap();
            assert!(
                (input - scale * 100.).abs() < 0.0051,
                "zoom input must match the fitted page, including fractional percentages"
            );
        })
        .unwrap();
        snapshot(cx, window, &format!("zoom-open-{suffix}"));

        cx.update_window(window, |_, window, cx| {
            window.click("zoom-in", cx);
            assert_zoom_panel_in_viewport(window);
            window.click("zoom-out", cx);
            assert_zoom_panel_in_viewport(window);
            window.click("zoom-input", cx);
            window.press(
                if cfg!(target_os = "macos") {
                    "cmd-a"
                } else {
                    "ctrl-a"
                },
                cx,
            );
            window.input("200%", cx);
            window.press("enter", cx);
        })
        .unwrap();
        settle(cx, window);
        cx.update_window(window, |_, window, _| {
            assert_zoom_closed(window, "縮放設定，自訂縮放，200%");
        })
        .unwrap();
        snapshot(cx, window, &format!("zoom-custom-{suffix}"));

        cx.update_window(window, |_, window, cx| {
            window.click("zoom-trigger", cx);
            window.click("zoom-fit-window", cx);
            assert_zoom_closed(window, "縮放設定，適合視窗，100%");
        })
        .unwrap();
        settle(cx, window);
    }
    // Keep the existing rendering workflows at their original size and state.
    cx.update_window(window, |_, window, cx| {
        window.resize(size(px(1280.), px(800.)));
        window.bounds_changed(cx);
        window.press("home", cx);
    })
    .unwrap();
    settle(cx, window);
    cx.update_window(window, |_, window, _| {
        assert_eq!(window.viewport_size(), size(px(1280.), px(800.)));
        assert_eq!(window.find("page-input").value(), Some("1"));
        assert_zoom_closed(window, "縮放設定，適合視窗，100%");
    })
    .unwrap();
}

fn assert_sidebar_tabs_in_bounds(window: &Window, language: Language) {
    let sidebar = window.find("sidebar");
    assert_eq!(sidebar.label(), Some(language.text(Text::Sidebar)));
    let bounds = sidebar.bounds();
    for (id, text) in [
        ("檔案", Text::File),
        ("縮圖", Text::Thumbnails),
        ("最近開啟", Text::Recent),
    ] {
        let tab = window.find(id);
        assert_eq!(tab.label(), Some(language.text(text)));
        assert!(tab.visible());
        let tab_bounds = tab.bounds();
        assert!(
            tab_bounds.left() >= bounds.left()
                && tab_bounds.top() >= bounds.top()
                && tab_bounds.right() <= bounds.right()
                && tab_bounds.bottom() <= bounds.bottom(),
            "translated sidebar tab must stay inside the sidebar: {language:?}, {id}, {tab_bounds:?}, sidebar: {bounds:?}"
        );
    }
}

fn capture_language_states(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    view: &Entity<ReaderView>,
) {
    const LANGUAGES: [(Language, &str, &str, &str); 3] = [
        (Language::English, "language-en", "English", "en"),
        (Language::Japanese, "language-ja", "日本語", "ja"),
        (
            Language::TraditionalChinese,
            "language-zh-Hant",
            "正體中文",
            "zh-TW",
        ),
    ];
    let preferences = open_preferences(cx, window, view);
    for (language, id, _, component_locale) in LANGUAGES {
        reach_preference(cx, window, preferences, id);
        cx.update_window(preferences, |_, window, cx| window.click(id, cx))
            .unwrap();
        settle_windows(cx, &[window, preferences]);
        cx.update_window(preferences, |_, window, _| {
            assert_eq!(&*gpui_kit::component::locale(), component_locale);
            for (choice, choice_id, label, _) in LANGUAGES {
                assert_eq!(window.find(choice_id).label(), Some(label));
                assert_eq!(window.find(choice_id).checked(), Some(choice == language));
            }
        })
        .unwrap();
        cx.update_window(window, |_, window, cx| {
            assert_eq!(
                window.find("preferences").label(),
                Some(language.text(Text::Preferences))
            );
            assert_eq!(
                window.find("page-input").label(),
                Some(language.text(Text::PageNumber))
            );
            assert_eq!(window.find("page-input").value(), Some("1"));
            assert_sidebar_tabs_in_bounds(window, language);
            let trigger = window.find("zoom-trigger");
            let label = trigger.label().unwrap();
            assert!(label.starts_with(language.text(Text::ZoomSettings)), "{label}");
            assert!(label.contains(language.text(Text::FitWindow)), "{label}");
            assert!(label.contains("100%"), "{label}");
            if let Some(menus) = cx.get_menus() {
                for text in [Text::Preferences, Text::FitWindow] {
                    assert!(menus.iter().flat_map(|menu| &menu.items).any(|item| {
                        matches!(item, OwnedMenuItem::Action { name, .. } if name == language.text(text))
                    }));
                }
            }
            // Also check the accessible reader menu on every platform.
            window.click(("menu", 2usize), cx);
        })
        .unwrap();
        settle(cx, window);
        cx.update_window(window, |_, window, cx| {
            assert!(
                window
                    .within("popup-menu")
                    .find_by_label(language.text(Text::FitWindow))
                    .visible()
            );
            window.press("escape", cx);
        })
        .unwrap();
        settle(cx, window);
        cx.update_window(window, |_, window, cx| {
            window.click("zoom-trigger", cx);
            assert_zoom_panel_in_viewport(window);
            assert_eq!(
                window.find("zoom-panel").label(),
                Some(language.text(Text::ZoomSettings))
            );
            assert_eq!(
                window.find("zoom-fit-window").label(),
                Some(language.text(Text::FitWindow))
            );
        })
        .unwrap();
        settle(cx, window);
        snapshot(cx, window, &format!("i18n-{}", language.code()));
        for (width, height, suffix) in [(960., 640., "960x640"), (800., 540., "800x540")] {
            cx.update_window(window, |_, window, cx| {
                window.resize(size(px(width), px(height)));
                window.bounds_changed(cx);
            })
            .unwrap();
            settle(cx, window);
            cx.update_window(window, |_, window, _| {
                assert_sidebar_tabs_in_bounds(window, language);
                assert_zoom_panel_in_viewport(window);
            })
            .unwrap();
            snapshot(cx, window, &format!("i18n-{}-{suffix}", language.code()));
        }
        cx.update_window(window, |_, window, cx| {
            window.resize(size(px(1280.), px(800.)));
            window.bounds_changed(cx);
            window.press("escape", cx);
        })
        .unwrap();
        settle(cx, window);
    }
    // Retain the Traditional Chinese baseline for the existing workflow.
    cx.update_window(window, |_, window, cx| {
        window.click("inspector-image", cx);
        assert_zoom_closed(window, "縮放設定，適合視窗，100%");
    })
    .unwrap();
    settle(cx, window);
}

fn assert_desktop_geometry(window: &Window, language: Language) {
    let viewport = Bounds::new(Default::default(), window.viewport_size());
    let inside = |inner: Bounds<_>, outer: Bounds<_>| {
        assert!(
            inner.left() >= outer.left() - px(0.5)
                && inner.top() >= outer.top() - px(0.5)
                && inner.right() <= outer.right() + px(0.5)
                && inner.bottom() <= outer.bottom() + px(0.5),
            "desktop bounds must stay inside their owner: {inner:?}, {outer:?}"
        );
    };
    let disjoint = |a: Bounds<_>, b: Bounds<_>| {
        assert!(
            a.right() <= b.left() + px(0.5)
                || b.right() <= a.left() + px(0.5)
                || a.bottom() <= b.top() + px(0.5)
                || b.bottom() <= a.top() + px(0.5),
            "desktop controls overlap: {a:?}, {b:?}"
        );
    };
    let viewer = window.find("viewer").bounds();
    assert!(
        viewer.size.width >= px(280.),
        "reader must keep 280 logical pixels: {viewer:?}"
    );
    inside(viewer, viewport);
    for (owner, ids) in [
        (
            "reader-toolbar",
            &[
                "open",
                "sidebar-toggle",
                "mode-single",
                "mode-spread",
                "mode-continuous",
                "zoom-trigger",
                "direction",
                "bookmark",
                "inspector-toggle",
                "fullscreen",
                "preferences",
            ][..],
        ),
        (
            "reader-status",
            &[
                "previous",
                "page-input",
                "next",
                "reading-progress",
                "info-trigger",
            ][..],
        ),
    ] {
        let bounds = window.find(owner).bounds();
        inside(bounds, viewport);
        disjoint(bounds, viewer);
        for (index, id) in ids.iter().enumerate() {
            let button = window.find(*id);
            assert!(button.visible(), "{id}");
            inside(button.bounds(), bounds);
            for other in &ids[index + 1..] {
                disjoint(button.bounds(), window.find(*other).bounds());
            }
        }
    }
    if let Some(sidebar) = window.try_find("sidebar") {
        inside(sidebar.bounds(), viewport);
        disjoint(sidebar.bounds(), viewer);
        assert_sidebar_tabs_in_bounds(window, language);
    }
    if let Some(inspector) = window.try_find("inspector") {
        inside(inspector.bounds(), viewport);
        disjoint(inspector.bounds(), viewer);
        assert!(window.try_find("inspector-reading").is_none());
        let tabs = ["inspector-image", "inspector-playback"];
        for (index, id) in tabs.iter().enumerate() {
            inside(window.find(*id).bounds(), inspector.bounds());
            for other in &tabs[index + 1..] {
                disjoint(window.find(*id).bounds(), window.find(*other).bounds());
            }
        }
    }
    for index in 0..7usize {
        let menu = window.find(("menu", index)).bounds();
        inside(menu, viewport);
        for other in index + 1..7 {
            disjoint(menu, window.find(("menu", other)).bounds());
        }
    }
}

fn assert_info_in_viewport(window: &Window) {
    let panel = window.find("info-panel");
    assert!(panel.visible());
    let bounds = panel.bounds();
    let viewport = window.viewport_size();
    assert!(bounds.size.width > px(0.) && bounds.size.height > px(0.));
    assert!(
        bounds.left() >= px(0.)
            && bounds.top() >= px(0.)
            && bounds.right() <= viewport.width
            && bounds.bottom() <= viewport.height
    );
    assert_eq!(window.find("info-name").label(), Some("1.png"));
    assert_eq!(window.find("info-dimensions").label(), Some("320×480"));
    assert!(window.find("info-size").label().unwrap().ends_with(" KiB"));
}

fn capture_desktop_states(
    cx: &mut HeadlessAppContext,
    window: AnyWindowHandle,
    view: &Entity<ReaderView>,
    sender: &mpsc::Sender<Vec<PathBuf>>,
    folder: &std::path::Path,
) {
    for (language, language_id) in [
        (Language::English, "language-en"),
        (Language::Japanese, "language-ja"),
        (Language::TraditionalChinese, "language-zh-Hant"),
    ] {
        let preferences = open_preferences(cx, window, view);
        reach_preference(cx, window, preferences, language_id);
        cx.update_window(preferences, |_, window, cx| window.click(language_id, cx))
            .unwrap();
        settle_windows(cx, &[window, preferences]);
        for (width, height, suffix) in [
            (1280., 800., "1280x800"),
            (960., 640., "960x640"),
            (800., 540., "800x540"),
        ] {
            cx.update_window(window, |_, window, cx| {
                window.resize(size(px(width), px(height)));
                window.bounds_changed(cx);
            })
            .unwrap();
            settle(cx, window);
            for (id, name) in [
                ("inspector-image", "image"),
                ("inspector-playback", "playback"),
            ] {
                cx.update_window(window, |_, window, cx| {
                    window.click(id, cx);
                    assert_desktop_geometry(window, language);
                    assert!(window.try_find("inspector-reading").is_none());
                    for choice in ["inspector-image", "inspector-playback"] {
                        assert_eq!(window.find(choice).checked(), Some(choice == id));
                    }
                    assert_eq!(window.try_find("reset-filters").is_some(), name == "image");
                    assert!(window.try_find("remember-position").is_none());
                    assert_eq!(
                        window.try_find("interval-input").is_some(),
                        name == "playback"
                    );
                })
                .unwrap();
                settle(cx, window);
                snapshot(
                    cx,
                    window,
                    &format!("desktop-{}-{suffix}-{name}", language.code()),
                );
            }
            for (toggle, absent, name) in [
                ("sidebar-toggle", "sidebar", "sidebar-closed"),
                ("inspector-toggle", "inspector", "both-closed"),
                ("sidebar-toggle", "inspector", "inspector-closed"),
            ] {
                cx.update_window(window, |_, window, cx| {
                    window.click(toggle, cx);
                    assert!(window.try_find(absent).is_none());
                    assert_desktop_geometry(window, language);
                })
                .unwrap();
                settle(cx, window);
                snapshot(
                    cx,
                    window,
                    &format!("desktop-{}-{suffix}-{name}", language.code()),
                );
            }
            cx.update_window(window, |_, window, cx| {
                window.click("inspector-toggle", cx);
                window.click("info-trigger", cx);
                assert_info_in_viewport(window);
                let before = window.find("zoom-trigger").label().unwrap().to_owned();
                let panel = window.find("info-panel").bounds();
                let viewer = window.find("viewer").bounds();
                let overlap = Bounds::from_corners(
                    point(
                        panel.left().max(viewer.left()),
                        panel.top().max(viewer.top()),
                    ),
                    point(
                        panel.right().min(viewer.right()),
                        panel.bottom().min(viewer.bottom()),
                    ),
                );
                assert!(overlap.size.width > px(0.) && overlap.size.height > px(0.));
                let position = overlap.center();
                assert!(panel.contains(&position) && viewer.contains(&position));
                window.dispatch_event(
                    PlatformInput::Scroll(ScrollEvent {
                        position,
                        delta: ScrollDelta::Lines(point(0., 2.)),
                        modifiers: Modifiers {
                            control: true,
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
                    cx,
                );
                window.render_frame(cx);
                assert_eq!(window.find("zoom-trigger").label(), Some(before.as_str()));
                window.dispatch_event(
                    PlatformInput::ModifiersChanged(gpui_kit::ModifiersChangedEvent::default()),
                    cx,
                );
            })
            .unwrap();
            settle(cx, window);
            snapshot(
                cx,
                window,
                &format!("desktop-{}-{suffix}-information", language.code()),
            );
            cx.update_window(window, |_, window, cx| {
                window.press("escape", cx);
                assert!(window.try_find("info-panel").is_none());
            })
            .unwrap();
            sender
                .send(vec![folder.join("missing-desktop.cbz")])
                .unwrap();
            settle(cx, window);
            cx.update_window(window, |_, window, _| {
                let status = window.find("status-message");
                assert!(status.visible());
                assert!(status.label().unwrap().contains("missing-desktop.cbz"));
                assert_eq!(window.find("page-input").value(), Some("1"));
                assert_desktop_geometry(window, language);
            })
            .unwrap();
            snapshot(
                cx,
                window,
                &format!("desktop-{}-{suffix}-error", language.code()),
            );
            sender.send(vec![folder.to_path_buf()]).unwrap();
            settle(cx, window);
        }
        cx.update_window(window, |_, window, cx| {
            window.resize(size(px(1280.), px(800.)));
            window.bounds_changed(cx);
        })
        .unwrap();
        settle(cx, window);
        for (left, delta) in [(true, 24.), (false, -24.), (true, -24.), (false, 24.)] {
            cx.update_window(window, |_, window, cx| {
                let viewer = window.find("viewer").bounds();
                let x = if left { viewer.left() } else { viewer.right() };
                window.drag(
                    point(x, viewer.center().y),
                    point(x + px(delta), viewer.center().y),
                    cx,
                );
                assert_desktop_geometry(window, language);
            })
            .unwrap();
            settle(cx, window);
            snapshot(
                cx,
                window,
                &format!(
                    "desktop-{}-1280x800-drag-{}-{}",
                    language.code(),
                    if left { "sidebar" } else { "inspector" },
                    if delta > 0. { "right" } else { "left" }
                ),
            );
        }
    }
    cx.update_window(window, |_, window, cx| {
        window.click("inspector-image", cx);
        assert_zoom_closed(window, "縮放設定，適合視窗，100%");
        assert_eq!(window.find("page-input").value(), Some("1"));
    })
    .unwrap();
    settle(cx, window);
}

fn capture_preferences_errors(cx: &mut HeadlessAppContext, temporary: &Path, folder: &Path) {
    const INVALID: &[u8] = b"{broken\n";
    for language in Language::ALL {
        let directory = temporary.join(format!("preferences-error-{}", language.code()));
        std::fs::create_dir(&directory).unwrap();
        let settings_path = directory.join("settings.json");
        std::fs::write(&settings_path, INVALID).unwrap();
        let warning = AppSettings::load_from(&settings_path).unwrap_err();
        let mut settings = AppSettings::default();
        settings.preferences.language = language;
        let (sender, launches) = mpsc::channel();
        let expected_warning = language.format(Text::SettingsError, &[("error", warning.clone())]);
        let (reader, view) =
            open_reader_fixture(cx, settings_path.clone(), settings, Some(warning), launches);
        sender.send(vec![folder.to_path_buf()]).unwrap();
        settle(cx, reader);
        assert_eq!(page(cx, reader), "1");
        let preferences = open_preferences(cx, reader, &view);
        cx.update_window(preferences, |_, window, _| {
            assert_eq!(
                window.find("settings-save-error").label(),
                Some(expected_warning.as_str())
            );
        })
        .unwrap();
        capture_preferences_sizes(cx, reader, preferences, language, true);
        assert_eq!(
            std::fs::read(&settings_path).unwrap(),
            INVALID,
            "rendering and navigation must not replace corrupt settings"
        );
        reach_preference(cx, reader, preferences, "reset-corrupt-settings");
        cx.update_window(preferences, |_, window, cx| {
            window.click("reset-corrupt-settings", cx)
        })
        .unwrap();
        settle_windows(cx, &[reader, preferences]);
        let backups: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path != &settings_path)
            .collect();
        assert_eq!(
            backups.len(),
            1,
            "explicit reset must create exactly one backup"
        );
        assert_eq!(
            std::fs::read(&backups[0]).unwrap(),
            INVALID,
            "backup must preserve the exact original bytes"
        );
        assert_eq!(
            AppSettings::load_from(&settings_path).unwrap(),
            AppSettings::default()
        );
        cx.update_window(preferences, |_, window, _| {
            assert!(window.try_find("reset-corrupt-settings").is_none())
        })
        .unwrap();
        scroll_preferences(cx, reader, preferences, 10_000.);
        snapshot(
            cx,
            preferences,
            &format!("preferences-{}-reset", language.code()),
        );
        // Remove fixture windows directly. Do not dispatch the application Quit command.
        for handle in [preferences, reader] {
            cx.update_window(handle, |_, window, _| window.remove_window())
                .unwrap();
        }
        cx.run_until_parked();
        drop(view);
    }
}

fn browser_shell_bounds(window: &Window) -> Vec<Option<Bounds<gpui_kit::Pixels>>> {
    let mut bounds: Vec<_> = [
        "reader-toolbar",
        "reader-status",
        "sidebar",
        "inspector",
        "viewer",
    ]
    .into_iter()
    .map(|id| window.try_find(id).map(|node| node.bounds()))
    .collect();
    bounds.extend((0..7usize).map(|index| Some(window.find(("menu", index)).bounds())));
    bounds
}

fn browser_controls_in_bounds(window: &Window, language: Language) {
    assert_desktop_geometry(window, language);
    let disjoint = |a: Bounds<_>, b: Bounds<_>| {
        assert!(
            a.right() <= b.left() + px(0.5)
                || b.right() <= a.left() + px(0.5)
                || a.bottom() <= b.top() + px(0.5)
                || b.bottom() <= a.top() + px(0.5),
            "browser controls overlap: {a:?}, {b:?}"
        );
    };
    let toolbar = window.find("reader-toolbar").bounds();
    let toggle = window.find("browser-toggle");
    assert!(contains_bounds(toolbar, toggle.bounds()));
    assert!(!toggle.label().unwrap_or_default().is_empty());
    for id in [
        "open",
        "sidebar-toggle",
        "mode-single",
        "mode-spread",
        "mode-continuous",
        "zoom-trigger",
        "direction",
        "bookmark",
        "inspector-toggle",
        "fullscreen",
        "preferences",
    ] {
        disjoint(toggle.bounds(), window.find(id).bounds());
    }
    let central = window.find("viewer").bounds();
    let grid = window.find("browser-grid").bounds();
    assert!(contains_bounds(central, grid));
    let ids = [
        "browser-search",
        "browser-parent",
        "browser-sort",
        "browser-reverse",
        "browser-folders-first",
        "browser-refresh",
        "browser-open",
    ];
    for (index, id) in ids.iter().enumerate() {
        let control = window.find(*id);
        assert!(control.visible(), "browser control: {language:?}, {id}");
        assert!(
            !control.label().unwrap_or_default().is_empty(),
            "{id} needs a name"
        );
        assert!(
            contains_bounds(central, control.bounds()),
            "{id} must stay in the central slot"
        );
        disjoint(control.bounds(), grid);
        for other in &ids[index + 1..] {
            disjoint(control.bounds(), window.find(*other).bounds());
        }
    }
    if let Some(inspector) = window.try_find("inspector") {
        let tab = window.find("inspector-browser");
        assert!(contains_bounds(inspector.bounds(), tab.bounds()));
        for id in ["inspector-image", "inspector-playback"] {
            disjoint(tab.bounds(), window.find(id).bounds());
        }
    }
}

// Use separate windows and settings files. Do not change the existing reader workflow.
fn capture_browser_states(cx: &mut HeadlessAppContext, temporary: &Path, folder: &Path) {
    let source = folder.join("2.png").canonicalize().unwrap();
    let original_bytes = std::fs::read(&source).unwrap();
    let original_windows = cx.update(|cx| cx.windows().len());
    for language in Language::ALL {
        let mut settings = AppSettings::default();
        settings.preferences.language = language;
        settings.preferences.panel_layout.sidebar_width = 272;
        settings.preferences.panel_layout.inspector_width = 300;
        let (sender, launches) = mpsc::channel();
        let (reader, view) = open_reader_fixture(
            cx,
            temporary.join(format!("browser-{}.json", language.code())),
            settings,
            None,
            launches,
        );
        settle(cx, reader);
        cx.update_window(reader, |_, window, cx| {
            assert!(window.find("browser-grid").visible());
            assert_eq!(window.find("browser-toggle").disabled(), Some(true));
            assert!(window.try_find("browser-resume").is_none());
            assert!(window.try_find("browser-shuffle").is_none());
            window.press("escape", cx);
            assert!(window.find("browser-grid").visible());
        })
        .unwrap();
        sender.send(vec![source.clone()]).unwrap();
        settle(cx, reader);
        assert_eq!(page(cx, reader), "2");
        for (width, height, suffix) in [
            (1280., 800., "1280x800"),
            (960., 640., "960x640"),
            (800., 540., "800x540"),
        ] {
            cx.update_window(reader, |_, window, cx| {
                window.resize(size(px(width), px(height)));
                window.bounds_changed(cx);
            })
            .unwrap();
            settle(cx, reader);
            for (toggle, panels) in [
                (None, "both"),
                (Some("sidebar-toggle"), "right"),
                (Some("inspector-toggle"), "neither"),
                (Some("sidebar-toggle"), "left"),
            ] {
                if let Some(toggle) = toggle {
                    cx.update_window(reader, |_, window, cx| window.click(toggle, cx))
                        .unwrap();
                    settle(cx, reader);
                }
                let before = cx
                    .update_window(reader, |_, window, cx| {
                        window.click("viewer", cx);
                        assert_desktop_geometry(window, language);
                        assert_eq!(window.find("page-input").value(), Some("2"));
                        browser_shell_bounds(window)
                    })
                    .unwrap();
                snapshot(
                    cx,
                    reader,
                    &format!("browser-{}-{suffix}-{panels}-reading", language.code()),
                );
                cx.update_window(reader, |_, window, cx| window.press("backspace", cx))
                    .unwrap();
                settle(cx, reader);
                cx.update_window(reader, |_, window, _| {
                    assert_eq!(
                        browser_shell_bounds(window),
                        before,
                        "only central content can change"
                    );
                    browser_controls_in_bounds(window, language);
                    assert_eq!(window.find("browser-search").value(), Some(""));
                    assert_eq!(
                        window
                            .within("browser-grid")
                            .find_by_label("2.png")
                            .checked(),
                        Some(true)
                    );
                    assert_eq!(window.find("next").disabled(), Some(true));
                    assert_eq!(window.find("zoom-trigger").disabled(), Some(true));
                })
                .unwrap();
                snapshot(
                    cx,
                    reader,
                    &format!("browser-{}-{suffix}-{panels}-browsing", language.code()),
                );
                // Text editing must not run the browser Backspace or Enter commands.
                cx.update_window(reader, |_, window, cx| {
                    window.click("browser-search", cx);
                    window.input("2.pngx", cx);
                    window.press("backspace", cx);
                    window.press("enter", cx);
                    assert_eq!(window.find("browser-search").value(), Some("2.png"));
                    assert!(window.find("browser-grid").visible());
                    window.press(
                        if cfg!(target_os = "macos") {
                            "cmd-a"
                        } else {
                            "ctrl-a"
                        },
                        cx,
                    );
                    window.press("backspace", cx);
                    // Resume explicitly. Clicking the grid's center can select
                    // a different tile and is not a neutral way to blur search.
                    window.click("browser-toggle", cx);
                })
                .unwrap();
                settle(cx, reader);
                cx.update_window(reader, |_, window, _| {
                    assert!(window.try_find("browser-grid").is_none());
                    assert_eq!(window.find("page-input").value(), Some("2"));
                    assert_eq!(browser_shell_bounds(window), before);
                })
                .unwrap();
            }
            cx.update_window(reader, |_, window, cx| window.click("inspector-toggle", cx))
                .unwrap();
            settle(cx, reader);
        }
        // Browser tools use the existing inspector, not a new window or overlay.
        cx.update_window(reader, |_, window, cx| {
            window.click("browser-toggle", cx);
            window.click("inspector-browser", cx);
            assert!(window.find("browser-tools").visible());
            assert_eq!(window.find("inspector-browser").checked(), Some(true));
            assert!(contains_bounds(
                window.find("inspector").bounds(),
                window.find("inspector-content").bounds()
            ));
        })
        .unwrap();
        // The existing inspector is scrollable. At minimum window height its
        // content may be taller than the pane; each control must be reachable.
        for id in [
            "browser-tile-width",
            "browser-show-names",
            "browser-metadata",
        ] {
            let mut reached = false;
            for _ in 0..16 {
                reached = cx
                    .update_window(reader, |_, window, _| {
                        contains_bounds(
                            window.find("inspector-content").bounds(),
                            window.find(id).bounds(),
                        )
                    })
                    .unwrap();
                if reached {
                    break;
                }
                cx.update_window(reader, |_, window, cx| {
                    window.scroll(
                        "inspector-content",
                        ScrollDelta::Pixels(point(px(0.), px(-64.))),
                        cx,
                    );
                })
                .unwrap();
                settle(cx, reader);
            }
            assert!(reached, "browser tool must be scroll-reachable: {id}");
        }
        cx.update_window(reader, |_, window, cx| window.click("browser-toggle", cx))
            .unwrap();
        settle(cx, reader);
        assert_eq!(page(cx, reader), "2");
        assert_eq!(cx.update(|cx| cx.windows().len()), original_windows + 1);
        cx.update_window(reader, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
        drop(view);
        drop(sender);
        assert_eq!(cx.update(|cx| cx.windows().len()), original_windows);
    }
    assert_eq!(std::fs::read(source).unwrap(), original_bytes);
}

// Files coverage uses separate windows and native fixtures. Keep the original
// reader and central-browser capture workflows unchanged.
fn capture_files_states(cx: &mut HeadlessAppContext, temporary: &Path, source_folder: &Path) {
    let root = temporary.join("files-native-漫畫");
    std::fs::create_dir(&root).unwrap();
    let source = root.join("1.png");
    std::fs::copy(source_folder.join("2.png"), &source).unwrap();
    let long_name = format!("長い名前-漫畫-{}-10.png", "readable-name-".repeat(10));
    std::fs::copy(source_folder.join("1.png"), root.join(&long_name)).unwrap();
    let a = root.join("folder2");
    let b = root.join("folder10");
    for folder in [&a, &b] {
        std::fs::create_dir(folder).unwrap();
        std::fs::copy(&source, folder.join("child.png")).unwrap();
    }
    let root = root.canonicalize().unwrap();
    let source = root.join("1.png");
    let bytes = std::fs::read(&source).unwrap();
    let entry =
        |path: &Path| gpui_kit::SharedString::from(format!("files-entry:{:?}", path.as_os_str()));
    let toggle =
        |path: &Path| gpui_kit::SharedString::from(format!("files-toggle:{:?}", path.as_os_str()));
    let original_windows = cx.update(|cx| cx.windows().len());
    for language in Language::ALL {
        for sidebar_width in [216, 224, 320] {
            let mut settings = AppSettings::default();
            settings.preferences.language = language;
            settings.preferences.sidebar = true;
            settings.preferences.sidebar_mode = mangameeya_rust::settings::SidebarMode::Files;
            settings.preferences.panel_layout.sidebar_width = sidebar_width;
            // The minimum viewport cannot contain a 320 px Files panel, the
            // inspector, and the required 280 px central slot at the same time.
            settings.preferences.panel_layout.inspector_visible = false;
            let (sender, launches) = mpsc::channel();
            let (reader, view) = open_reader_fixture(
                cx,
                temporary.join(format!("files-{}-{sidebar_width}.json", language.code())),
                settings,
                None,
                launches,
            );
            sender.send(vec![source.clone()]).unwrap();
            settle(cx, reader);
            for folder in [root.join("folder2"), root.join("folder10")] {
                cx.update_window(reader, |_, window, cx| window.click(toggle(&folder), cx))
                    .unwrap();
                settle(cx, reader);
            }
            for (width, height, suffix) in [
                (1280., 800., "1280x800"),
                (960., 640., "960x640"),
                (800., 540., "800x540"),
            ] {
                cx.update_window(reader, |_, window, cx| {
                    window.resize(size(px(width), px(height)));
                    window.bounds_changed(cx);
                    window.click("files-locate", cx);
                })
                .unwrap();
                settle(cx, reader);
                let shell = cx
                    .update_window(reader, |_, window, _| browser_shell_bounds(window))
                    .unwrap();
                for browsing in [false, true] {
                    if browsing {
                        cx.update_window(reader, |_, window, cx| {
                            window.click("browser-toggle", cx)
                        })
                        .unwrap();
                        settle(cx, reader);
                    }
                    cx.update_window(reader, |_, window, cx| window.click("files-locate", cx))
                        .unwrap();
                    settle(cx, reader);
                    cx.update_window(reader, |_, window, _| {
                        assert_desktop_geometry(window, language);
                        assert_eq!(
                            browser_shell_bounds(window),
                            shell,
                            "Files must not alter the shell between central modes"
                        );
                        assert_eq!(window.try_find("browser-grid").is_some(), browsing);
                        assert_eq!(window.find("page-input").value(), Some("1"));
                        let sidebar = window.find("sidebar").bounds();
                        assert!(
                            (f32::from(sidebar.size.width) - sidebar_width as f32).abs() < 1.,
                            "requested Files width: {sidebar_width}, {sidebar:?}"
                        );
                        let panel = window.find("files-panel").bounds();
                        assert!(contains_bounds(sidebar, panel));
                        let tree = window.find("files-tree");
                        assert_eq!(tree.role(), Some(gpui_kit::Role::Tree));
                        assert_eq!(tree.label(), Some(language.text(Text::FilesTree)));
                        assert!(contains_bounds(panel, tree.bounds()));
                        let controls = [
                            "files-back",
                            "files-parent",
                            "files-locate",
                            "files-hidden",
                            "path-input",
                            "files-search",
                            "files-search-hint",
                            "files-read-selected",
                            "files-read-folder",
                        ];
                        for (index, id) in controls.iter().enumerate() {
                            let control = window.find(*id);
                            assert!(
                                control.visible(),
                                "Files control: {language:?} {suffix} {sidebar_width} {id}"
                            );
                            assert!(
                                contains_bounds(panel, control.bounds()),
                                "Files control must fit at every width: {id}"
                            );
                            for other in &controls[index + 1..] {
                                let a = control.bounds();
                                let b = window.find(*other).bounds();
                                assert!(
                                    a.right() <= b.left() + px(0.5)
                                        || b.right() <= a.left() + px(0.5)
                                        || a.bottom() <= b.top() + px(0.5)
                                        || b.bottom() <= a.top() + px(0.5),
                                    "Files controls overlap: {id}, {other}"
                                );
                            }
                        }
                        for (id, text) in [
                            ("files-back", Text::FilesBack),
                            ("files-parent", Text::ParentFolder),
                            ("files-locate", Text::FilesLocate),
                            ("files-hidden", Text::FilesHidden),
                            ("files-search", Text::FilesSearchLoaded),
                            ("files-read-selected", Text::FilesReadSelected),
                            ("files-read-folder", Text::FilesReadFolder),
                        ] {
                            assert_eq!(
                                window.find(id).label(),
                                Some(language.text(text)),
                                "localized Files label: {id}"
                            );
                        }
                        assert_eq!(
                            window.find("files-search-hint").label(),
                            Some(language.text(Text::FilesSearchHint))
                        );
                        assert_ne!(window.find("files-read-selected").disabled(), Some(true));
                        assert_ne!(window.find("files-read-folder").disabled(), Some(true));
                        let active = window.find(entry(&source));
                        assert!(
                            active.visible() && contains_bounds(tree.bounds(), active.bounds()),
                            "current source must be reachable"
                        );
                        assert_eq!(active.checked(), Some(true));
                        assert!(window.find("files-current-marker").visible());
                        assert!(
                            window.try_find(entry(&root)).is_none(),
                            "location is not a duplicate tree row"
                        );
                    })
                    .unwrap();
                    snapshot(
                        cx,
                        reader,
                        &format!(
                            "files-{}-{suffix}-w{sidebar_width}-{}",
                            language.code(),
                            if browsing { "browsing" } else { "reading" }
                        ),
                    );
                    // Reveal the first siblings independently of the selected row.
                    cx.update_window(reader, |_, window, cx| {
                        window.click("files-locate", cx);
                        window.press("home", cx);
                    })
                    .unwrap();
                    settle(cx, reader);
                    cx.update_window(reader, |_, window, _| {
                        let icon = |path: &Path| {
                            gpui_kit::SharedString::from(format!(
                                "files-icon:{:?}",
                                path.as_os_str()
                            ))
                        };
                        let first = window.find(icon(&root.join("folder2"))).bounds();
                        let second = window.find(icon(&root.join("folder10"))).bounds();
                        assert!(
                            (f32::from(first.left() - second.left())).abs() < 0.5,
                            "sibling icon columns must align"
                        );
                        assert!(contains_bounds(window.find("files-tree").bounds(), first));
                    })
                    .unwrap();
                    // At 800x540 the tree can show two rows. Reveal the second
                    // sibling through the composite keyboard focus as well.
                    cx.update_window(reader, |_, window, cx| {
                        window.press("down", cx);
                        window.press("down", cx);
                    })
                    .unwrap();
                    settle(cx, reader);
                    cx.update_window(reader, |_, window, _| {
                        let second = window.find(entry(&root.join("folder10")));
                        assert!(second.visible() && contains_bounds(window.find("files-tree").bounds(), second.bounds()), "sibling must be keyboard-reachable: {language:?} {suffix} w{sidebar_width}");
                    }).unwrap();
                    cx.update_window(reader, |_, window, cx| window.press("end", cx))
                        .unwrap();
                    settle(cx, reader);
                    cx.update_window(reader, |_, window, _| {
                        let long = window.find(entry(&root.join(&long_name)));
                        assert_eq!(
                            long.label(),
                            Some(long_name.as_str()),
                            "long names must retain their complete accessible names"
                        );
                        assert!(
                            long.visible()
                                && contains_bounds(
                                    window.find("files-tree").bounds(),
                                    long.bounds()
                                )
                        );
                        assert_eq!(
                            window.find("page-input").value(),
                            Some("1"),
                            "Files End must not navigate the reader"
                        );
                    })
                    .unwrap();
                    snapshot(
                        cx,
                        reader,
                        &format!(
                            "files-{}-{suffix}-w{sidebar_width}-{}-end",
                            language.code(),
                            if browsing { "browsing" } else { "reading" }
                        ),
                    );
                }
                cx.update_window(reader, |_, window, cx| window.click("browser-toggle", cx))
                    .unwrap();
                settle(cx, reader);
            }
            cx.update_window(reader, |_, window, _| window.remove_window())
                .unwrap();
            cx.run_until_parked();
            drop(view);
            drop(sender);
            assert_eq!(cx.update(|cx| cx.windows().len()), original_windows);
        }
    }
    assert_eq!(
        std::fs::read(source).unwrap(),
        bytes,
        "Files captures must not modify source pages"
    );
}

fn open_reader(
    cx: &mut HeadlessAppContext,
    settings_path: PathBuf,
    launches: mpsc::Receiver<Vec<PathBuf>>,
) -> (AnyWindowHandle, Entity<ReaderView>) {
    let mut settings = AppSettings::default();
    settings.preferences.language = Language::TraditionalChinese;
    open_reader_fixture(cx, settings_path, settings, None, launches)
}

fn open_reader_fixture(
    cx: &mut HeadlessAppContext,
    settings_path: PathBuf,
    settings: AppSettings,
    warning: Option<String>,
    launches: mpsc::Receiver<Vec<PathBuf>>,
) -> (AnyWindowHandle, Entity<ReaderView>) {
    cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(1280.), px(800.)),
                ))),
                show: false,
                focus: true,
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    ReaderView::new(settings, settings_path, warning, None, launches, window, cx)
                })
            },
        )
        .unwrap()
    })
}

fn main() {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(ui::ReaderAssets),
        || {
            #[cfg(target_os = "macos")]
            {
                gpui_kit::platform::current_headless_renderer()
            }
            #[cfg(not(target_os = "macos"))]
            {
                Ok(None)
            }
        },
    );
    cx.update(ui::init);
    let temporary = tempfile::tempdir().unwrap();
    let folder = temporary.path().join("book");
    std::fs::create_dir(&folder).unwrap();
    let colors = [
        [220, 90, 70, 255],
        [70, 170, 120, 255],
        [70, 110, 220, 255],
        [210, 170, 60, 255],
        [170, 90, 190, 255],
        [90, 190, 210, 255],
    ];
    for (index, color) in colors.into_iter().enumerate() {
        let mut image = image::RgbaImage::from_pixel(320, 480, image::Rgba(color));
        for y in 0..480 {
            for x in 0..320 {
                if x < 12 || y < 12 || x >= 308 || y >= 468 || (x / 40 + y / 40) % 7 == 0 {
                    image.put_pixel(x, y, image::Rgba([245, 240, 230, 255]));
                }
            }
        }
        image
            .save(folder.join(format!("{}.png", index + 1)))
            .unwrap();
    }
    capture_browser_states(&mut cx, temporary.path(), &folder);
    browser_visual::capture(&mut cx, temporary.path());
    capture_files_states(&mut cx, temporary.path(), &folder);
    let settings_path = temporary.path().join("settings.json");
    let (sender, launches) = mpsc::channel();
    let (window, view) = open_reader(&mut cx, settings_path.clone(), launches);
    settle(&mut cx, window);
    cx.update_window(window, |_, window, cx| {
        assert_eq!(window.find("next").disabled(), Some(true));
        assert_eq!(window.find("zoom-trigger").disabled(), Some(true));
        assert!(window.try_find("zoom-panel").is_none());
        assert!(window.try_find("zoom-input").is_none());
        window.click("zoom-trigger", cx);
        assert!(window.try_find("zoom-panel").is_none());
        window.click("next", cx);
        assert_eq!(window.find("page-input").value(), Some("—"));
    })
    .unwrap();
    snapshot(&mut cx, window, "01-empty");

    // Exercise the same launch queue used by macOS document-open events.
    sender.send(vec![folder.clone()]).unwrap();
    settle(&mut cx, window);
    assert_eq!(page(&mut cx, window), "1");
    let single = snapshot(&mut cx, window, "02-single");
    if let Some(image) = &single {
        let red = image
            .pixels()
            .filter(|p| p[0].abs_diff(220) <= 2 && p[1].abs_diff(90) <= 2 && p[2].abs_diff(70) <= 2)
            .count();
        assert!(
            red > 20_000,
            "The decoded source page must reach the Metal scene, not a placeholder: {red}"
        );
    }
    capture_zoom_states(&mut cx, window);
    capture_preferences_states(&mut cx, window, &view);
    capture_preferences_errors(&mut cx, temporary.path(), &folder);
    capture_language_states(&mut cx, window, &view);
    capture_desktop_states(&mut cx, window, &view, &sender, &folder);

    // Cover archive indexing, natural order, extraction, decoding, and Metal in
    // one workflow, with central-directory order different from reading order.
    let archive_path = temporary.path().join("reader.cbz");
    let mut archive = zip::ZipWriter::new(std::fs::File::create(&archive_path).unwrap());
    for (name, source) in [("10.png", "1.png"), ("2.png", "2.png")] {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive
            .write_all(&std::fs::read(folder.join(source)).unwrap())
            .unwrap();
    }
    archive.finish().unwrap();
    sender.send(vec![archive_path]).unwrap();
    settle(&mut cx, window);
    if let Some(image) = snapshot(&mut cx, window, "10-cbz-natural-order") {
        assert!(
            image
                .pixels()
                .filter(|p| p[0].abs_diff(70) <= 2
                    && p[1].abs_diff(170) <= 2
                    && p[2].abs_diff(120) <= 2)
                .count()
                > 20_000
        );
    }
    sender.send(vec![folder.clone()]).unwrap();
    settle(&mut cx, window);
    cx.update_window(window, |_, window, cx| {
        window.click("next", cx);
        assert_eq!(window.find("page-input").value(), Some("2"));
        window.press("b", cx);
        window.press("space", cx);
        assert_eq!(window.find("page-input").value(), Some("3"));
        window.press("shift-b", cx);
        assert_eq!(window.find("page-input").value(), Some("2"));
        window.press("home", cx);
        window.click("mode-spread", cx);
        window.press("space", cx);
        assert_eq!(window.find("page-input").value(), Some("3"));
        window.press("home", cx);
    })
    .unwrap();
    settle(&mut cx, window);
    let spread = snapshot(&mut cx, window, "03-spread");
    if let (Some(single), Some(spread)) = (&single, &spread) {
        assert_ne!(single, spread);
    }

    // Input focus must not trigger single-letter reading commands.
    cx.update_window(window, |_, window, cx| {
        window.click("page-input", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input("5", cx);
        window.press("enter", cx);
    })
    .unwrap();
    settle(&mut cx, window);
    cx.update_window(window, |_, window, cx| {
        assert_eq!(window.find("page-input").value(), Some("5"));
        window.click("zoom-trigger", cx);
        assert_zoom_panel_in_viewport(window);
        window.click("zoom-input", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input("200", cx);
        window.press("enter", cx);
    })
    .unwrap();
    settle(&mut cx, window);
    cx.update_window(window, |_, window, cx| {
        assert_zoom_closed(window, "縮放設定，自訂縮放，200%");
        let before = window.find("page-input").value().unwrap().to_string();
        window.scroll("viewer", ScrollDelta::Lines(point(0., -8.)), cx);
        assert_eq!(
            window.find("page-input").value(),
            Some(before.as_str()),
            "ordinary wheel input must not turn a page"
        );
        let bounds = window.find("viewer").bounds();
        window.dispatch_event(
            PlatformInput::Scroll(ScrollEvent {
                position: bounds.center(),
                delta: ScrollDelta::Lines(point(0., 2.)),
                modifiers: Modifiers {
                    control: true,
                    ..Default::default()
                },
                ..Default::default()
            }),
            cx,
        );
        window.render_frame(cx);
        assert_zoom_closed(window, "縮放設定，自訂縮放，220%");
        window.dispatch_event(
            PlatformInput::ModifiersChanged(gpui_kit::ModifiersChangedEvent::default()),
            cx,
        );
        window.press("f", cx);
        window.press("home", cx);
        window.click("mode-continuous", cx);
    })
    .unwrap();
    settle(&mut cx, window);
    snapshot(&mut cx, window, "04-continuous");
    cx.update_window(window, |_, window, cx| {
        window.scroll("viewer", ScrollDelta::Pixels(point(px(0.), px(-700.))), cx);
    })
    .unwrap();
    settle(&mut cx, window);
    assert_ne!(
        page(&mut cx, window),
        "1",
        "continuous scrolling must update the active page"
    );
    cx.update_window(window, |_, window, cx| {
        window.press("end", cx);
        assert_eq!(window.find("page-input").value(), Some("6"));
        window.press("home", cx);
        window.press("]", cx);
    })
    .unwrap();
    let preferences = open_preferences(&mut cx, window, &view);
    reach_preference(&mut cx, window, preferences, "remember-position");
    for selected in [false, true] {
        cx.update_window(preferences, |_, window, cx| {
            assert!(window.find("remember-position").visible());
            window.click("remember-position", cx);
        })
        .unwrap();
        settle_windows(&mut cx, &[window, preferences]);
        cx.update_window(preferences, |_, window, _| {
            assert_eq!(window.find("remember-position").checked(), Some(selected));
        })
        .unwrap();
    }
    for _ in 0..2 {
        cx.update_window(window, |_, window, cx| window.click(("menu", 0usize), cx))
            .unwrap();
        settle(&mut cx, window);
        cx.update_window(window, |_, window, cx| {
            let item = window
                .within("popup-menu")
                .find_by_label(Language::TraditionalChinese.text(Text::IncludeSubfolders));
            let menu = window.find("popup-menu").bounds();
            window.click_at("popup-menu", item.bounds().center() - menu.origin, cx);
        })
        .unwrap();
        settle(&mut cx, window);
    }
    cx.update_window(window, |_, window, _| {
        assert!(window.try_find("recursive-folder").is_none());
    })
    .unwrap();
    reach_preference(&mut cx, window, preferences, "紙色");
    cx.update_window(preferences, |_, window, cx| window.click("紙色", cx))
        .unwrap();
    settle_windows(&mut cx, &[window, preferences]);
    cx.update_window(window, |_, window, cx| {
        window.click("inspector-playback", cx);
        window.click("interval-input", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input("2", cx);
        window.press("enter", cx);
    })
    .unwrap();
    settle(&mut cx, window);
    snapshot(&mut cx, window, "05-settings-rotation");
    let saved = AppSettings::load_from(&settings_path).unwrap();
    assert_eq!(saved.preferences.interval, 2);
    assert_eq!(
        saved.preferences.background,
        mangameeya_rust::settings::Background::Paper
    );
    assert!(saved.preferences.remember_position);
    assert_eq!(saved.bookmarks.values().copied().collect::<Vec<_>>(), [1]);
    assert_eq!(saved.recent.len(), 2);

    // A new document-open event can select a concrete image over saved position.
    sender.send(vec![folder.join("4.png")]).unwrap();
    settle(&mut cx, window);
    assert_eq!(page(&mut cx, window), "4");
    sender
        .send(vec![temporary.path().join("missing.cbz")])
        .unwrap();
    settle(&mut cx, window);
    assert_eq!(
        page(&mut cx, window),
        "4",
        "a failed open must keep the previous usable book"
    );
    // Native file drops must work over chrome, not only over the reading area.
    cx.update_window(window, |_, window, cx| {
        let position = point(px(30.), px(20.));
        window.dispatch_event(
            PlatformInput::FileDrop(gpui_kit::FileDropEvent::Entered {
                position,
                paths: gpui_kit::ExternalPaths(vec![folder.join("2.png")].into()),
            }),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            PlatformInput::FileDrop(gpui_kit::FileDropEvent::Submit { position }),
            cx,
        );
    })
    .unwrap();
    settle(&mut cx, window);
    assert_eq!(
        page(&mut cx, window),
        "2",
        "drops over the toolbar must open documents"
    );

    let long_folder = temporary.path().join("long");
    std::fs::create_dir(&long_folder).unwrap();
    image::RgbaImage::from_pixel(80, 20_000, image::Rgba([160, 70, 180, 255]))
        .save(long_folder.join("long.png"))
        .unwrap();
    sender.send(vec![long_folder]).unwrap();
    settle(&mut cx, window);
    cx.update_window(window, |_, window, cx| window.press("w", cx))
        .unwrap();
    settle(&mut cx, window);
    if let Some(image) = snapshot(&mut cx, window, "06-long-tiled-page") {
        assert!(
            image
                .pixels()
                .filter(|p| p[0].abs_diff(160) <= 2
                    && p[1].abs_diff(70) <= 2
                    && p[2].abs_diff(180) <= 2)
                .count()
                > 10_000
        );
    }
    cx.update_window(window, |_, window, cx| {
        window.scroll(
            "viewer",
            ScrollDelta::Pixels(point(px(0.), px(-100_000.))),
            cx,
        );
    })
    .unwrap();
    settle(&mut cx, window);
    if let Some(image) = snapshot(&mut cx, window, "07-long-page-bottom") {
        assert!(
            image
                .pixels()
                .filter(|p| p[0].abs_diff(160) <= 2
                    && p[1].abs_diff(70) <= 2
                    && p[2].abs_diff(180) <= 2)
                .count()
                > 10_000
        );
    }

    let animated_folder = temporary.path().join("animated");
    std::fs::create_dir(&animated_folder).unwrap();
    let mut encoder = image::codecs::gif::GifEncoder::new(
        std::fs::File::create(animated_folder.join("animation.gif")).unwrap(),
    );
    encoder
        .encode_frames(
            [[220, 40, 50, 255], [40, 80, 220, 255]]
                .into_iter()
                .map(|color| {
                    image::Frame::from_parts(
                        image::RgbaImage::from_pixel(160, 240, image::Rgba(color)),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(500, 1),
                    )
                }),
        )
        .unwrap();
    drop(encoder);
    sender.send(vec![animated_folder]).unwrap();
    settle(&mut cx, window);
    cx.update_window(window, |_, window, cx| window.press("f", cx))
        .unwrap();
    settle(&mut cx, window);
    let sample_point = cx
        .update_window(window, |_, window, _| {
            let bounds = window.find("viewer").bounds();
            (f32::from(bounds.center().x), f32::from(bounds.top()) + 60.)
        })
        .unwrap();
    cx.update_window(window, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    cx.update_window(window, |_, window, _| {
        assert!(
            window.is_window_active(),
            "animation requires an active reader"
        );
    })
    .unwrap();
    let before_animation = snapshot(&mut cx, window, "08-animation-first");
    if let Some(before) = before_animation {
        let sample = |image: &image::RgbaImage| {
            let scale = image.width() as f32 / 1280.;
            image
                .get_pixel(
                    (sample_point.0 * scale) as u32,
                    (sample_point.1 * scale) as u32,
                )
                .0
        };
        let colors = [[220u8, 40, 50, 255], [40u8, 80, 220, 255]];
        let source_frame = |pixel: [u8; 4]| {
            colors
                .iter()
                .position(|color| color.iter().zip(pixel).all(|(a, b)| a.abs_diff(b) <= 2))
        };
        let first = sample(&before);
        let first_frame = source_frame(first).expect("animated source color must reach the scene");
        // A fixed sleep can sample the same phase when rendering or animation
        // callbacks are delayed. Render and observe a different decoded source
        // frame within a fixed bound instead of assuming one exact phase.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut advanced = false;
        while std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(73));
            cx.update_window(window, |_, window, cx| window.render_frame(cx))
                .unwrap();
            let after = snapshot(&mut cx, window, "09-animation-next").unwrap();
            let next = sample(&after);
            let next_frame =
                source_frame(next).expect("animation must keep painting decoded source colors");
            if next_frame != first_frame {
                advanced = true;
                break;
            }
        }
        assert!(
            advanced,
            "animated page source pixels must advance in the real Metal renderer"
        );
    }

    cx.update_window(window, |_, window, cx| {
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            cx,
        );
        assert_eq!(window.find("next").disabled(), Some(true));
        assert_eq!(window.find("zoom-trigger").disabled(), Some(true));
        assert!(window.try_find("zoom-panel").is_none());
        assert!(window.try_find("zoom-input").is_none());
    })
    .unwrap();
    settle(&mut cx, window);
    drop(view);
    println!(
        "Native reader UI checks passed: launch, decoded pixels, navigation, spread, bookmarks, input focus, zoom, wheel, continuous pages, rotation, preferences, language switching, translated labels and menus, persistence, errors, and close."
    );
}
