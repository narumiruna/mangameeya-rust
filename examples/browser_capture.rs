//! Capture only generated browser sources with the production ReaderView.
//!
//! `cargo run --example browser_capture -- /absolute/new/output-directory`
//! With no argument, creates and retains a directory in the system temporary directory.
//! The output directory must be new and outside the repository. It contains the
//! fixture, isolated settings, PNG captures, and report.json. No user settings load.
//! macOS uses the production offscreen Metal renderer at four logical sizes.
//! Windows uses its own visible HWND and PrintWindow at 1600 x 1000 logical px.
//! Windows records current display scaling; it never changes OS display settings.
//! A watchdog limits the Windows process to 15 seconds, including native capture.

#[path = "support/browser_fixture.rs"]
mod browser_fixture;
#[cfg(target_os = "windows")]
#[path = "support/windows_capture.rs"]
mod windows_capture;

use anyhow::{Context, Result};
use browser_fixture::BrowserFixture;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use gpui_kit::{
    AppContext, Bounds, Window, WindowBounds, WindowOptions, px, size,
    test::{TestQueryExt, TestWindowExt},
};
#[cfg(any(target_os = "macos", target_os = "windows"))]
use mangameeya_rust::{
    i18n::Language,
    settings::{AppSettings, SidebarMode},
    ui::{self, ReaderAssets, ReaderView},
};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
    time::Duration,
};

fn main() -> Result<()> {
    // A native driver or PrintWindow can block. This also limits that failure path.
    let (finished, done) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let seconds = if cfg!(target_os = "windows") { 15 } else { 60 };
        if done.recv_timeout(Duration::from_secs(seconds)) == Err(mpsc::RecvTimeoutError::Timeout) {
            eprintln!("Browser capture exceeded {seconds} seconds; report may remain incomplete");
            std::process::exit(124);
        }
    });
    let output = output_directory()?;
    println!("Capture output: {}", output.display());
    let fixture = BrowserFixture::create(&output.join("fixture"))?;
    let mut report = json!({
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
        "revision": git_output(&["rev-parse", "HEAD"]),
        "worktree_status": git_output(&["status", "--short"]),
        "status": "pending",
        "fixture_root": fixture.root,
        "launch_source": fixture.source,
        "supported_paths": fixture.supported_paths,
        "unsupported_paths": fixture.unsupported_paths,
        "failed_paths": fixture.failed_paths,
        "cover_center_colors": fixture.supported_paths.iter().map(|path| json!({
            "path": path, "rgba": fixture.expected_color(path)
        })).collect::<Vec<_>>(),
        "notes": [
            "Only synthetic sources and isolated settings are used.",
            "ReaderView initially lists home internally. No initial-home frame is captured; Recent is prepopulated with only the synthetic source.",
            "The baseline rejects the valid >64 MiB classic ZIP; the approved P1 reader is expected to support it.",
            "Unsupported paths include empty and nested-only folders without shallow images.",
            "Fixed settling time is not a cold/warm performance measurement."
        ],
        "captures": []
    });
    save_report(&output, &report)?;
    let result = capture(&output, &fixture, &mut report);
    report["status"] = json!(if result.is_ok() { "complete" } else { "failed" });
    if let Err(error) = &result {
        report["error"] = json!(format!("{error:#}"));
    }
    save_report(&output, &report)?;
    drop(finished);
    result
}

fn output_directory() -> Result<PathBuf> {
    let mut args = std::env::args_os().skip(1);
    let requested = args.next();
    anyhow::ensure!(
        args.next().is_none(),
        "Usage: browser_capture [new-output-directory]"
    );
    let output = if let Some(path) = requested {
        let path = PathBuf::from(path);
        // create_dir refuses an existing directory, including a fixture from a prior run.
        fs::create_dir(&path).context("Output directory must be new; its parent must exist")?;
        path.canonicalize()?
    } else {
        tempfile::Builder::new()
            .prefix("mangameeya-browser-capture-")
            .tempdir()?
            .keep()
    };
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).canonicalize()?;
    if output.starts_with(repository) {
        fs::remove_dir(&output)?;
        anyhow::bail!("Capture artifacts must be outside the repository");
    }
    Ok(output)
}

fn git_output(arguments: &[&str]) -> Option<String> {
    let result = Command::new("git")
        .arg("-C")
        .arg(env!("CARGO_MANIFEST_DIR"))
        .args(arguments)
        .output()
        .ok()?;
    result
        .status
        .success()
        .then(|| String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

fn save_report(output: &Path, report: &Value) -> Result<()> {
    fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(report)?,
    )?;
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn settings(source: &Path) -> AppSettings {
    let mut settings = AppSettings::default();
    settings.preferences.language = Language::TraditionalChinese;
    // Do not render the Files tree, which the constructor initially points at home.
    settings.preferences.sidebar_mode = SidebarMode::Recent;
    settings.recent.push(source.to_path_buf());
    settings
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            Default::default(),
            size(px(1600.), px(1000.)),
        ))),
        show: false,
        focus: true,
        ..Default::default()
    }
}

// Use a generic source label, not browser toolbar IDs that changed in the plan.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn launch_source(window: &mut Window, cx: &mut gpui_kit::App, source: &Path) {
    window.render_frame(cx);
    let label = source.file_name().unwrap().to_string_lossy();
    let id = window.find_by_label(&label).path().last().unwrap().clone();
    window.click(id, cx);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn enter_browser(window: &mut Window, cx: &mut gpui_kit::App) -> Result<()> {
    window.render_frame(cx);
    anyhow::ensure!(
        window.try_find("browser-grid").is_none(),
        "Synthetic source did not open"
    );
    anyhow::ensure!(
        window.find("page-input").value() != Some("—"),
        "Source page is not ready"
    );
    window.click("viewer", cx);
    window.press("backspace", cx);
    Ok(())
}

#[cfg(target_os = "macos")]
fn verify_browser(window: &mut Window, cx: &mut gpui_kit::App, fixture: &BrowserFixture) {
    window.render_frame(cx);
    assert!(window.find("browser-grid").visible());
    // Only the synthetic source's parent can contain this selected item.
    assert_eq!(
        window
            .within("browser-grid")
            .find_by_label(&fixture.source.file_name().unwrap().to_string_lossy())
            .checked(),
        Some(true)
    );
}

#[cfg(target_os = "macos")]
fn capture(output: &Path, fixture: &BrowserFixture, report: &mut Value) -> Result<()> {
    use gpui_kit::HeadlessAppContext;
    use std::sync::Arc;

    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(ReaderAssets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(ui::init);
    let (_sender, launches) = mpsc::channel();
    let (reader, _view) = cx.update(|cx| {
        gpui_kit::open_window(options(), cx, |window, cx| {
            cx.new(|cx| {
                ReaderView::new(
                    settings(&fixture.source),
                    output.join("settings.json"),
                    None,
                    None,
                    launches,
                    window,
                    cx,
                )
            })
        })
    })?;
    let settle = |cx: &mut HeadlessAppContext| -> Result<()> {
        for _ in 0..12 {
            cx.advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
            cx.update_window(reader, |_, window, cx| window.render_frame(cx))?;
        }
        Ok(())
    };
    // Open the preloaded synthetic Recent item before advancing the clock or
    // polling background work. Otherwise the initial home listing can dispatch
    // cover reads even while its Files tree is hidden.
    cx.update_window(reader, |_, window, cx| {
        launch_source(window, cx, &fixture.source)
    })?;
    settle(&mut cx)?;
    cx.update_window(reader, |_, window, cx| enter_browser(window, cx))??;
    settle(&mut cx)?;
    cx.update_window(reader, |_, window, cx| verify_browser(window, cx, fixture))?;
    report["renderer"] = json!("macOS production offscreen Metal; not Windows evidence");
    for (width, height) in [(1600., 1000.), (1280., 800.), (960., 640.), (800., 540.)] {
        cx.update_window(reader, |_, window, cx| {
            window.resize(size(px(width), px(height)));
            window.bounds_changed(cx);
        })?;
        settle(&mut cx)?;
        let (logical, scale) = cx.update_window(reader, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("browser-grid").visible());
            let viewport = window.viewport_size();
            (
                [f32::from(viewport.width), f32::from(viewport.height)],
                window.scale_factor(),
            )
        })?;
        let screenshot = cx
            .capture_screenshot(reader)
            .context("Production Metal screenshot is required")?;
        let filename = format!("macos-browser-{width:.0}x{height:.0}.png");
        screenshot.save(output.join(&filename))?;
        report["captures"].as_array_mut().unwrap().push(json!({
            "file": filename, "logical_client_size": logical,
            "physical_image_size": [screenshot.width(), screenshot.height()],
            "scale_factor": scale, "dpi_equivalent": scale * 96.,
            "dpi_note": "96-DPI equivalent from headless scale; not a desktop display measurement"
        }));
        save_report(output, report)?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn capture(output: &Path, fixture: &BrowserFixture, report: &mut Value) -> Result<()> {
    use std::sync::{Arc, Mutex};
    const TITLE: &str = "MangaMeeya synthetic browser capture";
    let result = Arc::new(Mutex::new(None));
    let task_result = result.clone();
    let source = fixture.source.clone();
    let root = fixture.root.clone();
    let expected_source_color = fixture.expected_color(&source).unwrap();
    let output = output.to_path_buf();
    let capture_output = output.clone();
    gpui_kit::application()
        .with_assets(ReaderAssets)
        .run(move |cx| {
            ui::init(cx);
            let (_sender, launches) = mpsc::channel();
            let (reader, _view) = gpui_kit::open_window(options(), cx, |window, cx| {
                cx.new(|cx| {
                    ReaderView::new(
                        settings(&source),
                        output.join("settings.json"),
                        None,
                        None,
                        launches,
                        window,
                        cx,
                    )
                })
            })
            .expect("Cannot create synthetic capture window");
            cx.update_window(reader, |_, window, cx| launch_source(window, cx, &source))
                .unwrap();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let ready = cx
                    .update_window(reader, |_, window, cx| {
                        enter_browser(window, cx)?;
                        window.set_window_title(TITLE);
                        Ok::<_, anyhow::Error>(())
                    })
                    .and_then(|result| result);
                if let Err(error) = ready {
                    *task_result.lock().unwrap() = Some(Err(error));
                    cx.update(|cx| cx.quit());
                    return;
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let shown = cx
                    .update_window(reader, |_, window, cx| {
                        window.render_frame(cx);
                        assert!(
                            window
                                .within("browser-grid")
                                .find_by_label(&source.file_name().unwrap().to_string_lossy())
                                .checked()
                                == Some(true)
                        );
                        windows_capture::show_own_window(TITLE)
                    })
                    .and_then(|result| result);
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let dimensions = shown.and_then(|()| {
                    cx.update_window(reader, |_, window, _| {
                        let viewport = window.viewport_size();
                        (
                            [f32::from(viewport.width), f32::from(viewport.height)],
                            window.scale_factor(),
                        )
                    })
                });
                // PrintWindow runs off the UI thread so that it can service WM_PRINT.
                let captured = cx
                    .background_executor()
                    .spawn(async move {
                        let (logical, scale) = dimensions?;
                        let mut capture = windows_capture::capture_own_window(
                            TITLE,
                            &capture_output.join("windows-browser.png"),
                            expected_source_color,
                        )?;
                        capture["logical_client_size"] = json!(logical);
                        capture["gpui_scale_factor"] = json!(scale);
                        capture["fixture_root"] = json!(root);
                        Ok::<_, anyhow::Error>(capture)
                    })
                    .await;
                *task_result.lock().unwrap() = Some(captured);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    report["renderer"] = json!("Windows native visible HWND; client-only PrintWindow");
    let capture = result
        .lock()
        .unwrap()
        .take()
        .context("Window closed before capture")??;
    report["captures"].as_array_mut().unwrap().push(capture);
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn capture(_: &Path, _: &BrowserFixture, _: &mut Value) -> Result<()> {
    anyhow::bail!("Native browser capture is implemented only for macOS and Windows")
}

#[cfg(test)]
mod tests {
    use super::browser_fixture::BrowserFixture;
    use anyhow::Result;
    use std::{
        fs::{self, File},
        io::{self, Read},
    };
    use zip::{CompressionMethod, ZipArchive};

    fn assert_center(
        fixture: &BrowserFixture,
        path: &std::path::Path,
        image: image::DynamicImage,
    ) -> Result<()> {
        let rgba = image.to_rgba8();
        let center = rgba.get_pixel(rgba.width() / 2, rgba.height() / 2).0;
        let expected = fixture.expected_color(path).unwrap();
        anyhow::ensure!(
            center
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual.abs_diff(expected) <= 3),
            "Unexpected center pixel at {}: {center:?}",
            path.display()
        );
        Ok(())
    }

    #[test]
    fn synthetic_sources_are_real_and_bounded() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let fixture = BrowserFixture::create(&temporary.path().join("fixture"))?;
        assert!(BrowserFixture::create(&fixture.root).is_err());
        assert_eq!(fixture.supported_paths.len(), 11);
        assert_eq!(fixture.unsupported_paths.len(), 4);
        assert_eq!(fixture.failed_paths.len(), 2);
        for path in &fixture.supported_paths {
            if path
                .extension()
                .is_some_and(|extension| extension == "png" || extension == "jpg")
            {
                assert_center(&fixture, path, image::open(path)?)?;
            }
            if path.extension().is_some_and(|extension| extension == "cbz") {
                let mut archive = ZipArchive::new(File::open(path)?)?;
                let mut bytes = Vec::new();
                archive.by_name("2.png")?.read_to_end(&mut bytes)?;
                assert_center(&fixture, path, image::load_from_memory(&bytes)?)?;
                if path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("Large classic")
                {
                    assert!(fs::metadata(path)?.len() > 64 * 1024 * 1024);
                    let mut entry = archive.by_name("attachment/zeroes.dat")?;
                    assert_eq!(entry.compression(), CompressionMethod::Stored);
                    assert_eq!(entry.size(), 65 * 1024 * 1024);
                    assert_eq!(io::copy(&mut entry, &mut io::sink())?, 65 * 1024 * 1024);
                    let mut file = File::open(path)?;
                    use std::io::{Seek, SeekFrom};
                    file.seek(SeekFrom::End(-22))?;
                    let mut footer = [0; 22];
                    file.read_exact(&mut footer)?;
                    assert_eq!(&footer[..4], b"PK\x05\x06");
                    assert_eq!(u16::from_le_bytes([footer[10], footer[11]]), 2);
                    assert_ne!(&footer[16..20], &[255; 4]);
                }
            }
        }
        let zip64 = &fixture.unsupported_paths[2];
        let bytes = fs::read(zip64)?;
        let name_length = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
        let extra = 30 + name_length;
        assert_eq!(&bytes[extra..extra + 4], &[1, 0, 16, 0]);
        let mut archive = ZipArchive::new(File::open(zip64)?)?;
        let mut png = Vec::new();
        archive.by_name("1.png")?.read_to_end(&mut png)?;
        image::load_from_memory(&png)?;
        Ok(())
    }
}
