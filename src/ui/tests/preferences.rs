//! Separate-window ownership and Preferences regressions.

use super::*;
use gpui_kit::WeakEntity;

#[gpui_kit::test]
fn separate_window_can_defer_reader_updates_and_close_without_quitting(cx: &mut TestAppContext) {
    struct Probe {
        reader: WeakEntity<ReaderView>,
        main: AnyWindowHandle,
        focus: FocusHandle,
        _observer: Subscription,
    }
    impl Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().track_focus(&self.focus).child(
                Button::new("probe-change-reader")
                    .label("Change")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let reader = this.reader.clone();
                        let main = this.main;
                        cx.defer(move |cx| {
                            _ = main.update(cx, |_, _, cx| {
                                _ = reader.update(cx, |reader, cx| {
                                    reader.reader.settings.preferences.remember_position = false;
                                    reader.dirty = true;
                                    cx.notify();
                                });
                            });
                        });
                    })),
            )
        }
    }
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let make_window = |cx: &mut App| {
        fixture.view.update(cx, |_, cx| {
            let reader = cx.entity().downgrade();
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                window.on_window_should_close(cx, |window, _| {
                    window.remove_window();
                    false
                });
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    focus.focus(window, cx);
                    let observer =
                        cx.observe_in(&reader.upgrade().unwrap(), window, |_, _, _, cx| {
                            cx.notify()
                        });
                    Probe {
                        reader,
                        main: fixture.window,
                        focus,
                        _observer: observer,
                    }
                })
            })
            .unwrap()
            .0
        })
    };
    let auxiliary = cx.update(make_window);
    cx.update_window(auxiliary, |_, window, cx| {
        window.activate_window();
        window.click("probe-change-reader", cx);
    })
    .unwrap();
    cx.run_until_parked();
    fixture.view.read_with(cx, |view, _| {
        assert!(!view.reader.settings.preferences.remember_position)
    });
    cx.update_window(auxiliary, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert!(cx.update_window(auxiliary, |_, _, _| ()).is_err());
    assert!(cx.update_window(fixture.window, |_, _, _| ()).is_ok());
    let replacement = cx.update(make_window);
    assert_ne!(auxiliary, replacement);
    cx.update_window(replacement, |_, window, _| window.remove_window())
        .unwrap();
}

#[gpui_kit::test]
fn main_direction_updates_preferences_and_current_choice_does_not_schedule_a_save(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let preferences = open_preferences(&fixture, cx);
    for _ in 0..2 {
        cx.update_window(fixture.window, |_, window, cx| {
            window.click("direction", cx)
        })
        .unwrap();
        pump(&fixture, cx);
        let (rtl, epoch) = fixture.view.read_with(cx, |view, _| {
            assert!(!view.dirty && !view.saving);
            (
                view.reader.settings.preferences.rtl,
                view.save_epoch.load(Ordering::SeqCst),
            )
        });
        cx.update_window(preferences, |_, window, _| {
            assert_eq!(window.find("reading-direction-rtl").checked(), Some(rtl));
            assert_eq!(window.find("reading-direction-ltr").checked(), Some(!rtl));
        })
        .unwrap();
        let path = fixture.directory.path().join("settings.json");
        let bytes = std::fs::read(&path).unwrap();
        click_preference(
            &fixture,
            if rtl {
                "reading-direction-rtl"
            } else {
                "reading-direction-ltr"
            },
            cx,
        );
        fixture.view.read_with(cx, |view, _| {
            assert!(!view.dirty && !view.saving);
            assert_eq!(view.save_epoch.load(Ordering::SeqCst), epoch);
        });
        pump(&fixture, cx);
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.save_epoch.load(Ordering::SeqCst), epoch);
        });
    }
}

#[gpui_kit::test]
fn preferences_close_shortcuts_preserve_book_and_reopen_with_a_new_handle(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "shortcuts", 4)])
        .unwrap();
    pump(&fixture, cx);
    let original = fixture
        .view
        .read_with(cx, |view, _| view.reader.book.clone().unwrap());
    let keys = [
        "escape",
        "ctrl-w",
        #[cfg(target_os = "macos")]
        "cmd-w",
        #[cfg(target_os = "windows")]
        "alt-f4",
    ];
    let mut previous = None;
    for key in keys {
        let preferences = open_preferences(&fixture, cx);
        assert_ne!(Some(preferences), previous);
        let before = fixture.view.read_with(cx, |view, _| {
            (
                view.reader.current,
                view.reader.zoom,
                view.reader.settings.clone(),
                view.reader.slideshow,
            )
        });
        cx.update_window(preferences, |_, window, cx| {
            // These keys belong to Reader, not the Preferences key context.
            for key in ["pagedown", "right", "+", "b", "p", "space"] {
                window.press(key, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(
                (
                    view.reader.current,
                    view.reader.zoom,
                    view.reader.settings.clone(),
                    view.reader.slideshow,
                ),
                before
            );
        });
        cx.update_window(preferences, |_, window, cx| window.press(key, cx))
            .unwrap();
        cx.run_until_parked();
        assert!(cx.update_window(preferences, |_, _, _| ()).is_err());
        assert_eq!(cx.windows(), vec![fixture.window]);
        fixture.view.read_with(cx, |view, _| {
            assert!(Arc::ptr_eq(view.reader.book.as_ref().unwrap(), &original));
            assert_eq!(view.reader.current, before.0);
        });
        previous = Some(preferences);
    }
}

#[gpui_kit::test]
fn preferences_lifecycle_keeps_reader_geometry_and_panel_layout(cx: &mut TestAppContext) {
    let mut settings = AppSettings::default();
    settings.preferences.panel_layout.sidebar_width = 277;
    settings.preferences.panel_layout.inspector_width = 311;
    let fixture = fixture_with_settings(cx, settings);
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("inspector-playback", cx)
    })
    .unwrap();
    pump(&fixture, cx);
    let state = fixture.view.read_with(cx, |view, _| {
        (
            view.reader.settings.preferences.clone(),
            view.inspector_tab,
            view.viewer_bounds,
        )
    });
    let geometry = cx
        .update_window(fixture.window, |_, window, _| {
            (window.bounds(), window.find("viewer").bounds())
        })
        .unwrap();
    let mut previous = None;
    for _ in 0..2 {
        let preferences = open_preferences(&fixture, cx);
        assert_ne!(Some(preferences), previous);
        for _ in 0..2 {
            assert_eq!(open_preferences(&fixture, cx), preferences);
            pump(&fixture, cx);
            fixture.view.read_with(cx, |view, _| {
                assert_eq!(
                    (
                        view.reader.settings.preferences.clone(),
                        view.inspector_tab,
                        view.viewer_bounds
                    ),
                    state
                );
            });
            cx.update_window(fixture.window, |_, window, _| {
                assert_eq!((window.bounds(), window.find("viewer").bounds()), geometry);
            })
            .unwrap();
        }
        cx.update_window(preferences, |_, window, cx| window.press("escape", cx))
            .unwrap();
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(
                (
                    view.reader.settings.preferences.clone(),
                    view.inspector_tab,
                    view.viewer_bounds
                ),
                state
            );
        });
        cx.update_window(fixture.window, |_, window, _| {
            assert_eq!((window.bounds(), window.find("viewer").bounds()), geometry);
        })
        .unwrap();
        previous = Some(preferences);
    }
}

#[gpui_kit::test]
fn native_close_from_preferences_closes_comic_and_resets_main_inputs(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "native-close", 3)])
        .unwrap();
    pump(&fixture, cx);
    let preferences = open_preferences(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.page_input
                .update(cx, |input, cx| input.set_value("003", window, cx));
            view.zoom_input
                .update(cx, |input, cx| input.set_value("62.", window, cx));
            view.zoom_open = true;
        });
    })
    .unwrap();
    cx.update_window(preferences, |_, window, cx| {
        window.dispatch_action(Box::new(Command::Close), cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, cx| {
        assert!(view.reader.book.is_none());
        assert!(view.pages.is_empty() && view.thumbs.is_empty());
        assert!(!view.zoom_open);
        assert_eq!(view.page_input.read(cx).value().as_ref(), "—");
        assert_eq!(view.zoom_input.read(cx).value().as_ref(), "");
        assert_eq!(view.preferences_window(), Some(preferences));
    });
    assert_eq!(cx.windows().len(), 2);
    cx.update_window(preferences, |_, window, _| {
        assert!(window.find("preferences-window").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn drafts_created_before_preferences_open_survive_language_change_and_reopen(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.controls_pending = false;
            view.path_pending = false;
            for (input, value) in [
                (&view.page_input, "003"),
                (&view.path_input, "/draft/日本語/正體中文"),
                (&view.interval_input, "17"),
            ] {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
            view.path_input.read(cx).focus_handle(cx).focus(window, cx);
        });
    })
    .unwrap();
    for _ in 0..2 {
        let preferences = open_preferences(&fixture, cx);
        click_preference(&fixture, "language-ja", cx);
        cx.update_window(preferences, |_, window, cx| window.press("ctrl-w", cx))
            .unwrap();
        cx.run_until_parked();
        render_windows(&fixture, cx);
        fixture.view.read_with(cx, |view, cx| {
            assert_eq!(view.page_input.read(cx).value().as_ref(), "003");
            assert_eq!(
                view.path_input.read(cx).value().as_ref(),
                "/draft/日本語/正體中文"
            );
            assert_eq!(view.interval_input.read(cx).value().as_ref(), "17");
            assert_eq!(view.reader.settings.preferences.interval, 5);
        });
        assert_eq!(cx.windows(), vec![fixture.window]);
    }
}

#[gpui_kit::test]
fn delayed_snapshots_cannot_undo_preferences_language_or_corruption_recovery(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let preferences = open_preferences(&fixture, cx);
    let (snapshot, path, lock, epoch, revision) = fixture.view.read_with(cx, |view, _| {
        (
            view.reader.settings.clone(),
            view.settings_path.clone(),
            view.save_lock.clone(),
            view.save_epoch.clone(),
            view.save_epoch.fetch_add(1, Ordering::SeqCst) + 1,
        )
    });
    click_preference(&fixture, "language-ja", cx);
    pump(&fixture, cx);
    let saved = std::fs::read(&path).unwrap();
    // Run an older writer after the UI has already saved the newer language.
    save_snapshot(&snapshot, &path, &lock, &epoch, revision).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), saved);
    assert_eq!(
        AppSettings::load_from(&path).unwrap().preferences.language,
        Language::Japanese
    );

    std::fs::write(&path, b"{broken queued settings").unwrap();
    let revision = epoch.fetch_add(1, Ordering::SeqCst) + 1;
    fixture.view.update(cx, |view, cx| {
        view.save_error = Some("Invalid settings".into());
        cx.notify();
    });
    render_windows(&fixture, cx);
    cx.update_window(preferences, |_, window, cx| {
        window.scroll(
            "preferences-content",
            gpui_kit::ScrollDelta::Lines(point(0., -100.)),
            cx,
        );
    })
    .unwrap();
    click_preference(&fixture, "reset-corrupt-settings", cx);
    pump(&fixture, cx);
    let reset_bytes = std::fs::read(&path).unwrap();
    save_snapshot(&snapshot, &path, &lock, &epoch, revision).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), reset_bytes);
    assert_eq!(
        AppSettings::load_from(&path).unwrap(),
        AppSettings::default()
    );
    let backups: Vec<_> = std::fs::read_dir(fixture.directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("settings.json.corrupt-")
        })
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        std::fs::read(&backups[0]).unwrap(),
        b"{broken queued settings"
    );
    cx.update_window(preferences, |_, window, _| {
        assert!(window.try_find("settings-save-error").is_none());
        assert!(window.try_find("reset-corrupt-settings").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn pending_preferences_request_cannot_reopen_during_quit(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.perform(Command::Settings, window, cx);
            view.perform(Command::Quit, window, cx);
            view.perform(Command::Settings, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    fixture.view.read_with(cx, |view, _| {
        assert!(view.closing);
        assert!(view.preferences_window().is_none());
    });
    assert_eq!(cx.windows(), vec![fixture.window]);
}

#[gpui_kit::test]
fn launch_requests_still_replace_the_reader_with_preferences_open(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let first = book(&fixture, "first-with-preferences", 2);
    let second = book(&fixture, "second-with-preferences", 3)
        .canonicalize()
        .unwrap();
    fixture.sender.send(vec![first]).unwrap();
    pump(&fixture, cx);
    let preferences = open_preferences(&fixture, cx);
    fixture.sender.send(vec![second.clone()]).unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.book.as_ref().unwrap().path, second);
        assert_eq!(view.preferences_window(), Some(preferences));
        assert_eq!(view.reader.current, 0);
    });
    cx.update_window(preferences, |_, window, _| {
        assert!(window.find("preferences-window").visible());
    })
    .unwrap();
    assert_eq!(cx.windows().len(), 2);
}

#[gpui_kit::test]
fn native_quit_from_preferences_removes_auxiliary_window(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let preferences = open_preferences(&fixture, cx);
    cx.update_window(preferences, |_, window, cx| {
        window.dispatch_action(Box::new(Command::Quit), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update_window(preferences, |_, _, _| ()).is_err());
    fixture
        .view
        .read_with(cx, |view, _| assert!(view.preferences_window().is_none()));
    // TestPlatform::quit is a no-op. TestAppContext::on_quit observes test
    // teardown, not App::quit. Process exit and OS close need native evidence.
}

#[gpui_kit::test]
fn reader_cleanup_removes_preferences_and_pending_open_cannot_orphan_a_window(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let preferences = open_preferences(&fixture, cx);
    // simulate_close restores a callback after removal and panics in GPUI.
    // Exercise the same production cleanup used by the reader close callback.
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.close_preferences(cx);
            view.flush_settings().unwrap();
            view.perform(Command::Settings, window, cx);
        });
        // Keep the reader entity alive: only the main-window guard can prevent
        // this deferred Settings request from creating an orphan.
        window.remove_window();
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.windows().is_empty());
    assert!(cx.update_window(preferences, |_, _, _| ()).is_err());
    fixture
        .view
        .read_with(cx, |view, _| assert!(view.preferences_window().is_none()));
}

#[gpui_kit::test]
fn failed_preferences_result_clears_stale_handle_and_allows_localized_recovery(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let stale = open_preferences(&fixture, cx);
    cx.update_window(stale, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    let detail = "window creation failed: 日本語/{error}";
    fixture.view.update(cx, |view, cx| {
        assert_eq!(view.preferences_window(), Some(stale));
        view.preferences_opened(Err(anyhow::anyhow!(detail)), cx);
        assert!(view.preferences_window().is_none());
        assert!(view.message_error);
        assert_eq!(
            view.message,
            view.language()
                .format(Text::PreferencesOpenFailed, &[("error", detail.into())])
        );
    });
    render_windows(&fixture, cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert!(
            window
                .find("status-message")
                .label()
                .unwrap()
                .contains(detail)
        );
    })
    .unwrap();
    let replacement = open_preferences(&fixture, cx);
    assert_ne!(replacement, stale);
    assert_eq!(cx.windows().len(), 2);
}

#[cfg(any(unix, windows))]
#[gpui_kit::test]
fn queued_preference_then_close_flush_preserves_native_reading_data(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "queued-save", 4)])
        .unwrap();
    pump(&fixture, cx);
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(b"native-\xff".to_vec())
    };
    #[cfg(windows)]
    let name = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[110, 97, 116, 105, 118, 101, 45, 0xd800])
    };
    let native = fixture.directory.path().join(name);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            let mut book = (**view.reader.book.as_ref().unwrap()).clone();
            book.path = native.clone();
            view.reader.set_book(Arc::new(book), None);
            view.perform(Command::Bookmark, window, cx);
            view.navigate(2, window, cx);
        });
    })
    .unwrap();
    pump(&fixture, cx);
    let preferences = open_preferences(&fixture, cx);
    cx.update_window(preferences, |_, window, cx| {
        // Queue the change and close in the same event turn. The deferred
        // reader mutation must not depend on the Preferences entity surviving.
        window.click("紙色", cx);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(cx.update_window(preferences, |_, _, _| ()).is_err());
    let expected = cx
        .update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.perform(Command::Close, window, cx);
                assert_eq!(
                    view.reader.settings.preferences.background,
                    Background::Paper
                );
                view.flush_settings().unwrap();
                view.reader.settings.clone()
            })
        })
        .unwrap();
    pump(&fixture, cx);
    let saved = AppSettings::load_from(&fixture.directory.path().join("settings.json")).unwrap();
    assert_eq!(saved, expected);
    assert!(saved.recent.contains(&native));
    assert_eq!(saved.positions[&native], 2);
    assert_eq!(saved.bookmarks[&native], 0);
}

#[gpui_kit::test]
fn failed_save_is_visible_in_preferences_and_valid_settings_refuse_reset(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let preferences = open_preferences(&fixture, cx);
    let valid_path = fixture.directory.path().join("valid.json");
    fixture.view.update(cx, |view, _| {
        view.reader.settings.save_to(&valid_path).unwrap()
    });
    let original = std::fs::read(&valid_path).unwrap();
    fixture.view.update(cx, |view, _| {
        // A directory cannot be replaced with a settings file on any platform.
        view.settings_path = fixture.directory.path().to_path_buf();
    });
    click_preference(&fixture, "紙色", cx);
    pump(&fixture, cx);
    let error = fixture
        .view
        .read_with(cx, |view, _| view.save_error.clone().unwrap());
    cx.update_window(preferences, |_, window, cx| {
        window.scroll(
            "preferences-content",
            gpui_kit::ScrollDelta::Lines(point(0., -100.)),
            cx,
        );
        assert!(
            window
                .find("settings-save-error")
                .label()
                .unwrap()
                .contains(&error)
        );
        assert!(window.find("settings-save-error").visible());
    })
    .unwrap();
    let settings = fixture
        .view
        .read_with(cx, |view, _| view.reader.settings.clone());
    fixture
        .view
        .update(cx, |view, _| view.settings_path = valid_path.clone());
    click_preference(&fixture, "reset-corrupt-settings", cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.message_error);
        assert_eq!(view.reader.settings, settings);
        assert!(view.save_error.is_some());
    });
    assert_eq!(std::fs::read(valid_path).unwrap(), original);
    assert!(
        !std::fs::read_dir(fixture.directory.path())
            .unwrap()
            .any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".corrupt-")
            })
    );
}
