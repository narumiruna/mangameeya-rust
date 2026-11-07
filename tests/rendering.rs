//! Production-window integration checks. macOS uses the real Metal renderer.
//! Run explicitly with `cargo test --test rendering`.

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
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

fn settle(cx: &mut HeadlessAppContext, window: AnyWindowHandle) {
    for _ in 0..8 {
        cx.advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }
    cx.run_until_parked();
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

fn capture_language_states(cx: &mut HeadlessAppContext, window: AnyWindowHandle) {
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
    cx.update_window(window, |_, window, cx| window.click("preferences", cx))
        .unwrap();
    for (language, id, _, component_locale) in LANGUAGES {
        cx.update_window(window, |_, window, cx| {
            window.click(id, cx);
            assert_eq!(&*gpui_kit::component::locale(), component_locale);
            for (choice, choice_id, label, _) in LANGUAGES {
                assert_eq!(window.find(choice_id).label(), Some(label));
                assert_eq!(window.find(choice_id).checked(), Some(choice == language));
            }
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
        let tabs = ["inspector-image", "inspector-reading", "inspector-playback"];
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
    sender: &mpsc::Sender<Vec<PathBuf>>,
    folder: &std::path::Path,
) {
    for (language, language_id) in [
        (Language::English, "language-en"),
        (Language::Japanese, "language-ja"),
        (Language::TraditionalChinese, "language-zh-Hant"),
    ] {
        cx.update_window(window, |_, window, cx| {
            window.click("preferences", cx);
            window.click(language_id, cx);
        })
        .unwrap();
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
                ("inspector-reading", "reading"),
                ("inspector-playback", "playback"),
            ] {
                cx.update_window(window, |_, window, cx| {
                    window.click(id, cx);
                    assert_desktop_geometry(window, language);
                    for choice in ["inspector-image", "inspector-reading", "inspector-playback"] {
                        assert_eq!(window.find(choice).checked(), Some(choice == id));
                    }
                    assert_eq!(window.try_find("reset-filters").is_some(), name == "image");
                    assert_eq!(
                        window.try_find("remember-position").is_some(),
                        name == "reading"
                    );
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

fn open_reader(
    cx: &mut HeadlessAppContext,
    settings_path: PathBuf,
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
                    ReaderView::new(
                        {
                            let mut settings = AppSettings::default();
                            settings.preferences.language = Language::TraditionalChinese;
                            settings
                        },
                        settings_path,
                        None,
                        None,
                        launches,
                        window,
                        cx,
                    )
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
    capture_language_states(&mut cx, window);
    capture_desktop_states(&mut cx, window, &sender, &folder);

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
        window.click("preferences", cx);
        assert!(window.find("remember-position").visible());
        window.click("remember-position", cx);
        window.click("remember-position", cx);
    })
    .unwrap();
    settle(&mut cx, window);
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
    cx.update_window(window, |_, window, cx| {
        assert!(window.try_find("recursive-folder").is_none());
        window.scroll("inspector-content", ScrollDelta::Lines(point(0., 100.)), cx);
        window.click("紙色", cx);
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
    let before_animation = snapshot(&mut cx, window, "08-animation-first");
    std::thread::sleep(Duration::from_millis(550));
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();
    let after_animation = snapshot(&mut cx, window, "09-animation-next");
    if let (Some(before), Some(after)) = (before_animation, after_animation) {
        let sample = |image: &image::RgbaImage| {
            let scale = image.width() as f32 / 1280.;
            image
                .get_pixel(
                    (sample_point.0 * scale) as u32,
                    (sample_point.1 * scale) as u32,
                )
                .0
        };
        let first = sample(&before);
        let next = sample(&after);
        assert!(
            first[0].abs_diff(220) <= 2 || first[2].abs_diff(220) <= 2,
            "animated source color must reach the scene: {first:?}"
        );
        assert_ne!(
            first, next,
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
