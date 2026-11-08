//! Native Files tree events, source safety, focus, and virtual-row regressions.

use super::super::files::{disclosure_id, entry_id};
use super::*;
use crate::file_tree::{FileTree, RowKind, load_directory};
use std::io::Write;

fn files_fixture(cx: &mut TestAppContext) -> Fixture {
    let mut settings = AppSettings::default();
    settings.preferences.sidebar = true;
    settings.preferences.sidebar_mode = SidebarMode::Files;
    fixture_with_settings(cx, settings)
}

fn browse(fixture: &Fixture, path: &Path, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.files_browse(path.to_path_buf(), true, window, cx);
        });
    })
    .unwrap();
    pump(fixture, cx);
}

fn click(fixture: &Fixture, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| window.click(id, cx))
        .unwrap();
    pump(fixture, cx);
}

fn press(fixture: &Fixture, key: &str, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| window.press(key, cx))
        .unwrap();
    pump(fixture, cx);
}

fn replace_input(fixture: &Fixture, id: &str, value: &str, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        window.click(id.to_owned(), cx);
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
        window.press("backspace", cx);
        window.input(value, cx);
    })
    .unwrap();
    pump(fixture, cx);
}

fn focus_tree(fixture: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .view
            .read(cx)
            .files_ui
            .focus
            .clone()
            .focus(window, cx);
    })
    .unwrap();
}

fn archive(fixture: &Fixture, folder: &Path, name: &str) -> PathBuf {
    let path = folder.join(name);
    let source = book(fixture, &format!("archive-source-{name}"), 2);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for index in [2, 1] {
        zip.start_file(
            format!("{index}.png"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(&std::fs::read(source.join(format!("{index}.png"))).unwrap())
            .unwrap();
    }
    zip.finish().unwrap();
    path
}

fn visible(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<PathBuf> {
    fixture.view.read_with(cx, |view, _| {
        view.files_ui
            .model
            .visible_rows()
            .iter()
            .filter(|row| row.kind == RowKind::Entry)
            .map(|row| row.path.clone())
            .collect()
    })
}

#[gpui_kit::test]
fn disclosure_navigation_and_selection_preserve_reader_central_state_and_settings(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    let reading = book(&fixture, "reading", 4);
    fixture.sender.send(vec![reading.join("2.png")]).unwrap();
    pump(&fixture, cx);
    let root = book(&fixture, "library", 2).canonicalize().unwrap();
    let a = root.join("folder2");
    let b = root.join("folder10");
    for child in [&a, &b] {
        std::fs::create_dir(child).unwrap();
        std::fs::copy(root.join("1.png"), child.join("page.png")).unwrap();
    }
    browse(&fixture, &root, cx);
    fixture.view.update(cx, |view, _| {
        view.reader.slideshow = true;
        view.next_slide = Instant::now() + Duration::from_secs(3600);
        view.flush_settings().unwrap();
    });
    let settings_path = fixture.directory.path().join("settings.json");
    let settings_bytes = std::fs::read(&settings_path).unwrap();
    let source_bytes = std::fs::read(reading.join("2.png")).unwrap();
    let before = fixture.view.read_with(cx, |view, _| {
        (
            view.reader.book.as_ref().unwrap().clone(),
            view.reader.current,
            view.reader.zoom,
            view.reader.filters,
            view.reader.settings.clone(),
            view.listing_revision,
            view.listing.as_ref().map(|listing| listing.path.clone()),
            view.browser.model.query.clone(),
            view.browser.model.selected.clone(),
            view.browser.model.order.clone(),
            view.browser.grid.logical_scroll_top(),
            view.save_epoch.load(Ordering::SeqCst),
            view.browser.requested_count(),
            view.content_mode,
        )
    });
    click(&fixture, disclosure_id(&a), cx);
    click(&fixture, disclosure_id(&b), cx);
    for _ in 0..3 {
        click(&fixture, disclosure_id(&a), cx);
        click(&fixture, disclosure_id(&a), cx);
    }
    assert!(visible(&fixture, cx).contains(&a.join("page.png")));
    assert!(visible(&fixture, cx).contains(&b.join("page.png")));
    click(&fixture, entry_id(&root.join("2.png")), cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.files_ui.model.selected(),
            Some(root.join("2.png").as_path())
        );
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()));
    });
    click(&fixture, entry_id(&a), cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(a.as_path()));
        assert_eq!(view.files_ui.model.root(), Some(root.as_path()));
        assert!(view.files_ui.model.node(&b).unwrap().expanded);
    });
    click(&fixture, "files-parent", cx);
    replace_input(&fixture, "files-search", "page", cx);
    click(&fixture, "files-hidden", cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(Arc::ptr_eq(view.reader.book.as_ref().unwrap(), &before.0));
        assert_eq!(
            (view.reader.current, view.reader.zoom, view.reader.filters),
            (before.1, before.2, before.3)
        );
        assert_eq!(view.reader.settings, before.4);
        assert_eq!(view.listing_revision, before.5);
        assert_eq!(
            view.listing.as_ref().map(|listing| listing.path.clone()),
            before.6
        );
        assert_eq!(view.browser.model.query, before.7);
        assert_eq!(view.browser.model.selected, before.8);
        assert_eq!(view.browser.model.order, before.9);
        let top = view.browser.grid.logical_scroll_top();
        assert_eq!(
            (top.item_ix, top.offset_in_item),
            (before.10.item_ix, before.10.offset_in_item)
        );
        assert_eq!(view.save_epoch.load(Ordering::SeqCst), before.11);
        assert_eq!(view.browser.requested_count(), before.12);
        assert_eq!(view.content_mode, before.13);
        assert!(
            view.reader.slideshow,
            "sidebar navigation must not pause playback"
        );
    });
    assert_eq!(std::fs::read(settings_path).unwrap(), settings_bytes);
    assert_eq!(std::fs::read(reading.join("2.png")).unwrap(), source_bytes);
}

#[gpui_kit::test]
fn selection_requires_explicit_read_and_double_click_opens_native_archive(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "selection", 3).canonicalize().unwrap();
    let zip = archive(&fixture, &root, "comic.cbz");
    let bytes = std::fs::read(&zip).unwrap();
    browse(&fixture, &root, cx);
    click(&fixture, entry_id(&root.join("2.png")), cx);
    fixture
        .view
        .read_with(cx, |view, _| assert!(view.reader.book.is_none()));
    cx.update_window(fixture.window, |_, window, cx| {
        assert_ne!(window.find("files-read-selected").disabled(), Some(true));
        assert!(fixture.view.read(cx).files_ui.focus.is_focused(window));
        window.press("enter", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.content_mode, ContentMode::Reading);
        assert_eq!(view.reader.current, 1);
        assert_eq!(
            view.files_ui.source.as_deref(),
            Some(root.join("2.png").as_path())
        );
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.double_click(entry_id(&zip), cx)
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.book.as_ref().unwrap().path, zip);
        assert_eq!(view.page_count(), 2);
        assert_eq!(view.files_ui.source.as_deref(), Some(zip.as_path()));
    });
    assert_eq!(std::fs::read(zip).unwrap(), bytes);
}

#[gpui_kit::test]
fn failed_read_retains_book_source_sidebar_query_and_selection(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "failed-read", 2).canonicalize().unwrap();
    fixture.sender.send(vec![root.join("2.png")]).unwrap();
    pump(&fixture, cx);
    let broken = root.join("broken.cbz");
    std::fs::write(&broken, b"not an archive\0\xff").unwrap();
    browse(&fixture, &root, cx);
    replace_input(&fixture, "files-search", "broken", cx);
    let old = fixture
        .view
        .read_with(cx, |view, _| view.reader.book.as_ref().unwrap().clone());
    click(&fixture, entry_id(&broken), cx);
    click(&fixture, "files-read-selected", cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(Arc::ptr_eq(view.reader.book.as_ref().unwrap(), &old));
        assert_eq!(view.reader.current, 1);
        assert_eq!(view.content_mode, ContentMode::Reading);
        assert_eq!(view.files_ui.model.query(), "broken");
        assert_eq!(view.files_ui.model.selected(), Some(broken.as_path()));
        assert_eq!(
            view.files_ui.source.as_deref(),
            Some(root.join("2.png").as_path())
        );
        assert!(view.message_error);
        assert!(!view.opening);
    });
    assert_eq!(std::fs::read(broken).unwrap(), b"not an archive\0\xff");
}

#[gpui_kit::test]
fn external_success_establishes_initial_location_but_preserves_manual_location(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    let first = book(&fixture, "external-first", 3).canonicalize().unwrap();
    let second = book(&fixture, "external-second", 2).canonicalize().unwrap();
    fixture.sender.send(vec![first.join("3.png")]).unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(first.as_path()));
        assert_eq!(
            view.files_ui.model.selected(),
            Some(first.join("3.png").as_path())
        );
        assert!(!view.files_ui.manually_navigated);
    });
    browse(&fixture, &second, cx);
    fixture.sender.send(vec![first.join("2.png")]).unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(second.as_path()));
        assert_eq!(
            view.files_ui.source.as_deref(),
            Some(first.join("2.png").as_path())
        );
        assert!(view.files_ui.manually_navigated);
    });
    replace_input(&fixture, "files-search", "excluded", cx);
    click(&fixture, "files-locate", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(first.as_path()));
        assert_eq!(view.files_ui.model.query(), "");
        assert_eq!(
            view.files_ui.model.selected(),
            Some(first.join("2.png").as_path())
        );
        assert_eq!(view.reader.current, 1);
    });
    cx.update_window(fixture.window, |_, window, cx| {
        assert!(window.find(entry_id(&first.join("2.png"))).visible());
        assert!(window.find("files-current-marker").visible());
        assert!(fixture.view.read(cx).files_ui.focus.is_focused(window));
    })
    .unwrap();
}

#[gpui_kit::test]
fn locate_supports_images_archives_folder_books_and_hidden_ancestor_chains(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "locate", 2).canonicalize().unwrap();
    let hidden = root.join(".隠し-漫畫");
    std::fs::create_dir(&hidden).unwrap();
    let image = hidden.join("選択.png");
    std::fs::copy(root.join("1.png"), &image).unwrap();
    let zip = archive(&fixture, &root, "漫畫.cbz");
    for source in [image.clone(), zip, hidden.clone()] {
        fixture.sender.send(vec![source.clone()]).unwrap();
        pump(&fixture, cx);
        browse(&fixture, &root, cx);
        replace_input(&fixture, "files-search", "no-match", cx);
        fixture.view.update(cx, |view, cx| {
            view.files_ui.model.set_show_hidden(false);
            view.files_rebuild(cx);
        });
        click(&fixture, "files-locate", cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.files_ui.source.as_deref(), Some(source.as_path()));
            assert_eq!(view.files_ui.model.selected(), Some(source.as_path()));
            assert_eq!(view.files_ui.model.focused(), Some(source.as_path()));
            assert_eq!(view.files_ui.model.query(), "");
            assert!(view.files_ui.model.selected_visible_entry().is_some());
            if source.starts_with(&hidden) {
                assert!(view.files_ui.model.show_hidden());
            }
        });
        cx.update_window(fixture.window, |_, window, _| {
            assert!(window.find(entry_id(&source)).visible());
            assert_eq!(
                window.find("files-read-selected").disabled() == Some(true),
                source.is_dir()
            );
            assert!(window.find("files-current-marker").visible());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn loaded_only_search_locks_expansion_and_collapse_disables_hidden_selection(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "search", 1).canonicalize().unwrap();
    let loaded = root.join("loaded");
    let unloaded = root.join("unloaded");
    for folder in [&loaded, &unloaded] {
        std::fs::create_dir(folder).unwrap();
        std::fs::copy(root.join("1.png"), folder.join("needle.png")).unwrap();
    }
    browse(&fixture, &root, cx);
    click(&fixture, disclosure_id(&loaded), cx);
    click(&fixture, entry_id(&loaded.join("needle.png")), cx);
    click(&fixture, disclosure_id(&loaded), cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("files-read-selected").disabled(), Some(true));
    })
    .unwrap();
    replace_input(&fixture, "files-search", "needle", cx);
    assert_eq!(
        visible(&fixture, cx),
        [loaded.clone(), loaded.join("needle.png")]
    );
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(window.find(disclosure_id(&loaded)).disabled(), Some(true));
        assert_ne!(window.find("files-read-selected").disabled(), Some(true));
        fixture
            .view
            .read(cx)
            .files_ui
            .focus
            .clone()
            .focus(window, cx);
        window.press("home", cx);
        window.press("left", cx);
        window.press("right", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(!view.files_ui.model.node(&loaded).unwrap().expanded);
        assert!(!view.files_ui.model.node(&unloaded).unwrap().loaded);
        assert_eq!(
            view.files_ui.model.focused(),
            Some(loaded.join("needle.png").as_path())
        );
    });
    replace_input(&fixture, "files-search", "", cx);
    assert_eq!(
        visible(&fixture, cx),
        [loaded.clone(), unloaded, root.join("1.png")]
    );
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("files-read-selected").disabled(), Some(true));
    })
    .unwrap();
    replace_input(&fixture, "files-search", "absent", cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert!(window.find("files-empty").visible())
    })
    .unwrap();
}

#[gpui_kit::test]
fn composite_keyboard_focus_moves_rows_without_reader_or_input_shortcuts(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "keyboard", 3).canonicalize().unwrap();
    let folder = root.join("folder");
    std::fs::create_dir(&folder).unwrap();
    std::fs::copy(root.join("1.png"), folder.join("child.png")).unwrap();
    fixture.sender.send(vec![root.join("2.png")]).unwrap();
    pump(&fixture, cx);
    browse(&fixture, &root, cx);
    focus_tree(&fixture, cx);
    for (key, expected) in [
        ("home", folder.clone()),
        ("end", root.join("3.png")),
        ("up", root.join("2.png")),
        ("down", root.join("3.png")),
        ("right", root.join("3.png")),
    ] {
        press(&fixture, key, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            let view = fixture.view.read(cx);
            assert!(view.files_ui.focus.is_focused(window), "{key}");
            assert_eq!(
                view.files_ui.model.focused(),
                Some(expected.as_path()),
                "{key}"
            );
            assert_eq!(view.reader.current, 1, "tree keys must not turn pages");
            assert_eq!(view.content_mode, ContentMode::Reading);
        })
        .unwrap();
    }
    press(&fixture, "home", cx);
    press(&fixture, "right", cx);
    press(&fixture, "right", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.files_ui.model.focused(),
            Some(folder.join("child.png").as_path())
        )
    });
    press(&fixture, "left", cx);
    press(&fixture, "left", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.focused(), Some(folder.as_path()));
        assert!(!view.files_ui.model.node(&folder).unwrap().expanded);
    });
    replace_input(&fixture, "files-search", "2.pngx", cx);
    press(&fixture, "backspace", cx);
    press(&fixture, "enter", cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("files-search").value(), Some("2.png"));
        assert_eq!(window.find("files-search").focused(), Some(true));
    })
    .unwrap();
    replace_input(&fixture, "path-input", "discard-this-draft", cx);
    press(&fixture, "backspace", cx);
    press(&fixture, "escape", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(
            window.find("path-input").value(),
            Some(crate::file_tree::display_path(&root).as_str())
        );
        assert!(fixture.view.read(cx).files_ui.focus.is_focused(window));
        assert_eq!(fixture.view.read(cx).reader.current, 1);
        assert_eq!(
            fixture.view.read(cx).files_ui.model.location(),
            Some(root.as_path())
        );
    })
    .unwrap();
    replace_input(&fixture, "files-search", "", cx);
    focus_tree(&fixture, cx);
    press(&fixture, "home", cx);
    press(&fixture, "enter", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(folder.as_path()))
    });
    press(&fixture, "alt-up", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()));
        assert_eq!(view.content_mode, ContentMode::Reading);
    });
}

#[gpui_kit::test]
fn history_restores_query_selection_and_exact_scroll_anchor(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "history", 100).canonicalize().unwrap();
    let child = root.join("child");
    std::fs::create_dir(&child).unwrap();
    browse(&fixture, &root, cx);
    replace_input(&fixture, "files-search", ".png", cx);
    fixture.view.update(cx, |view, cx| {
        view.files_ui.model.select(&root.join("50.png"));
        view.files_ui.list.scroll_to(ListOffset {
            item_ix: 45,
            offset_in_item: px(7.),
        });
        cx.notify();
    });
    pump(&fixture, cx);
    let before = fixture
        .view
        .read_with(cx, |view, _| view.files_ui.list.logical_scroll_top());
    assert_eq!((before.item_ix, before.offset_in_item), (45, px(7.)));
    browse(&fixture, &child, cx);
    click(&fixture, "files-back", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()));
        assert_eq!(view.files_ui.model.query(), ".png");
        assert_eq!(
            view.files_ui.model.selected(),
            Some(root.join("50.png").as_path())
        );
        let after = view.files_ui.list.logical_scroll_top();
        assert_eq!(
            (after.item_ix, after.offset_in_item),
            (before.item_ix, before.offset_in_item)
        );
    });
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("files-search").value(), Some(".png"))
    })
    .unwrap();
}

#[gpui_kit::test]
fn invalid_navigation_retains_tree_and_retry_accepts_newly_created_unicode_folder(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "navigation-retry", 2)
        .canonicalize()
        .unwrap();
    browse(&fixture, &root, cx);
    click(&fixture, entry_id(&root.join("2.png")), cx);
    let target = root.join("新しい-漫畫");
    browse(&fixture, &target, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.files_ui.model.error().is_some());
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()));
        assert_eq!(
            view.files_ui.model.selected(),
            Some(root.join("2.png").as_path())
        );
    });
    cx.update_window(fixture.window, |_, window, _| {
        assert!(window.find("files-navigation-error").visible());
        assert_ne!(window.find("files-retry-navigation").disabled(), Some(true));
    })
    .unwrap();
    std::fs::create_dir(&target).unwrap();
    click(&fixture, "files-retry-navigation", cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.files_ui.model.error().is_none());
        assert_eq!(view.files_ui.model.location(), Some(target.as_path()));
    });
    cx.update_window(fixture.window, |_, window, _| {
        assert!(window.try_find("files-navigation-error").is_none());
        assert_eq!(window.find("files-read-selected").disabled(), Some(true));
        assert_eq!(window.find("files-read-folder").disabled(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn deleted_branch_reports_error_and_retry_does_not_change_other_expansion(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "branch-retry", 1).canonicalize().unwrap();
    let a = root.join("a");
    let b = root.join("b");
    for folder in [&a, &b] {
        std::fs::create_dir(folder).unwrap();
    }
    browse(&fixture, &root, cx);
    click(&fixture, disclosure_id(&b), cx);
    std::fs::remove_dir(&a).unwrap();
    click(&fixture, disclosure_id(&a), cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.files_ui.model.node_error(&a).is_some())
    });
    std::fs::create_dir(&a).unwrap();
    std::fs::copy(root.join("1.png"), a.join("restored.png")).unwrap();
    click(
        &fixture,
        SharedString::from(format!("files-retry:{:?}", a.as_os_str())),
        cx,
    );
    fixture.view.read_with(cx, |view, _| {
        assert!(view.files_ui.model.node_error(&a).is_none());
        assert!(view.files_ui.model.node(&a).unwrap().expanded);
        assert!(view.files_ui.model.node(&b).unwrap().expanded);
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()));
    });
    assert!(visible(&fixture, cx).contains(&a.join("restored.png")));
}

#[gpui_kit::test]
fn rapid_disclosures_do_not_reopen_collapsed_branch_or_drop_queued_siblings(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "rapid-branches", 1).canonicalize().unwrap();
    let branches = ["a", "b", "c"].map(|name| root.join(name));
    for folder in &branches {
        std::fs::create_dir(folder).unwrap();
        std::fs::copy(root.join("1.png"), folder.join("child.png")).unwrap();
    }
    browse(&fixture, &root, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        for folder in &branches {
            window.click(disclosure_id(folder), cx);
        }
        for _ in 0..3 {
            window.click(disclosure_id(&branches[0]), cx);
        }
        assert!(
            !fixture
                .view
                .read(cx)
                .files_ui
                .model
                .node(&branches[0])
                .unwrap()
                .expanded
        );
    })
    .unwrap();
    pump(&fixture, cx);
    let rows = visible(&fixture, cx);
    assert!(!rows.contains(&branches[0].join("child.png")));
    for folder in &branches[1..] {
        assert!(
            rows.contains(&folder.join("child.png")),
            "queued sibling must finish loading"
        );
    }
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()));
        assert!(!view.files_ui.model.node(&branches[0]).unwrap().expanded);
        assert!(!view.files_ui.model.is_loading(&branches[0]));
    });
}

#[gpui_kit::test]
fn rapid_navigation_and_close_reject_stale_directory_and_book_results(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let first = book(&fixture, "rapid-first", 3).canonicalize().unwrap();
    let last = book(&fixture, "rapid-last", 1).canonicalize().unwrap();
    browse(&fixture, &first, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            for path in [&first, &last, &first, &last] {
                view.files_browse(path.clone(), true, window, cx);
            }
        });
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(last.as_path()))
    });
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.files_browse(first.clone(), true, window, cx);
            view.open_path_from(first.join("2.png"), OpenSource::Sidebar, window, cx);
            view.perform(Command::Close, window, cx);
        });
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(last.as_path()));
        assert!(view.files_ui.source.is_none());
        assert!(view.files_ui.model.pending_navigation().is_none());
        assert!(view.reader.book.is_none());
        assert!(!view.opening);
        assert_eq!(view.content_mode, ContentMode::Browsing);
    });
}

#[gpui_kit::test]
fn read_folder_uses_recursive_image_policy_and_never_combines_archives(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "folder-reading", 1).canonicalize().unwrap();
    let child = root.join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::copy(root.join("1.png"), child.join("2.png")).unwrap();
    archive(&fixture, &root, "separate.cbz");
    browse(&fixture, &root, cx);
    for (recursive, count) in [(false, 1), (true, 2)] {
        fixture.view.update(cx, |view, cx| {
            view.reader.settings.preferences.recursive_folder = recursive;
            cx.notify();
        });
        render_windows(&fixture, cx);
        click(&fixture, "files-read-folder", cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.page_count(), count);
            assert_eq!(view.reader.book.as_ref().unwrap().path, root);
            assert_eq!(view.files_ui.source.as_deref(), Some(root.as_path()));
        });
    }
    let archives_only = root.join("archives-only");
    std::fs::create_dir(&archives_only).unwrap();
    std::fs::copy(
        root.join("separate.cbz"),
        archives_only.join("separate.cbz"),
    )
    .unwrap();
    browse(&fixture, &archives_only, cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(
            window.find("files-read-folder").disabled(),
            Some(true),
            "archives must remain separate books"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn ten_thousand_native_rows_render_only_viewport_and_end_reveals_last_entry(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    pump(&fixture, cx);
    let root = book(&fixture, "virtual", 1).canonicalize().unwrap();
    let mut loaded = load_directory(&root).unwrap();
    let template = loaded.entries.pop().unwrap();
    loaded.entries = (0..10_000)
        .map(|index| {
            let mut item = template.clone();
            item.entry.name = format!("comic-{index:05}.png");
            item.entry.entry_path = root.join(&item.entry.name);
            item.entry.path = item.entry.entry_path.clone();
            item
        })
        .collect();
    fixture.view.update(cx, |view, cx| {
        let mut model = FileTree::new();
        model.request_navigation(root.clone());
        let request = model.take_requests().pop().unwrap();
        assert!(model.complete(request.token, Ok(loaded)));
        view.files_ui.model = model;
        view.files_rebuild(cx);
    });
    render_windows(&fixture, cx);
    focus_tree(&fixture, cx);
    press(&fixture, "end", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.visible_rows().len(), 10_000);
        assert_eq!(
            view.files_ui.model.focused(),
            Some(root.join("comic-09999.png").as_path())
        );
        assert!(view.files_ui.list.logical_scroll_top().item_ix > 9900);
        let rendered = view.files_ui.rendered.borrow();
        assert!(
            !rendered.is_empty() && rendered.len() < 40,
            "viewport rows: {}",
            rendered.len()
        );
        assert!(rendered.iter().all(|index| *index < 10_000));
    });
    cx.update_window(fixture.window, |_, window, _| {
        assert!(
            window
                .find(entry_id(&root.join("comic-09999.png")))
                .visible()
        );
        for index in [0, 100, 1000, 5000] {
            assert!(
                window
                    .try_find(entry_id(&root.join(format!("comic-{index:05}.png"))))
                    .is_none()
            );
        }
    })
    .unwrap();
    assert_eq!(
        std::fs::read_dir(root).unwrap().count(),
        1,
        "virtual coverage must not create 10,000 files"
    );
}

#[gpui_kit::test]
fn sidebar_read_returns_to_its_source_not_the_saved_central_location(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let central = book(&fixture, "saved-central", 3).canonicalize().unwrap();
    let sidebar = book(&fixture, "sidebar-source", 3).canonicalize().unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .view
            .update(cx, |view, cx| view.browse(central.clone(), window, cx));
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.update(cx, |view, cx| {
        view.browser.model.query = "2.png".into();
        view.rebuild_browser(cx);
        view.remember_browser_location();
    });
    browse(&fixture, &sidebar, cx);
    click(&fixture, entry_id(&sidebar.join("3.png")), cx);
    click(&fixture, "files-read-selected", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("viewer", cx);
        window.press("backspace", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.content_mode, ContentMode::Browsing);
        assert_eq!(view.listing.as_ref().unwrap().path, sidebar);
        assert_eq!(view.browser.model.selected, Some(sidebar.join("3.png")));
        assert_eq!(
            view.files_ui.source.as_deref(),
            Some(sidebar.join("3.png").as_path())
        );
        assert_eq!(view.files_ui.model.location(), Some(sidebar.as_path()));
    });
    click(&fixture, "files-locate", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.files_ui.model.selected(),
            Some(sidebar.join("3.png").as_path())
        );
        assert!(
            view.reading_origin.is_none(),
            "Locate must not depend on the consumed return origin"
        );
    });
}

#[gpui_kit::test]
fn removing_files_controls_restores_reader_focus_without_losing_tree_state(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "focus-removal", 2).canonicalize().unwrap();
    fixture.sender.send(vec![root.join("1.png")]).unwrap();
    pump(&fixture, cx);
    browse(&fixture, &root, cx);
    click(&fixture, entry_id(&root.join("2.png")), cx);
    click(&fixture, "縮圖", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert!(fixture.view.read(cx).focus.is_focused(window));
        assert!(window.try_find("files-tree").is_none());
    })
    .unwrap();
    click(&fixture, "檔案", cx);
    click(&fixture, entry_id(&root.join("2.png")), cx);
    click(&fixture, "sidebar-toggle", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert!(fixture.view.read(cx).focus.is_focused(window));
        assert!(window.try_find("files-tree").is_none());
    })
    .unwrap();
    click(&fixture, "sidebar-toggle", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()));
        assert_eq!(
            view.files_ui.model.selected(),
            Some(root.join("2.png").as_path())
        );
        assert_eq!(view.reader.current, 0);
    });
}

#[cfg(any(unix, windows))]
#[gpui_kit::test]
fn colliding_lossy_names_keep_distinct_native_row_and_selection_identities(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    pump(&fixture, cx);
    let root = book(&fixture, "native-identities", 1)
        .canonicalize()
        .unwrap();
    #[cfg(windows)]
    let names = {
        use std::os::windows::ffi::OsStringExt;
        [0xd800, 0xd801].map(|unit| {
            let mut units: Vec<_> = "native-".encode_utf16().collect();
            units.push(unit);
            units.extend(".png".encode_utf16());
            std::ffi::OsString::from_wide(&units)
        })
    };
    #[cfg(unix)]
    let names = {
        use std::os::unix::ffi::OsStringExt;
        [0xfe, 0xff].map(|byte| {
            let mut bytes = b"native-".to_vec();
            bytes.push(byte);
            bytes.extend(b".png");
            std::ffi::OsString::from_vec(bytes)
        })
    };
    assert_eq!(names[0].to_string_lossy(), names[1].to_string_lossy());
    let paths = names.map(|name| root.join(name));
    assert_ne!(entry_id(&paths[0]), entry_id(&paths[1]));
    let mut loaded = load_directory(&root).unwrap();
    let template = loaded.entries.pop().unwrap();
    loaded.entries = paths
        .iter()
        .map(|path| {
            let mut item = template.clone();
            item.entry.entry_path = path.clone();
            item.entry.path = path.clone();
            item.entry.name = path.file_name().unwrap().to_string_lossy().into_owned();
            item
        })
        .collect();
    fixture.view.update(cx, |view, cx| {
        let mut model = FileTree::new();
        model.request_navigation(root.clone());
        let request = model.take_requests().pop().unwrap();
        assert!(model.complete(request.token, Ok(loaded)));
        view.files_ui.model = model;
        view.files_rebuild(cx);
    });
    render_windows(&fixture, cx);
    for path in &paths {
        click(&fixture, entry_id(path), cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.files_ui.model.selected(), Some(path.as_path()))
        });
        cx.update_window(fixture.window, |_, window, _| {
            assert_eq!(window.find(entry_id(path)).checked(), Some(true))
        })
        .unwrap();
    }
}

#[cfg(windows)]
#[gpui_kit::test]
fn verbatim_location_submission_preserves_native_path_and_invalid_draft_retains_tree(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "verbatim-漫畫", 1).canonicalize().unwrap();
    assert!(root.as_os_str().to_string_lossy().starts_with(r"\\?\"));
    browse(&fixture, &root, cx);
    let displayed = crate::file_tree::display_path(&root);
    assert!(!displayed.starts_with(r"\\?\"));
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(window.find("path-input").value(), Some(displayed.as_str()));
        window.click("path-input", cx);
        window.press("enter", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()))
    });
    // An incomplete UNC prefix fails locally; it does not contact a network share.
    replace_input(&fixture, "path-input", r"\\?\UNC\", cx);
    press(&fixture, "enter", cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.files_ui.model.error().is_some());
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()));
        assert_eq!(view.files_ui.model.root(), Some(root.as_path()));
    });
    assert_eq!(
        crate::file_tree::display_path(Path::new(r"\\?\UNC\server\share\漫畫")),
        r"\\server\share\漫畫"
    );
}

#[cfg(windows)]
#[gpui_kit::test]
fn windows_hidden_attributes_are_session_only_and_locate_reveals_source(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "attributes", 3).canonicalize().unwrap();
    for (name, flag) in [("2.png", "+H"), ("3.png", "+S")] {
        assert!(
            std::process::Command::new("attrib.exe")
                .arg(flag)
                .arg(root.join(name))
                .status()
                .unwrap()
                .success()
        );
    }
    browse(&fixture, &root, cx);
    assert_eq!(visible(&fixture, cx), [root.join("1.png")]);
    fixture
        .view
        .update(cx, |view, _| view.flush_settings().unwrap());
    let settings = fixture.directory.path().join("settings.json");
    let bytes = std::fs::read(&settings).unwrap();
    click(&fixture, "files-hidden", cx);
    assert_eq!(visible(&fixture, cx).len(), 3);
    click(&fixture, "files-hidden", cx);
    assert_eq!(std::fs::read(settings).unwrap(), bytes);
    fixture.sender.send(vec![root.join("2.png")]).unwrap();
    pump(&fixture, cx);
    click(&fixture, "files-locate", cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.files_ui.model.show_hidden());
        assert_eq!(
            view.files_ui.model.selected(),
            Some(root.join("2.png").as_path())
        );
    });
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("files-hidden").checked(), Some(true));
        assert!(window.find(entry_id(&root.join("2.png"))).visible());
    })
    .unwrap();
    for (name, flag) in [("2.png", "-H"), ("3.png", "-S")] {
        assert!(
            std::process::Command::new("attrib.exe")
                .arg(flag)
                .arg(root.join(name))
                .status()
                .unwrap()
                .success()
        );
    }
}

#[cfg(windows)]
#[gpui_kit::test]
fn windows_junction_aliases_keep_native_rows_and_locate_alias_source(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "junctions", 1).canonicalize().unwrap();
    let target = book(&fixture, "junction-target", 2).canonicalize().unwrap();
    let alias = root.join("alias");
    let cycle = root.join("cycle");
    for (link, destination) in [(&alias, &target), (&cycle, &root)] {
        assert!(
            std::process::Command::new("cmd.exe")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(crate::file_tree::display_path(link))
                .arg(crate::file_tree::display_path(destination))
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    browse(&fixture, &root, cx);
    click(&fixture, disclosure_id(&alias), cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.files_ui.model.node(&cycle).unwrap().cycle);
        assert_eq!(view.files_ui.model.node(&alias).unwrap().entry.path, target);
        assert!(view.files_ui.model.node(&alias.join("2.png")).is_some());
    });
    cx.update_window(fixture.window, |_, window, cx| {
        assert_eq!(window.find(disclosure_id(&cycle)).disabled(), Some(true));
        window.double_click(entry_id(&alias.join("2.png")), cx);
    })
    .unwrap();
    pump(&fixture, cx);
    click(&fixture, "files-locate", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.files_ui.source.as_deref(),
            Some(alias.join("2.png").as_path())
        );
        assert_eq!(
            view.files_ui.model.selected(),
            Some(alias.join("2.png").as_path())
        );
        assert_eq!(view.reader.book.as_ref().unwrap().path, target);
        assert_eq!(view.reader.current, 1);
    });
    for link in [&alias, &cycle] {
        std::fs::remove_dir(link).unwrap();
    }
}

#[gpui_kit::test]
fn accepted_parent_components_show_normalized_paths_and_navigate_the_real_parent(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "normalized-location", 2)
        .canonicalize()
        .unwrap();
    std::fs::create_dir(root.join("series")).unwrap();
    let ordinary = PathBuf::from(crate::file_tree::display_path(&root));
    let expected = std::path::absolute(&ordinary).unwrap();
    browse(&fixture, &expected, cx);
    let draft = ordinary.join("series").join("..");
    replace_input(&fixture, "path-input", &draft.to_string_lossy(), cx);
    press(&fixture, "enter", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(expected.as_path()));
    });
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(
            window.find("path-input").value(),
            Some(crate::file_tree::display_path(&expected).as_str())
        );
    })
    .unwrap();
    click(&fixture, "files-parent", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), expected.parent());
    });

    let source = ordinary.join("series").join("..").join("2.png");
    fixture.sender.send(vec![source]).unwrap();
    pump(&fixture, cx);
    click(&fixture, "files-locate", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(
            view.files_ui.source.as_deref(),
            Some(expected.join("2.png").as_path())
        );
        assert_eq!(view.files_ui.model.location(), Some(expected.as_path()));
        assert_eq!(
            view.files_ui.model.selected(),
            Some(expected.join("2.png").as_path())
        );
        assert_eq!(view.reader.current, 1);
    });
}

#[gpui_kit::test]
fn navigating_to_a_far_sibling_reveals_its_focused_tree_row(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "navigation-focus", 1)
        .canonicalize()
        .unwrap();
    for index in 0..100 {
        std::fs::create_dir(root.join(format!("series-{index:03}"))).unwrap();
    }
    let target = root.join("series-099");
    browse(&fixture, &root, cx);
    fixture.view.update(cx, |view, cx| {
        view.files_ui.list.scroll_to(ListOffset {
            item_ix: 99,
            offset_in_item: px(0.),
        });
        cx.notify();
    });
    pump(&fixture, cx);
    click(&fixture, entry_id(&target), cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(target.as_path()));
        assert_eq!(view.files_ui.model.focused(), Some(target.as_path()));
        assert!(view.files_ui.list.logical_scroll_top().item_ix > 50);
    });
    cx.update_window(fixture.window, |_, window, _| {
        let tree = window.find("files-tree").bounds();
        let row = window.find(entry_id(&target)).bounds();
        assert!(row.top() >= tree.top() && row.bottom() <= tree.bottom());
    })
    .unwrap();
}

#[gpui_kit::test]
fn sidebar_book_replacement_releases_the_borrowed_windows_image_atlas(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let first = book(&fixture, "atlas-first", 1).canonicalize().unwrap();
    let second = book(&fixture, "atlas-second", 1).canonicalize().unwrap();
    fixture.sender.send(vec![first.join("1.png")]).unwrap();
    pump(&fixture, cx);
    let original = fixture
        .view
        .read_with(cx, |view, _| view.pages[&0].tiles[0].image.clone());
    cx.update_window(fixture.window, |_, window, _| {
        assert!(window.has_image_atlas_entry(&original));
    })
    .unwrap();
    browse(&fixture, &second, cx);
    click(&fixture, entry_id(&second.join("1.png")), cx);
    click(&fixture, "files-read-selected", cx);
    let replacement = fixture
        .view
        .read_with(cx, |view, _| view.pages[&0].tiles[0].image.clone());
    assert_ne!(original.id, replacement.id);
    cx.update_window(fixture.window, |_, window, _| {
        assert!(!window.has_image_atlas_entry(&original));
        assert!(window.has_image_atlas_entry(&replacement));
    })
    .unwrap();
}

#[gpui_kit::test]
fn failed_locate_retry_repeats_locate_not_previous_navigation(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let source_folder = book(&fixture, "locate-retry-source", 1)
        .canonicalize()
        .unwrap();
    let source = source_folder.join("1.png");
    let bytes = std::fs::read(&source).unwrap();
    fixture.sender.send(vec![source.clone()]).unwrap();
    pump(&fixture, cx);
    let unrelated = book(&fixture, "locate-retry-unrelated", 1)
        .canonicalize()
        .unwrap();
    browse(&fixture, &unrelated, cx);
    std::fs::remove_file(&source).unwrap();
    click(&fixture, "files-locate", cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.files_ui.model.error().is_some());
        assert_eq!(view.files_ui.model.location(), Some(unrelated.as_path()));
    });
    std::fs::write(&source, &bytes).unwrap();
    click(&fixture, "files-retry-navigation", cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.files_ui.model.error().is_none());
        assert_eq!(
            view.files_ui.model.location(),
            Some(source_folder.as_path())
        );
        assert_eq!(view.files_ui.model.selected(), Some(source.as_path()));
        assert_eq!(view.files_ui.source.as_deref(), Some(source.as_path()));
    });
    assert_eq!(std::fs::read(&source).unwrap(), bytes);
}

#[gpui_kit::test]
fn completed_or_delayed_locate_does_not_reclaim_reader_focus(cx: &mut TestAppContext) {
    let fixture = files_fixture(cx);
    let root = book(&fixture, "locate-focus", 1).canonicalize().unwrap();
    let child = root.join("child");
    std::fs::create_dir(&child).unwrap();
    fixture.sender.send(vec![root.join("1.png")]).unwrap();
    pump(&fixture, cx);
    browse(&fixture, &root, cx);
    click(&fixture, "files-locate", cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click(disclosure_id(&child), cx);
        fixture.view.read(cx).focus.clone().focus(window, cx);
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        assert!(fixture.view.read(cx).focus.is_focused(window));
    })
    .unwrap();

    let other = book(&fixture, "locate-focus-other", 1)
        .canonicalize()
        .unwrap();
    browse(&fixture, &other, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.files_locate(true, window, cx);
            view.perform(Command::Sidebar, window, cx);
        });
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        let view = fixture.view.read(cx);
        assert!(!view.reader.settings.preferences.sidebar);
        assert!(view.focus.is_focused(window));
        assert_eq!(view.reader.current, 0);
    })
    .unwrap();
}

#[gpui_kit::test]
fn newer_book_intent_cancels_older_sidebar_locate_before_success_or_failure(
    cx: &mut TestAppContext,
) {
    let fixture = files_fixture(cx);
    let first = book(&fixture, "source-first", 1).canonicalize().unwrap();
    let second = book(&fixture, "source-second", 1).canonicalize().unwrap();
    let location = book(&fixture, "source-manual", 1).canonicalize().unwrap();
    fixture.sender.send(vec![first.join("1.png")]).unwrap();
    pump(&fixture, cx);
    browse(&fixture, &location, cx);
    for (target, succeeds) in [
        (second.join("missing.cbz"), false),
        (second.join("1.png"), true),
    ] {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, cx| {
                view.files_locate(true, window, cx);
                view.open_path(target.clone(), window, cx);
            });
        })
        .unwrap();
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.files_ui.model.location(), Some(location.as_path()));
            assert_eq!(view.content_mode, ContentMode::Reading);
            let expected = if succeeds {
                second.join("1.png")
            } else {
                first.join("1.png")
            };
            assert_eq!(view.files_ui.source.as_deref(), Some(expected.as_path()));
            assert_eq!(view.message_error, !succeeds);
        });
        cx.update_window(fixture.window, |_, window, cx| {
            assert!(fixture.view.read(cx).focus.is_focused(window));
        })
        .unwrap();
    }
}
