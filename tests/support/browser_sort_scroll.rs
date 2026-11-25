//! Native Preferences controls and production browser scroll geometry.

use super::*;
use mangameeya_rust::settings::BrowserSortScroll;

pub(super) const OPTIONS: [(&str, Text); 3] = [
    ("browser-sort-scroll-anchor", Text::BrowserSortScrollAnchor),
    ("browser-sort-scroll-top", Text::BrowserSortScrollTop),
    ("browser-sort-scroll-offset", Text::BrowserSortScrollOffset),
];

pub(super) fn exercise_options(
    cx: &mut HeadlessAppContext,
    reader: AnyWindowHandle,
    preferences: AnyWindowHandle,
) {
    // End at the default so each locale/size starts with the same state.
    for selected in [1, 2, 0] {
        let id = OPTIONS[selected].0;
        reach_preference(cx, reader, preferences, id);
        cx.update_window(preferences, |_, window, cx| {
            assert!(preference_in_view(window, id));
            window.click(id, cx);
        })
        .unwrap();
        settle_windows(cx, &[reader, preferences]);
        cx.update_window(preferences, |_, window, _| {
            for (index, (id, _)) in OPTIONS.iter().enumerate() {
                assert_eq!(window.find(*id).checked(), Some(index == selected));
            }
        })
        .unwrap();
    }
}

fn entry_id(path: &Path) -> gpui_kit::SharedString {
    format!("browser-entry:{:?}", path.as_os_str()).into()
}

#[derive(Debug)]
struct Tile {
    index: usize,
    bounds: Bounds<gpui_kit::Pixels>,
}

fn visible_tiles(window: &mut Window, paths: &[PathBuf]) -> Vec<Tile> {
    let grid = window.find("browser-grid").bounds();
    let mut tiles: Vec<_> = paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let tile = window.try_find(entry_id(path))?;
            let bounds = tile.bounds();
            (tile.visible() && bounds.bottom() > grid.top() && bounds.top() < grid.bottom())
                .then_some(Tile { index, bounds })
        })
        .collect();
    tiles.sort_by(|a, b| {
        f32::from(a.bounds.top())
            .total_cmp(&f32::from(b.bounds.top()))
            .then_with(|| f32::from(a.bounds.left()).total_cmp(&f32::from(b.bounds.left())))
    });
    assert!(!tiles.is_empty(), "production list must have visible tiles");
    tiles
}

pub(super) fn capture(cx: &mut HeadlessAppContext, temporary: &Path) {
    // Fixed-width names make ascending/reverse positions deterministic. All
    // content is generated; no user library or external fixture is required.
    let folder = temporary.join("browser-sort-scroll-library");
    std::fs::create_dir(&folder).unwrap();
    let image = image::RgbaImage::from_pixel(2, 3, image::Rgba([70, 110, 220, 255]));
    let paths: Vec<_> = (0..120)
        .map(|index| {
            let path = folder.join(format!("{index:03}.png"));
            image.save(&path).unwrap();
            path.canonicalize().unwrap()
        })
        .collect();
    for mode in [
        BrowserSortScroll::Anchor,
        BrowserSortScroll::Top,
        BrowserSortScroll::Offset,
    ] {
        let mut settings = AppSettings::default();
        settings.preferences.language = Language::English;
        settings.preferences.browser_sort_scroll = mode;
        settings.preferences.sidebar = false;
        settings.preferences.panel_layout.inspector_visible = false;
        let (sender, launches) = mpsc::channel();
        let (reader, _view) = open_reader_fixture(
            cx,
            temporary.join(format!("browser-sort-scroll-{mode:?}.json")),
            settings,
            None,
            launches,
        );
        sender.send(vec![paths[0].clone()]).unwrap();
        settle(cx, reader);
        cx.update_window(reader, |_, window, cx| {
            window.click("viewer", cx);
            window.press("backspace", cx);
        })
        .unwrap();
        settle(cx, reader);
        let initial = cx
            .update_window(reader, |_, window, _| visible_tiles(window, &paths))
            .unwrap();
        assert_eq!(initial[0].index, 0);
        let top = initial[0].bounds.top();
        cx.update_window(reader, |_, window, cx| {
            window.scroll(
                "browser-grid",
                ScrollDelta::Pixels(point(px(0.), px(-537.))),
                cx,
            );
            window.refresh();
        })
        .unwrap();
        settle(cx, reader);
        let before = cx
            .update_window(reader, |_, window, _| visible_tiles(window, &paths))
            .unwrap();
        let anchor = &before[0];
        assert!(anchor.index > 0 && anchor.index < paths.len() / 3);
        assert!(
            anchor.bounds.top() < top - px(1.),
            "use a nonzero row inset"
        );
        // The selected item differs from the top-row anchor. Sorting must not
        // substitute selection for the visible path anchor.
        let selected = before[before.len() / 2].index;
        assert_ne!(selected, anchor.index);
        cx.update_window(reader, |_, window, cx| {
            window.click(entry_id(&paths[selected]), cx);
            window.click("browser-sort", cx);
        })
        .unwrap();
        settle(cx, reader);
        cx.update_window(reader, |_, window, cx| {
            let item = window
                .within("popup-menu")
                .find_by_label(Language::English.text(Text::BrowserReverse));
            let menu = window.find("popup-menu").bounds();
            window.click_at("popup-menu", item.bounds().center() - menu.origin, cx);
        })
        .unwrap();
        settle(cx, reader);
        cx.update_window(reader, |_, window, _| {
            let after = visible_tiles(window, &paths);
            match mode {
                BrowserSortScroll::Anchor => {
                    let restored = after
                        .iter()
                        .find(|tile| tile.index == anchor.index)
                        .expect("the old top-row path must remain visible after reverse sort");
                    assert!((restored.bounds.top() - anchor.bounds.top()).abs() <= px(1.));
                    assert_ne!(after[0].index, paths.len() - 1 - anchor.index);
                }
                BrowserSortScroll::Top => {
                    assert_eq!(after[0].index, paths.len() - 1);
                    assert!((after[0].bounds.top() - top).abs() <= px(1.));
                }
                BrowserSortScroll::Offset => {
                    assert_eq!(after[0].index, paths.len() - 1 - anchor.index);
                    assert!((after[0].bounds.top() - anchor.bounds.top()).abs() <= px(1.));
                    assert!(!after.iter().any(|tile| tile.index == anchor.index));
                }
            }
        })
        .unwrap();
        snapshot(cx, reader, &format!("browser-sort-scroll-{mode:?}"));
        cx.update_window(reader, |_, window, _| window.remove_window())
            .unwrap();
        cx.run_until_parked();
    }
}
