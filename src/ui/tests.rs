use super::*;
use crate::i18n::Language;
use gpui_kit::{AnyWindowHandle, TestAppContext, test::TestWindowExt};
use tempfile::TempDir;

mod desktop;
mod i18n;
mod visual;
mod zoom;

struct Fixture {
    directory: TempDir,
    window: AnyWindowHandle,
    view: Entity<ReaderView>,
    sender: mpsc::Sender<Vec<PathBuf>>,
}

fn fixture(cx: &mut TestAppContext) -> Fixture {
    let mut settings = AppSettings::default();
    settings.preferences.language = Language::TraditionalChinese;
    fixture_with_settings(cx, settings)
}

fn fixture_with_settings(cx: &mut TestAppContext, settings: AppSettings) -> Fixture {
    cx.update(init);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
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
                cx.new(|cx| ReaderView::new(settings, path, None, None, launches, window, cx))
            },
        )
        .unwrap()
    });
    Fixture {
        directory,
        window,
        view,
        sender,
    }
}

fn book(fixture: &Fixture, name: &str, count: usize) -> PathBuf {
    let folder = fixture.directory.path().join(name);
    std::fs::create_dir(&folder).unwrap();
    for index in 0..count {
        image::RgbaImage::from_pixel(120, 200, image::Rgba([80, 160, 220, 255]))
            .save(folder.join(format!("{}.png", index + 1)))
            .unwrap();
    }
    folder
}

fn animated_thumbnail() -> CachedPage {
    let frames = (0..200)
        .map(|_| {
            let pixels = vec![80; 144 * 144 * 4];
            (
                imaging::DecodedPage {
                    width: 144,
                    height: 144,
                    byte_size: pixels.len(),
                    pixels,
                },
                image::Delay::from_numer_denom_ms(100, 1),
            )
        })
        .collect();
    cached_page(frames, 1, 0).unwrap()
}

fn pump(fixture: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..6 {
        cx.run_until_parked();
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| view.tick(window, cx));
            window.render_frame(cx);
        })
        .unwrap();
    }
    cx.run_until_parked();
}

#[gpui_kit::test]
fn controlled_popover_updates_content_and_isolates_reader_shortcuts(cx: &mut TestAppContext) {
    use gpui_kit::component::popover::Popover;

    struct Probe {
        open: bool,
        changes: usize,
        trigger_focus: FocusHandle,
        input: Entity<InputState>,
        outside: Entity<InputState>,
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let owner = cx.entity().downgrade();
            let input = self.input.clone();
            let label = self.changes.to_string();
            div()
                .size_full()
                .key_context("Reader")
                .on_action(cx.listener(|this, _: &Command, _, cx| {
                    this.changes += 100;
                    cx.notify();
                }))
                .child(
                    Popover::new("probe-popover")
                        .open(self.open)
                        .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                            this.open = *open;
                            cx.notify();
                        }))
                        .trigger(
                            Button::new("probe-trigger")
                                .label("Open")
                                .track_focus(&self.trigger_focus),
                        )
                        .content(move |_, _, _| {
                            let owner = owner.clone();
                            div()
                                .id("probe-panel")
                                .test_support()
                                .flex()
                                .flex_col()
                                .child(Input::new(&input).id("probe-input"))
                                .child(Button::new("probe-change").label(label.clone()).on_click(
                                    move |_, _, cx| {
                                        _ = owner.update(cx, |this, cx| {
                                            this.changes += 1;
                                            cx.notify();
                                        });
                                    },
                                ))
                        }),
                )
                .child(Input::new(&self.outside).id("probe-outside").w_32())
        }
    }

    cx.update(init);
    let (window, probe) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Probe {
                open: false,
                changes: 0,
                trigger_focus: cx.focus_handle(),
                input: cx.new(|cx| InputState::new(window, cx).default_value("62")),
                outside: cx.new(|cx| InputState::new(window, cx)),
            })
        })
        .unwrap()
    });
    cx.update_window(window, |_, window, cx| {
        probe.read(cx).trigger_focus.clone().focus(window, cx);
        window.press("enter", cx);
        assert!(window.try_find("probe-panel").is_some());
        window.click("probe-change", cx);
        assert_eq!(window.find("probe-change").label(), Some("1"));
        for key in ["down", "pagedown", "tab"] {
            window.press(key, cx);
        }
        assert_eq!(probe.read(cx).changes, 1);
        window.press("escape", cx);
        assert!(!probe.read(cx).open);
        assert!(probe.read(cx).trigger_focus.is_focused(window));
        window.click("probe-trigger", cx);
        window.click("probe-outside", cx);
        assert!(!probe.read(cx).open);
        assert_eq!(window.find("probe-outside").focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn latest_launch_wins_and_late_book_results_do_not_replace_it(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let first = book(&fixture, "first", 4);
    let second = book(&fixture, "second", 2);
    fixture.sender.send(vec![first.clone()]).unwrap();
    fixture.sender.send(vec![second.clone()]).unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.reader.book.as_ref().unwrap().path,
            second.canonicalize().unwrap()
        )
    });
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.open_path(first, window, cx);
            view.open_path(second.clone(), window, cx);
        })
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.reader.book.as_ref().unwrap().path,
            second.canonicalize().unwrap()
        )
    });
}

#[gpui_kit::test]
fn image_replacement_and_close_release_native_atlas_entries(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let path = book(&fixture, "comic", 3);
    fixture.sender.send(vec![path]).unwrap();
    pump(&fixture, cx);
    let original = fixture
        .view
        .read_with(cx, |view, _| view.pages[&0].tiles[0].image.clone());
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.has_image_atlas_entry(&original));
        window.press("]", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    let replacement = fixture
        .view
        .read_with(cx, |view, _| view.pages[&0].tiles[0].image.clone());
    assert_ne!(original.id, replacement.id);
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(!window.has_image_atlas_entry(&original));
        assert!(window.has_image_atlas_entry(&replacement));
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            cx,
        );
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert!(!window.has_image_atlas_entry(&replacement))
    })
    .unwrap();
    fixture.view.read_with(cx, |view, _| {
        assert!(view.pages.is_empty());
        assert!(view.thumbs.is_empty());
    });
}

#[gpui_kit::test]
fn paged_fitted_pages_do_not_scroll_into_blank_space(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "comic", 2);
    fixture.sender.send(vec![folder]).unwrap();
    pump(&fixture, cx);

    for mode in [ReadingMode::Single, ReadingMode::Spread] {
        for fit in [FitMode::Window, FitMode::Height, FitMode::Original] {
            cx.update_window(fixture.window, |_, window, cx| {
                fixture.view.update(cx, |view, cx| {
                    view.reader.set_mode(mode);
                    view.reader.set_fit(fit);
                    view.relayout(cx);
                    view.navigate(0, window, cx);
                });
            })
            .unwrap();
            pump(&fixture, cx);
            fixture.view.read_with(cx, |view, _| {
                assert_eq!(
                    view.paged_scroll.max_offset(),
                    point(px(0.), px(0.)),
                    "{mode:?} with {fit:?} must not add a blank scroll range"
                );
            });
            cx.update_window(fixture.window, |_, window, cx| {
                window.scroll(
                    "viewer",
                    gpui_kit::ScrollDelta::Pixels(point(px(-10_000.), px(-10_000.))),
                    cx,
                );
                window.press("down", cx);
            })
            .unwrap();
            pump(&fixture, cx);
            fixture.view.read_with(cx, |view, _| {
                assert_eq!(view.paged_scroll.offset(), point(px(0.), px(0.)));
                assert_eq!(view.reader.current, 0);
            });
        }
    }
}

#[gpui_kit::test]
fn paged_zoom_scroll_stops_at_page_edges_and_clamps_after_fit(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "comic", 2);
    for (index, width, height) in [(1, 320, 480), (2, 400, 600)] {
        image::RgbaImage::from_pixel(width, height, image::Rgba([80, 160, 220, 255]))
            .save(folder.join(format!("{index}.png")))
            .unwrap();
    }
    fixture.sender.send(vec![folder.clone()]).unwrap();
    pump(&fixture, cx);

    for mode in [ReadingMode::Single, ReadingMode::Spread] {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.reader.set_mode(mode);
                view.reader.set_zoom(8.);
                view.relayout(cx);
                view.navigate(0, window, cx);
            });
        })
        .unwrap();
        pump(&fixture, cx);
        let (maximum, indices) = fixture.view.read_with(cx, |view, _| {
            let maximum = view.paged_scroll.max_offset();
            assert!(maximum.x > px(0.));
            assert!(maximum.y > px(0.));
            (maximum, view.reader.display_indices())
        });
        cx.update_window(fixture.window, |_, window, cx| {
            window.scroll(
                "viewer",
                gpui_kit::ScrollDelta::Pixels(point(px(-100_000.), px(0.))),
                cx,
            );
            window.scroll(
                "viewer",
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-100_000.))),
                cx,
            );
        })
        .unwrap();
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.paged_scroll.offset(), point(-maximum.x, -maximum.y));
            assert_eq!(view.reader.current, 0);
        });
        cx.update_window(fixture.window, |_, window, cx| {
            window.render_frame(cx);
            let viewer = window.find("viewer").bounds();
            let pages: Vec<_> = indices
                .iter()
                .map(|index| {
                    window
                        .find(SharedString::from(format!(
                            "page:{}:{}.png",
                            folder.canonicalize().unwrap().display(),
                            index + 1
                        )))
                        .bounds()
                })
                .collect();
            let right = pages.iter().map(|page| page.right()).max().unwrap();
            let bottom = pages.iter().map(|page| page.bottom()).max().unwrap();
            assert!(f32::from(right - viewer.right()).abs() < 1.);
            assert!(f32::from(bottom - viewer.bottom()).abs() < 1.);

            window.press("up", cx);
            assert_eq!(
                fixture.view.read(cx).paged_scroll.offset().y,
                -maximum.y + px(80.)
            );
            window.press("down", cx);
            window.press("down", cx);
            assert_eq!(fixture.view.read(cx).paged_scroll.offset().y, -maximum.y);

            window.drag(
                point(viewer.right() - px(6.), viewer.bottom() - px(20.)),
                point(viewer.right() - px(6.), viewer.top() + px(20.)),
                cx,
            );
            assert_eq!(fixture.view.read(cx).paged_scroll.offset().y, px(0.));
            window.drag(
                point(viewer.right() - px(20.), viewer.bottom() - px(6.)),
                point(viewer.left() + px(20.), viewer.bottom() - px(6.)),
                cx,
            );
            assert_eq!(
                fixture.view.read(cx).paged_scroll.offset(),
                point(px(0.), px(0.))
            );
            for position in [
                point(viewer.size.width - px(6.), viewer.size.height / 2.),
                point(viewer.size.width / 2., viewer.size.height - px(6.)),
            ] {
                window.click_at("viewer", position, cx);
            }
            let view = fixture.view.read(cx);
            let offset = view.paged_scroll.offset();
            assert!(offset.x < px(0.) && offset.x >= -maximum.x);
            assert!(offset.y < px(0.) && offset.y >= -maximum.y);
            assert_eq!(view.reader.current, 0);
            window.press("f", cx);
        })
        .unwrap();
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.reader.settings.preferences.fit, FitMode::Window);
            assert_eq!(view.paged_scroll.max_offset(), point(px(0.), px(0.)));
            assert_eq!(view.paged_scroll.offset(), point(px(0.), px(0.)));
        });
    }
}

#[gpui_kit::test]
fn continuous_remeasurement_preserves_progress_and_explicit_page_target(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let path = book(&fixture, "comic", 5);
    fixture.sender.send(vec![path]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.reader.set_zoom(4.);
            view.reader.set_mode(ReadingMode::Continuous);
            view.relayout(cx);
            view.navigate(2, window, cx);
        })
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.current, 2);
        assert_eq!(view.continuous_target, Some(2));
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.scroll(
            "viewer",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-200.))),
            cx,
        );
    })
    .unwrap();
    pump(&fixture, cx);
    let before = fixture
        .view
        .read_with(cx, |view, _| view.continuous.logical_scroll_top());
    assert!(before.offset_in_item > px(0.));
    cx.update_window(fixture.window, |_, _window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.image_revision += 1;
            view.reader.filters.brightness = 120.;
            view.relayout(cx);
            view.load_requested(cx);
        })
    })
    .unwrap();
    pump(&fixture, cx);
    let after = fixture
        .view
        .read_with(cx, |view, _| view.continuous.logical_scroll_top());
    assert_eq!(before.item_ix, after.item_ix);
    assert!((f32::from(before.offset_in_item - after.offset_in_item)).abs() < 1.);
}

#[gpui_kit::test]
fn corrupt_settings_reset_is_explicit_and_preserves_original_bytes(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let path = fixture.directory.path().join("settings.json");
    std::fs::write(&path, b"{broken").unwrap();
    fixture.view.update(cx, |view, cx| {
        view.save_error = Some("Invalid settings".into());
        cx.notify();
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        window.click("preferences", cx);
        assert_eq!(fixture.view.read(cx).inspector_tab, InspectorTab::Reading);
        window.scroll(
            "inspector",
            gpui_kit::ScrollDelta::Lines(point(0., -100.)),
            cx,
        );
        window.click("reset-corrupt-settings", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    assert!(AppSettings::load_from(&path).is_ok());
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
    assert_eq!(std::fs::read(&backups[0]).unwrap(), b"{broken");
}

#[gpui_kit::test]
fn thumbnail_eviction_tracks_the_sidebar_viewport_instead_of_the_reading_page(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    fixture.view.update(cx, |view, cx| {
        view.thumbnails.reset_with_uniform_height(1000, px(160.));
        view.thumbnails.scroll_to(ListOffset {
            item_ix: 900,
            offset_in_item: px(0.),
        });
        for index in (0..160).chain(std::iter::once(900)) {
            let pixels = vec![80, 160, 220, 255];
            let page = imaging::DecodedPage {
                width: 1,
                height: 1,
                byte_size: 4,
                pixels,
            };
            view.thumbs.insert(
                index,
                cached_page(vec![(page, image::Delay::from_numer_denom_ms(0, 1))], 4, 0).unwrap(),
            );
        }
        view.evict_cache(true, None, cx);
        assert_eq!(view.thumbs.len(), 160);
        assert!(view.thumbs.contains_key(&900));
        assert!(!view.thumbs.contains_key(&0));
    });
}

#[gpui_kit::test]
fn visible_animated_thumbnails_remain_cached_above_the_budget(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "animations", 3);
    fixture.view.update(cx, |view, cx| {
        view.reader
            .set_book(Arc::new(Book::open(&folder, false).unwrap()), None);
        for index in 0..3 {
            view.thumbs.insert(index, animated_thumbnail());
        }
        let original = [
            view.thumbs[&0].tiles[0].image.id,
            view.thumbs[&1].tiles[0].image.id,
        ];
        assert!(view.thumbs[&0].allocation + view.thumbs[&1].allocation > THUMB_CACHE_BYTES);
        for _ in 0..5 {
            for index in 0..2 {
                let _ = view.thumbnail(index, cx);
            }
            view.evict_cache(true, None, cx);
            assert_eq!(view.thumbs.len(), 2);
            assert_eq!(view.thumbs[&0].tiles[0].image.id, original[0]);
            assert_eq!(view.thumbs[&1].tiles[0].image.id, original[1]);
            view.load_requested(cx);
            assert!(!view.inflight.iter().any(|key| key.3));
            assert!(!view.wanted.borrow().iter().any(|(_, thumb)| *thumb));
        }
        // A thumbnail can be released once it leaves the rendered list range.
        view.visible_thumbnails.borrow_mut().clear();
        let _ = view.thumbnail(1, cx);
        view.evict_cache(true, None, cx);
        assert_eq!(view.thumbs.len(), 1);
        assert!(view.thumbs.contains_key(&1));
        assert!(
            view.thumbs
                .values()
                .map(|page| page.allocation)
                .sum::<usize>()
                <= THUMB_CACHE_BYTES
        );
    });
}

#[gpui_kit::test]
fn hiding_the_sidebar_trims_previously_visible_animated_thumbnails(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "animations", 2);
    fixture.sender.send(vec![folder]).unwrap();
    pump(&fixture, cx);
    fixture.view.update(cx, |view, cx| {
        for index in 0..2 {
            view.thumbs
                .insert(index, animated_thumbnail())
                .unwrap()
                .release(None, cx);
        }
        cx.notify();
    });
    pump(&fixture, cx);
    let images = fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.visible_thumbnails.borrow().len(), 2);
        assert!(
            view.thumbs
                .values()
                .map(|page| page.allocation)
                .sum::<usize>()
                > THUMB_CACHE_BYTES
        );
        view.thumbs
            .values()
            .map(|page| page.tiles[0].image.clone())
            .collect::<Vec<_>>()
    });
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.perform(Command::Sidebar, window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    pump(&fixture, cx);
    let retained = fixture.view.read_with(cx, |view, _| {
        assert!(view.visible_thumbnails.borrow().is_empty());
        assert_eq!(view.thumbs.len(), 1);
        assert!(
            view.thumbs
                .values()
                .map(|page| page.allocation)
                .sum::<usize>()
                <= THUMB_CACHE_BYTES
        );
        view.thumbs.values().next().unwrap().tiles[0].image.id
    });
    cx.update_window(fixture.window, |_, window, _| {
        for image in images.iter().filter(|image| image.id != retained) {
            assert!(!window.has_image_atlas_entry(image));
        }
    })
    .unwrap();
}

#[cfg(any(unix, windows))]
#[gpui_kit::test]
fn native_book_paths_do_not_block_settings_after_close(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "native", 3);
    fixture.sender.send(vec![folder]).unwrap();
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
    // Keep real page sources but use a native book identity. APFS rejects
    // invalid UTF-8 filenames, so this checks settings without that dependency.
    fixture.view.update(cx, |view, cx| {
        let mut book = (**view.reader.book.as_ref().unwrap()).clone();
        book.path = native.clone();
        view.reader.set_book(Arc::new(book), None);
        view.dirty = true;
        cx.notify();
    });
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("b", cx);
        window.press("pagedown", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-w"
            } else {
                "ctrl-w"
            },
            cx,
        );
        fixture.view.update(cx, |view, cx| {
            view.reader.settings.preferences.interval = 17;
            view.dirty = true;
            cx.notify();
        });
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.reader.book.is_none());
        assert!(view.save_error.is_none(), "{:?}", view.save_error);
    });
    let saved = AppSettings::load_from(&fixture.directory.path().join("settings.json")).unwrap();
    assert!(saved.recent.contains(&native));
    assert_eq!(saved.positions[&native], 1);
    assert_eq!(saved.bookmarks[&native], 0);
    assert_eq!(saved.preferences.interval, 17);
}

#[gpui_kit::test]
fn continuous_narrow_pages_center_in_the_viewport_not_the_widest_page(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "mixed", 3);
    image::RgbaImage::from_pixel(2000, 200, image::Rgba([80, 160, 220, 255]))
        .save(folder.join("1.png"))
        .unwrap();
    fixture.sender.send(vec![folder.clone()]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.reader.set_mode(ReadingMode::Continuous);
            view.reader.set_fit(FitMode::Original);
            view.relayout(cx);
            view.navigate(1, window, cx);
        })
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        let page = window
            .find(SharedString::from(format!(
                "page:{}:2.png",
                folder.canonicalize().unwrap().display()
            )))
            .bounds();
        let viewer = window.find("viewer").bounds();
        assert!((f32::from(page.center().x - viewer.center().x)).abs() < 1.);
    })
    .unwrap();
}

#[test]
fn long_pages_use_lossless_gpu_tiles_and_keep_original_geometry() {
    let pixels = [10, 20, 30, 255].repeat(7 * 20_000);
    let page = imaging::DecodedPage {
        width: 7,
        height: 20_000,
        byte_size: pixels.len(),
        pixels,
    };
    let cached = cached_page(
        vec![(page, image::Delay::from_numer_denom_ms(80, 1))],
        123,
        0,
    )
    .unwrap();
    assert_eq!((cached.width, cached.height), (7, 20_000));
    assert_eq!(cached.tiles.len(), 3);
    assert_eq!(cached.allocation, 7 * 20_000 * 4);
    assert_eq!(cached.tiles[1].y, MAX_TEXTURE_SIDE);
    assert_eq!(cached.tiles[2].height, 20_000 - 2 * MAX_TEXTURE_SIDE);
    for tile in &cached.tiles {
        assert!(tile.width <= MAX_TEXTURE_SIDE && tile.height <= MAX_TEXTURE_SIDE);
        assert_eq!(&tile.image.as_bytes(0).unwrap()[..4], &[30, 20, 10, 255]);
        assert_eq!(tile.image.delay(0).numer_denom_ms(), (80, 1));
    }
}

#[gpui_kit::test]
fn continuous_small_pages_load_the_whole_visible_range(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let path = book(&fixture, "comic", 40);
    fixture.sender.send(vec![path]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.reader.set_mode(ReadingMode::Continuous);
            view.reader.set_zoom(0.1);
            view.relayout(cx);
            view.navigate(0, window, cx);
        })
    })
    .unwrap();
    for _ in 0..12 {
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        assert!(view.visible.borrow().iter().any(|index| *index > 8));
        for index in view.visible.borrow().iter() {
            assert!(
                view.pages.contains_key(index),
                "visible page {index} must not remain a placeholder"
            );
        }
        assert!(view.geometry.len() > 12);
    });
}

#[gpui_kit::test]
fn native_open_and_save_dialogs_route_original_page_bytes(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "comic", 3);
    cx.update_window(fixture.window, |_, window, cx| window.press("ctrl-o", cx))
        .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec![folder.join("2.png")])
    });
    pump(&fixture, cx);
    fixture
        .view
        .read_with(cx, |view, _| assert_eq!(view.reader.current, 1));
    let destination = fixture.directory.path().join("export.png");
    cx.update_window(fixture.window, |_, window, cx| window.press("ctrl-s", cx))
        .unwrap();
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| Some(destination.clone()));
    pump(&fixture, cx);
    assert_eq!(
        std::fs::read(destination).unwrap(),
        std::fs::read(folder.join("2.png")).unwrap()
    );
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("ctrl-shift-o", cx)
    })
    .unwrap();
    cx.simulate_path_prompt_response(|options| {
        assert!(!options.files && options.directories);
        None
    });
    pump(&fixture, cx);
    fixture
        .view
        .read_with(cx, |view, _| assert_eq!(view.reader.current, 1));
}

#[gpui_kit::test]
fn file_browser_history_fullscreen_and_slideshow_commands_work(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "comic", 3);
    fixture.sender.send(vec![folder.clone()]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("檔案", cx);
        window.click("path-input", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input(&folder.to_string_lossy(), cx);
        window.press("enter", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        let entry = SharedString::from(format!(
            "entry:{}",
            folder.join("2.png").canonicalize().unwrap().display()
        ));
        window.click(entry, cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture
        .view
        .read_with(cx, |view, _| assert_eq!(view.reader.current, 1));
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("f11", cx);
        assert!(window.is_fullscreen());
        window.press("escape", cx);
        assert!(!window.is_fullscreen());
        window.click("inspector-playback", cx);
        window.click("slideshow", cx);
    })
    .unwrap();
    fixture.view.update(cx, |view, _| {
        assert!(view.reader.slideshow);
        view.next_slide = Instant::now();
    });
    pump(&fixture, cx);
    fixture
        .view
        .read_with(cx, |view, _| assert_eq!(view.reader.current, 2));
    fixture
        .view
        .update(cx, |view, _| view.next_slide = Instant::now());
    pump(&fixture, cx);
    fixture
        .view
        .read_with(cx, |view, _| assert!(!view.reader.slideshow));
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("最近開啟", cx);
        window.click("clear-history", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.reader.settings.recent.is_empty())
    });
}

#[test]
fn delayed_snapshot_cannot_replace_a_newer_flush() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("settings.json");
    let lock = Mutex::new(());
    let epoch = AtomicU64::new(2);
    let mut newest = AppSettings::default();
    newest.preferences.interval = 23;
    save_snapshot(&newest, &path, &lock, &epoch, 2).unwrap();
    save_snapshot(&AppSettings::default(), &path, &lock, &epoch, 1).unwrap();
    assert_eq!(
        AppSettings::load_from(&path).unwrap().preferences.interval,
        23
    );
}
