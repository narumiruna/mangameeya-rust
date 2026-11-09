//! Native central-browser events, state, geometry, and request lifecycle checks.

use super::*;
use crate::{book::DirectoryEntry, browser::SortKey};
use gpui_kit::{Modifiers, PlatformInput, ScrollDelta, ScrollEvent, test::TestQueryExt};

const SIZES: [(f32, f32); 3] = [(1280., 800.), (960., 640.), (800., 540.)];
const BROWSER_CONTROLS: [&str; 7] = [
    "browser-search",
    "browser-parent",
    "browser-sort",
    "browser-reverse",
    "browser-folders-first",
    "browser-refresh",
    "browser-open",
];

fn press(fixture: &Fixture, key: &str, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| window.press(key, cx))
        .unwrap();
    pump(fixture, cx);
}

fn browse(fixture: &Fixture, path: &Path, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.browse(path.to_path_buf(), window, cx));
    })
    .unwrap();
    pump(fixture, cx);
}

fn replace_search(fixture: &Fixture, value: &str, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-search", cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.input(value, cx);
    })
    .unwrap();
    pump(fixture, cx);
}

fn entry_id(path: &Path) -> SharedString {
    super::super::browser::browser_entry_id(path)
}

fn selected(fixture: &Fixture, cx: &mut TestAppContext) -> Option<PathBuf> {
    fixture
        .view
        .read_with(cx, |view, _| view.browser.model.selected.clone())
}

fn ordered_paths(view: &ReaderView) -> Vec<PathBuf> {
    let listing = view.listing.as_ref().unwrap();
    view.browser
        .model
        .order
        .iter()
        .map(|&index| listing.entries[index].entry_path.clone())
        .collect()
}

fn synthetic_listing(path: PathBuf, count: usize) -> DirectoryListing {
    DirectoryListing {
        parent: path.parent().map(Path::to_path_buf),
        entries: (0..count)
            .map(|index| {
                let name = format!("comic-{index:05}.cbz");
                let entry_path = path.join(&name);
                DirectoryEntry {
                    name,
                    path: entry_path.clone(),
                    entry_path,
                    is_directory: false,
                    kind: EntryKind::Archive,
                    size: Some(index as u64 + 1),
                    modified: None,
                    created: None,
                }
            })
            .collect(),
        path,
    }
}

fn install_listing(fixture: &Fixture, listing: DirectoryListing, cx: &mut TestAppContext) {
    fixture.view.update(cx, |view, cx| {
        view.listing_revision += 1;
        view.listing = Some(listing);
        view.browser_listing_changed(cx);
        cx.notify();
    });
    render_windows(fixture, cx);
}

fn inside(inner: Bounds<Pixels>, outer: Bounds<Pixels>) {
    assert!(
        inner.size.width > px(0.)
            && inner.size.height > px(0.)
            && inner.left() >= outer.left() - px(0.5)
            && inner.top() >= outer.top() - px(0.5)
            && inner.right() <= outer.right() + px(0.5)
            && inner.bottom() <= outer.bottom() + px(0.5),
        "{inner:?} must be inside {outer:?}"
    );
}

fn disjoint(a: Bounds<Pixels>, b: Bounds<Pixels>) {
    assert!(
        a.right() <= b.left() + px(0.5)
            || b.right() <= a.left() + px(0.5)
            || a.bottom() <= b.top() + px(0.5)
            || b.bottom() <= a.top() + px(0.5),
        "controls overlap: {a:?}, {b:?}"
    );
}

fn shell_bounds(window: &Window) -> Vec<Option<Bounds<Pixels>>> {
    let mut bounds: Vec<_> = [
        "reader-toolbar",
        "reader-status",
        "sidebar",
        "inspector",
        "viewer",
    ]
    .into_iter()
    .map(|id| window.try_find(id).map(|node| node.bounds()))
    .collect();
    bounds.extend((0..7usize).map(|index| Some(window.find(("menu", index)).bounds())));
    bounds
}

fn assert_browser_geometry(window: &Window) {
    let viewport = Bounds::new(Default::default(), window.viewport_size());
    let viewer = window.find("viewer").bounds();
    inside(viewer, viewport);
    assert!(viewer.size.width >= px(280.));
    let grid = window.find("browser-grid").bounds();
    inside(grid, viewer);
    let controls: Vec<_> = BROWSER_CONTROLS
        .iter()
        .map(|id| {
            let node = window.find((*id).to_owned());
            assert!(node.visible(), "{id}");
            assert!(
                !node.label().unwrap_or_default().is_empty(),
                "{id} needs a name"
            );
            inside(node.bounds(), viewer);
            disjoint(node.bounds(), grid);
            node.bounds()
        })
        .collect();
    for (index, a) in controls.iter().enumerate() {
        for b in &controls[index + 1..] {
            disjoint(*a, *b);
        }
    }
    assert!(
        window.try_find("browser-resume").is_none(),
        "resume has one entrance"
    );
    let header = window.find("browser-header").bounds();
    inside(header, viewer);
    assert!(header.size.height <= px(80.), "compact header: {header:?}");
    let search = window.find("browser-search").bounds();
    assert!(
        search.size.width <= px(160.),
        "search width must be bounded"
    );
    let actions = window.find("browser-actions").bounds();
    if viewer.size.width >= px(900.) {
        assert!((search.center().y - actions.center().y).abs() < px(1.));
    } else {
        assert!(
            search.bottom() <= actions.top(),
            "narrow header must have two rows"
        );
    }
    assert_eq!(
        window.find("browser-folders-first").role(),
        Some(gpui_kit::Role::CheckBox)
    );
    let toolbar = window.find("reader-toolbar").bounds();
    let toggle = window.find("browser-toggle");
    inside(toggle.bounds(), toolbar);
    assert!(!toggle.label().unwrap_or_default().is_empty());
    for id in [
        "open",
        "sidebar-toggle",
        "mode-single",
        "mode-spread",
        "mode-continuous",
        "zoom-trigger",
        "direction",
        "bookmark",
        "inspector-toggle",
        "fullscreen",
        "preferences",
    ] {
        disjoint(toggle.bounds(), window.find(id).bounds());
    }
    for id in ["reader-toolbar", "reader-status", "sidebar", "inspector"] {
        if let Some(node) = window.try_find(id) {
            inside(node.bounds(), viewport);
            disjoint(node.bounds(), viewer);
        }
    }
}

#[gpui_kit::test]
fn initial_browser_cannot_resume_without_a_book(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(fixture.view.read(cx).content_mode, ContentMode::Browsing);
        assert!(window.find("browser-grid").visible());
        assert!(window.try_find("browser-resume").is_none());
        assert_eq!(window.find("browser-toggle").disabled(), Some(true));
        window.click("browser-toggle", cx);
        window.press("escape", cx);
        assert_eq!(fixture.view.read(cx).content_mode, ContentMode::Browsing);
        assert!(fixture.view.read(cx).reader.book.is_none());
        assert_eq!(cx.windows().len(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn backspace_and_escape_replace_only_the_central_content_in_every_shell_layout(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "shell", 6)])
        .unwrap();
    pump(&fixture, cx);
    let panel = fixture
        .view
        .read_with(cx, |view, _| view.panel_state.entity_id());
    for language in Language::ALL {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture
                .view
                .update(cx, |view, cx| view.set_language(language, window, cx));
        })
        .unwrap();
        for (width, height) in SIZES {
            cx.update_window(fixture.window, |_, window, cx| {
                window.resize(size(px(width), px(height)));
                window.bounds_changed(cx);
            })
            .unwrap();
            pump(&fixture, cx);
            // Both panels, right only, neither panel, left only, and both again.
            for toggle in [
                None,
                Some("sidebar-toggle"),
                Some("inspector-toggle"),
                Some("sidebar-toggle"),
                Some("inspector-toggle"),
            ] {
                if let Some(toggle) = toggle {
                    cx.update_window(fixture.window, |_, window, cx| window.click(toggle, cx))
                        .unwrap();
                    pump(&fixture, cx);
                }
                let before = cx
                    .update_window(fixture.window, |_, window, cx| {
                        window.click("viewer", cx);
                        shell_bounds(window)
                    })
                    .unwrap();
                let (layout, tab) = fixture.view.read_with(cx, |view, _| {
                    (
                        view.reader.settings.preferences.panel_layout.clone(),
                        view.inspector_tab,
                    )
                });
                press(&fixture, "backspace", cx);
                cx.update_window(fixture.window, |_, window, cx| {
                    assert_eq!(
                        shell_bounds(window),
                        before,
                        "{language:?} {width}x{height}"
                    );
                    assert_browser_geometry(window);
                    assert_eq!(fixture.view.read(cx).panel_state.entity_id(), panel);
                    assert_eq!(cx.windows().len(), 1);
                })
                .unwrap();
                fixture.view.read_with(cx, |view, _| {
                    assert_eq!(view.reader.settings.preferences.panel_layout, layout);
                    assert_eq!(view.inspector_tab, tab);
                });
                press(&fixture, "escape", cx);
                cx.update_window(fixture.window, |_, window, cx| {
                    assert_eq!(fixture.view.read(cx).content_mode, ContentMode::Reading);
                    assert!(window.try_find("browser-grid").is_none());
                    assert_eq!(shell_bounds(window), before);
                })
                .unwrap();
            }
        }
    }
}

#[gpui_kit::test]
fn browsing_pauses_playback_and_blocks_reading_keys_commands_and_control_wheel(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "paused", 8)])
        .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.navigate(3, window, cx);
            view.reader.set_zoom(2.);
            view.reader.rotation = 90;
            view.reader.filters.brightness = 117.;
            view.reader.slideshow = true;
            view.next_slide = Instant::now();
        });
        window.press("backspace", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    let before = fixture.view.read_with(cx, |view, _| {
        assert!(!view.reader.slideshow);
        (
            view.reader.current,
            view.reader.zoom,
            view.reader.rotation,
            view.reader.filters,
            view.reader.settings.preferences.clone(),
            view.reader.settings.bookmarks.clone(),
        )
    });
    cx.update_window(fixture.window, |_, window, cx| {
        for key in [
            "space", "pagedown", "pageup", "+", "-", "f", "w", "h", "1", "s", "c", "d", "p", "[",
            "]", "b",
        ] {
            window.press(key, cx);
        }
        for command in [
            Command::Next,
            Command::Previous,
            Command::ZoomIn,
            Command::ZoomOut,
            Command::RotateLeft,
            Command::RotateRight,
            Command::Slideshow,
            Command::SetContinuous,
            Command::SetSpread,
            Command::Single,
            Command::ResetFilters,
            Command::Bookmark,
        ] {
            window.dispatch_action(Box::new(command), cx);
        }
        for id in [
            "next",
            "previous",
            "zoom-trigger",
            "mode-single",
            "mode-spread",
            "mode-continuous",
        ] {
            assert_eq!(window.find(id).disabled(), Some(true), "{id}");
            window.click(id, cx);
        }
        window.dispatch_event(
            PlatformInput::Scroll(ScrollEvent {
                position: window.find("browser-grid").bounds().center(),
                delta: ScrollDelta::Lines(point(0., 2.)),
                modifiers: Modifiers {
                    control: true,
                    ..Default::default()
                },
                ..Default::default()
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::ModifiersChanged(gpui_kit::ModifiersChangedEvent::default()),
            cx,
        );
        fixture.view.update(cx, |view, cx| {
            view.next_slide = Instant::now();
            view.tick(window, cx);
        });
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            (
                view.reader.current,
                view.reader.zoom,
                view.reader.rotation,
                view.reader.filters,
                view.reader.settings.preferences.clone(),
                view.reader.settings.bookmarks.clone()
            ),
            before
        );
        assert!(!view.reader.slideshow);
    });
    press(&fixture, "escape", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.current, 3);
        assert!(!view.reader.slideshow, "resume must not restart playback");
    });
}

#[gpui_kit::test]
fn paged_and_continuous_reading_keep_exact_scroll_and_book_identity(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "anchors", 6);
    for index in 1..=6 {
        image::RgbaImage::from_pixel(320, 1200, image::Rgba([80, 160, 220, 255]))
            .save(folder.join(format!("{index}.png")))
            .unwrap();
    }
    fixture.sender.send(vec![folder]).unwrap();
    pump(&fixture, cx);
    for mode in [
        ReadingMode::Single,
        ReadingMode::Spread,
        ReadingMode::Continuous,
    ] {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.reader.set_mode(mode);
                view.reader.set_zoom(4.);
                view.reader.rotation = 90;
                view.reader.filters.contrast = 113.;
                view.relayout(cx);
                view.navigate(2, window, cx);
            });
        })
        .unwrap();
        pump(&fixture, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            window.scroll(
                "viewer",
                ScrollDelta::Pixels(point(px(-40.), px(-160.))),
                cx,
            );
        })
        .unwrap();
        pump(&fixture, cx);
        let before = fixture.view.read_with(cx, |view, _| {
            (
                view.reader.book.as_ref().unwrap().clone(),
                view.reader.current,
                view.reader.zoom,
                view.reader.rotation,
                view.reader.filters,
                view.continuous.logical_scroll_top(),
                view.paged_scroll.offset(),
            )
        });
        if mode == ReadingMode::Continuous {
            assert!(before.5.offset_in_item > px(0.));
        } else {
            assert!(before.6.x < px(0.) || before.6.y < px(0.));
        }
        press(&fixture, "backspace", cx);
        // Repeated hidden ticks must not infer a page from the browser viewport.
        for _ in 0..3 {
            pump(&fixture, cx);
        }
        fixture
            .view
            .read_with(cx, |view, _| assert_eq!(view.reader.current, before.1));
        press(&fixture, "escape", cx);
        fixture.view.read_with(cx, |view, _| {
            assert!(Arc::ptr_eq(view.reader.book.as_ref().unwrap(), &before.0));
            assert_eq!(
                (
                    view.reader.current,
                    view.reader.zoom,
                    view.reader.rotation,
                    view.reader.filters
                ),
                (before.1, before.2, before.3, before.4)
            );
            let top = view.continuous.logical_scroll_top();
            assert_eq!(top.item_ix, before.5.item_ix);
            assert!((top.offset_in_item - before.5.offset_in_item).abs() < px(1.));
            assert_eq!(view.paged_scroll.offset(), before.6);
        });
    }
}

#[gpui_kit::test]
fn grid_click_selects_enter_opens_and_backspace_selects_the_original_image(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "images", 4);
    browse(&fixture, &folder, cx);
    let image = folder.canonicalize().unwrap().join("3.png");
    cx.update_window(fixture.window, |_, window, cx| {
        window.click(entry_id(&image), cx);
        assert_eq!(fixture.view.read(cx).content_mode, ContentMode::Browsing);
        assert!(
            fixture.view.read(cx).reader.book.is_none(),
            "single click must not open"
        );
        assert_eq!(window.find(entry_id(&image)).checked(), Some(true));
        window.press("enter", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.content_mode, ContentMode::Reading);
        assert_eq!(view.reader.current, 2);
    });
    press(&fixture, "backspace", cx);
    assert_eq!(selected(&fixture, cx), Some(image));
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.listing.as_ref().unwrap().path,
            folder.canonicalize().unwrap()
        )
    });
}

#[gpui_kit::test]
fn double_click_enters_a_folder_and_opens_an_image_without_a_new_window(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let parent = book(&fixture, "double-click", 1);
    let child = parent.join("nested");
    std::fs::create_dir(&child).unwrap();
    std::fs::copy(parent.join("1.png"), child.join("2.png")).unwrap();
    browse(&fixture, &parent, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.double_click(entry_id(&child.canonicalize().unwrap()), cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.listing.as_ref().unwrap().path,
            child.canonicalize().unwrap()
        );
        assert_eq!(view.content_mode, ContentMode::Browsing);
        assert!(view.reader.book.is_none());
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.double_click(entry_id(&child.join("2.png").canonicalize().unwrap()), cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.content_mode, ContentMode::Reading);
        assert_eq!(
            view.reader.book.as_ref().unwrap().path,
            child.canonicalize().unwrap()
        );
        assert_eq!(view.page_count(), 1);
    });
    cx.update_window(fixture.window, |_, _, cx| assert_eq!(cx.windows().len(), 1))
        .unwrap();
}

#[gpui_kit::test]
fn listing_failure_retains_the_location_and_refresh_retries_it(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "retry-listing", 1);
    browse(&fixture, &folder, cx);
    let canonical = folder.canonicalize().unwrap();
    std::fs::remove_file(folder.join("1.png")).unwrap();
    std::fs::remove_dir(&folder).unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-refresh", cx)
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.listing_error.is_some());
        assert_eq!(view.listing.as_ref().unwrap().path, canonical);
        assert_eq!(view.content_mode, ContentMode::Browsing);
    });
    std::fs::create_dir(&folder).unwrap();
    image::RgbaImage::from_pixel(120, 200, image::Rgba([80, 160, 220, 255]))
        .save(folder.join("2.png"))
        .unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-refresh", cx)
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.listing_error.is_none());
        assert_eq!(view.listing.as_ref().unwrap().path, canonical);
        assert_eq!(view.listing.as_ref().unwrap().entries.len(), 1);
        assert_eq!(view.listing.as_ref().unwrap().entries[0].name, "2.png");
    });
}

#[gpui_kit::test]
fn external_first_image_returns_to_its_parent_and_selects_image_not_book_folder(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "external", 4);
    let image = folder.join("2.png").canonicalize().unwrap();
    fixture.sender.send(vec![image.clone()]).unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reading_origin, Some(image.clone()));
        assert_eq!(
            view.reader.book.as_ref().unwrap().path,
            folder.canonicalize().unwrap()
        );
        assert_eq!(view.reader.current, 1);
    });
    press(&fixture, "backspace", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.listing.as_ref().unwrap().path,
            folder.canonicalize().unwrap()
        );
        assert_eq!(view.browser.model.selected, Some(image));
    });
}

#[gpui_kit::test]
fn arrows_home_end_and_enter_follow_grid_order_and_parent_stops_at_filesystem_root(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "navigation", 9);
    let nested = folder.join("nested");
    std::fs::create_dir(&nested).unwrap();
    browse(&fixture, &folder, cx);
    let paths = fixture.view.read_with(cx, |view, _| ordered_paths(view));
    let columns = fixture.view.read_with(cx, |view, _| view.browser.columns);
    for (key, index) in [
        ("home", 0),
        ("right", 1),
        ("down", 1 + columns),
        ("up", 1),
        ("left", 0),
        ("end", paths.len() - 1),
    ] {
        press(&fixture, key, cx);
        assert_eq!(selected(&fixture, cx), Some(paths[index].clone()), "{key}");
    }
    press(&fixture, "home", cx);
    press(&fixture, "enter", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.listing.as_ref().unwrap().path,
            nested.canonicalize().unwrap()
        );
        assert_eq!(view.content_mode, ContentMode::Browsing);
        assert!(view.reader.book.is_none());
    });
    press(&fixture, "backspace", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.listing.as_ref().unwrap().path,
            folder.canonicalize().unwrap()
        )
    });
    // Use the actual filesystem root identity, without enumerating protected
    // Windows system files or writing outside the temporary fixture.
    let canonical = folder.canonicalize().unwrap();
    let root = canonical.ancestors().last().unwrap();
    assert!(root.parent().is_none());
    install_listing(
        &fixture,
        DirectoryListing {
            path: root.to_path_buf(),
            parent: None,
            entries: Vec::new(),
        },
        cx,
    );
    let revision = fixture.view.read_with(cx, |view, _| {
        assert!(view.listing.as_ref().unwrap().parent.is_none());
        view.listing_revision
    });
    press(&fixture, "backspace", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.listing_revision, revision);
        assert_eq!(view.listing.as_ref().unwrap().path, root);
    });
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("browser-parent").disabled(), Some(true))
    })
    .unwrap();
}

#[gpui_kit::test]
fn search_editing_keeps_backspace_enter_and_tab_out_of_navigation(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "input", 3);
    browse(&fixture, &folder, cx);
    press(&fixture, "home", cx);
    let revision = fixture.view.read_with(cx, |view, _| view.listing_revision);
    replace_search(&fixture, "2.pngx", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("backspace", cx);
        window.press("enter", cx);
        assert_eq!(window.find("browser-search").value(), Some("2.png"));
        assert_eq!(window.find("browser-search").focused(), Some(true));
        let sidebar = fixture.view.read(cx).reader.settings.preferences.sidebar;
        window.press("tab", cx);
        assert_eq!(
            fixture.view.read(cx).reader.settings.preferences.sidebar,
            sidebar
        );
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.content_mode, ContentMode::Browsing);
        assert!(view.reader.book.is_none());
        assert_eq!(view.listing_revision, revision);
        assert_eq!(view.browser.model.query, "2.png");
        assert_eq!(view.browser.model.order.len(), 1);
    });
    replace_search(&fixture, "no-such-comic", cx);
    fixture
        .view
        .read_with(cx, |view, _| assert!(view.browser.model.order.is_empty()));
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("viewer", cx);
        for key in ["home", "end", "up", "down", "enter"] {
            window.press(key, cx);
        }
        assert!(fixture.view.read(cx).reader.book.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn escape_closes_sort_popover_before_resuming_and_resumes_before_fullscreen_exit(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "escape", 3)])
        .unwrap();
    pump(&fixture, cx);
    press(&fixture, "backspace", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-sort", cx);
        assert!(window.find("popup-menu").visible());
        window.press("escape", cx);
    })
    .unwrap();
    // PopupMenu dismissal is deferred until the event callback returns.
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert!(window.try_find("popup-menu").is_none());
        assert_eq!(fixture.view.read(cx).content_mode, ContentMode::Browsing);
    })
    .unwrap();
    press(&fixture, "escape", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.content_mode, ContentMode::Reading)
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("f11", cx);
        // Some headless platforms do not implement native fullscreen.
        if window.is_fullscreen() {
            window.click("viewer", cx);
            window.press("backspace", cx);
            window.press("escape", cx);
            assert_eq!(fixture.view.read(cx).content_mode, ContentMode::Reading);
            assert!(
                window.is_fullscreen(),
                "resume must precede fullscreen exit"
            );
            window.press("escape", cx);
            assert!(!window.is_fullscreen());
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn failed_grid_open_keeps_book_query_selection_and_browser_mode(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let original = book(&fixture, "usable", 4);
    fixture.sender.send(vec![original]).unwrap();
    pump(&fixture, cx);
    let directory = fixture.directory.path().join("broken");
    std::fs::create_dir(&directory).unwrap();
    let broken = directory.join("broken.cbz");
    std::fs::write(&broken, b"not a ZIP archive").unwrap();
    browse(&fixture, &directory, cx);
    replace_search(&fixture, "broken", cx);
    let old_book = fixture
        .view
        .read_with(cx, |view, _| view.reader.book.as_ref().unwrap().clone());
    cx.update_window(fixture.window, |_, window, cx| {
        window.click(entry_id(&broken.canonicalize().unwrap()), cx);
        window.press("enter", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(Arc::ptr_eq(view.reader.book.as_ref().unwrap(), &old_book));
        assert_eq!(view.content_mode, ContentMode::Browsing);
        assert_eq!(view.browser.model.query, "broken");
        assert_eq!(
            view.browser.model.selected,
            Some(broken.canonicalize().unwrap())
        );
        assert!(view.message_error);
        assert!(!view.opening);
    });
    press(&fixture, "escape", cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(Arc::ptr_eq(view.reader.book.as_ref().unwrap(), &old_book))
    });
}

#[gpui_kit::test]
fn resume_and_close_reject_book_and_listing_results_from_older_intents(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let original = book(&fixture, "retained", 4);
    let replacement = book(&fixture, "late", 2);
    fixture.sender.send(vec![original]).unwrap();
    pump(&fixture, cx);
    let old_book = fixture
        .view
        .read_with(cx, |view, _| view.reader.book.as_ref().unwrap().clone());
    for close in [false, true] {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.browse(replacement.clone(), window, cx);
                view.open_browser_path(replacement.clone(), window, cx);
                if close {
                    view.perform(Command::Close, window, cx);
                } else {
                    view.resume_reading(window, cx);
                }
            });
        })
        .unwrap();
        let listing_revision = fixture.view.read_with(cx, |view, _| view.listing_revision);
        let listing = fixture.view.read_with(cx, |view, _| {
            view.listing.as_ref().map(|listing| listing.path.clone())
        });
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.listing_revision, listing_revision);
            assert_eq!(
                view.listing.as_ref().map(|listing| listing.path.clone()),
                listing
            );
            assert!(!view.opening);
            if close {
                assert!(view.reader.book.is_none());
                assert_eq!(view.content_mode, ContentMode::Browsing);
            } else {
                assert!(Arc::ptr_eq(view.reader.book.as_ref().unwrap(), &old_book));
                assert_eq!(view.content_mode, ContentMode::Reading);
            }
        });
    }
}

#[gpui_kit::test]
fn query_sort_and_grid_preferences_remain_window_local_without_settings_writes(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "local", 4);
    browse(&fixture, &folder, cx);
    fixture
        .view
        .update(cx, |view, _| view.flush_settings().unwrap());
    let path = fixture.directory.path().join("settings.json");
    let bytes = std::fs::read(&path).unwrap();
    let epoch = fixture
        .view
        .read_with(cx, |view, _| view.save_epoch.load(Ordering::SeqCst));
    replace_search(&fixture, "2", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-reverse", cx);
        window.click("browser-folders-first", cx);
        window.click("inspector-browser", cx);
        window.click("browser-show-names", cx);
        window.click(("browser-name-lines", 3usize), cx);
        window.click("browser-metadata", cx);
        assert!(window.try_find("browser-shuffle").is_none());
        window.click("browser-sort", cx);
        let item = window.find_by_label(
            fixture
                .view
                .read(cx)
                .language()
                .text(Text::BrowserSortRandom),
        );
        let menu = window.find("popup-menu").bounds();
        window.click_at("popup-menu", item.bounds().center() - menu.origin, cx);
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-shuffle", cx)
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.inspector_tab, InspectorTab::Browser);
        assert_eq!(view.browser.model.query, "2");
        assert_eq!(view.browser.model.sort, SortKey::Random);
        assert_eq!(view.save_epoch.load(Ordering::SeqCst), epoch);
    });
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "browser preferences must not change the settings schema or file"
    );
    assert!(AppSettings::load_from(&path).is_ok());
}

#[gpui_kit::test]
fn ten_thousand_entries_use_virtual_rows_and_retain_native_selection_after_sort(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let path = fixture.directory.path().join("synthetic");
    install_listing(&fixture, synthetic_listing(path.clone(), 10_000), cx);
    press(&fixture, "end", cx);
    assert_eq!(selected(&fixture, cx), Some(path.join("comic-09999.cbz")));
    let top = fixture
        .view
        .read_with(cx, |view, _| view.browser.grid.logical_scroll_top());
    assert!(top.item_ix > 100, "End must reveal a distant virtual row");
    cx.update_window(fixture.window, |_, window, cx| {
        assert!(
            window
                .find(entry_id(&path.join("comic-09999.cbz")))
                .visible()
        );
        // Absence of remote nodes proves that tiles are not all constructed.
        for index in [0, 100, 1000, 5000] {
            assert!(
                window
                    .try_find(entry_id(&path.join(format!("comic-{index:05}.cbz"))))
                    .is_none()
            );
        }
        window.click("browser-reverse", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.browser.model.order.len(), 10_000);
        assert_eq!(
            view.browser.model.selected,
            Some(path.join("comic-09999.cbz"))
        );
        assert_eq!(ordered_paths(view)[0], path.join("comic-09999.cbz"));
    });
    fixture.view.read_with(cx, |view, _| {
        let rows = view.browser.visible_row_count();
        assert!(
            rows > 0 && rows < 10,
            "only viewport rows can be rendered: {rows}"
        );
        assert!(view.browser.rendered_tile_count() <= rows * view.browser.columns);
        assert!(view.browser.wanted_cover_count() <= (rows + 2) * view.browser.columns);
        assert!(view.browser.active_jobs() <= 2);
        assert!(view.browser.cache_bytes() <= 32 * 1024 * 1024);
    });
    assert!(
        !path.exists(),
        "the synthetic listing must not require 10,000 files"
    );
}

#[gpui_kit::test]
fn grid_book_round_trip_restores_query_sort_selection_and_exact_scroll(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "round-trip", 120);
    browse(&fixture, &folder, cx);
    replace_search(&fixture, ".png", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-reverse", cx);
        window.click("browser-folders-first", cx);
        fixture.view.update(cx, |view, cx| {
            view.browser.grid.scroll_to(ListOffset {
                item_ix: 10,
                offset_in_item: px(27.),
            });
            cx.notify();
        });
    })
    .unwrap();
    pump(&fixture, cx);
    let target = fixture.view.read_with(cx, |view, _| {
        let top = view.browser.grid.logical_scroll_top();
        assert_eq!(top.item_ix, 10);
        assert_eq!(top.offset_in_item, px(27.));
        ordered_paths(view)[top.item_ix * view.browser.columns].clone()
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.click(entry_id(&target), cx)
    })
    .unwrap();
    let before = fixture.view.read_with(cx, |view, _| {
        (
            view.browser.model.query.clone(),
            view.browser.model.sort,
            view.browser.model.descending,
            view.browser.model.folders_first,
            view.browser.model.selected.clone(),
            view.browser.grid.logical_scroll_top(),
            ordered_paths(view),
        )
    });
    press(&fixture, "enter", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.content_mode, ContentMode::Reading)
    });
    press(&fixture, "backspace", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            (
                view.browser.model.query.clone(),
                view.browser.model.sort,
                view.browser.model.descending,
                view.browser.model.folders_first,
                view.browser.model.selected.clone()
            ),
            (
                before.0.clone(),
                before.1,
                before.2,
                before.3,
                before.4.clone()
            )
        );
        let top = view.browser.grid.logical_scroll_top();
        assert_eq!(top.item_ix, before.5.item_ix);
        assert_eq!(top.offset_in_item, before.5.offset_in_item);
        assert_eq!(ordered_paths(view), before.6);
        assert_eq!(
            view.listing.as_ref().unwrap().path,
            folder.canonicalize().unwrap()
        );
    });
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("browser-search").value(), Some(".png"));
        assert_eq!(window.find(entry_id(&target)).checked(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn opening_current_folder_keeps_its_browser_location_instead_of_browsing_its_parent(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "current-folder", 8);
    browse(&fixture, &folder, cx);
    replace_search(&fixture, "2", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.dispatch_action(Box::new(Command::OpenListedFolder), cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.content_mode, ContentMode::Reading);
        assert_eq!(
            view.page_count(),
            8,
            "filter must not change book page order or count"
        );
    });
    press(&fixture, "backspace", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.listing.as_ref().unwrap().path,
            folder.canonicalize().unwrap()
        );
        assert_eq!(view.browser.model.query, "2");
        assert_eq!(view.browser.model.order.len(), 1);
    });
}

#[gpui_kit::test]
fn sort_is_metadata_only_and_far_scrolling_requests_only_the_new_viewport(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let path = fixture.directory.path().join("missing-covers");
    install_listing(&fixture, synthetic_listing(path, 10_000), cx);
    let initial = fixture
        .view
        .read_with(cx, |view, _| view.browser.requested_count());
    fixture.view.update(cx, |view, cx| {
        for sort in SortKey::ALL {
            view.browser.model.sort = sort;
            view.rebuild_browser(cx);
            assert_eq!(
                view.browser.requested_count(),
                initial,
                "sort must not open archive contents"
            );
        }
        view.browser.model.sort = SortKey::Natural;
        view.rebuild_browser(cx);
    });
    for _ in 0..10 {
        pump(&fixture, cx);
        fixture
            .view
            .read_with(cx, |view, _| assert!(view.browser.active_jobs() <= 2));
    }
    let before = fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.browser.active_jobs(), 0);
        assert!(view.browser.failure_count() > 0);
        assert!(view.browser.requested_count() - initial <= view.browser.wanted_cover_count());
        view.browser.requested_count()
    });
    for _ in 0..3 {
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.browser.requested_count(),
            before,
            "failed covers must not be retried on redraw"
        )
    });
    press(&fixture, "end", cx);
    for _ in 0..10 {
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        let delta = view.browser.requested_count() - before;
        assert!(delta > 0);
        assert!(
            delta <= view.browser.wanted_cover_count(),
            "far scroll dispatched {delta} covers for {} wanted tiles",
            view.browser.wanted_cover_count()
        );
        assert!(
            view.browser.failure_count() < 100,
            "must not decode the 10,000-entry gap"
        );
        assert_eq!(view.browser.cache_count(), 0);
        assert!(view.browser.visible_row_count() < 10);
    });
}

#[gpui_kit::test]
fn generation_change_rejects_cover_results_and_refresh_clears_cached_failures(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "cover-retained", 2)])
        .unwrap();
    pump(&fixture, cx);
    press(&fixture, "backspace", cx);
    install_listing(
        &fixture,
        synthetic_listing(fixture.directory.path().join("stale"), 100),
        cx,
    );
    let retained_failures = cx
        .update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.browser_tick(cx);
                let retained_failures = view.browser.failure_sources();
                assert!(view.browser.active_jobs() > 0);
                assert!(view.browser.active_jobs() <= 2);
                let generation = view.browser.cover_generation();
                view.clear_browser_covers(cx);
                assert!(view.browser.cover_generation() > generation);
                view.resume_reading(window, cx);
                retained_failures
            })
        })
        .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.browser.active_jobs(), 0);
        assert_eq!(view.browser.cache_count(), 0);
        assert_eq!(
            view.browser.failure_sources(),
            retained_failures,
            "stale results must not add errors; prior source failures remain cached"
        );
    });
    let directory = fixture.directory.path().join("refresh-failure");
    std::fs::create_dir(&directory).unwrap();
    let corrupt = directory.join("bad.png");
    std::fs::write(&corrupt, b"invalid PNG").unwrap();
    browse(&fixture, &directory, cx);
    for _ in 0..3 {
        pump(&fixture, cx);
    }
    let before = fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.browser.failure_count(), retained_failures.len() + 1);
        assert!(
            view.browser
                .failure_sources()
                .contains(&corrupt.canonicalize().unwrap())
        );
        view.browser.requested_count()
    });
    image::RgbaImage::from_pixel(120, 200, image::Rgba([10, 80, 150, 255]))
        .save(&corrupt)
        .unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-refresh", cx)
    })
    .unwrap();
    for _ in 0..3 {
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.browser.failure_count(), 0);
        assert_eq!(view.browser.cache_count(), 1);
        assert!(view.browser.requested_count() > before);
        assert!(view.browser.cache_bytes() <= 32 * 1024 * 1024);
    });
}

#[gpui_kit::test]
fn column_and_tile_width_changes_keep_the_native_item_and_row_displacement(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    install_listing(
        &fixture,
        synthetic_listing(fixture.directory.path().join("resize-grid"), 1000),
        cx,
    );
    fixture.view.update(cx, |view, cx| {
        view.browser.grid.scroll_to(ListOffset {
            item_ix: 20,
            offset_in_item: px(31.),
        });
        cx.notify();
    });
    pump(&fixture, cx);
    for change in 0..2 {
        let (anchor, offset, columns) = fixture.view.read_with(cx, |view, _| {
            let top = view.browser.grid.logical_scroll_top();
            (
                ordered_paths(view)[top.item_ix * view.browser.columns].clone(),
                top.offset_in_item,
                view.browser.columns,
            )
        });
        cx.update_window(fixture.window, |_, window, cx| {
            if change == 0 {
                window.resize(size(px(960.), px(640.)));
                window.bounds_changed(cx);
            } else {
                window.click("inspector-browser", cx);
                let slider = window.find("browser-tile-width").bounds();
                window.drag(
                    slider.center(),
                    point(slider.right() - px(1.), slider.center().y),
                    cx,
                );
            }
        })
        .unwrap();
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            let position = ordered_paths(view)
                .iter()
                .position(|path| *path == anchor)
                .unwrap();
            let top = view.browser.grid.logical_scroll_top();
            assert_ne!(
                view.browser.columns, columns,
                "test must change the column count"
            );
            assert_eq!(top.item_ix, position / view.browser.columns);
            assert!((top.offset_in_item - offset).abs() < px(1.));
        });
    }
}

#[gpui_kit::test]
fn cover_cache_evicts_to_its_byte_budget_and_releases_old_atlas_images(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "cover-budget", 1);
    image::RgbaImage::from_pixel(480, 672, image::Rgba([50, 130, 210, 255]))
        .save(folder.join("1.png"))
        .unwrap();
    for index in 2..=160 {
        std::fs::copy(folder.join("1.png"), folder.join(format!("{index}.png"))).unwrap();
    }
    browse(&fixture, &folder, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("inspector-browser", cx);
        let slider = window.find("browser-tile-width").bounds();
        window.drag(
            slider.center(),
            point(slider.right() - px(1.), slider.center().y),
            cx,
        );
    })
    .unwrap();
    for _ in 0..4 {
        pump(&fixture, cx);
    }
    let first = fixture
        .view
        .read_with(cx, |view, _| view.browser.cached_images());
    assert!(!first.is_empty());
    let columns = fixture.view.read_with(cx, |view, _| view.browser.columns);
    let rows = crate::browser::row_count(160, columns);
    for row in (0..rows).step_by(2) {
        fixture.view.update(cx, |view, cx| {
            view.browser.grid.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: px(0.),
            });
            cx.notify();
        });
        for _ in 0..4 {
            pump(&fixture, cx);
        }
        fixture.view.read_with(cx, |view, _| {
            assert!(view.browser.active_jobs() <= 2);
            assert!(view.browser.cache_bytes() <= 32 * 1024 * 1024);
        });
    }
    let retained = fixture.view.read_with(cx, |view, _| {
        assert!(
            view.browser.requested_count() >= 160,
            "test must request more than one cache budget"
        );
        assert!(
            view.browser.cache_count() < 160,
            "the cache must evict old covers"
        );
        view.browser.cached_images()
    });
    let evicted: Vec<_> = first
        .iter()
        .filter(|old| !retained.iter().any(|new| new.id == old.id))
        .collect();
    assert!(
        !evicted.is_empty(),
        "old covers must leave the bounded cache"
    );
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        for image in evicted {
            assert!(!window.has_image_atlas_entry(image));
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn refresh_and_close_release_native_cover_images_without_using_reader_cache(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let folder = book(&fixture, "cover-images", 6);
    browse(&fixture, &folder, cx);
    for _ in 0..8 {
        pump(&fixture, cx);
    }
    let images = fixture.view.read_with(cx, |view, _| {
        assert!(view.pages.is_empty());
        assert!(view.thumbs.is_empty());
        assert!(view.browser.cache_count() > 0);
        assert!(view.browser.cache_bytes() <= 32 * 1024 * 1024);
        view.browser.cached_images()
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            images
                .iter()
                .any(|image| window.has_image_atlas_entry(image))
        );
        fixture
            .view
            .update(cx, |view, cx| view.clear_browser_covers(cx));
    })
    .unwrap();
    // Native image release must run after the borrowed window is returned.
    render_windows(&fixture, cx);
    cx.update_window(fixture.window, |_, window, _| {
        for image in &images {
            assert!(!window.has_image_atlas_entry(image));
        }
    })
    .unwrap();
    for _ in 0..8 {
        pump(&fixture, cx);
    }
    let replacement = fixture
        .view
        .read_with(cx, |view, _| view.browser.cached_images());
    assert!(!replacement.is_empty());
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.perform(Command::Close, window, cx));
        assert_eq!(fixture.view.read(cx).browser.cache_count(), 0);
    })
    .unwrap();
    render_windows(&fixture, cx);
    cx.update_window(fixture.window, |_, window, _| {
        for image in &replacement {
            assert!(!window.has_image_atlas_entry(image));
        }
    })
    .unwrap();
}

fn set_cover_width(fixture: &Fixture, width: f32, cx: &mut TestAppContext) {
    fixture.view.update(cx, |view, cx| {
        view.browser.tile_slider.update(cx, |_, cx| {
            cx.emit(SliderEvent::Change(
                gpui_kit::component::slider::SliderValue::Single(width),
            ));
        });
    });
    render_windows(fixture, cx);
    render_windows(fixture, cx);
}

#[gpui_kit::test]
fn long_names_paint_two_or_three_lines_at_each_cover_width(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let mut listing = synthetic_listing(fixture.directory.path().join("wrapping"), 4);
    for (entry, name) in listing.entries.iter_mut().zip([
        "[Shared Author Event] The common title has a long prefix Volume 01.cbz",
        "[Shared Author Event] The common title has a long prefix Volume 02.cbz",
        "日本語正體中文長篇漫畫名稱連續文字第十二卷測試沒有空白文字.cbz",
        "LongUnspacedCommonFilenamePrefixWithDifferentVolumeNumber0123456789.cbz",
    ]) {
        // Keep each sample long enough to reach both clamp limits at 220 px.
        entry.name = name.repeat(2);
        entry.path = listing.path.join(&entry.name);
        entry.entry_path = entry.path.clone();
    }
    let names: Vec<_> = listing
        .entries
        .iter()
        .map(|entry| (entry.entry_path.clone(), entry.name.clone()))
        .collect();
    install_listing(&fixture, listing, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("inspector-browser", cx)
    })
    .unwrap();
    for width in [100., 140., 220.] {
        set_cover_width(&fixture, width, cx);
        for lines in [2usize, 3] {
            cx.update_window(fixture.window, |_, window, cx| {
                window.click(("browser-name-lines", lines), cx)
            })
            .unwrap();
            render_windows(&fixture, cx);
            let columns = fixture.view.read_with(cx, |view, _| view.browser.columns);
            for (index, (path, name)) in names.iter().enumerate() {
                fixture.view.update(cx, |view, cx| {
                    let position = ordered_paths(view)
                        .iter()
                        .position(|candidate| candidate == path)
                        .unwrap();
                    view.browser.grid.scroll_to_reveal_item(position / columns);
                    cx.notify();
                });
                render_windows(&fixture, cx);
                cx.update_window(fixture.window, |_, window, _| {
                    let id: gpui_kit::ElementId = entry_id(path).into();
                    let tile = window.find(id.clone());
                    let title = window.within(id.clone()).find("browser-title");
                    assert_eq!(
                        tile.label(),
                        Some(name.as_str()),
                        "full accessible filename"
                    );
                    assert!(tile.description().unwrap().contains(name));
                    assert!(
                        tile.description()
                            .unwrap()
                            .contains(&crate::file_tree::display_path(path))
                    );
                    let text = gpui_kit::base::test_support::texts(window)
                        .into_iter()
                        .find(|text| {
                            text.path().contains(&id) && text.text() == name && text.visible()
                        })
                        .unwrap_or_else(|| panic!("filename must actually paint: {width}px/{lines}/{index}; tile={:?}; texts={:?}", tile.bounds(), gpui_kit::base::test_support::texts(window).into_iter().filter(|text| text.path().contains(&id)).collect::<Vec<_>>()));
                    let height = f32::from(text.bounds().size.height);
                    assert_eq!(title.bounds().size.height, px(lines as f32 * 18.), "line control must change title height");
                    assert!(
                        (height - lines as f32 * 18.).abs() < 1.,
                        "{width}px, {lines} lines: {name}: painted {height}px; text={:?}, title={:?}", text.bounds(), title.bounds()
                    );
                    inside(text.bounds(), title.bounds());
                    inside(title.bounds(), tile.bounds());
                    let cover = window.within(id.clone()).find("browser-cover");
                    assert!((f32::from(cover.bounds().size.width) - width).abs() < 1.);
                    assert!(
                        window.within(id).try_find("browser-tile-metadata").is_none(),
                        "metadata and its space are off by default"
                    );
                })
                .unwrap();
            }
        }
    }
}

#[gpui_kit::test]
fn metadata_toggle_changes_row_height_without_losing_the_native_scroll_anchor(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    let listing = synthetic_listing(fixture.directory.path().join("metadata"), 500);
    install_listing(&fixture, listing, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("inspector-browser", cx)
    })
    .unwrap();
    fixture.view.update(cx, |view, cx| {
        view.browser.grid.scroll_to(ListOffset {
            item_ix: 20,
            offset_in_item: px(19.),
        });
        cx.notify();
    });
    render_windows(&fixture, cx);
    let (path, top) = fixture.view.read_with(cx, |view, _| {
        let top = view.browser.grid.logical_scroll_top();
        (
            ordered_paths(view)[top.item_ix * view.browser.columns].clone(),
            top,
        )
    });
    let before = cx
        .update_window(fixture.window, |_, window, _| {
            window.find(entry_id(&path)).bounds()
        })
        .unwrap();
    for enabled in [true, false] {
        cx.update_window(fixture.window, |_, window, cx| {
            window.click("browser-metadata", cx)
        })
        .unwrap();
        render_windows(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            let current = view.browser.grid.logical_scroll_top();
            assert_eq!(current.item_ix, top.item_ix);
            assert_eq!(current.offset_in_item, top.offset_in_item);
            assert_eq!(
                ordered_paths(view)[current.item_ix * view.browser.columns],
                path
            );
        });
        cx.update_window(fixture.window, |_, window, _| {
            let id: gpui_kit::ElementId = entry_id(&path).into();
            let tile = window.find(id.clone()).bounds();
            assert_eq!(
                tile.size.height,
                before.size.height + px(if enabled { 22. } else { 0. })
            );
            if enabled {
                let metadata = window.within(id.clone()).find("browser-tile-metadata");
                inside(metadata.bounds(), tile);
                assert!(
                    gpui_kit::base::test_support::texts(window)
                        .iter()
                        .any(|text| text.path().contains(&id)
                            && text.visible()
                            && text.text()
                                == Language::TraditionalChinese
                                    .text(Text::BrowserInformationUnknown))
                );
            } else {
                assert!(
                    window
                        .within(id)
                        .try_find("browser-tile-metadata")
                        .is_none()
                );
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn random_header_controls_and_result_count_stay_reachable_in_every_language(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    install_listing(
        &fixture,
        synthetic_listing(fixture.directory.path().join("header"), 17),
        cx,
    );
    for language in Language::ALL {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.set_language(language, window, cx);
                view.browser.model.sort = SortKey::Random;
                view.rebuild_browser(cx);
            });
        })
        .unwrap();
        for (width, height) in [
            (1280., 800.),
            (960., 640.),
            (800., 540.),
            (900., 640.),
            (899., 640.),
        ] {
            cx.update_window(fixture.window, |_, window, cx| {
                window.resize(size(px(width), px(height)));
                window.bounds_changed(cx);
            })
            .unwrap();
            // Both panels, right only, neither, left only, and both again.
            for toggle in [
                None,
                Some("sidebar-toggle"),
                Some("inspector-toggle"),
                Some("sidebar-toggle"),
                Some("inspector-toggle"),
            ] {
                if let Some(toggle) = toggle {
                    cx.update_window(fixture.window, |_, window, cx| window.click(toggle, cx))
                        .unwrap();
                }
                for _ in 0..3 {
                    render_windows(&fixture, cx);
                }
                cx.update_window(fixture.window, |_, window, _| {
                    assert_browser_geometry(window);
                    let shuffle = window.find("browser-shuffle");
                    assert!(shuffle.visible());
                    inside(shuffle.bounds(), window.find("browser-header").bounds());
                    for id in BROWSER_CONTROLS {
                        disjoint(shuffle.bounds(), window.find(id).bounds());
                    }
                    assert_eq!(
                        window.find("browser-results").label(),
                        Some(
                            language
                                .format(Text::BrowserResults, &[("count", "17".into())])
                                .as_str()
                        )
                    );
                })
                .unwrap();
            }
        }
        fixture.view.update(cx, |view, cx| {
            view.browser.model.query = "00001".into();
            view.rebuild_browser(cx);
        });
        render_windows(&fixture, cx);
        cx.update_window(fixture.window, |_, window, _| {
            assert_eq!(
                window.find("browser-results").label(),
                Some(
                    language
                        .format(Text::BrowserResults, &[("count", "1".into())])
                        .as_str()
                )
            );
        })
        .unwrap();
        fixture.view.update(cx, |view, cx| {
            view.browser.model.query.clear();
            view.rebuild_browser(cx);
        });
    }
}
