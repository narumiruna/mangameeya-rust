//! Native picker coverage for anonymous browser and Files targets.

use super::*;

fn enable_picker(fixture: &Fixture, cx: &mut TestAppContext) {
    fixture.view.update(cx, |view, cx| {
        view.apply_developer_enabled(true, cx);
        assert!(view.developer.reader_picker.is_some());
    });
    render_windows(fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("developer-picker", cx);
    })
    .unwrap();
    render_windows(fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.developer.reader_picker.as_ref().unwrap().active());
    });
}

fn assert_safe_target(
    fixture: &Fixture,
    prefix: &str,
    label: &str,
    kind: crate::developer::TargetKind,
    cx: &mut TestAppContext,
) {
    fixture.view.read_with(cx, |view, _| {
        let target = view.developer.selected_target().expect("picker selection");
        assert!(target.id.starts_with(prefix), "{}", target.id);
        assert!(
            target.id[prefix.len()..]
                .bytes()
                .all(|b| b.is_ascii_digit())
        );
        assert_eq!(target.label, label);
        assert_eq!(target.target_kind, kind);
        let preview = serde_json::to_string(&target).unwrap();
        assert!(!preview.contains("PRIVATE"));
        assert!(!preview.contains(&fixture.directory.path().to_string_lossy().to_string()));
    });
}

#[gpui_kit::test]
fn browser_picker_uses_anonymous_tile_identity_and_consumes_double_click(cx: &mut TestAppContext) {
    for language in Language::ALL {
        let mut settings = AppSettings::default();
        settings.preferences.language = language;
        let fixture = fixture_with_settings(cx, settings);
        let root = book(&fixture, "PRIVATE-library", 1).canonicalize().unwrap();
        let child = root.join("PRIVATE-folder");
        std::fs::create_dir(&child).unwrap();
        cx.update_window(fixture.window, |_, window, cx| {
            fixture
                .view
                .update(cx, |view, cx| view.browse(root.clone(), window, cx));
        })
        .unwrap();
        pump(&fixture, cx);
        let before = fixture
            .view
            .read_with(cx, |view, _| view.browser.model.selected.clone());
        enable_picker(&fixture, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            window.double_click(super::super::browser::browser_entry_id(&child), cx);
        })
        .unwrap();
        render_windows(&fixture, cx);
        assert_safe_target(
            &fixture,
            "browser-item-",
            "Browser item",
            crate::developer::TargetKind::Element,
            cx,
        );
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.listing.as_ref().unwrap().path, root);
            assert_eq!(view.browser.model.selected, before);
            assert!(view.reader.book.is_none());
        });
    }
}

#[gpui_kit::test]
fn files_picker_uses_anonymous_item_identity_without_navigation(cx: &mut TestAppContext) {
    let mut settings = AppSettings::default();
    settings.preferences.sidebar = true;
    settings.preferences.sidebar_mode = SidebarMode::Files;
    let fixture = fixture_with_settings(cx, settings);
    let root = book(&fixture, "PRIVATE-tree", 1).canonicalize().unwrap();
    let child = root.join("PRIVATE-folder");
    std::fs::create_dir(&child).unwrap();
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.files_browse(root.clone(), true, window, cx)
        });
    })
    .unwrap();
    pump(&fixture, cx);
    enable_picker(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.double_click(super::super::files::entry_id(&child), cx);
    })
    .unwrap();
    render_windows(&fixture, cx);
    assert_safe_target(
        &fixture,
        "files-entry-",
        "File tree item",
        crate::developer::TargetKind::Element,
        cx,
    );
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.files_ui.model.location(), Some(root.as_path()));
        assert!(view.reader.book.is_none());
    });
}

#[gpui_kit::test]
fn browser_search_picker_reports_a_safe_region_not_input_text(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.browser.search.update(cx, |input, cx| {
                input.set_value("PRIVATE-search", window, cx);
            });
        });
    })
    .unwrap();
    render_windows(&fixture, cx);
    enable_picker(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("browser-search", cx);
    })
    .unwrap();
    render_windows(&fixture, cx);
    fixture.view.read_with(cx, |view, cx| {
        let target = view
            .developer
            .selected_target()
            .expect("search region selection");
        assert_eq!(target.id, "browser-search");
        assert_eq!(target.label, "Search browser");
        assert_eq!(target.target_kind, crate::developer::TargetKind::Region);
        assert_eq!(
            view.browser.search.read(cx).value().as_ref(),
            "PRIVATE-search"
        );
        assert!(!serde_json::to_string(&target).unwrap().contains("PRIVATE"));
    });
}
