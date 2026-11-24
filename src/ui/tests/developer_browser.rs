//! Native picker coverage for anonymous browser and Files targets.

use super::*;
use gpui_kit::{MouseDownEvent, MouseUpEvent, PlatformInput};
use std::{cell::RefCell, rc::Rc};

struct PointerProbe {
    picker: super::super::developer::Picker,
    releases: Rc<RefCell<Vec<MouseButton>>>,
    selections: Rc<RefCell<usize>>,
}

impl Render for PointerProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let selections = self.selections.clone();
        let releases = self.releases.clone();
        div()
            .size_full()
            .child(
                self.picker
                    .frame(None, move |_, _, _| *selections.borrow_mut() += 1),
            )
            .child(
                div()
                    .relative()
                    .size_full()
                    .block_mouse_except_scroll()
                    .child(self.picker.mark(super::super::developer_session::metadata(
                        "probe",
                        "Probe",
                        "src/ui/tests/developer_browser.rs",
                        crate::developer::TargetKind::Element,
                    )))
                    .child(
                        gpui_kit::canvas(
                            |_, _, _| {},
                            move |_, (), window, _| {
                                let releases = releases.clone();
                                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, _| {
                                    if phase == gpui_kit::DispatchPhase::Bubble {
                                        releases.borrow_mut().push(event.button);
                                    }
                                });
                            },
                        )
                        .size_full(),
                    ),
            )
    }
}

#[gpui_kit::test]
fn picker_consumes_every_captured_button_in_all_release_orders(cx: &mut TestAppContext) {
    use MouseButton::{Left, Middle, Right};
    let orders = [
        [Left, Right, Middle],
        [Left, Middle, Right],
        [Right, Left, Middle],
        [Right, Middle, Left],
        [Middle, Left, Right],
        [Middle, Right, Left],
    ];
    cx.update(init);
    for cancel in [false, true] {
        for down_order in orders {
            for up_order in orders {
                let picker = super::super::developer::Picker::new("probe");
                let releases = Rc::new(RefCell::new(Vec::new()));
                let selections = Rc::new(RefCell::new(0));
                let (handle, _) = cx.update(|cx| {
                    gpui_kit::open_window(Default::default(), cx, |_, cx| {
                        cx.new(|_| PointerProbe {
                            picker: picker.clone(),
                            releases: releases.clone(),
                            selections: selections.clone(),
                        })
                    })
                    .unwrap()
                });
                cx.update_window(handle, |_, window, cx| {
                    picker.start(window, cx);
                    window.render_frame(cx);
                    for button in down_order {
                        window.dispatch_event(
                            PlatformInput::MouseDown(MouseDownEvent {
                                button,
                                position: point(px(10.), px(10.)),
                                click_count: 1,
                                ..Default::default()
                            }),
                            cx,
                        );
                    }
                    if cancel {
                        picker.cancel(window, cx);
                    }
                    window.render_frame(cx);
                })
                .unwrap();
                for button in up_order {
                    cx.update_window(handle, |_, window, cx| {
                        window.dispatch_event(
                            PlatformInput::MouseUp(MouseUpEvent {
                                button,
                                position: point(px(10.), px(10.)),
                                click_count: 1,
                                ..Default::default()
                            }),
                            cx,
                        );
                    })
                    .unwrap();
                    cx.run_until_parked();
                    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                        .unwrap();
                    assert!(
                        releases.borrow().is_empty(),
                        "leaked {button:?}: {down_order:?} / {up_order:?}, cancel={cancel}"
                    );
                }
                assert_eq!(
                    *selections.borrow(),
                    usize::from(!cancel && down_order[0] == Left)
                );
                cx.update_window(handle, |_, window, cx| {
                    picker.cancel(window, cx);
                    window.render_frame(cx);
                    window.dispatch_event(
                        PlatformInput::MouseUp(MouseUpEvent {
                            button: Right,
                            position: point(px(10.), px(10.)),
                            ..Default::default()
                        }),
                        cx,
                    );
                    window.remove_window();
                })
                .unwrap();
                assert_eq!(
                    *releases.borrow(),
                    vec![Right],
                    "capture must end after all releases"
                );
            }
        }
    }
}

#[gpui_kit::test]
fn preferences_picker_drops_held_capture_on_actual_close(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture
        .view
        .update(cx, |view, cx| view.apply_developer_enabled(true, cx));
    render_windows(&fixture, cx);
    let preferences = open_preferences(&fixture, cx);
    let picker = fixture.view.read_with(cx, |view, _| {
        view.developer.preferences_picker.clone().unwrap()
    });
    cx.update_window(preferences, |_, window, cx| {
        picker.start(window, cx);
        window.render_frame(cx);
        let position = window.find("reading-direction-ltr").bounds().center();
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position,
                ..Default::default()
            }),
            cx,
        );
        super::super::preferences::close(fixture.view.downgrade(), fixture.window, window, cx);
    })
    .unwrap();
    cx.run_until_parked();
    let reopened = open_preferences(&fixture, cx);
    cx.update_window(reopened, |_, window, cx| {
        window.click("reading-direction-ltr", cx)
    })
    .unwrap();
    render_windows(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(
            !view.reader.settings.preferences.rtl,
            "reopened preferences must accept its first click"
        );
        assert!(view.developer.selected_target().is_none());
    });
    let picker = fixture.view.read_with(cx, |view, _| {
        view.developer.preferences_picker.clone().unwrap()
    });
    cx.update_window(reopened, |_, window, cx| {
        picker.start(window, cx);
        window.render_frame(cx);
        window.click("reading-direction-rtl", cx);
    })
    .unwrap();
    render_windows(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.developer.selected_target().unwrap().id,
            "reading-direction-rtl"
        );
        assert!(
            !view.reader.settings.preferences.rtl,
            "selection must not change preferences"
        );
    });
}

fn enable_picker(fixture: &Fixture, cx: &mut TestAppContext) -> std::net::TcpStream {
    fixture.view.update(cx, |view, cx| {
        view.apply_developer_enabled(true, cx);
        assert!(view.developer.reader_picker.is_some());
    });
    super::developer::wait_for(fixture, cx, |view| {
        view.developer.state_text() == Text::DeveloperWaiting
    });
    let connection = super::developer::pair(fixture, cx);
    render_windows(fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("developer-picker", cx);
    })
    .unwrap();
    render_windows(fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.developer.reader_picker.as_ref().unwrap().active());
    });
    connection
}

fn assert_safe_target(
    fixture: &Fixture,
    prefix: &str,
    label: &str,
    kind: crate::developer::TargetKind,
    cx: &mut TestAppContext,
) {
    fixture.view.read_with(cx, |view, _| {
        let target = view.developer.selected_target().expect("picker selection");
        assert!(target.id.starts_with(prefix), "{}", target.id);
        assert!(
            target.id[prefix.len()..]
                .bytes()
                .all(|b| b.is_ascii_digit())
        );
        assert_eq!(target.label, label);
        assert_eq!(target.target_kind, kind);
        let preview = serde_json::to_string(&target).unwrap();
        assert!(!preview.contains("PRIVATE"));
        assert!(!preview.contains(&fixture.directory.path().to_string_lossy().to_string()));
    });
}

#[gpui_kit::test]
fn developer_mode_keeps_browser_dropdowns_interactive_and_selectable(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let root = book(&fixture, "dropdown-library", 1)
        .canonicalize()
        .unwrap();
    let nested = root.join("one").join("two").join("three");
    std::fs::create_dir_all(&nested).unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.browse(nested.clone(), window, cx);
            view.apply_developer_enabled(true, cx);
        });
    })
    .unwrap();
    pump(&fixture, cx);
    for trigger in ["browser-ancestors", "browser-sort"] {
        cx.update_window(fixture.window, |_, window, cx| {
            window.click(trigger, cx);
            assert!(window.find("popup-menu").visible(), "{trigger} must open");
            window.press("escape", cx);
        })
        .unwrap();
        pump(&fixture, cx);
        cx.update_window(fixture.window, |_, window, _| {
            assert!(window.try_find("popup-menu").is_none());
        })
        .unwrap();
        let _connection = enable_picker(&fixture, cx);
        cx.update_window(fixture.window, |_, window, cx| window.click(trigger, cx))
            .unwrap();
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(
                view.developer
                    .selected_target()
                    .expect("dropdown selection")
                    .id,
                trigger
            );
            assert_eq!(view.listing.as_ref().unwrap().path, nested);
        });
        cx.update_window(fixture.window, |_, window, cx| {
            assert!(window.try_find("popup-menu").is_none());
            window.press("escape", cx);
        })
        .unwrap();
        pump(&fixture, cx);
    }
}

#[gpui_kit::test]
fn browser_picker_uses_anonymous_tile_identity_and_consumes_double_click(cx: &mut TestAppContext) {
    for language in Language::ALL {
        let mut settings = AppSettings::default();
        settings.preferences.language = language;
        let fixture = fixture_with_settings(cx, settings);
        let root = book(&fixture, "PRIVATE-library", 1).canonicalize().unwrap();
        let child = root.join("PRIVATE-folder");
        std::fs::create_dir(&child).unwrap();
        cx.update_window(fixture.window, |_, window, cx| {
            fixture
                .view
                .update(cx, |view, cx| view.browse(root.clone(), window, cx));
        })
        .unwrap();
        pump(&fixture, cx);
        let before = fixture
            .view
            .read_with(cx, |view, _| view.browser.model.selected.clone());
        let _connection = enable_picker(&fixture, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            window.double_click(super::super::browser::browser_entry_id(&child), cx);
        })
        .unwrap();
        render_windows(&fixture, cx);
        assert_safe_target(
            &fixture,
            "browser-item-",
            "Browser item",
            crate::developer::TargetKind::Element,
            cx,
        );
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.listing.as_ref().unwrap().path, root);
            assert_eq!(view.browser.model.selected, before);
            assert!(view.reader.book.is_none());
        });
    }
}

#[gpui_kit::test]
fn files_picker_uses_anonymous_item_identity_without_navigation(cx: &mut TestAppContext) {
    let mut settings = AppSettings::default();
    settings.preferences.sidebar = true;
    settings.preferences.sidebar_mode = SidebarMode::Files;
    let fixture = fixture_with_settings(cx, settings);
    let root = book(&fixture, "PRIVATE-tree", 1).canonicalize().unwrap();
    let child = root.join("PRIVATE-folder");
    std::fs::create_dir(&child).unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.files_browse(root.clone(), true, window, cx)
        });
    })
    .unwrap();
    pump(&fixture, cx);
    let _connection = enable_picker(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.double_click(super::super::files::entry_id(&child), cx);
    })
    .unwrap();
    render_windows(&fixture, cx);
    assert_safe_target(
        &fixture,
        "files-entry-",
        "File tree item",
        crate::developer::TargetKind::Element,
        cx,
    );
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()));
        assert!(view.reader.book.is_none());
    });
}

#[gpui_kit::test]
fn browser_search_picker_reports_a_safe_region_not_input_text(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.browser.search.update(cx, |input, cx| {
                input.set_value("PRIVATE-search", window, cx);
            });
        });
    })
    .unwrap();
    render_windows(&fixture, cx);
    let _connection = enable_picker(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-search", cx);
    })
    .unwrap();
    render_windows(&fixture, cx);
    fixture.view.read_with(cx, |view, cx| {
        let target = view
            .developer
            .selected_target()
            .expect("search region selection");
        assert_eq!(target.id, "browser-search");
        assert_eq!(target.label, "Search browser");
        assert_eq!(target.target_kind, crate::developer::TargetKind::Region);
        assert_eq!(
            view.browser.search.read(cx).value().as_ref(),
            "PRIVATE-search"
        );
        assert!(!serde_json::to_string(&target).unwrap().contains("PRIVATE"));
    });
}
