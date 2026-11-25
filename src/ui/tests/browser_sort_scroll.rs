//! Manual sorting must not change selection or apply its policy to ordinary rebuilds.

use super::*;
use crate::settings::BrowserSortScroll;
use std::time::Duration;

pub(super) const POLICIES: [BrowserSortScroll; 3] = [
    BrowserSortScroll::Anchor,
    BrowserSortScroll::Top,
    BrowserSortScroll::Offset,
];

fn listing(path: PathBuf, count: usize) -> DirectoryListing {
    let mut listing = synthetic_listing(path, count);
    for (index, entry) in listing.entries.iter_mut().enumerate() {
        // Distinct canonical and native paths expose identity regressions.
        entry.path = entry.entry_path.with_extension("canonical");
        if index >= 20 {
            entry.name.push_str(".keep");
        }
        entry.is_directory = index % 7 == 0;
        if entry.is_directory {
            entry.kind = EntryKind::Folder;
        }
        entry.size = Some((count - index) as u64);
        entry.modified = Some(std::time::UNIX_EPOCH + Duration::from_secs(index as u64));
        entry.created = Some(std::time::UNIX_EPOCH + Duration::from_secs((count - index) as u64));
    }
    listing
}

fn prepare(fixture: &Fixture, policy: BrowserSortScroll, sort: SortKey, cx: &mut TestAppContext) {
    fixture.view.update(cx, |view, cx| {
        view.reader.settings.preferences.browser_sort_scroll = policy;
        view.browser.model.sort = sort;
        view.browser.model.query.clear();
        view.browser.model.descending = false;
        view.browser.model.folders_first = false;
        let entries = &view.listing.as_ref().unwrap().entries;
        for (index, entry) in entries.iter().enumerate() {
            let count = entries.len() * 2 - index;
            let recursive = entry.kind == EntryKind::Folder
                && view.reader.settings.preferences.recursive_folder;
            view.browser
                .model
                .known_pages
                .insert((entry.path.clone(), recursive), count);
            view.reader
                .settings
                .positions
                .insert(entry.path.clone(), count - 2);
        }
        view.reader.settings.recent = entries
            .iter()
            .rev()
            .take(15)
            .map(|entry| entry.path.clone())
            .collect();
        view.rebuild_browser(cx);
        view.browser.model.selected = Some(ordered_paths(view).last().unwrap().clone());
        view.browser.grid.scroll_to(ListOffset {
            item_ix: 20,
            offset_in_item: px(23.),
        });
        cx.notify();
    });
    render_windows(fixture, cx);
    fixture.view.update(cx, |view, cx| view.rebuild_browser(cx));
    render_windows(fixture, cx);
}

fn sort_label(sort: SortKey) -> Text {
    match sort {
        SortKey::Natural => Text::BrowserSortNatural,
        SortKey::Name => Text::BrowserSortName,
        SortKey::Modified => Text::BrowserSortModified,
        SortKey::Created => Text::BrowserSortCreated,
        SortKey::Size => Text::BrowserSortSize,
        SortKey::Type => Text::BrowserSortType,
        SortKey::Pages => Text::BrowserSortPages,
        SortKey::Progress => Text::BrowserSortProgress,
        SortKey::Recent => Text::BrowserSortRecent,
        SortKey::Random => Text::BrowserSortRandom,
    }
}

fn offset_eq(actual: ListOffset, expected: ListOffset) {
    assert_eq!(actual.item_ix, expected.item_ix);
    assert!(
        (actual.offset_in_item - expected.offset_in_item).abs() < px(0.5),
        "{actual:?} != {expected:?}"
    );
}

fn check_reorder(
    fixture: &Fixture,
    policy: BrowserSortScroll,
    action: impl FnOnce(&Fixture, &mut TestAppContext),
    cx: &mut TestAppContext,
) {
    let (anchor, before, selection) = fixture.view.read_with(cx, |view, _| {
        let top = view.browser.grid.logical_scroll_top();
        let anchor = view.browser_anchor().0.unwrap();
        assert_ne!(Some(&anchor), view.browser.model.selected.as_ref());
        assert_eq!(top.offset_in_item, px(23.));
        assert!(top.item_ix > 0);
        assert!(view.browser.scroll_anchor.is_some());
        (anchor, top, view.browser.model.selected.clone())
    });
    action(fixture, cx);
    render_windows(fixture, cx);
    let clamping_check = fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.browser.model.selected, selection,
            "selection uses native paths"
        );
        let actual = view.browser.grid.logical_scroll_top();
        match policy {
            BrowserSortScroll::Anchor => {
                let position = ordered_paths(view)
                    .iter()
                    .position(|path| path == &anchor)
                    .unwrap();
                let requested = ListOffset {
                    item_ix: position / view.browser.columns,
                    offset_in_item: before.offset_in_item,
                };
                assert_eq!(view.browser.scroll_anchor.as_ref().unwrap().0, anchor);
                offset_eq(view.browser.scroll_anchor.as_ref().unwrap().1, requested);
                return Some((actual, requested));
            }
            BrowserSortScroll::Top => {
                offset_eq(actual, ListOffset::default());
                assert!(view.browser.scroll_anchor.is_none());
            }
            BrowserSortScroll::Offset => {
                offset_eq(actual, before);
                assert!(view.browser.scroll_anchor.is_none());
            }
        }
        None
    });
    if let Some((actual, requested)) = clamping_check {
        // Compute the target from native identity, then let layout clamp it.
        // This is exact even when Random puts the anchor in the final rows.
        fixture.view.update(cx, |view, cx| {
            view.browser.grid.scroll_to(requested);
            cx.notify();
        });
        render_windows(fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            offset_eq(actual, view.browser.grid.logical_scroll_top())
        });
    }
}

#[gpui_kit::test]
fn manual_sort_scroll_all_native_menu_keys(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    install_listing(
        &fixture,
        listing(fixture.directory.path().join("sort-policies"), 1000),
        cx,
    );
    for policy in POLICIES {
        for sort in SortKey::ALL {
            prepare(
                &fixture,
                policy,
                if sort == SortKey::Natural {
                    SortKey::Name
                } else {
                    SortKey::Natural
                },
                cx,
            );
            let before = fixture.view.read_with(cx, |view, _| ordered_paths(view));
            check_reorder(
                &fixture,
                policy,
                |fixture, cx| {
                    choose_sort_option(fixture, sort_label(sort), cx);
                },
                cx,
            );
            fixture.view.read_with(cx, |view, _| {
                assert_eq!(view.browser.model.sort, sort);
                if matches!(sort, SortKey::Pages | SortKey::Progress | SortKey::Recent) {
                    assert_ne!(
                        ordered_paths(view),
                        before,
                        "known metadata must change the order for {sort:?}"
                    );
                }
            });
        }
    }
}

#[gpui_kit::test]
fn manual_sort_scroll_direction_folders_and_shuffle(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    install_listing(
        &fixture,
        listing(fixture.directory.path().join("sort-actions"), 1000),
        cx,
    );
    for policy in POLICIES {
        for option in [
            Text::BrowserReverse,
            Text::BrowserAscending,
            Text::BrowserFoldersFirst,
        ] {
            prepare(&fixture, policy, SortKey::Natural, cx);
            if option == Text::BrowserAscending {
                fixture.view.update(cx, |view, cx| {
                    view.browser.model.descending = true;
                    view.rebuild_browser(cx);
                });
                // Re-establish an interior anchor after reversing the order.
                fixture.view.update(cx, |view, cx| {
                    view.browser.grid.scroll_to(ListOffset {
                        item_ix: 20,
                        offset_in_item: px(23.),
                    });
                    view.rebuild_browser(cx);
                });
                render_windows(&fixture, cx);
            }
            check_reorder(
                &fixture,
                policy,
                |fixture, cx| choose_sort_option(fixture, option, cx),
                cx,
            );
            if option == Text::BrowserReverse {
                fixture.view.read_with(cx, |view, _| {
                    let top = view.browser.grid.logical_scroll_top();
                    match policy {
                        BrowserSortScroll::Anchor => {
                            assert_ne!(top.item_ix, 0);
                            assert_ne!(
                                top.item_ix, 20,
                                "reverse must move the native anchor, not retain its old offset"
                            );
                        }
                        BrowserSortScroll::Top => offset_eq(top, ListOffset::default()),
                        BrowserSortScroll::Offset => offset_eq(
                            top,
                            ListOffset {
                                item_ix: 20,
                                offset_in_item: px(23.),
                            },
                        ),
                    }
                });
            }
        }
        prepare(&fixture, policy, SortKey::Random, cx);
        check_reorder(
            &fixture,
            policy,
            |fixture, cx| {
                cx.update_window(fixture.window, |_, window, cx| {
                    window.click("browser-shuffle", cx)
                })
                .unwrap();
            },
            cx,
        );
    }
}

#[gpui_kit::test]
fn manual_sort_scroll_same_key_and_direction_are_noops_without_writes(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    install_listing(
        &fixture,
        listing(fixture.directory.path().join("sort-noops"), 1000),
        cx,
    );
    for policy in POLICIES {
        for descending in [false, true] {
            prepare(&fixture, policy, SortKey::Random, cx);
            fixture.view.update(cx, |view, cx| {
                view.browser.model.descending = descending;
                view.rebuild_browser(cx);
                view.flush_settings().unwrap();
            });
            render_windows(&fixture, cx);
            let path = fixture.directory.path().join("settings.json");
            let bytes = std::fs::read(&path).unwrap();
            let before = fixture.view.read_with(cx, |view, _| {
                (
                    ordered_paths(view),
                    view.browser.grid.logical_scroll_top(),
                    view.browser.scroll_anchor.clone(),
                    view.browser.model.selected.clone(),
                    view.save_epoch.load(Ordering::SeqCst),
                )
            });
            for option in [
                Text::BrowserSortRandom,
                if descending {
                    Text::BrowserReverse
                } else {
                    Text::BrowserAscending
                },
            ] {
                choose_sort_option(&fixture, option, cx);
                fixture.view.read_with(cx, |view, _| {
                    assert_eq!(ordered_paths(view), before.0);
                    offset_eq(view.browser.grid.logical_scroll_top(), before.1);
                    assert_eq!(view.browser.scroll_anchor, before.2);
                    assert_eq!(view.browser.model.selected, before.3);
                    assert_eq!(view.save_epoch.load(Ordering::SeqCst), before.4);
                });
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
            }
        }
    }
}

#[gpui_kit::test]
fn manual_sort_scroll_empty_and_bottom_are_clamped_after_layout(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    for policy in POLICIES {
        install_listing(
            &fixture,
            listing(fixture.directory.path().join("sort-bottom"), 1000),
            cx,
        );
        prepare(&fixture, policy, SortKey::Natural, cx);
        fixture.view.update(cx, |view, cx| {
            view.browser.grid.scroll_to(ListOffset::default());
            view.rebuild_browser(cx);
        });
        render_windows(&fixture, cx);
        choose_sort_option(&fixture, Text::BrowserReverse, cx);
        fixture.view.read_with(cx, |view, _| {
            let top = view.browser.grid.logical_scroll_top();
            let rows = view
                .browser
                .model
                .order
                .len()
                .div_ceil(view.browser.columns);
            if policy == BrowserSortScroll::Anchor {
                assert!(top.item_ix > 0);
                assert!(
                    top.item_ix < rows - 1,
                    "last-row anchor must clamp to fill viewport"
                );
            } else {
                offset_eq(top, ListOffset::default());
                assert!(view.browser.scroll_anchor.is_none());
            }
        });
        // Start at the actual bottom, including its partial-row displacement.
        fixture.view.update(cx, |view, cx| {
            view.browser.grid.scroll_to(ListOffset {
                item_ix: 999,
                offset_in_item: px(23.),
            });
            cx.notify();
        });
        render_windows(&fixture, cx);
        let bottom = fixture
            .view
            .read_with(cx, |view, _| view.browser.grid.logical_scroll_top());
        choose_sort_option(&fixture, Text::BrowserAscending, cx);
        if policy == BrowserSortScroll::Offset {
            fixture.view.read_with(cx, |view, _| {
                offset_eq(view.browser.grid.logical_scroll_top(), bottom)
            });
        }
        install_listing(
            &fixture,
            listing(fixture.directory.path().join("sort-empty"), 0),
            cx,
        );
        choose_sort_option(&fixture, Text::BrowserSortSize, cx);
        fixture.view.read_with(cx, |view, _| {
            offset_eq(
                view.browser.grid.logical_scroll_top(),
                ListOffset::default(),
            );
            assert!(view.browser.scroll_anchor.is_none());
            assert!(view.browser.model.selected.is_none());
        });
    }
}

#[gpui_kit::test]
fn manual_sort_scroll_policy_does_not_apply_to_search_or_metadata_rebuild(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    install_listing(
        &fixture,
        listing(fixture.directory.path().join("ordinary-rebuild"), 1000),
        cx,
    );
    for policy in [BrowserSortScroll::Top, BrowserSortScroll::Offset] {
        prepare(&fixture, policy, SortKey::Natural, cx);
        let (anchor, before) = fixture.view.read_with(cx, |view, _| {
            (
                view.browser_anchor().0.unwrap(),
                view.browser.grid.logical_scroll_top(),
            )
        });
        replace_search(&fixture, ".keep", cx);
        fixture.view.read_with(cx, |view, _| {
            let position = ordered_paths(view)
                .iter()
                .position(|path| path == &anchor)
                .unwrap();
            let actual = view.browser.grid.logical_scroll_top();
            assert_ne!(
                actual.item_ix, before.item_ix,
                "filter must remove rows above the native anchor"
            );
            offset_eq(
                actual,
                ListOffset {
                    item_ix: position / view.browser.columns,
                    offset_in_item: before.offset_in_item,
                },
            );
            assert_eq!(view.browser_anchor().0, Some(anchor.clone()));
        });
        // Simulate a background page-count update. Ordinary rebuild still follows the path.
        fixture.view.update(cx, |view, cx| {
            view.browser.model.sort = SortKey::Pages;
            let entries = &view.listing.as_ref().unwrap().entries;
            for (index, entry) in entries.iter().enumerate() {
                view.browser
                    .model
                    .known_pages
                    .insert((entry.path.clone(), false), entries.len() - index);
            }
            view.rebuild_browser(cx);
        });
        render_windows(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            let position = ordered_paths(view)
                .iter()
                .position(|path| path == &anchor)
                .unwrap();
            offset_eq(
                view.browser.grid.logical_scroll_top(),
                ListOffset {
                    item_ix: position / view.browser.columns,
                    offset_in_item: before.offset_in_item,
                },
            );
            assert_eq!(view.browser_anchor().0, Some(anchor));
        });
    }
}
