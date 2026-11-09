//! Production pixels and long-name captures for the central browser.

use super::*;
#[path = "../../examples/support/browser_fixture.rs"]
mod browser_fixture;
use browser_fixture::BrowserFixture;

fn entry_id(path: &Path) -> gpui_kit::SharedString {
    format!("browser-entry:{:?}", path.as_os_str()).into()
}

fn search(cx: &mut HeadlessAppContext, reader: AnyWindowHandle, query: &str) {
    cx.update_window(reader, |_, window, cx| {
        window.click("browser-search", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input(query, cx);
        if query.is_empty() {
            window.press("backspace", cx);
        }
    })
    .unwrap();
    settle(cx, reader);
}

fn wait_for_cover(
    cx: &mut HeadlessAppContext,
    reader: AnyWindowHandle,
    path: &Path,
    language: Language,
    state: Text,
) {
    for _ in 0..50 {
        let done = cx
            .update_window(reader, |_, window, _| {
                window.within(entry_id(path)).find("browser-cover").label()
                    == Some(language.text(state))
            })
            .unwrap();
        if done {
            return;
        }
        settle(cx, reader);
    }
    panic!("cover must settle: {path:?} -> {state:?}");
}

fn assert_cover_pixels(
    screenshot: &image::RgbaImage,
    bounds: Bounds<gpui_kit::Pixels>,
    logical_width: f32,
    color: [u8; 4],
    path: &Path,
) {
    let scale = screenshot.width() as f32 / logical_width;
    let left = (f32::from(bounds.left()) * scale).max(0.) as u32;
    let top = (f32::from(bounds.top()) * scale).max(0.) as u32;
    let right = ((f32::from(bounds.right()) * scale) as u32).min(screenshot.width());
    let bottom = ((f32::from(bounds.bottom()) * scale) as u32).min(screenshot.height());
    let mut matches = 0;
    for y in top..bottom {
        for x in left..right {
            let pixel = screenshot.get_pixel(x, y);
            if (0..3).all(|channel| pixel[channel].abs_diff(color[channel]) <= 5) {
                matches += 1;
            }
        }
    }
    assert!(
        matches > 100,
        "{path:?}: expected decoded source pixels, found {matches}"
    );
}

fn assert_landscape_contained(
    screenshot: &image::RgbaImage,
    bounds: Bounds<gpui_kit::Pixels>,
    logical_width: f32,
) {
    let scale = screenshot.width() as f32 / logical_width;
    let left = f32::from(bounds.left());
    let top = f32::from(bounds.top());
    let width = f32::from(bounds.size.width);
    let image_height = width * 288. / 512.;
    let image_top = top + (f32::from(bounds.size.height) - image_height) / 2.;
    // All four source corners have a paper border. Cropping to fill the cover
    // well loses these corners, even though interior artwork pixels still match.
    for (x, y) in [
        (left + 2., image_top + 2.),
        (left + width - 2., image_top + 2.),
        (left + 2., image_top + image_height - 2.),
        (left + width - 2., image_top + image_height - 2.),
    ] {
        let pixel = screenshot.get_pixel((x * scale) as u32, (y * scale) as u32);
        assert!(
            (0..3).all(|channel| pixel[channel].abs_diff([248, 244, 230][channel]) <= 5),
            "landscape source corner must remain visible: {pixel:?}"
        );
    }
}

fn capture_title_lines(
    cx: &mut HeadlessAppContext,
    reader: AnyWindowHandle,
    fixture: &BrowserFixture,
) {
    search(cx, reader, "Synthetic");
    let paths: Vec<_> = fixture
        .supported_paths
        .iter()
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.starts_with("[Synthetic Press]") || name.starts_with("UnbrokenSynthetic")
        })
        .collect();
    assert_eq!(paths.len(), 4);
    for width in [100_f32, 140., 220.] {
        cx.update_window(reader, |_, window, cx| {
            let slider = window.find("browser-tile-width").bounds();
            let fraction = ((width - 100.) / 120.).clamp(0.001, 0.999);
            window.click_at(
                "browser-tile-width",
                point(slider.size.width * fraction, slider.size.height / 2.),
                cx,
            );
        })
        .unwrap();
        settle(cx, reader);
        for lines in [2_usize, 3] {
            cx.update_window(reader, |_, window, cx| {
                window.click(("browser-name-lines", lines), cx)
            })
            .unwrap();
            settle(cx, reader);
            cx.update_window(reader, |_, window, _| {
                for path in &paths {
                    let name = path.file_name().unwrap().to_string_lossy();
                    let id: gpui_kit::ElementId = entry_id(path).into();
                    let tile = window.find(id.clone());
                    let title = window.within(id.clone()).find("browser-title");
                    let cover = window.within(id.clone()).find("browser-cover");
                    assert!((f32::from(cover.bounds().size.width) - width).abs() <= 1.);
                    assert_eq!(tile.label(), Some(name.as_ref()));
                    let text = gpui_kit::base::test_support::texts(window)
                        .into_iter()
                        .find(|text| {
                            text.path().contains(&id) && text.text() == name && text.visible()
                        })
                        .expect("the real native text shaper must paint the filename");
                    assert_eq!(
                        text.bounds().size.height,
                        px(lines as f32 * 18.),
                        "native {width}px/{lines} lines: {name}, bounds {:?}",
                        text.bounds()
                    );
                    assert!(contains_bounds(title.bounds(), text.bounds()));
                    assert!(contains_bounds(tile.bounds(), title.bounds()));
                    assert!(
                        window
                            .within(id)
                            .try_find("browser-tile-metadata")
                            .is_none()
                    );
                }
            })
            .unwrap();
            snapshot(
                cx,
                reader,
                &format!("browser-parity-native-titles-{width:.0}-{lines}-lines"),
            );
        }
    }
    // Leave the following language/size matrix at the specified defaults.
    cx.update_window(reader, |_, window, cx| {
        let bounds = window.find("browser-tile-width").bounds();
        window.click_at(
            "browser-tile-width",
            point(bounds.size.width / 3., bounds.size.height / 2.),
            cx,
        );
        window.click(("browser-name-lines", 2_usize), cx);
    })
    .unwrap();
    settle(cx, reader);
}

pub(super) fn capture(cx: &mut HeadlessAppContext, temporary: &Path) {
    let fixture = BrowserFixture::create(&temporary.join("browser-parity-library")).unwrap();
    let original_source = std::fs::read(&fixture.source).unwrap();
    let mut settings = AppSettings::default();
    settings.preferences.language = Language::TraditionalChinese;
    settings.preferences.sidebar_mode = mangameeya_rust::settings::SidebarMode::Recent;
    settings.preferences.panel_layout.inspector_visible = false;
    settings.recent.push(fixture.source.clone());
    let (_sender, launches) = mpsc::channel();
    let (reader, view) = open_reader_fixture(
        cx,
        temporary.join("browser-parity-settings.json"),
        settings,
        None,
        launches,
    );
    // Select the known synthetic Recent item before polling any startup work.
    // Merely hiding Files does not prevent home covers from loading.
    cx.update_window(reader, |_, window, cx| {
        window.render_frame(cx);
        window.click(format!("recent:{}", fixture.source.display()), cx);
    })
    .unwrap();
    settle(cx, reader);
    let cold_start = std::time::Instant::now();
    cx.update_window(reader, |_, window, cx| {
        window.resize(size(px(1600.), px(1000.)));
        window.bounds_changed(cx);
        window.click("viewer", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    settle(cx, reader);
    for path in &fixture.supported_paths {
        wait_for_cover(
            cx,
            reader,
            path,
            Language::TraditionalChinese,
            Text::CoverReady,
        );
    }
    for path in &fixture.unsupported_paths {
        wait_for_cover(
            cx,
            reader,
            path,
            Language::TraditionalChinese,
            Text::CoverUnsupported,
        );
    }
    for path in &fixture.failed_paths {
        wait_for_cover(
            cx,
            reader,
            path,
            Language::TraditionalChinese,
            Text::CoverFailed,
        );
    }
    let cold_ms = cold_start.elapsed().as_secs_f64() * 1000.;
    let warm_start = std::time::Instant::now();
    cx.update_window(reader, |_, window, cx| window.click("browser-toggle", cx))
        .unwrap();
    settle(cx, reader);
    cx.update_window(reader, |_, window, cx| window.click("browser-toggle", cx))
        .unwrap();
    settle(cx, reader);
    for path in &fixture.supported_paths {
        wait_for_cover(
            cx,
            reader,
            path,
            Language::TraditionalChinese,
            Text::CoverReady,
        );
    }
    let warm_ms = warm_start.elapsed().as_secs_f64() * 1000.;
    eprintln!(
        "Browser fixture cold-cache {cold_ms:.1} ms; retained-cache round trip {warm_ms:.1} ms; {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    std::fs::create_dir_all("target/ui-smoke").unwrap();
    std::fs::write("target/ui-smoke/browser-parity-metrics.json", serde_json::to_vec_pretty(&serde_json::json!({
        "platform": std::env::consts::OS, "architecture": std::env::consts::ARCH,
        "cold_cover_cache_ms": cold_ms, "retained_cache_round_trip_ms": warm_ms,
        "logical_size": [1600, 1000], "supported_sources": fixture.supported_paths.len(),
        "note": "Real wall time including production frame and event processing; cold refers to application cover cache, not the OS disk cache. No latency SLA is asserted."
    })).unwrap()).unwrap();
    snapshot(cx, reader, "browser-parity-zh-Hant-wide-default");
    cx.update_window(reader, |_, window, _| {
        assert!(window.try_find("browser-tile-metadata").is_none());
        assert!(window.try_find("browser-resume").is_none());
        assert!(window.try_find("browser-shuffle").is_none());
        let header = window.find("browser-header").bounds();
        assert!(
            header.size.height <= px(64.),
            "wide header must stay compact: {header:?}"
        );
    })
    .unwrap();
    for (index, path) in fixture.supported_paths.iter().enumerate() {
        search(cx, reader, &path.file_name().unwrap().to_string_lossy());
        wait_for_cover(
            cx,
            reader,
            path,
            Language::TraditionalChinese,
            Text::CoverReady,
        );
        let (bounds, width) = cx
            .update_window(reader, |_, window, _| {
                let node = window.within(entry_id(path)).find("browser-cover");
                assert!(node.visible());
                (node.bounds(), f32::from(window.viewport_size().width))
            })
            .unwrap();
        if let Some(image) = snapshot(cx, reader, &format!("browser-parity-cover-{index:02}")) {
            assert_cover_pixels(
                &image,
                bounds,
                width,
                fixture.expected_color(path).unwrap(),
                path,
            );
            if path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("Landscape")
            {
                assert_landscape_contained(&image, bounds, width);
            }
        }
    }
    for (paths, state, suffix) in [
        (
            &fixture.unsupported_paths,
            Text::CoverUnsupported,
            "unsupported",
        ),
        (&fixture.failed_paths, Text::CoverFailed, "failed"),
    ] {
        for (index, path) in paths.iter().enumerate() {
            search(cx, reader, &path.file_name().unwrap().to_string_lossy());
            wait_for_cover(cx, reader, path, Language::TraditionalChinese, state);
            cx.update_window(reader, |_, window, _| {
                let description = window
                    .find(entry_id(path))
                    .description()
                    .unwrap()
                    .to_owned();
                assert!(
                    window
                        .within(entry_id(path))
                        .find("browser-cover-status")
                        .visible()
                );
                assert!(
                    description.contains(&path.file_name().unwrap().to_string_lossy().into_owned())
                );
                assert!(description.contains(Language::TraditionalChinese.text(state)));
            })
            .unwrap();
            snapshot(cx, reader, &format!("browser-parity-{suffix}-{index}"));
        }
    }
    cx.update_window(reader, |_, window, cx| window.click("inspector-toggle", cx))
        .unwrap();
    settle(cx, reader);
    cx.update_window(reader, |_, window, cx| {
        window.click("inspector-browser", cx)
    })
    .unwrap();
    settle(cx, reader);
    cx.update_window(reader, |_, window, cx| {
        window.click("browser-metadata", cx);
        window.click(entry_id(fixture.failed_paths.last().unwrap()), cx);
    })
    .unwrap();
    settle(cx, reader);
    cx.update_window(reader, |_, window, _| {
        let path = fixture.failed_paths.last().unwrap();
        assert_eq!(window.find(entry_id(path)).checked(), Some(true));
        assert!(
            window
                .within(entry_id(path))
                .find("browser-tile-metadata")
                .visible()
        );
        assert!(
            !gpui_kit::base::test_support::texts(window)
                .iter()
                .any(|text| text.text().contains("— · —"))
        );
    })
    .unwrap();
    snapshot(cx, reader, "browser-parity-selected-error-metadata");
    cx.update_window(reader, |_, window, cx| window.click("browser-metadata", cx))
        .unwrap();
    settle(cx, reader);
    capture_title_lines(cx, reader, &fixture);
    cx.update_window(reader, |_, window, cx| window.click("inspector-toggle", cx))
        .unwrap();
    settle(cx, reader);
    search(cx, reader, "No matching synthetic book exists");
    cx.update_window(reader, |_, window, _| {
        assert_eq!(window.find("browser-open").disabled(), Some(true))
    })
    .unwrap();
    snapshot(cx, reader, "browser-parity-empty-disabled");
    search(cx, reader, "");
    for language in Language::ALL {
        // Preferences language uses the production deferred reader update.
        let preferences = open_preferences(cx, reader, &view);
        let language_id = format!("language-{}", language.code());
        reach_preference(cx, reader, preferences, &language_id);
        cx.update_window(preferences, |_, window, cx| {
            window.click(language_id, cx);
        })
        .unwrap();
        settle_windows(cx, &[reader, preferences]);
        cx.update_window(preferences, |_, window, cx| window.press("escape", cx))
            .unwrap();
        settle(cx, reader);
        for (width, height) in [(1280., 800.), (960., 640.), (800., 540.)] {
            cx.update_window(reader, |_, window, cx| {
                window.resize(size(px(width), px(height)));
                window.bounds_changed(cx);
            })
            .unwrap();
            settle(cx, reader);
            cx.update_window(reader, |_, window, _| {
                browser_controls_in_bounds(window, language)
            })
            .unwrap();
            snapshot(
                cx,
                reader,
                &format!(
                    "browser-parity-{}-{}x{}",
                    language.code(),
                    width as u32,
                    height as u32
                ),
            );
        }
    }
    cx.update_window(reader, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    assert_eq!(std::fs::read(&fixture.source).unwrap(), original_source);
}
