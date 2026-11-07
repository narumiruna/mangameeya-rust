//! Native desktop layout and event regressions for the approved efficiency plan.

use super::*;
use gpui_kit::{Modifiers, PlatformInput, ScrollDelta, ScrollEvent, test::TestEventExt};

const SIZES: [(f32, f32); 3] = [(1280., 800.), (960., 640.), (800., 540.)];
const MODES: [(&str, ReadingMode); 3] = [
    ("mode-single", ReadingMode::Single),
    ("mode-spread", ReadingMode::Spread),
    ("mode-continuous", ReadingMode::Continuous),
];
const TABS: [(&str, InspectorTab); 3] = [
    ("inspector-image", InspectorTab::Image),
    ("inspector-reading", InspectorTab::Reading),
    ("inspector-playback", InspectorTab::Playback),
];

fn replace_input(window: &mut Window, id: &'static str, draft: &str, cx: &mut App) {
    window.click(id, cx);
    window.press(
        if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        },
        cx,
    );
    window.input(draft, cx);
    assert_eq!(window.find(id).value(), Some(draft));
}

fn progress_label(percent: u32) -> String {
    Language::TraditionalChinese.format(Text::ReadProgress, &[("percent", percent.to_string())])
}

fn resize(fixture: &Fixture, width: f32, height: f32, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        window.resize(size(px(width), px(height)));
        window.bounds_changed(cx);
    })
    .unwrap();
    pump(fixture, cx);
}

fn inside(inner: Bounds<Pixels>, outer: Bounds<Pixels>) {
    assert!(
        inner.size.width > px(0.)
            && inner.size.height > px(0.)
            && inner.left() >= outer.left() - px(0.5)
            && inner.top() >= outer.top() - px(0.5)
            && inner.right() <= outer.right() + px(0.5)
            && inner.bottom() <= outer.bottom() + px(0.5),
        "{inner:?} must be inside {outer:?}"
    );
}

fn disjoint(a: Bounds<Pixels>, b: Bounds<Pixels>) {
    assert!(
        a.right() <= b.left() + px(0.5)
            || b.right() <= a.left() + px(0.5)
            || a.bottom() <= b.top() + px(0.5)
            || b.bottom() <= a.top() + px(0.5),
        "controls overlap: {a:?} and {b:?}"
    );
}

fn distinct_controls(window: &Window, ids: &[&'static str], bounds: Bounds<Pixels>) {
    let controls: Vec<_> = ids
        .iter()
        .map(|id| {
            let control = window.find(*id);
            assert!(control.visible(), "{id} must be visible");
            assert!(
                !control.label().unwrap_or_default().is_empty(),
                "{id} needs an accessible name"
            );
            inside(control.bounds(), bounds);
            control.bounds()
        })
        .collect();
    for (index, first) in controls.iter().enumerate() {
        for second in &controls[index + 1..] {
            disjoint(*first, *second);
        }
    }
}

fn assert_layout(window: &Window) {
    let viewport = Bounds::new(Default::default(), window.viewport_size());
    let viewer = window.find("viewer").bounds();
    assert!(viewer.size.width >= px(280.), "reader width: {viewer:?}");
    inside(viewer, viewport);
    let toolbar = window.find("reader-toolbar").bounds();
    let status = window.find("reader-status").bounds();
    inside(toolbar, viewport);
    inside(status, viewport);
    disjoint(toolbar, viewer);
    disjoint(status, viewer);
    distinct_controls(
        window,
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
        ],
        toolbar,
    );
    distinct_controls(
        window,
        &[
            "previous",
            "page-input",
            "next",
            "reading-progress",
            "info-trigger",
        ],
        status,
    );
    for id in ["previous", "next", "page-input", "zoom-trigger"] {
        assert_eq!(
            window.find_all(id).len(),
            1,
            "{id} must have one entry point"
        );
    }
    if let Some(sidebar) = window.try_find("sidebar") {
        inside(sidebar.bounds(), viewport);
        disjoint(sidebar.bounds(), viewer);
        distinct_controls(window, &["檔案", "縮圖", "最近開啟"], sidebar.bounds());
    }
    if let Some(inspector) = window.try_find("inspector") {
        inside(inspector.bounds(), viewport);
        disjoint(inspector.bounds(), viewer);
        distinct_controls(
            window,
            &["inspector-image", "inspector-reading", "inspector-playback"],
            inspector.bounds(),
        );
        if let Some(sidebar) = window.try_find("sidebar") {
            disjoint(sidebar.bounds(), inspector.bounds());
        }
    }
}

fn drag_panel(window: &mut Window, left: bool, delta: f32, cx: &mut App) {
    let viewer = window.find("viewer").bounds();
    let x = if left {
        (window.find("sidebar").bounds().right() + viewer.left()) / 2.
    } else {
        (viewer.right() + window.find("inspector").bounds().left()) / 2.
    };
    let start = point(x, viewer.center().y);
    window.drag(start, point(x + px(delta), start.y), cx);
}

fn click_recursive(window: &mut Window, language: Language, cx: &mut App) {
    window.click(("menu", 0usize), cx);
    let item = window
        .within("popup-menu")
        .find_by_label(language.text(Text::IncludeSubfolders));
    let menu = window.find("popup-menu").bounds();
    window.click_at("popup-menu", item.bounds().center() - menu.origin, cx);
}

fn ctrl_wheel(window: &mut Window, position: gpui_kit::Point<Pixels>, cx: &mut App) {
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
    window.dispatch_event(
        PlatformInput::ModifiersChanged(gpui_kit::ModifiersChangedEvent::default()),
        cx,
    );
}

#[gpui_kit::test]
fn desktop_all_locales_sizes_and_tabs_keep_controls_and_reader_separate(cx: &mut TestAppContext) {
    let fixture = fixture_with_settings(cx, AppSettings::default());
    let path = book(&fixture, "desktop-layout", 6);
    fixture.sender.send(vec![path]).unwrap();
    pump(&fixture, cx);
    for language in Language::ALL {
        resize(&fixture, 1280., 800., cx);
        cx.update_window(fixture.window, |_, window, cx| {
            window.click("preferences", cx);
            window.click(
                SharedString::from(format!("language-{}", language.code())),
                cx,
            );
        })
        .unwrap();
        for (width, height) in SIZES {
            resize(&fixture, width, height, cx);
            for (id, tab) in TABS {
                cx.update_window(fixture.window, |_, window, cx| {
                    window.click(id, cx);
                    assert_layout(window);
                    assert_eq!(fixture.view.read(cx).inspector_tab, tab);
                    for (choice, selected) in TABS {
                        assert_eq!(window.find(choice).checked(), Some(selected == tab));
                    }
                    if tab == InspectorTab::Reading {
                        window.scroll("inspector", ScrollDelta::Lines(point(0., -100.)), cx);
                        for choice in Language::ALL {
                            assert!(
                                window
                                    .find(SharedString::from(format!("language-{}", choice.code())))
                                    .visible()
                            );
                        }
                        assert_layout(window);
                    }
                    for (choice, mode) in MODES {
                        assert_eq!(window.find(choice).role(), Some(gpui_kit::Role::Tab));
                        assert_eq!(
                            window.find(choice).checked(),
                            Some(mode == ReadingMode::Single)
                        );
                    }
                    for index in 0..7usize {
                        let bounds = window.find(("menu", index)).bounds();
                        inside(
                            bounds,
                            Bounds::new(Default::default(), window.viewport_size()),
                        );
                        for other in index + 1..7 {
                            disjoint(bounds, window.find(("menu", other)).bounds());
                        }
                    }
                })
                .unwrap();
            }
        }
    }
}

#[gpui_kit::test]
fn desktop_mode_clicks_are_explicit_idempotent_and_preserve_every_fit_and_zoom(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "desktop-modes", 6)])
        .unwrap();
    pump(&fixture, cx);
    for (fit_id, fit) in zoom::FIT_CHOICES {
        cx.update_window(fixture.window, |_, window, cx| {
            window.click("zoom-trigger", cx);
            window.click(fit_id, cx);
        })
        .unwrap();
        for (id, mode) in MODES {
            cx.update_window(fixture.window, |_, window, cx| {
                let before = fixture.view.read(cx).reader.zoom;
                for _ in 0..2 {
                    window.click(id, cx);
                    let view = fixture.view.read(cx);
                    assert_eq!(view.reader.settings.preferences.mode, mode);
                    assert_eq!(view.reader.settings.preferences.fit, fit);
                    assert_eq!(view.reader.zoom, before);
                    assert_eq!(view.reader.current, 0);
                    for (choice, selected) in MODES {
                        assert_eq!(window.find(choice).checked(), Some(selected == mode));
                    }
                }
            })
            .unwrap();
        }
    }
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("zoom-trigger", cx)
    })
    .unwrap();
    zoom::submit_draft(&fixture, "175", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        for (id, mode) in MODES {
            window.click(id, cx);
            window.click(id, cx);
            let view = fixture.view.read(cx);
            assert_eq!(view.reader.settings.preferences.mode, mode);
            assert_eq!(view.reader.settings.preferences.fit, FitMode::Custom);
            assert_eq!(view.reader.zoom, 1.75);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn desktop_s_c_d_shortcuts_keep_toggle_direction_and_page_order_semantics(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "desktop-keys", 6)])
        .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("viewer", cx);
        for (key, mode) in [
            ("s", ReadingMode::Spread),
            ("s", ReadingMode::Single),
            ("c", ReadingMode::Continuous),
            ("c", ReadingMode::Single),
        ] {
            window.press(key, cx);
            assert_eq!(fixture.view.read(cx).reader.settings.preferences.mode, mode);
        }
        window.click("mode-spread", cx);
        assert_eq!(fixture.view.read(cx).reader.display_indices(), [1, 0]);
        window.press("left", cx);
        assert_eq!(fixture.view.read(cx).reader.current, 2);
        window.press("right", cx);
        assert_eq!(fixture.view.read(cx).reader.current, 0);
        window.press("d", cx);
        assert!(!fixture.view.read(cx).reader.settings.preferences.rtl);
        assert_eq!(fixture.view.read(cx).reader.display_indices(), [0, 1]);
        window.press("right", cx);
        assert_eq!(fixture.view.read(cx).reader.current, 2);
        window.press("left", cx);
        assert_eq!(fixture.view.read(cx).reader.current, 0);
        window.click("direction", cx);
        assert!(fixture.view.read(cx).reader.settings.preferences.rtl);
        window.click("next", cx);
        assert_eq!(fixture.view.read(cx).reader.current, 2);
        window.click("previous", cx);
        assert_eq!(fixture.view.read(cx).reader.current, 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn desktop_panels_resize_independently_persist_and_restore_in_a_new_view(cx: &mut TestAppContext) {
    let mut fixture = fixture(cx);
    pump(&fixture, cx);
    let identity = fixture
        .view
        .read_with(cx, |view, _| view.panel_state.entity_id());
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(window.find("sidebar").bounds().size.width, px(224.));
        assert_eq!(window.find("inspector").bounds().size.width, px(256.));
        drag_panel(window, true, 48., cx);
        drag_panel(window, false, -44., cx);
    })
    .unwrap();
    pump(&fixture, cx);
    let expected = fixture.view.read_with(cx, |view, _| {
        let layout = view.reader.settings.preferences.panel_layout.clone();
        assert!((271..=273).contains(&layout.sidebar_width), "{layout:?}");
        assert!((299..=301).contains(&layout.inspector_width), "{layout:?}");
        layout
    });
    for (toggle, hidden, retained) in [
        ("sidebar-toggle", "sidebar", "inspector"),
        ("inspector-toggle", "inspector", "sidebar"),
    ] {
        cx.update_window(fixture.window, |_, window, cx| {
            let width = window.find(retained).bounds().size.width;
            window.click(toggle, cx);
            assert!(window.try_find(hidden).is_none());
            assert_eq!(window.find(retained).bounds().size.width, width);
            assert_eq!(fixture.view.read(cx).panel_state.entity_id(), identity);
            window.click(toggle, cx);
            assert!(window.find(hidden).visible());
            assert_eq!(
                fixture
                    .view
                    .read(cx)
                    .reader
                    .settings
                    .preferences
                    .panel_layout,
                expected
            );
        })
        .unwrap();
    }
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("sidebar-toggle", cx);
        window.click("inspector-toggle", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    let settings_path = fixture.directory.path().join("settings.json");
    let saved = AppSettings::load_from(&settings_path).unwrap();
    assert!(!saved.preferences.sidebar);
    assert!(!saved.preferences.panel_layout.inspector_visible);
    assert_eq!(
        saved.preferences.panel_layout.sidebar_width,
        expected.sidebar_width
    );
    assert_eq!(
        saved.preferences.panel_layout.inspector_width,
        expected.inspector_width
    );
    cx.update_window(fixture.window, |_, window, _| window.remove_window())
        .unwrap();
    let (sender, launches) = mpsc::channel();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(1280.), px(800.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| ReaderView::new(saved, settings_path, None, None, launches, window, cx))
            },
        )
        .unwrap()
    });
    fixture.window = window;
    fixture.view = view;
    fixture.sender = sender;
    pump(&fixture, cx);
    cx.update_window(window, |_, window, cx| {
        assert!(window.try_find("sidebar").is_none());
        assert!(window.try_find("inspector").is_none());
        window.click("sidebar-toggle", cx);
        window.click("inspector-toggle", cx);
        assert!(
            (f32::from(window.find("sidebar").bounds().size.width) - expected.sidebar_width as f32)
                .abs()
                < 1.
        );
        assert!(
            (f32::from(window.find("inspector").bounds().size.width)
                - expected.inspector_width as f32)
                .abs()
                < 1.
        );
        assert_eq!(fixture.view.read(cx).inspector_tab, InspectorTab::Image);
    })
    .unwrap();
}

#[gpui_kit::test]
fn desktop_window_limits_restore_preferences_and_drag_limits_keep_reader_minimum(
    cx: &mut TestAppContext,
) {
    let mut settings = AppSettings::default();
    settings.preferences.panel_layout.sidebar_width = 320;
    settings.preferences.panel_layout.inspector_width = 360;
    let fixture = fixture_with_settings(cx, settings);
    pump(&fixture, cx);
    let path = fixture.directory.path().join("settings.json");
    cx.update_window(fixture.window, |_, _, cx| {
        fixture
            .view
            .update(cx, |view, _| view.flush_settings().unwrap())
    })
    .unwrap();
    let original = std::fs::read(&path).unwrap();
    for (width, height) in [(960., 640.), (800., 540.), (1280., 800.)] {
        resize(&fixture, width, height, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            assert_layout(window);
            let layout = &fixture
                .view
                .read(cx)
                .reader
                .settings
                .preferences
                .panel_layout;
            assert_eq!((layout.sidebar_width, layout.inspector_width), (320, 360));
            let right = window.find("inspector").bounds().size.width;
            let left = window.find("sidebar").bounds().size.width;
            if width == 960. {
                assert!(
                    (f32::from(left) - 320.).abs() < 1.,
                    "right must shrink first"
                );
                assert!(right < px(360.));
            } else if width == 800. {
                assert!((f32::from(right) - 240.).abs() < 1.);
                assert!((px(216.)..px(320.)).contains(&left));
            } else {
                assert_eq!((left, right), (px(320.), px(360.)));
            }
        })
        .unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            original,
            "window resize must not save effective widths"
        );
    }
    for (left, delta, expected) in [
        (true, -2000., 216),
        (true, 2000., 320),
        (false, 2000., 240),
        (false, -2000., 360),
    ] {
        cx.update_window(fixture.window, |_, window, cx| {
            drag_panel(window, left, delta, cx)
        })
        .unwrap();
        pump(&fixture, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            assert_layout(window);
            let layout = &fixture
                .view
                .read(cx)
                .reader
                .settings
                .preferences
                .panel_layout;
            assert_eq!(
                if left {
                    layout.sidebar_width
                } else {
                    layout.inspector_width
                },
                expected
            );
        })
        .unwrap();
    }
    resize(&fixture, 800., 540., cx);
    for (left, delta) in [(true, 2000.), (false, -2000.)] {
        cx.update_window(fixture.window, |_, window, cx| {
            drag_panel(window, left, delta, cx);
            assert_layout(window);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn desktop_panel_drag_saves_only_after_release_and_only_when_width_changes(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    fixture
        .view
        .update(cx, |view, _| view.flush_settings().unwrap());
    let path = fixture.directory.path().join("settings.json");
    let before = std::fs::read(&path).unwrap();
    let epoch = fixture
        .view
        .read_with(cx, |view, _| view.save_epoch.load(Ordering::SeqCst));
    cx.update_window(fixture.window, |_, window, cx| {
        let start = point(
            window.find("viewer").bounds().left(),
            window.find("viewer").bounds().center().y,
        );
        window.pointer_move(start, None, cx);
        window.pointer_down(start, MouseButton::Left, cx);
        for delta in [12., 24., 48.] {
            window.pointer_move(
                point(start.x + px(delta), start.y),
                Some(MouseButton::Left),
                cx,
            );
            fixture.view.update(cx, |view, cx| view.tick(window, cx));
            assert_eq!(
                fixture
                    .view
                    .read(cx)
                    .reader
                    .settings
                    .preferences
                    .panel_layout
                    .sidebar_width,
                224
            );
            assert_eq!(
                fixture.view.read(cx).save_epoch.load(Ordering::SeqCst),
                epoch
            );
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
        window.pointer_up(point(start.x + px(48.), start.y), MouseButton::Left, cx);
    })
    .unwrap();
    pump(&fixture, cx);
    let saved = AppSettings::load_from(&path).unwrap();
    assert!((271..=273).contains(&saved.preferences.panel_layout.sidebar_width));
    let after = std::fs::read(&path).unwrap();
    let epoch = fixture
        .view
        .read_with(cx, |view, _| view.save_epoch.load(Ordering::SeqCst));
    cx.update_window(fixture.window, |_, window, cx| {
        drag_panel(window, true, 0., cx)
    })
    .unwrap();
    pump(&fixture, cx);
    assert_eq!(std::fs::read(&path).unwrap(), after);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.save_epoch.load(Ordering::SeqCst), epoch)
    });
}

#[gpui_kit::test]
fn desktop_tabs_preserve_input_slider_entities_values_and_filter_reset_scope(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "desktop-tools", 3)])
        .unwrap();
    pump(&fixture, cx);
    let entities = fixture.view.read_with(cx, |view, _| {
        (
            view.interval_input.entity_id(),
            view.page_input.entity_id(),
            view.path_input.entity_id(),
            view.zoom_input.entity_id(),
            view.filter_sliders.each_ref().map(Entity::entity_id),
        )
    });
    cx.update_window(fixture.window, |_, window, cx| {
        for text in [Text::Brightness, Text::Contrast, Text::Grayscale] {
            let bounds = window
                .within("inspector")
                .find_by_label(Language::TraditionalChinese.text(text))
                .bounds();
            window.drag(
                bounds.center(),
                point(bounds.left() + bounds.size.width * 0.7, bounds.center().y),
                cx,
            );
        }
        window.click("rotate-right", cx);
        window.click("inspector-playback", cx);
        replace_input(window, "interval-input", "017", cx);
        window.click("inspector-reading", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    let (filters, preferences) = fixture.view.read_with(cx, |view, cx| {
        assert_ne!(view.reader.filters, Filters::default());
        assert_eq!(view.reader.rotation, 90);
        assert_eq!(view.reader.settings.preferences.interval, 17);
        assert_eq!(view.interval_input.read(cx).value().as_ref(), "017");
        (
            view.reader.filters,
            view.reader.settings.preferences.clone(),
        )
    });
    cx.update_window(fixture.window, |_, window, cx| {
        for (id, tab) in TABS.into_iter().cycle().take(9) {
            window.click(id, cx);
            let view = fixture.view.read(cx);
            assert_eq!(view.inspector_tab, tab);
            assert_eq!(
                (
                    view.interval_input.entity_id(),
                    view.page_input.entity_id(),
                    view.path_input.entity_id(),
                    view.zoom_input.entity_id(),
                    view.filter_sliders.each_ref().map(Entity::entity_id)
                ),
                entities
            );
            assert_eq!(view.reader.filters, filters);
            assert_eq!(view.reader.rotation, 90);
            assert_eq!(view.interval_input.read(cx).value().as_ref(), "017");
            assert_eq!(
                window.try_find("reset-filters").is_some(),
                tab == InspectorTab::Image
            );
            assert_eq!(
                window.try_find("remember-position").is_some(),
                tab == InspectorTab::Reading
            );
            assert_eq!(
                window.try_find("language-en").is_some(),
                tab == InspectorTab::Reading
            );
            assert_eq!(
                window.try_find("slideshow").is_some(),
                tab == InspectorTab::Playback
            );
            assert_eq!(
                window.try_find("interval-input").is_some(),
                tab == InspectorTab::Playback
            );
        }
        window.click("inspector-image", cx);
        window.click("reset-filters", cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.reader.filters, Filters::default());
        assert_eq!(view.reader.rotation, 90);
        assert_eq!(view.reader.settings.preferences, preferences);
    })
    .unwrap();
}

#[gpui_kit::test]
fn desktop_preferences_is_idempotent_and_hiding_focused_tools_returns_focus(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("inspector-toggle", cx);
        assert!(window.try_find("inspector").is_none());
        for _ in 0..2 {
            window.click("preferences", cx);
            assert!(window.find("remember-position").visible());
            assert_eq!(fixture.view.read(cx).inspector_tab, InspectorTab::Reading);
            assert!(
                fixture
                    .view
                    .read(cx)
                    .reader
                    .settings
                    .preferences
                    .panel_layout
                    .inspector_visible
            );
        }
        window.click(("menu", 5usize), cx);
        let item = window
            .within("popup-menu")
            .find_by_label(Language::TraditionalChinese.text(Text::Preferences));
        let bounds = window.find("popup-menu").bounds();
        window.click_at("popup-menu", item.bounds().center() - bounds.origin, cx);
        assert_eq!(fixture.view.read(cx).inspector_tab, InspectorTab::Reading);
        window.click("inspector-playback", cx);
        window.click("interval-input", cx);
        assert_eq!(window.find("interval-input").focused(), Some(true));
        window.click("inspector-toggle", cx);
        assert!(window.try_find("interval-input").is_none());
        let view = fixture.view.read(cx);
        assert!(
            !view
                .interval_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        assert!(
            view.focus.is_focused(window)
                || window.find("inspector-toggle").focused() == Some(true)
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn desktop_native_preferences_moves_focus_out_of_hidden_playback_input(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("inspector-playback", cx);
        replace_input(window, "interval-input", "19", cx);
        window.dispatch_action(Box::new(Command::Settings), cx);
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.inspector_tab, InspectorTab::Reading);
        assert!(
            !view
                .interval_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        );
        assert!(view.focus.is_focused(window));
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.settings.preferences.interval, 19)
    });
}

#[gpui_kit::test]
fn desktop_recursive_option_is_in_file_menu_only_and_applies_on_next_open(cx: &mut TestAppContext) {
    let mut settings = AppSettings::default();
    settings.preferences.language = Language::TraditionalChinese;
    settings.preferences.recursive_folder = false;
    let fixture = fixture_with_settings(cx, settings);
    let folder = book(&fixture, "recursive", 1);
    let nested = folder.join("nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::copy(folder.join("1.png"), nested.join("2.png")).unwrap();
    fixture.sender.send(vec![folder.clone()]).unwrap();
    pump(&fixture, cx);
    let revision = fixture
        .view
        .read_with(cx, |view, _| (view.book_revision, view.open_revision));
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("preferences", cx);
        assert!(window.try_find("recursive-folder").is_none());
        assert!(
            window
                .within("inspector")
                .try_find_by_label(Language::TraditionalChinese.text(Text::IncludeSubfolders))
                .is_none()
        );
        assert_eq!(fixture.view.read(cx).page_count(), 1);
        click_recursive(window, Language::TraditionalChinese, cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.reader.settings.preferences.recursive_folder);
        assert_eq!(view.page_count(), 1);
        assert_eq!((view.book_revision, view.open_revision), revision);
    });
    fixture.sender.send(vec![folder]).unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.page_count(), 2);
        assert!(view.book_revision > revision.0);
    });
}

#[gpui_kit::test]
fn desktop_page_entry_applies_on_enter_progress_rounds_and_boundaries_are_safe(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        for id in ["previous", "next", "info-trigger"] {
            assert_eq!(window.find(id).disabled(), Some(true), "{id}");
        }
        assert!(
            fixture
                .view
                .read(cx)
                .page_input
                .read(cx)
                .presentation()
                .is_disabled()
        );
        window.click("page-input", cx);
        window.input("999", cx);
        window.press("enter", cx);
        assert_eq!(window.find("page-input").value(), Some("—"));
        assert_eq!(fixture.view.read(cx).reader.current, 0);
        assert_eq!(window.find("reading-progress").label(), Some("—"));
    })
    .unwrap();
    fixture
        .sender
        .send(vec![book(&fixture, "progress", 6)])
        .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(
            window.find("reading-progress").label(),
            Some(progress_label(17).as_str())
        );
        assert_eq!(window.find("previous").disabled(), Some(true));
        replace_input(window, "page-input", "4", cx);
        assert_eq!(fixture.view.read(cx).reader.current, 0);
        window.press("tab", cx);
        assert_eq!(
            fixture.view.read(cx).reader.current,
            0,
            "Blur must not apply page entry"
        );
        window.click("page-input", cx);
        window.press("enter", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(fixture.view.read(cx).reader.current, 3);
        assert_eq!(
            window.find("reading-progress").label(),
            Some(progress_label(67).as_str())
        );
        window.press("end", cx);
        assert_eq!(
            window.find("reading-progress").label(),
            Some(progress_label(100).as_str())
        );
        assert_eq!(window.find("next").disabled(), Some(true));
        window.click("next", cx);
        assert_eq!(fixture.view.read(cx).reader.current, 5);
        window.press("home", cx);
        window.click("previous", cx);
        assert_eq!(fixture.view.read(cx).reader.current, 0);
        replace_input(window, "page-input", "999999", cx);
        window.press("enter", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture
        .view
        .read_with(cx, |view, _| assert_eq!(view.reader.current, 5));
    fixture
        .sender
        .send(vec![book(&fixture, "one-page", 1)])
        .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(
            window.find("reading-progress").label(),
            Some(progress_label(100).as_str())
        );
        for id in ["previous", "next"] {
            assert_eq!(window.find(id).disabled(), Some(true));
            window.click(id, cx);
        }
        assert_eq!(fixture.view.read(cx).reader.current, 0);
        assert!(!fixture.view.read(cx).message_error);
    })
    .unwrap();
}

#[gpui_kit::test]
fn desktop_information_updates_real_data_and_closes_on_replacement_and_close(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "information", 2);
    let name =
        "2-a-very-long-日本語-正體中文-filename-that-must-not-be-truncated-in-information.png";
    std::fs::rename(folder.join("2.png"), folder.join(name)).unwrap();
    image::RgbaImage::from_pixel(240, 160, image::Rgba([80, 160, 220, 255]))
        .save(folder.join(name))
        .unwrap();
    let replacement = book(&fixture, "replacement", 1);
    fixture.sender.send(vec![folder.clone()]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("info-trigger", cx);
        assert!(window.find("info-panel").visible());
        assert_eq!(window.find("info-name").label(), Some("1.png"));
        assert_eq!(window.find("info-dimensions").label(), Some("120×200"));
        let bytes = std::fs::metadata(folder.join("1.png")).unwrap().len();
        let expected = format!("{:.1} KiB", bytes as f32 / 1024.);
        assert_eq!(window.find("info-size").label(), Some(expected.as_str()));
        fixture
            .view
            .update(cx, |view, cx| view.navigate(1, window, cx));
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert!(fixture.view.read(cx).info_open);
        assert_eq!(window.find("info-name").label(), Some(name));
        assert_eq!(window.find("info-dimensions").label(), Some("240×160"));
        let bytes = std::fs::metadata(folder.join(name)).unwrap().len();
        let expected = format!("{:.1} KiB", bytes as f32 / 1024.);
        assert_eq!(window.find("info-size").label(), Some(expected.as_str()));
        fixture.view.update(cx, |view, cx| {
            view.open_path(replacement.clone(), window, cx)
        });
        window.render_frame(cx);
        assert!(!fixture.view.read(cx).info_open);
        assert!(window.try_find("info-panel").is_none());
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(
            fixture.view.read(cx).reader.book.as_ref().unwrap().path,
            replacement.canonicalize().unwrap()
        );
        assert!(!fixture.view.read(cx).info_open);
        assert!(window.try_find("info-panel").is_none());
        window.click("info-trigger", cx);
        assert_eq!(window.find("info-name").label(), Some("1.png"));
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            cx,
        );
        assert!(!fixture.view.read(cx).info_open);
        assert!(window.try_find("info-panel").is_none());
        assert!(window.try_find("info-name").is_none());
        assert_eq!(window.find("info-trigger").disabled(), Some(true));
        assert!(fixture.view.read(cx).focus.is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn desktop_unknown_information_does_not_fabricate_dimensions_or_decode_more_pages(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "unknown-info", 40);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.reader
                .set_book(Arc::new(Book::open(&folder, false).unwrap()), None);
            view.errors.insert((0, false), "Invalid image data".into());
            cx.notify();
        });
        window.render_frame(cx);
        let inflight = fixture.view.read(cx).inflight.clone();
        window.click("info-trigger", cx);
        assert_eq!(window.find("info-name").label(), Some("1.png"));
        let dimensions = window.find("info-dimensions");
        let bytes = window.find("info-size");
        assert_eq!(
            dimensions.label(),
            Some(Language::TraditionalChinese.text(Text::NotAvailable))
        );
        assert_eq!(
            bytes.label(),
            Some(Language::TraditionalChinese.text(Text::NotAvailable))
        );
        assert_eq!(fixture.view.read(cx).inflight, inflight);
        assert!(fixture.view.read(cx).pages.is_empty());
        assert!(fixture.view.read(cx).geometry.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn desktop_thumbnail_frames_names_selection_and_virtualization_survive_navigation(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "desktop-thumbnails", 40);
    let long_name = "1-a-very-long-日本語-正體中文-thumbnail-filename.png";
    std::fs::rename(folder.join("1.png"), folder.join(long_name)).unwrap();
    for (name, width, height) in [(long_name, 600, 100), ("2.png", 100, 600)] {
        image::RgbaImage::from_pixel(width, height, image::Rgba([80, 160, 220, 255]))
            .save(folder.join(name))
            .unwrap();
    }
    fixture.sender.send(vec![folder]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        let view = fixture.view.read(cx);
        let book = view.reader.book.as_ref().unwrap();
        for index in 0..2 {
            let id = SharedString::from(format!(
                "thumb:{}:{}",
                book.path.display(),
                book.pages[index].name
            ));
            let button = window.find(id.clone());
            assert_eq!(button.checked(), Some(index == 0));
            let scope = window.within(id);
            let container = scope.find("thumbnail-container").bounds();
            assert_eq!(container.size, size(px(110.), px(156.)));
            inside(container, button.bounds());
            let number = scope.find(("thumbnail-page", index));
            let name = scope.find(("thumbnail-name", index));
            assert_eq!(number.label(), Some((index + 1).to_string().as_str()));
            assert_eq!(name.label(), Some(book.pages[index].name.as_str()));
            inside(number.bounds(), button.bounds());
            inside(name.bounds(), button.bounds());
            disjoint(number.bounds(), name.bounds());
            disjoint(container, number.bounds());
            let thumb = &view.thumbs[&index];
            let expected = if index == 0 { 6. } else { 1. / 6. };
            assert!((thumb.width as f32 / thumb.height as f32 - expected).abs() < 0.02);
        }
        assert!(view.visible_thumbnails.borrow().len() < 10);
        assert!(
            view.thumbs.len() < 40,
            "thumbnail rendering must remain virtualized"
        );
        window.click("mode-continuous", cx);
        window.click("zoom-trigger", cx);
        window.click("zoom-original", cx);
        window.click("viewer", cx);
        window.press("end", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        let view = fixture.view.read(cx);
        assert_eq!(view.reader.current, 39);
        let book = view.reader.book.as_ref().unwrap();
        let id = SharedString::from(format!(
            "thumb:{}:{}",
            book.path.display(),
            book.pages[39].name
        ));
        assert!(
            window.find(id.clone()).visible(),
            "selected thumbnail must be revealed"
        );
        assert_eq!(window.find(id).checked(), Some(true));
        let before = view.continuous.logical_scroll_top();
        window.scroll("sidebar", ScrollDelta::Lines(point(0., 30.)), cx);
        let after = fixture.view.read(cx).continuous.logical_scroll_top();
        assert_eq!(before.item_ix, after.item_ix);
        assert_eq!(before.offset_in_item, after.offset_in_item);
        assert_eq!(fixture.view.read(cx).reader.current, 39);
        assert!(fixture.view.read(cx).visible_thumbnails.borrow().len() < 10);
    })
    .unwrap();
}

#[gpui_kit::test]
fn desktop_panel_drags_and_visibility_preserve_continuous_page_and_scroll_anchor(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "desktop-anchor", 6);
    for index in 1..=6 {
        image::RgbaImage::from_pixel(320, 1200, image::Rgba([80, 160, 220, 255]))
            .save(folder.join(format!("{index}.png")))
            .unwrap();
    }
    fixture.sender.send(vec![folder]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("mode-continuous", cx);
        window.click("zoom-trigger", cx);
        window.click("zoom-original", cx);
        replace_input(window, "page-input", "3", cx);
        window.press("enter", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.scroll("viewer", ScrollDelta::Pixels(point(px(0.), px(-150.))), cx);
    })
    .unwrap();
    pump(&fixture, cx);
    let before = fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.current, 2);
        let top = view.continuous.logical_scroll_top();
        assert!(top.offset_in_item > px(0.));
        top
    });
    for change in 0..6 {
        cx.update_window(fixture.window, |_, window, cx| match change {
            0 => drag_panel(window, true, 32., cx),
            1 => drag_panel(window, false, -24., cx),
            2 | 3 => window.click("sidebar-toggle", cx),
            _ => window.click("inspector-toggle", cx),
        })
        .unwrap();
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.reader.current, 2);
            assert_eq!(view.reader.settings.preferences.fit, FitMode::Original);
            let top = view.continuous.logical_scroll_top();
            assert_eq!(top.item_ix, before.item_ix);
            assert!((f32::from(top.offset_in_item - before.offset_in_item)).abs() < 1.);
        });
    }
}

#[gpui_kit::test]
fn desktop_text_inputs_isolate_reading_keys_and_playback_interval_clamps_on_enter_and_blur(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "desktop-inputs", 4)])
        .unwrap();
    pump(&fixture, cx);
    for id in ["page-input", "path-input", "interval-input"] {
        cx.update_window(fixture.window, |_, window, cx| {
            if id == "path-input" {
                window.click("檔案", cx);
            }
            if id == "interval-input" {
                window.click("inspector-playback", cx);
            }
            replace_input(window, id, "17", cx);
            let view = fixture.view.read(cx);
            let before = (
                view.reader.current,
                view.reader.settings.preferences.mode,
                view.reader.settings.preferences.rtl,
                view.reader.settings.preferences.sidebar,
                view.reader.rotation,
                view.reader.slideshow,
                view.reader.settings.bookmarks.clone(),
            );
            for key in [
                "s", "c", "d", "b", "p", "[", "]", "left", "right", "up", "down", "pagedown",
                "pageup",
            ] {
                window.press(key, cx);
                let view = fixture.view.read(cx);
                assert_eq!(
                    (
                        view.reader.current,
                        view.reader.settings.preferences.mode,
                        view.reader.settings.preferences.rtl,
                        view.reader.settings.preferences.sidebar,
                        view.reader.rotation,
                        view.reader.slideshow,
                        view.reader.settings.bookmarks.clone()
                    ),
                    before,
                    "{id} leaked {key}"
                );
                assert_eq!(window.find(id).focused(), Some(true));
            }
            replace_input(window, id, "17", cx);
        })
        .unwrap();
    }
    for (draft, expected, enter) in [("0", 1, true), ("999", 120, false), ("17", 17, true)] {
        cx.update_window(fixture.window, |_, window, cx| {
            replace_input(window, "interval-input", draft, cx);
            if enter {
                window.press("enter", cx);
            } else {
                window.click("inspector-reading", cx);
            }
        })
        .unwrap();
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.reader.settings.preferences.interval, expected)
        });
        cx.update_window(fixture.window, |_, window, cx| {
            window.click("inspector-playback", cx)
        })
        .unwrap();
    }
    cx.update_window(fixture.window, |_, window, cx| {
        replace_input(window, "interval-input", "invalid", cx);
        window.press("enter", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.settings.preferences.interval, 17);
        assert!(view.message_error);
    });
}

#[gpui_kit::test]
fn desktop_noop_constrained_overdrag_preserves_preferred_width_and_settings_bytes(
    cx: &mut TestAppContext,
) {
    let mut settings = AppSettings::default();
    settings.preferences.panel_layout.sidebar_width = 320;
    settings.preferences.panel_layout.inspector_width = 300;
    let fixture = fixture_with_settings(cx, settings);
    pump(&fixture, cx);
    resize(&fixture, 800., 540., cx);
    fixture
        .view
        .update(cx, |view, _| view.flush_settings().unwrap());
    let path = fixture.directory.path().join("settings.json");
    let bytes = std::fs::read(&path).unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(window.find("inspector").bounds().size.width, px(240.));
        drag_panel(window, false, -2000., cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        let prefs = &view.reader.settings.preferences.panel_layout;
        assert_eq!((prefs.sidebar_width, prefs.inspector_width), (320, 300));
    });
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "no visible width change must not save"
    );
    resize(&fixture, 1280., 800., cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("inspector").bounds().size.width, px(300.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn desktop_consecutive_clamped_drags_preserve_untouched_preferences_and_reconcile_overdrag(
    cx: &mut TestAppContext,
) {
    let mut settings = AppSettings::default();
    settings.preferences.panel_layout.sidebar_width = 320;
    settings.preferences.panel_layout.inspector_width = 360;
    let fixture = fixture_with_settings(cx, settings);
    pump(&fixture, cx);
    resize(&fixture, 800., 540., cx);
    for delta in [-22., -10.] {
        let before = cx
            .update_window(fixture.window, |_, window, cx| {
                let width = window.find("sidebar").bounds().size.width;
                drag_panel(window, true, delta, cx);
                width
            })
            .unwrap();
        pump(&fixture, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            let view = fixture.view.read(cx);
            let layout = &view.reader.settings.preferences.panel_layout;
            assert_eq!(
                layout.inspector_width, 360,
                "untouched preferred width must survive"
            );
            assert!((layout.sidebar_width as f32 - f32::from(before) - delta).abs() < 1.);
            assert_layout(window);
        })
        .unwrap();
    }
    resize(&fixture, 1280., 800., cx);
    cx.update_window(fixture.window, |_, window, cx| {
        drag_panel(window, false, -2000., cx)
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        let view = fixture.view.read(cx);
        assert_eq!(
            view.reader
                .settings
                .preferences
                .panel_layout
                .inspector_width,
            360
        );
        assert!(
            (view.panel_state.read(cx).sizes().last().unwrap().as_f32()
                - f32::from(window.find("inspector").bounds().size.width))
            .abs()
                < 1.
        );
    })
    .unwrap();
    let before = fixture.view.read_with(cx, |view, _| {
        view.reader.settings.preferences.panel_layout.sidebar_width
    });
    cx.update_window(fixture.window, |_, window, cx| {
        drag_panel(window, true, 20., cx)
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.reader.settings.preferences.panel_layout.sidebar_width,
            before + 20
        );
        assert_eq!(
            view.reader
                .settings
                .preferences
                .panel_layout
                .inspector_width,
            360
        );
    });
}

#[gpui_kit::test]
fn desktop_information_isolates_keys_wheel_and_coordinates_zoom_and_focus(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "info-events", 4)])
        .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("info-trigger", cx);
        let reader = fixture.view.read(cx);
        let before = (
            reader.reader.current,
            reader.reader.settings.preferences.clone(),
            reader.reader.zoom,
            reader.reader.rotation,
            reader.paged_scroll.offset(),
            reader.horizontal_scroll.offset(),
        );
        for key in [
            "space", "left", "right", "up", "down", "pagedown", "pageup", "s", "c", "d", "b", "p",
            "tab", "f", "w", "h", "1", "+", "-", "0",
        ] {
            window.press(key, cx);
            let view = fixture.view.read(cx);
            assert_eq!(
                (
                    view.reader.current,
                    view.reader.settings.preferences.clone(),
                    view.reader.zoom,
                    view.reader.rotation,
                    view.paged_scroll.offset(),
                    view.horizontal_scroll.offset()
                ),
                before,
                "reader received {key}"
            );
            assert!(view.info_open);
        }
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
        ctrl_wheel(window, position, cx);
        assert_eq!(fixture.view.read(cx).reader.zoom, before.2);
        assert_eq!(
            fixture.view.read(cx).reader.settings.preferences.fit,
            before.1.fit
        );
        window.press("escape", cx);
        assert!(!fixture.view.read(cx).info_open);
        assert!(window.try_find("info-panel").is_none());
        assert!(
            fixture.view.read(cx).focus.is_focused(window)
                || window.find("info-trigger").focused() == Some(true)
        );
        window.click("info-trigger", cx);
        window.click("page-input", cx);
        assert!(!fixture.view.read(cx).info_open);
        assert_eq!(window.find("page-input").focused(), Some(true));
        window.click("zoom-trigger", cx);
        assert!(fixture.view.read(cx).zoom_open);
        window.click("info-trigger", cx);
        assert!(!fixture.view.read(cx).zoom_open);
        assert!(fixture.view.read(cx).info_open);
        window.click("zoom-trigger", cx);
        assert!(!fixture.view.read(cx).info_open);
        assert!(fixture.view.read(cx).zoom_open);
        window.press("escape", cx);
    })
    .unwrap();
}
