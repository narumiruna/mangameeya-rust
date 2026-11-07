use super::*;

pub(super) const FIT_CHOICES: [(&str, FitMode); 4] = [
    ("zoom-fit-window", FitMode::Window),
    ("zoom-fit-width", FitMode::Width),
    ("zoom-fit-height", FitMode::Height),
    ("zoom-original", FitMode::Original),
];

fn loaded(cx: &mut TestAppContext, dimensions: &[(u32, u32)]) -> Fixture {
    let fixture = fixture(cx);
    let folder = book(&fixture, "zoom", dimensions.len());
    for (index, &(width, height)) in dimensions.iter().enumerate() {
        image::RgbaImage::from_pixel(width, height, image::Rgba([80, 160, 220, 255]))
            .save(folder.join(format!("{}.png", index + 1)))
            .unwrap();
    }
    fixture.sender.send(vec![folder]).unwrap();
    pump(&fixture, cx);
    fixture
}

fn close_enough(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.00001,
        "expected {expected}, got {actual}"
    );
}

fn trigger(window: &Window, mode: FitMode, percentage: &str) {
    assert_eq!(window.find_all("zoom-trigger").len(), 1);
    let expected = format!(
        "縮放設定，{}，{percentage}",
        fit_label(Language::TraditionalChinese, mode)
    );
    assert_eq!(window.find("zoom-trigger").label(), Some(expected.as_str()));
}

fn replace_draft(window: &mut Window, draft: &str, cx: &mut App) {
    window.click("zoom-input", cx);
    window.press(
        if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        },
        cx,
    );
    window.input(draft, cx);
    assert_eq!(window.find("zoom-input").value(), Some(draft));
}

pub(super) fn submit_draft(fixture: &Fixture, draft: &str, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        replace_draft(window, draft, cx);
        window.press("enter", cx);
    })
    .unwrap();
    // Input subscriptions are delivered after the window update returns.
    pump(fixture, cx);
}

fn focus_trigger(window: &mut Window, cx: &mut App) {
    for _ in 0..64 {
        if window.find("zoom-trigger").focused() == Some(true) {
            return;
        }
        window.focus_next(cx);
        window.render_frame(cx);
    }
    panic!("zoom trigger must be keyboard focusable");
}

fn selected_choice(window: &Window, fit: FitMode) {
    for (id, choice) in FIT_CHOICES {
        assert_eq!(window.find(id).checked(), Some(choice == fit), "{id}");
    }
}

// Page frames add four pixels of padding on each side. Check the rendered
// width, not only agreement between effective_scale and another helper.
fn rendered_scale(window: &Window, view: &ReaderView, index: usize, width: u32) -> f32 {
    let book = view.reader.book.as_ref().unwrap();
    let id = SharedString::from(format!(
        "page:{}:{}",
        book.path.display(),
        book.pages[index].name
    ));
    (f32::from(window.find(id).bounds().size.width) - 8.) / width as f32
}

#[test]
fn percentage_format_distinguishes_missing_tiny_and_unlimited_fitted_scales() {
    for scale in [
        None,
        Some(0.),
        Some(-1.),
        Some(f32::NAN),
        Some(f32::INFINITY),
    ] {
        assert_eq!(zoom_percentage(scale), "—");
    }
    for (scale, expected) in [
        (0.00001, "<1%"),
        (0.00999, "<1%"),
        (0.01, "1%"),
        (0.062, "6%"),
        (0.62, "62%"),
        (9., "900%"),
    ] {
        assert_eq!(zoom_percentage(Some(scale)), expected);
        let numeric = zoom_input_value(scale);
        assert!(!numeric.contains('%') && !numeric.contains('<'));
        let parsed = numeric.parse::<f32>().unwrap();
        assert!(parsed > 0.);
        assert!((parsed / 100. - scale).abs() < 0.0001);
    }
}

#[gpui_kit::test]
fn effective_scale_resolves_geometry_rotation_and_missing_layout(cx: &mut TestAppContext) {
    let fixture = loaded(cx, &[(120, 200)]);
    fixture.view.update(cx, |view, cx| {
        view.viewer_size = size(px(644.), px(424.));
        view.viewer_bounds.size = view.viewer_size;
        view.reader.set_fit(FitMode::Window);
        // Cached dimensions must win over retained geometry.
        view.geometry.insert(
            0,
            PageGeometry {
                width: 300,
                height: 900,
                rotation: 0,
            },
        );
        assert_eq!(view.page_dimensions(0), Some((120, 200)));
        close_enough(view.effective_scale().unwrap(), 1.);
        close_enough(view.page_layout(120, 200, 1).height, 200.);
        view.clear_images(None, cx);
        assert_eq!(view.page_dimensions(0), Some((300, 900)));
        close_enough(view.effective_scale().unwrap(), 400. / 900.);
        view.reader.rotation = 90;
        assert_eq!(view.page_dimensions(0), Some((900, 300)));
        close_enough(view.effective_scale().unwrap(), 620. / 900.);
        view.reader.rotation = 270;
        assert_eq!(view.page_dimensions(0), Some((900, 300)));
        view.reader.rotation = 180;
        assert_eq!(view.page_dimensions(0), Some((300, 900)));

        view.errors.insert((0, true), "thumbnail error".into());
        assert!(view.effective_scale().is_some());
        view.errors.insert((0, false), "page error".into());
        assert_eq!(view.effective_scale(), None);
        view.errors.clear();
        view.viewer_bounds.size.width = px(0.);
        assert_eq!(view.effective_scale(), None);
        view.viewer_bounds.size.width = px(644.);
        view.viewer_bounds.size.height = px(0.);
        assert_eq!(view.effective_scale(), None);
        view.viewer_bounds.size = view.viewer_size;
        view.geometry.clear();
        assert_eq!(view.effective_scale(), None);

        // These are geometry records, not large decoded image allocations.
        view.reader.set_fit(FitMode::Width);
        for (width, height, expected) in [(100_000, 100_000, 0.0062), (1, 1, 620.)] {
            view.geometry.insert(
                0,
                PageGeometry {
                    width,
                    height,
                    rotation: 180,
                },
            );
            close_enough(view.effective_scale().unwrap(), expected);
        }
        view.reader.close();
        assert_eq!(view.effective_scale(), None);
    });
}

#[gpui_kit::test]
fn unequal_spreads_report_logical_current_page_and_last_page_uses_full_width(
    cx: &mut TestAppContext,
) {
    let fixture = loaded(cx, &[(120, 200), (240, 120), (60, 80)]);
    for rtl in [true, false] {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.reader.set_mode(ReadingMode::Spread);
                view.reader.set_fit(FitMode::Width);
                view.reader.settings.preferences.rtl = rtl;
                view.navigate(0, window, cx);
                view.relayout(cx);
            });
        })
        .unwrap();
        pump(&fixture, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            let available = f32::from(view.viewer_size.width) - 24.;
            let expected = available / 2. / 120.;
            close_enough(view.effective_scale().unwrap(), expected);
            assert!((rendered_scale(window, view, 0, 120) - expected).abs() < 0.01);
            assert!((rendered_scale(window, view, 1, 240) - expected / 2.).abs() < 0.01);
            trigger(window, FitMode::Width, &format!("{:.0}%", expected * 100.));
            assert_eq!(view.reader.display_indices()[0], if rtl { 1 } else { 0 });
        })
        .unwrap();
    }
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.navigate(2, window, cx));
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        let expected = (f32::from(view.viewer_size.width) - 24.) / 60.;
        assert_eq!(view.reader.visible_indices(), vec![2]);
        close_enough(view.effective_scale().unwrap(), expected);
        assert!((rendered_scale(window, view, 2, 60) - expected).abs() < 0.01);
        trigger(window, FitMode::Width, &format!("{:.0}%", expected * 100.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn continuous_scroll_changes_the_active_page_percentage(cx: &mut TestAppContext) {
    let fixture = loaded(cx, &[(120, 200), (240, 200), (80, 200)]);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.reader.set_mode(ReadingMode::Continuous);
            view.reader.set_fit(FitMode::Width);
            view.relayout(cx);
            view.navigate(0, window, cx);
        });
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("zoom-trigger", cx);
        fixture.view.update(cx, |view, _| {
            // Use the list position that tick reads after user scrolling.
            view.continuous_target = None;
            view.continuous.scroll_to(ListOffset {
                item_ix: 1,
                offset_in_item: px(0.),
            });
        });
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.reader.current, 1);
        let expected = (f32::from(view.viewer_size.width) - 24.) / 240.;
        close_enough(view.effective_scale().unwrap(), expected);
        assert!((rendered_scale(window, view, 1, 240) - expected).abs() < 0.01);
        trigger(window, FitMode::Width, &format!("{:.0}%", expected * 100.));
        assert!(window.try_find("zoom-panel").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn first_manual_step_multiplies_unrounded_fit_before_manual_clamping(cx: &mut TestAppContext) {
    let fixture = loaded(cx, &[(120, 200)]);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.clear_images(Some(window), cx);
            view.reader.set_fit(FitMode::Width);
            view.geometry.insert(
                0,
                PageGeometry {
                    width: 1000,
                    height: 1000,
                    rotation: 0,
                },
            );
            view.viewer_size = size(px(644.), px(644.));
            view.viewer_bounds.size = view.viewer_size;
            close_enough(view.page_layout(1000, 1000, 1).width, 620.);
            close_enough(view.effective_scale().unwrap(), 0.62);
            assert_eq!(view.reader.zoom, 1.);
            view.perform(Command::ZoomIn, window, cx);
            close_enough(view.reader.zoom, 0.71);
            assert_eq!(zoom_percentage(view.effective_scale()), "71%");
            assert_eq!(view.reader.settings.preferences.fit, FitMode::Custom);

            view.reader.set_fit(FitMode::Width);
            view.viewer_size.width = px(647.4);
            let fitted = view.page_layout(1000, 1000, 1).width / 1000.;
            view.zoom_by(1.15, window, cx);
            close_enough(fitted, 0.6234);
            close_enough(view.reader.zoom, 0.72);
            // Rounding 62.34% before multiplication would produce 71%, not 72%.

            // Multiply fitted 9% first, then normalize 10.35% to 10%.
            // Clamping the fit first would incorrectly produce 12%.
            view.reader.set_fit(FitMode::Width);
            view.viewer_size.width = px(114.);
            view.zoom_by(1.15, window, cx);
            close_enough(view.reader.zoom, 0.1);
            // A fitted 900% normalizes to 783%, not 696%.
            view.reader.set_fit(FitMode::Width);
            view.viewer_size.width = px(9024.);
            view.zoom_by(1. / 1.15, window, cx);
            close_enough(view.reader.zoom, 7.83);
            view.apply_zoom(0.001, window, cx);
            assert_eq!(view.reader.zoom, 0.1);
            view.perform(Command::ZoomOut, window, cx);
            assert_eq!(view.reader.zoom, 0.1);
            view.apply_zoom(100., window, cx);
            assert_eq!(view.reader.zoom, 8.);
            view.perform(Command::ZoomIn, window, cx);
            assert_eq!(view.reader.zoom, 8.);
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn real_panel_choices_steps_and_keyboard_submission_keep_modes_distinct(cx: &mut TestAppContext) {
    let fixture = loaded(cx, &[(120, 200)]);
    cx.update_window(fixture.window, |_, window, cx| {
        assert!(window.try_find("zoom-panel").is_none());
        assert!(window.try_find("zoom-input").is_none());
        assert!(window.try_find("zoom-in").is_none());
        for (id, fit) in FIT_CHOICES {
            window.click("zoom-trigger", cx);
            assert!(window.find("zoom-panel").visible());
            window.click(id, cx);
            assert!(!fixture.view.read(cx).zoom_open);
            assert!(window.try_find("zoom-panel").is_none());
            assert_eq!(fixture.view.read(cx).reader.settings.preferences.fit, fit);
            let view = fixture.view.read(cx);
            let available_width = f32::from(view.viewer_size.width) - 24.;
            let available_height = f32::from(view.viewer_size.height) - 24.;
            let expected = match fit {
                FitMode::Window => 1.0_f32
                    .min(available_width / 120.)
                    .min(available_height / 200.),
                FitMode::Width => available_width / 120.,
                FitMode::Height => available_height / 200.,
                FitMode::Original => 1.,
                FitMode::Custom => unreachable!(),
            };
            close_enough(view.effective_scale().unwrap(), expected);
            trigger(window, fit, &format!("{:.0}%", expected * 100.));
            let previous_focus = window.focused(cx);
            window.click("zoom-trigger", cx);
            selected_choice(window, fit);
            window.press("escape", cx);
            assert_eq!(window.focused(cx), previous_focus);
        }
        focus_trigger(window, cx);
        window.press("enter", cx);
        assert!(window.try_find("zoom-panel").is_some());
    })
    .unwrap();
    submit_draft(&fixture, "100", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        trigger(window, FitMode::Custom, "100%");
        assert!(window.try_find("zoom-panel").is_none());
        window.click("zoom-trigger", cx);
        selected_choice(window, FitMode::Custom);
        window.click("zoom-in", cx);
        assert!(window.try_find("zoom-panel").is_some());
        trigger(window, FitMode::Custom, "115%");
        assert_eq!(window.find("zoom-input").value(), Some("115"));
        window.click("zoom-out", cx);
        assert!(window.try_find("zoom-panel").is_some());
        trigger(window, FitMode::Custom, "100%");
        // Navigate through the actual panel's tab order to a fit choice.
        window.click("zoom-input", cx);
        let mut found = false;
        for _ in 0..10 {
            window.press("tab", cx);
            if window.find("zoom-original").focused() == Some(true) {
                found = true;
                break;
            }
        }
        assert!(found, "Tab must reach the original-size choice");
        window.press("enter", cx);
        trigger(window, FitMode::Original, "100%");
        assert!(window.try_find("zoom-panel").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn control_wheel_uses_custom_scale_and_reader_zoom_shortcuts_work_outside_panel(
    cx: &mut TestAppContext,
) {
    let fixture = loaded(cx, &[(120, 200), (120, 200)]);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("zoom-trigger", cx);
    })
    .unwrap();
    submit_draft(&fixture, "200", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.dispatch_event(
            gpui_kit::PlatformInput::Scroll(gpui_kit::ScrollEvent {
                position: window.find("viewer").bounds().center(),
                delta: gpui_kit::ScrollDelta::Lines(point(0., 2.)),
                modifiers: gpui_kit::Modifiers {
                    control: true,
                    ..Default::default()
                },
                ..Default::default()
            }),
            cx,
        );
        window.render_frame(cx);
        close_enough(fixture.view.read(cx).reader.zoom, 2.2);
        trigger(window, FitMode::Custom, "220%");
        for delta in [point(px(24.), px(0.)), point(px(0.), px(0.))] {
            window.dispatch_event(
                gpui_kit::PlatformInput::Scroll(gpui_kit::ScrollEvent {
                    position: window.find("viewer").bounds().center(),
                    delta: gpui_kit::ScrollDelta::Pixels(delta),
                    modifiers: gpui_kit::Modifiers {
                        control: true,
                        ..Default::default()
                    },
                    ..Default::default()
                }),
                cx,
            );
            window.render_frame(cx);
            close_enough(fixture.view.read(cx).reader.zoom, 2.2);
            trigger(window, FitMode::Custom, "220%");
        }
        window.dispatch_event(
            gpui_kit::PlatformInput::ModifiersChanged(gpui_kit::ModifiersChangedEvent::default()),
            cx,
        );
        // Reader focus is acquired by a center click, without page turning.
        window.click("viewer", cx);
        window.press("=", cx);
        close_enough(fixture.view.read(cx).reader.zoom, 2.2 * 1.15);
        window.press("0", cx);
        trigger(window, FitMode::Custom, "100%");
        window.press("1", cx);
        trigger(window, FitMode::Original, "100%");
        assert_eq!(fixture.view.read(cx).reader.current, 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn panel_zoom_shortcuts_and_global_close_remain_available_outside_text_editing(
    cx: &mut TestAppContext,
) {
    let fixture = loaded(cx, &[(120, 200)]);
    cx.update_window(fixture.window, |_, window, cx| {
        for (key, fit) in [
            ("w", FitMode::Width),
            ("h", FitMode::Height),
            ("1", FitMode::Original),
            ("f", FitMode::Window),
        ] {
            window.click("zoom-trigger", cx);
            window.press(key, cx);
            assert_eq!(fixture.view.read(cx).reader.settings.preferences.fit, fit);
            assert!(!fixture.view.read(cx).zoom_open);
        }
        window.click("zoom-trigger", cx);
        window.press("=", cx);
        assert!(fixture.view.read(cx).zoom_open);
        trigger(window, FitMode::Custom, "115%");
        window.press("0", cx);
        trigger(window, FitMode::Custom, "100%");
        replace_draft(window, "137", cx);
        window.press("w", cx);
        assert_eq!(
            fixture.view.read(cx).reader.settings.preferences.fit,
            FitMode::Custom
        );
        assert!(window.find("zoom-input").value().unwrap().contains('w'));
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            cx,
        );
        assert!(fixture.view.read(cx).reader.book.is_none());
        assert!(!fixture.view.read(cx).zoom_open);
        assert!(window.try_find("zoom-panel").is_none());
        assert!(fixture.view.read(cx).focus.is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_dismisses_the_panel_before_leaving_fullscreen(cx: &mut TestAppContext) {
    let fixture = loaded(cx, &[(120, 200)]);
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("f11", cx);
        assert!(window.is_fullscreen());
        window.click("zoom-trigger", cx);
        window.press("escape", cx);
        assert!(!fixture.view.read(cx).zoom_open);
        assert!(window.is_fullscreen());
        window.press("escape", cx);
        assert!(!window.is_fullscreen());
    })
    .unwrap();
}

#[gpui_kit::test]
fn a_closed_input_cannot_apply_a_delayed_submission(cx: &mut TestAppContext) {
    let fixture = loaded(cx, &[(120, 200)]);
    cx.update_window(fixture.window, |_, window, cx| {
        let input = fixture.view.read(cx).zoom_input.clone();
        input.update(cx, |input, cx| {
            input.set_value("200", window, cx);
            cx.emit(InputEvent::PressEnter {
                secondary: false,
                shift: false,
            });
        });
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.settings.preferences.fit, FitMode::Window);
        assert_eq!(view.reader.zoom, 1.);
        assert!(!view.zoom_open);
    });
}

#[gpui_kit::test]
fn control_wheel_does_not_cross_the_open_panel_or_replace_its_draft(cx: &mut TestAppContext) {
    let fixture = loaded(cx, &[(120, 200)]);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("zoom-trigger", cx);
        replace_draft(window, "137.25%", cx);
        let position = window.find("zoom-panel").bounds().center();
        assert!(window.find("viewer").bounds().contains(&position));
        let before = fixture.view.read(cx).effective_scale();
        window.dispatch_event(
            gpui_kit::PlatformInput::Scroll(gpui_kit::ScrollEvent {
                position,
                delta: gpui_kit::ScrollDelta::Lines(point(0., 2.)),
                modifiers: gpui_kit::Modifiers {
                    control: true,
                    ..Default::default()
                },
                ..Default::default()
            }),
            cx,
        );
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.effective_scale(), before);
        assert_eq!(view.reader.settings.preferences.fit, FitMode::Window);
        assert_eq!(window.find("zoom-input").value(), Some("137.25%"));
        assert_eq!(window.find("zoom-input").focused(), Some(true));
        assert!(view.zoom_open);
    })
    .unwrap();
}

#[gpui_kit::test]
fn panel_keyboard_navigation_does_not_move_pages_scroll_or_sidebar(cx: &mut TestAppContext) {
    let fixture = loaded(cx, &[(120, 200), (120, 200), (120, 200)]);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.apply_zoom(8., window, cx));
        window.render_frame(cx);
        window.click("zoom-trigger", cx);
        let previous_focus = fixture.view.read(cx).focus.clone();
        let sidebar = fixture.view.read(cx).reader.settings.preferences.sidebar;
        let paged = fixture.view.read(cx).paged_scroll.offset();
        let horizontal = fixture.view.read(cx).horizontal_scroll.offset();
        let continuous = fixture.view.read(cx).continuous.logical_scroll_top();
        for key in [
            "space", "left", "right", "up", "down", "pagedown", "pageup", "tab",
        ] {
            window.press(key, cx);
            let view = fixture.view.read(cx);
            assert_eq!(view.reader.current, 0, "reader received {key}");
            assert_eq!(
                view.reader.settings.preferences.sidebar, sidebar,
                "reader received {key}"
            );
            assert_eq!(view.paged_scroll.offset(), paged, "reader received {key}");
            assert_eq!(view.horizontal_scroll.offset(), horizontal);
            let top = view.continuous.logical_scroll_top();
            assert_eq!(top.item_ix, continuous.item_ix);
            assert_eq!(top.offset_in_item, continuous.offset_in_item);
            assert!(view.zoom_open);
        }
        window.click("zoom-input", cx);
        for key in ["left", "right", "up", "down", "pagedown", "space"] {
            window.press(key, cx);
            assert_eq!(fixture.view.read(cx).reader.current, 0);
            assert_eq!(
                fixture.view.read(cx).reader.settings.preferences.sidebar,
                sidebar
            );
        }
        window.press("escape", cx);
        assert!(window.try_find("zoom-panel").is_none());
        assert!(previous_focus.is_focused(window));
        window.click("zoom-trigger", cx);
        window.click("zoom-input", cx);
        window.click("page-input", cx);
        assert!(window.try_find("zoom-panel").is_none());
        assert_eq!(window.find("page-input").focused(), Some(true));
        window.input("2", cx);
        assert_eq!(window.find("page-input").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn invalid_manual_values_keep_state_and_draft_but_finite_values_clamp(cx: &mut TestAppContext) {
    let fixture = loaded(cx, &[(120, 200)]);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("zoom-trigger", cx)
    })
    .unwrap();
    let (before_zoom, before_fit) = fixture.view.read_with(cx, |view, _| {
        (view.reader.zoom, view.reader.settings.preferences.fit)
    });
    for draft in ["NaN", "inf", "-inf", "1e100", "not a number", "%", "150%%"] {
        submit_draft(&fixture, draft, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            let view = fixture.view.read(cx);
            assert_eq!(view.reader.zoom, before_zoom, "{draft}");
            assert_eq!(view.reader.settings.preferences.fit, before_fit, "{draft}");
            assert!(view.zoom_open && view.message_error, "{draft}");
            assert!(!view.message.is_empty());
            assert_eq!(window.find("zoom-input").value(), Some(draft));
            assert_eq!(window.find("zoom-input").focused(), Some(true));
            assert_eq!(
                window.find("status-message").label(),
                Some(view.message.as_str())
            );
        })
        .unwrap();
    }
    for (draft, expected, label) in [
        (" 125% ", 1.25, "125%"),
        ("-12", 0.1, "10%"),
        ("0", 0.1, "10%"),
        ("9999%", 8., "800%"),
    ] {
        submit_draft(&fixture, draft, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            let view = fixture.view.read(cx);
            close_enough(view.reader.zoom, expected);
            assert!(!view.zoom_open && !view.message_error);
            trigger(window, FitMode::Custom, label);
            assert!(window.try_find("zoom-panel").is_none());
            window.click("zoom-trigger", cx);
            let numeric = window.find("zoom-input").value().unwrap().to_string();
            close_enough(numeric.parse::<f32>().unwrap(), expected * 100.);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn focused_draft_survives_resize_panels_navigation_rotation_and_async_geometry(
    cx: &mut TestAppContext,
) {
    let fixture = loaded(cx, &[(120, 200), (240, 120)]);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.perform(Command::FitWidth, window, cx));
        window.click("zoom-trigger", cx);
        replace_draft(window, "137.25%", cx);
        window.resize(size(px(960.), px(640.)));
        window.bounds_changed(cx);
    })
    .unwrap();
    pump(&fixture, cx);
    for change in 0..4 {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| match change {
                0 => view.perform(Command::Sidebar, window, cx),
                1 => {
                    let layout = &mut view.reader.settings.preferences.panel_layout;
                    layout.inspector_visible = !layout.inspector_visible;
                    cx.notify();
                }
                2 => view.navigate(1, window, cx),
                _ => view.perform(Command::RotateRight, window, cx),
            });
        })
        .unwrap();
        pump(&fixture, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            assert!(view.zoom_open);
            assert_eq!(window.find("zoom-input").focused(), Some(true));
            assert_eq!(window.find("zoom-input").value(), Some("137.25%"));
            let (width, _) = view.page_dimensions(view.reader.current).unwrap();
            let expected = (f32::from(view.viewer_size.width) - 24.) / width as f32;
            close_enough(view.effective_scale().unwrap(), expected);
            trigger(window, FitMode::Width, &format!("{:.0}%", expected * 100.));
            assert!(
                (rendered_scale(window, view, view.reader.current, width) - expected).abs() < 0.01
            );
        })
        .unwrap();
    }
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.clear_images(Some(window), cx);
            view.geometry.clear();
            cx.notify();
        });
        window.render_frame(cx);
        trigger(window, FitMode::Width, "—");
        assert_eq!(window.find("zoom-input").value(), Some("137.25%"));
        assert_eq!(window.find("zoom-in").disabled(), Some(true));
        fixture.view.update(cx, |view, cx| {
            let index = view.reader.current;
            view.geometry.insert(
                index,
                PageGeometry {
                    width: 60,
                    height: 80,
                    rotation: 90,
                },
            );
            cx.notify();
        });
        window.render_frame(cx);
        let expected = (f32::from(fixture.view.read(cx).viewer_size.width) - 24.) / 60.;
        trigger(window, FitMode::Width, &format!("{:.0}%", expected * 100.));
        assert_eq!(window.find("zoom-input").value(), Some("137.25%"));
        fixture.view.update(cx, |view, cx| view.load_requested(cx));
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        let view = fixture.view.read(cx);
        assert_eq!(view.page_dimensions(1), Some((120, 240)));
        let expected = (f32::from(view.viewer_size.width) - 24.) / 120.;
        trigger(window, FitMode::Width, &format!("{:.0}%", expected * 100.));
        assert!(window.try_find("zoom-panel").is_some());
        assert_eq!(window.find("zoom-input").value(), Some("137.25%"));
        assert_eq!(window.find("zoom-input").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn pending_geometry_disables_only_manual_controls_and_book_lifecycle_closes_panel(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let first = book(&fixture, "first-zoom", 1);
    let second = book(&fixture, "second-zoom", 1);
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        trigger(window, FitMode::Window, "—");
        assert_eq!(window.find("zoom-trigger").disabled(), Some(true));
        window.click("zoom-trigger", cx);
        assert!(window.try_find("zoom-panel").is_none());
        fixture.view.update(cx, |view, cx| {
            view.reader
                .set_book(Arc::new(Book::open(&first, false).unwrap()), None);
            cx.notify();
        });
        window.render_frame(cx);
        assert_ne!(window.find("zoom-trigger").disabled(), Some(true));
        window.click("zoom-trigger", cx);
        trigger(window, FitMode::Window, "—");
        for id in ["zoom-in", "zoom-out"] {
            assert_eq!(window.find(id).disabled(), Some(true), "{id}");
        }
        assert!(
            fixture
                .view
                .read(cx)
                .zoom_input
                .read(cx)
                .presentation()
                .is_disabled()
        );
        window.click("zoom-input", cx);
        window.input("200", cx);
        assert!(fixture.view.read(cx).zoom_input.read(cx).value().is_empty());
        for (id, _) in FIT_CHOICES {
            assert_ne!(window.find(id).disabled(), Some(true), "{id}");
        }
        window.click("zoom-in", cx);
        assert_eq!(fixture.view.read(cx).reader.zoom, 1.);
        window.click("zoom-fit-height", cx);
        assert_eq!(
            fixture.view.read(cx).reader.settings.preferences.fit,
            FitMode::Height
        );
        assert!(window.try_find("zoom-panel").is_none());
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("zoom-trigger", cx);
        assert_ne!(window.find("zoom-in").disabled(), Some(true));
        fixture
            .view
            .update(cx, |view, cx| view.perform(Command::Close, window, cx));
        window.render_frame(cx);
        assert!(window.try_find("zoom-panel").is_none());
        assert!(!fixture.view.read(cx).zoom_open);
        assert_eq!(window.find("zoom-trigger").disabled(), Some(true));
        fixture
            .view
            .update(cx, |view, cx| view.open_path(first, window, cx));
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("zoom-trigger", cx);
        fixture
            .view
            .update(cx, |view, cx| view.open_path(second.clone(), window, cx));
        window.render_frame(cx);
        assert!(!fixture.view.read(cx).zoom_open);
        assert!(window.try_find("zoom-panel").is_none());
        // The old book remains usable while the background replacement loads.
        window.click("zoom-trigger", cx);
        replace_draft(window, "199%", cx);
        assert!(fixture.view.read(cx).zoom_open);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(!view.zoom_open);
        assert_eq!(
            view.reader.book.as_ref().unwrap().path,
            second.canonicalize().unwrap()
        );
    });
}

#[gpui_kit::test]
fn native_panel_drags_remeasure_fitted_percentage_without_replacing_zoom_entities(
    cx: &mut TestAppContext,
) {
    let fixture = loaded(cx, &[(120, 200), (240, 120)]);
    let entity = fixture
        .view
        .read_with(cx, |view, _| view.zoom_input.entity_id());
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("zoom-trigger", cx);
        window.click("zoom-fit-width", cx);
    })
    .unwrap();
    for (left, delta) in [(true, 40.), (false, -32.)] {
        cx.update_window(fixture.window, |_, window, cx| {
            window.click("zoom-trigger", cx);
            replace_draft(window, "137.25%", cx);
            let viewer = window.find("viewer").bounds();
            let x = if left { viewer.left() } else { viewer.right() };
            window.drag(
                point(x, viewer.center().y),
                point(x + px(delta), viewer.center().y),
                cx,
            );
        })
        .unwrap();
        pump(&fixture, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            window.render_frame(cx);
            let view = fixture.view.read(cx);
            assert_eq!(view.reader.current, 0);
            assert_eq!(view.reader.settings.preferences.fit, FitMode::Width);
            assert_eq!(
                view.reader.zoom, 1.,
                "an outside drag must not apply the draft"
            );
            assert!(
                !view.zoom_open,
                "an outside drag keeps the existing cancel rule"
            );
            assert!(window.try_find("zoom-panel").is_none());
            assert_eq!(view.zoom_input.entity_id(), entity);
            let expected = (f32::from(view.viewer_size.width) - 24.) / 120.;
            close_enough(view.effective_scale().unwrap(), expected);
            trigger(window, FitMode::Width, &format!("{:.0}%", expected * 100.));
            assert!((rendered_scale(window, view, 0, 120) - expected).abs() < 0.01);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn display_only_zoom_sync_does_not_dirty_or_write_settings(cx: &mut TestAppContext) {
    let fixture = loaded(cx, &[(120, 200)]);
    pump(&fixture, cx);
    let path = fixture.directory.path().join("settings.json");
    let before = std::fs::read(&path).unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            assert!(!view.saving);
            view.dirty = false;
            let epoch = view.save_epoch.load(Ordering::SeqCst);
            let preferences = view.reader.settings.clone();
            for width in [640., 720., 800.] {
                view.viewer_size.width = px(width);
                view.viewer_bounds.size = view.viewer_size;
                view.sync_zoom_input(false, window, cx);
                view.sync_zoom_input(true, window, cx);
                assert!(!view.dirty);
                assert_eq!(view.save_epoch.load(Ordering::SeqCst), epoch);
                assert_eq!(view.reader.settings, preferences);
                let actual = view.zoom_input.read(cx).value().parse::<f32>().unwrap();
                let available_width = width - 24.;
                let available_height = f32::from(view.viewer_size.height) - 24.;
                let expected = 1.0_f32
                    .min(available_width / 120.)
                    .min(available_height / 200.);
                assert!((actual / 100. - expected).abs() < 0.0001);
            }
        });
    })
    .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), before);
}
