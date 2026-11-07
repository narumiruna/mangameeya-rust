//! Run with `cargo run --example ui_capture` for a 15-second native visual fixture.

use gpui_kit::{AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use image::{Rgba, RgbaImage};
use mangameeya_rust::{
    i18n::Language,
    settings::{AppSettings, Background, FitMode, ReadingMode, SidebarMode},
    ui::{self, ReaderAssets, ReaderView},
};
use std::{fs, sync::mpsc, time::Duration};

fn main() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let pages = directory.path().join("Synthetic pages");
    fs::create_dir(&pages)?;
    for index in 0..6u8 {
        let mut page = RgbaImage::from_pixel(720, 1000, Rgba([248, 245, 235, 255]));
        // Fixed panels give each thumbnail a distinct, repeatable appearance.
        for (left, top, right, bottom, color) in [
            (32, 32, 688, 300, [65 + index * 20, 85, 115, 255]),
            (32, 324, 348, 648, [165, 180 - index * 15, 175, 255]),
            (372, 324, 688, 648, [195, 145, 95 + index * 20, 255]),
            (32, 672, 688, 968, [100, 110 + index * 15, 135, 255]),
        ] {
            for y in top..bottom {
                for x in left..right {
                    let border = x < left + 5 || x >= right - 5 || y < top + 5 || y >= bottom - 5;
                    page.put_pixel(x, y, Rgba(if border { [30, 35, 40, 255] } else { color }));
                }
            }
        }
        page.save(pages.join(format!("{:02}.png", index + 1)))?;
    }

    let settings_path = directory.path().join("settings.json");
    let mut settings = AppSettings::default();
    settings.preferences.language = Language::TraditionalChinese;
    settings.preferences.sidebar_mode = SidebarMode::Thumbnails;
    settings.preferences.background = Background::Gray;
    settings.preferences.mode = ReadingMode::Single;
    settings.preferences.fit = FitMode::Window;

    let (sender, launches) = mpsc::channel();
    sender.send(vec![pages])?;
    gpui_kit::application()
        .with_assets(ReaderAssets)
        .run(move |cx| {
            ui::init(cx);
            let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("MangaMeeya UI verification".into()),
                        ..Default::default()
                    }),
                    show: true,
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    cx.new(|cx| {
                        ReaderView::new(settings, settings_path, None, None, launches, window, cx)
                    })
                },
            )
            .expect("Cannot open the native visual fixture window");
            cx.activate(true);
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(15))
                    .await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    // Keep the pages and settings directory alive until the native event loop exits.
    drop(directory);
    Ok(())
}
