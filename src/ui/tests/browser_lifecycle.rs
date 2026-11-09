//! Regressions found during the integrated browser lifecycle and security review.

use super::*;
use gpui_kit::{MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput, test::TestQueryExt};

fn browse(fixture: &Fixture, path: &Path, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.browse(path.to_path_buf(), window, cx));
    })
    .unwrap();
    pump(fixture, cx);
}

#[gpui_kit::test]
fn unchanged_cover_failures_survive_navigation_until_refresh_or_source_change(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let a = book(&fixture, "negative-cache-a", 0);
    let b = book(&fixture, "negative-cache-b", 0);
    let corrupt = a.join("broken.png");
    std::fs::write(&corrupt, b"invalid PNG").unwrap();
    std::fs::write(a.join("unsupported.rar"), b"not parsed").unwrap();
    browse(&fixture, &a, cx);
    for _ in 0..5 {
        pump(&fixture, cx);
    }
    let attempted = fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.browser.active_jobs(), 0);
        assert_eq!(view.browser.failure_count(), 2);
        view.browser.requested_count()
    });
    browse(&fixture, &b, cx);
    browse(&fixture, &a, cx);
    for _ in 0..3 {
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.browser.requested_count(),
            attempted,
            "returning to unchanged failed/unsupported sources must not retry"
        );
        assert_eq!(view.browser.failure_count(), 2);
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-refresh", cx)
    })
    .unwrap();
    for _ in 0..5 {
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.browser.requested_count(),
            attempted + 2,
            "explicit refresh must retry both terminal outcomes"
        );
        assert_eq!(view.browser.failure_count(), 2);
    });
    image::RgbaImage::from_pixel(120, 200, image::Rgba([80, 160, 220, 255]))
        .save(&corrupt)
        .unwrap();
    browse(&fixture, &b, cx);
    browse(&fixture, &a, cx);
    for _ in 0..5 {
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.browser.requested_count(),
            attempted + 3,
            "only the changed source should get another attempt"
        );
        assert_eq!(view.browser.cache_count(), 1);
        assert_eq!(view.browser.active_jobs(), 0);
    });
}

#[gpui_kit::test]
fn canonical_aliases_share_one_cover_worker_slot(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "aliases", 1);
    browse(&fixture, &folder, cx);
    let before = fixture.view.update(cx, |view, cx| {
        let source = view.listing.as_ref().unwrap().entries[0].clone();
        let entries = (0..20)
            .map(|index| {
                let mut entry = source.clone();
                entry.name = format!("alias-{index}.png");
                entry.entry_path = folder.join(&entry.name);
                entry
            })
            .collect();
        view.listing.as_mut().unwrap().entries = entries;
        view.listing_revision += 1;
        view.browser_listing_changed(cx);
        view.browser.requested_count()
    });
    for _ in 0..5 {
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.browser.requested_count() - before, 1);
        assert_eq!(view.browser.cache_count(), 1);
        assert_eq!(view.browser.active_jobs(), 0);
        assert_eq!(view.browser.model.order.len(), 20);
    });
}

#[gpui_kit::test]
fn external_origin_clears_only_an_excluding_filter_and_reveals_the_source(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "external-origin", 40);
    browse(&fixture, &folder, cx);
    fixture.view.update(cx, |view, cx| {
        view.browser.model.query = "1.png".into();
        view.rebuild_browser(cx);
        view.remember_browser_location();
    });
    let image = folder.join("40.png");
    fixture.sender.send(vec![image.clone()]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("backspace", cx)
    })
    .unwrap();
    pump(&fixture, cx);
    let canonical = image.canonicalize().unwrap();
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.browser.model.query, "");
        assert_eq!(view.browser.model.selected.as_ref(), Some(&canonical));
    });
    cx.update_window(fixture.window, |_, window, _| {
        let tile = window.find(super::super::browser::browser_entry_id(&canonical));
        let grid = window.find("browser-grid").bounds();
        assert!(tile.visible());
        assert!(tile.bounds().top() >= grid.top() - px(0.5));
        assert!(tile.bounds().bottom() <= grid.bottom() + px(0.5));
    })
    .unwrap();
}

#[gpui_kit::test]
fn newer_book_open_rejects_a_pending_listing_before_success_or_failure(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let first = book(&fixture, "first-location", 3);
    let second = book(&fixture, "stale-location", 3);
    for succeeds in [true, false] {
        browse(&fixture, &first, cx);
        fixture.view.update(cx, |view, cx| {
            view.browser.model.query = "2.png".into();
            view.rebuild_browser(cx);
        });
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.browse(second.clone(), window, cx);
                view.open_browser_path(
                    first.join(if succeeds { "2.png" } else { "missing.png" }),
                    window,
                    cx,
                );
            });
        })
        .unwrap();
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(
                view.listing.as_ref().unwrap().path,
                first.canonicalize().unwrap()
            );
            assert_eq!(view.browser.model.query, "2.png");
            assert_eq!(
                view.content_mode,
                if succeeds {
                    ContentMode::Reading
                } else {
                    ContentMode::Browsing
                }
            );
        });
        if succeeds {
            cx.update_window(fixture.window, |_, window, cx| {
                window.press("backspace", cx)
            })
            .unwrap();
            pump(&fixture, cx);
            fixture.view.read_with(cx, |view, _| {
                assert_eq!(
                    view.listing.as_ref().unwrap().path,
                    first.canonicalize().unwrap()
                );
                assert_eq!(view.browser.model.query, "2.png");
            });
        }
    }
}

#[gpui_kit::test]
fn close_restarts_visible_cover_loading_without_refresh(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "close-covers", 3);
    fixture.sender.send(vec![folder.join("1.png")]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("backspace", cx)
    })
    .unwrap();
    for _ in 0..5 {
        pump(&fixture, cx);
    }
    let before = fixture
        .view
        .read_with(cx, |view, _| view.browser.requested_count());
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.perform(Command::Close, window, cx));
    })
    .unwrap();
    for _ in 0..5 {
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        assert!(view.reader.book.is_none());
        assert_eq!(view.content_mode, ContentMode::Browsing);
        assert!(view.browser.requested_count() > before);
        assert!(view.browser.cache_count() > 0);
    });
}

#[gpui_kit::test]
fn known_folder_counts_keep_the_policy_that_actually_opened_the_book(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "count-policy", 1);
    let nested = folder.join("nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::copy(folder.join("1.png"), nested.join("2.png")).unwrap();
    fixture.view.update(cx, |view, _| {
        view.reader.settings.preferences.recursive_folder = false
    });
    fixture.sender.send(vec![folder.clone()]).unwrap();
    pump(&fixture, cx);
    fixture
        .view
        .read_with(cx, |view, _| assert_eq!(view.page_count(), 1));
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.perform(Command::ToggleRecursiveFolder, window, cx)
        });
        window.press("backspace", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        let path = folder.canonicalize().unwrap();
        let entry = view
            .listing
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.path == path)
            .unwrap();
        assert_eq!(
            view.browser.model.known_pages.get(&(path.clone(), false)),
            Some(&1)
        );
        assert!(!view.browser.model.known_pages.contains_key(&(path, true)));
        assert_eq!(
            view.browser.model.page_count(entry, &view.reader.settings),
            None
        );
    });
}

#[gpui_kit::test]
fn browser_filter_sliders_and_delayed_slider_events_do_not_change_the_retained_book(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "retained-filters", 2)])
        .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("backspace", cx)
    })
    .unwrap();
    pump(&fixture, cx);
    let before = fixture.view.read_with(cx, |view, _| view.reader.filters);
    cx.update_window(fixture.window, |_, window, cx| {
        let slider = window.find_by_label(Language::TraditionalChinese.text(Text::Brightness));
        assert_eq!(slider.disabled(), Some(true));
        window.drag(
            slider.bounds().center(),
            point(slider.bounds().right() - px(1.), slider.bounds().center().y),
            cx,
        );
    })
    .unwrap();
    fixture.view.update(cx, |view, cx| {
        let slider = view.filter_sliders[0].clone();
        slider.update(cx, |_, cx| {
            cx.emit(SliderEvent::Change(
                gpui_kit::component::slider::SliderValue::Single(140.),
            ))
        });
    });
    pump(&fixture, cx);
    fixture
        .view
        .read_with(cx, |view, _| assert_eq!(view.reader.filters, before));
}

#[gpui_kit::test]
fn panel_drag_blocks_browser_navigation_and_escape(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "drag-guards", 20);
    fixture.sender.send(vec![folder.join("1.png")]).unwrap();
    pump(&fixture, cx);
    browse(&fixture, &folder, cx);
    let selected = fixture
        .view
        .read_with(cx, |view, _| view.browser.model.selected.clone());
    cx.update_window(fixture.window, |_, window, cx| {
        let handle = window.find(("resizable-handle", 0usize)).bounds().center();
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: handle,
                ..Default::default()
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseMove(MouseMoveEvent {
                position: handle + point(px(18.), px(0.)),
                pressed_button: Some(MouseButton::Left),
                ..Default::default()
            }),
            cx,
        );
        assert!(
            cx.has_active_drag(),
            "fixture must start an actual resize drag"
        );
        for key in [
            "right",
            "down",
            "home",
            "end",
            "backspace",
            "enter",
            "escape",
        ] {
            window.press(key, cx);
            assert_eq!(fixture.view.read(cx).content_mode, ContentMode::Browsing);
            assert_eq!(fixture.view.read(cx).browser.model.selected, selected);
        }
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position: handle + point(px(18.), px(0.)),
                ..Default::default()
            }),
            cx,
        );
        cx.stop_active_drag(window);
    })
    .unwrap();
}

#[gpui_kit::test]
fn close_menu_stays_available_for_the_retained_book_while_browsing(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "close-menu", 2)])
        .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("backspace", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click(("menu", 0usize), cx);
        let item = window.find_by_label(Language::TraditionalChinese.text(Text::CloseComic));
        assert_ne!(item.disabled(), Some(true));
        window.press("escape", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.reader.book.is_some());
        assert_eq!(view.content_mode, ContentMode::Browsing);
    });
}

#[gpui_kit::test]
fn continuous_page_anchor_survives_a_resize_while_the_browser_is_visible(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "hidden-resize", 8)])
        .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.set_reading_mode(ReadingMode::Continuous, window, cx);
            view.navigate(2, window, cx);
            view.continuous.scroll_to(ListOffset {
                item_ix: 2,
                offset_in_item: px(35.),
            });
        });
    })
    .unwrap();
    pump(&fixture, cx);
    let before = fixture.view.read_with(cx, |view, _| {
        (
            view.reader.current,
            view.continuous.logical_scroll_top().item_ix,
            view.reader.zoom,
        )
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("backspace", cx)
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.resize(size(px(960.), px(640.)));
        window.bounds_changed(cx);
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.current, before.0);
        assert_eq!(view.continuous.logical_scroll_top().item_ix, before.1);
        assert_eq!(view.reader.zoom, before.2);
        assert!(view.continuous.logical_scroll_top().offset_in_item >= px(0.));
    });
}

#[gpui_kit::test]
fn failed_navigation_keeps_loading_the_retained_listing_after_scroll(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "failed-navigation", 60);
    browse(&fixture, &folder, cx);
    fixture.view.update(cx, |view, cx| {
        view.browser.model.query = "png".into();
        view.rebuild_browser(cx);
    });
    for _ in 0..5 {
        pump(&fixture, cx);
    }
    let before = fixture.view.read_with(cx, |view, _| {
        (
            view.browser.requested_count(),
            view.browser.cache_count(),
            view.browser.model.selected.clone(),
        )
    });
    browse(&fixture, &folder.join("missing"), cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.listing_error.is_some());
        assert_eq!(
            view.listing.as_ref().unwrap().path,
            folder.canonicalize().unwrap()
        );
        assert_eq!(view.browser.model.query, "png");
        assert_eq!(view.browser.model.selected, before.2);
    });
    cx.update_window(fixture.window, |_, window, cx| window.press("end", cx))
        .unwrap();
    for _ in 0..5 {
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        assert!(view.browser.requested_count() > before.0);
        assert!(view.browser.cache_count() > before.1);
        assert_eq!(view.browser.active_jobs(), 0);
        assert_eq!(
            view.browser.model.selected.as_ref(),
            Some(&folder.join("60.png").canonicalize().unwrap())
        );
    });
}

fn delayed_listing(fixture: &Fixture, name: &str, cx: &mut TestAppContext) -> PathBuf {
    let path = fixture.directory.path().join(name);
    fixture.view.update(cx, |view, cx| {
        view.listing_revision += 1;
        view.listing = Some(DirectoryListing {
            path: path.clone(),
            parent: path.parent().map(Path::to_path_buf),
            entries: (0..1000)
                .map(|index| {
                    let name = format!("comic-{index:05}.cbz");
                    let path = path.join(&name);
                    book::DirectoryEntry {
                        name,
                        path: path.clone(),
                        entry_path: path,
                        kind: EntryKind::Archive,
                        is_directory: false,
                        size: Some(12),
                        modified: None,
                        created: None,
                    }
                })
                .collect(),
        });
        view.browser_listing_changed(cx);
    });
    render_windows(fixture, cx);
    render_windows(fixture, cx);
    path
}

#[gpui_kit::test]
fn delayed_workers_prioritize_visible_rows_and_refill_on_completion_without_ticks(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let gate = Arc::new(Mutex::new(super::super::browser::CoverJobGate::default()));
    fixture
        .view
        .update(cx, |view, _| view.browser.worker_gate = Some(gate.clone()));
    let path = delayed_listing(&fixture, "priority", cx);
    fixture.view.update(cx, |view, cx| {
        view.browser.grid.scroll_to(ListOffset {
            item_ix: 10,
            offset_in_item: px(17.),
        });
        cx.notify();
    });
    render_windows(&fixture, cx);
    let (first, before, retained_failures) = fixture.view.update(cx, |view, cx| {
        let first = view.browser.grid.logical_scroll_top().item_ix * view.browser.columns;
        let before = view.browser.requested_count();
        let retained_failures = view.browser.failure_sources();
        view.browser_tick(cx);
        assert_eq!(view.browser.active_jobs(), 2);
        (first, before, retained_failures)
    });
    cx.run_until_parked();
    let expected: Vec<_> = (first..first + 3)
        .map(|index| path.join(format!("comic-{index:05}.cbz")))
        .collect();
    assert_eq!(
        gate.lock().unwrap().started,
        expected[..2],
        "previous-row overscan must not delay the visible first row"
    );
    for _ in 0..4 {
        fixture.view.update(cx, |view, cx| view.browser_tick(cx));
        render_windows(&fixture, cx);
    }
    assert_eq!(
        gate.lock().unwrap().started.len(),
        2,
        "held jobs cannot be replaced by redraws"
    );
    // Finish the second job first. No tick or timer runs after this release.
    gate.lock().unwrap().release(expected[1].clone());
    cx.run_until_parked();
    assert_eq!(gate.lock().unwrap().started, expected);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.browser.active_jobs(), 2);
        assert_eq!(view.browser.requested_count() - before, 3);
        let mut expected_failures = retained_failures.clone();
        expected_failures.insert(expected[1].clone());
        assert_eq!(
            view.browser.failure_sources(),
            expected_failures,
            "only the completed current source may add a failure"
        );
    });
    render_windows(&fixture, cx);
    cx.update_window(fixture.window, |_, window, _| {
        let pending = path.join(format!("comic-{:05}.cbz", first + 3));
        for (path, state) in [
            (&expected[0], Text::CoverLoading),
            (&expected[1], Text::CoverFailed),
            (&pending, Text::CoverPending),
        ] {
            let id: gpui_kit::ElementId = super::super::browser::browser_entry_id(path).into();
            let label = Language::TraditionalChinese.text(state);
            let status = window.within(id.clone()).find("browser-cover-status");
            assert_eq!(status.label(), Some(label));
            assert!(status.visible());
            assert!(
                gpui_kit::base::test_support::texts(window)
                    .iter()
                    .any(|text| text.path().contains(&id)
                        && text.visible()
                        && text.text() == label),
                "cover state must paint text, not only expose an accessible label"
            );
        }
        let tile = window.find(super::super::browser::browser_entry_id(&expected[1]));
        let description = tile.description().unwrap();
        assert!(description.contains(&crate::file_tree::display_path(&expected[1])));
        assert!(
            description.contains("Cannot read"),
            "failed tooltip must retain its reason"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn obsolete_delayed_jobs_hold_actual_slots_across_navigation_scroll_and_resize(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let gate = Arc::new(Mutex::new(super::super::browser::CoverJobGate::default()));
    fixture
        .view
        .update(cx, |view, _| view.browser.worker_gate = Some(gate.clone()));
    delayed_listing(&fixture, "obsolete", cx);
    fixture.view.update(cx, |view, cx| view.browser_tick(cx));
    cx.run_until_parked();
    let old = gate.lock().unwrap().started.clone();
    assert_eq!(old.len(), 2);
    let current = delayed_listing(&fixture, "current", cx);
    let retained_failures = fixture
        .view
        .read_with(cx, |view, _| view.browser.failure_sources());
    cx.update_window(fixture.window, |_, window, cx| {
        window.resize(size(px(800.), px(540.)));
        window.bounds_changed(cx);
    })
    .unwrap();
    for _ in 0..3 {
        render_windows(&fixture, cx);
    }
    fixture.view.update(cx, |view, cx| {
        view.browser.grid.scroll_to(ListOffset {
            item_ix: 8,
            offset_in_item: px(11.),
        });
        cx.notify();
    });
    render_windows(&fixture, cx);
    let first = fixture.view.update(cx, |view, cx| {
        view.browser_tick(cx);
        assert_eq!(view.browser.active_jobs(), 2, "obsolete jobs still execute");
        view.browser.grid.logical_scroll_top().item_ix * view.browser.columns
    });
    cx.run_until_parked();
    assert_eq!(
        gate.lock().unwrap().started,
        old,
        "navigation cannot free synchronous slots"
    );
    for (index, path) in old.iter().enumerate() {
        gate.lock().unwrap().release(path.clone());
        cx.run_until_parked();
        let started = gate.lock().unwrap().started.clone();
        assert_eq!(
            started.len(),
            3 + index,
            "each completion frees exactly one actual slot"
        );
        assert_eq!(
            started.last(),
            Some(&current.join(format!("comic-{:05}.cbz", first + index)))
        );
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.browser.active_jobs(), 2);
            assert_eq!(
                view.browser.failure_sources(),
                retained_failures,
                "obsolete errors must not enter the cache; prior source failures remain"
            );
            assert_eq!(view.browser.cache_count(), 0);
        });
    }
}

#[gpui_kit::test]
fn unsupported_sources_do_not_retry_on_redraw_or_decoded_dimension_changes(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = fixture.directory.path().join("unsupported");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("original-name.cbr");
    std::fs::write(&file, b"RAR placeholder; classified before parsing").unwrap();
    browse(&fixture, &folder, cx);
    pump(&fixture, cx);
    let before = fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.browser.failure_count(), 1);
        assert_eq!(view.browser.active_jobs(), 0);
        view.browser.requested_count()
    });
    for width in [100., 220., 140., 100.] {
        cx.update_window(fixture.window, |_, window, cx| {
            window.resize(size(px(400. + width * 4.), px(540.)));
            window.bounds_changed(cx);
        })
        .unwrap();
        fixture.view.update(cx, |view, cx| {
            view.browser.tile_slider.update(cx, |_, cx| {
                cx.emit(SliderEvent::Change(
                    gpui_kit::component::slider::SliderValue::Single(width),
                ));
            });
        });
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.browser.requested_count(), before)
        });
    }
    cx.update_window(fixture.window, |_, window, _| {
        let id = super::super::browser::browser_entry_id(&file.canonicalize().unwrap());
        let tile = window.find(id.clone());
        let description = tile.description().unwrap();
        assert!(description.contains("original-name.cbr"));
        assert!(description.contains(&crate::file_tree::display_path(
            &file.canonicalize().unwrap()
        )));
        assert!(
            description.contains("RAR/CBR"),
            "full backend reason remains available"
        );
        let status = window.within(id).find("browser-cover-status");
        assert_eq!(
            status.label(),
            Some(Language::TraditionalChinese.text(Text::CoverUnsupported))
        );
    })
    .unwrap();
}
