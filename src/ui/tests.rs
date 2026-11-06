use super::*;
use gpui_kit::{AnyWindowHandle, TestAppContext, test::TestWindowExt};
use tempfile::TempDir;

struct Fixture {
    directory: TempDir,
    window: AnyWindowHandle,
    view: Entity<ReaderView>,
    sender: mpsc::Sender<Vec<PathBuf>>,
}

fn fixture(cx: &mut TestAppContext) -> Fixture {
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
                cx.new(|cx| {
                    ReaderView::new(
                        AppSettings::default(),
                        path,
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
        view.preferences_open = true;
        cx.notify();
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
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
        view.evict_cache(true, cx);
        assert_eq!(view.thumbs.len(), 160);
        assert!(view.thumbs.contains_key(&900));
        assert!(!view.thumbs.contains_key(&0));
    });
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
