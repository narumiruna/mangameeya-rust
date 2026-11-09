//! Bookmark commands, stable folder images, persistence, and unavailable targets.

use super::*;

fn folder(fixture: &Fixture) -> PathBuf {
    let root = book(fixture, "bookmark-folder", 2);
    std::fs::rename(root.join("1.png"), root.join("10.png")).unwrap();
    std::fs::rename(root.join("2.png"), root.join("20.png")).unwrap();
    std::fs::create_dir(root.join("00")).unwrap();
    std::fs::copy(root.join("10.png"), root.join("00/1.png")).unwrap();
    root
}

fn press(fixture: &Fixture, key: &str, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| window.press(key, cx))
        .unwrap();
    pump(fixture, cx);
}

fn reopen(fixture: &Fixture, root: &Path, cx: &mut TestAppContext) {
    fixture.sender.send(vec![root.to_path_buf()]).unwrap();
    pump(fixture, cx);
}

fn toggle_recursion(fixture: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        window.dispatch_action(Box::new(Command::ToggleRecursiveFolder), cx);
    })
    .unwrap();
    pump(fixture, cx);
}

fn bookmark_menu(fixture: &Fixture, text: Text, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        window.click(("menu", 4usize), cx)
    })
    .unwrap();
    render_windows(fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        let label = fixture.view.read(cx).language().text(text);
        let item = window.within("popup-menu").find_by_label(label);
        let menu = window.find("popup-menu").bounds();
        window.click_at("popup-menu", item.bounds().center() - menu.origin, cx);
    })
    .unwrap();
    pump(fixture, cx);
}

#[gpui_kit::test]
fn bookmark_controls_keep_the_image_after_recursion_and_save_update_and_delete(
    cx: &mut TestAppContext,
) {
    let mut settings = AppSettings::default();
    settings.preferences.recursive_folder = false;
    settings.preferences.remember_position = false;
    let fixture = fixture_with_settings(cx, settings);
    let root = folder(&fixture);
    let source = std::fs::read(root.join("20.png")).unwrap();
    reopen(&fixture, &root, cx);
    press(&fixture, "pagedown", cx);
    press(&fixture, "b", cx);
    let key = root.canonicalize().unwrap();
    let settings_path = fixture.directory.path().join("settings.json");
    let saved = AppSettings::load_from(&settings_path).unwrap();
    assert_eq!(saved.bookmarks[&key], 1);
    assert_eq!(
        saved.bookmark_images[&key].image,
        root.join("20.png").canonicalize().unwrap()
    );
    assert!(saved.positions.is_empty());
    toggle_recursion(&fixture, cx);
    reopen(&fixture, &root, cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("bookmark").checked(), Some(false));
    })
    .unwrap();
    press(&fixture, "shift-b", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.current_name(), Some("20.png"));
        assert_eq!(view.reader.current, 2);
        assert_eq!(view.reader.settings.bookmarks[&key], 2);
        assert_eq!(view.reader.settings.bookmark_images[&key].page, 2);
    });
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("bookmark").checked(), Some(true));
    })
    .unwrap();
    press(&fixture, "home", cx);
    cx.update_window(fixture.window, |_, window, cx| window.click("bookmark", cx))
        .unwrap();
    pump(&fixture, cx);
    let saved = AppSettings::load_from(&settings_path).unwrap();
    assert_eq!(saved.bookmarks.len(), 1);
    assert_eq!(saved.bookmarks[&key], 0);
    assert_eq!(
        saved.bookmark_images[&key].image,
        root.join("00/1.png").canonicalize().unwrap()
    );
    press(&fixture, "pagedown", cx);
    bookmark_menu(&fixture, Text::GoToBookmark, cx);
    fixture
        .view
        .read_with(cx, |view, _| assert_eq!(view.reader.current, 0));
    bookmark_menu(&fixture, Text::DeleteBookmark, cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert_eq!(window.find("bookmark").checked(), Some(false));
    })
    .unwrap();
    let saved = AppSettings::load_from(&settings_path).unwrap();
    assert!(saved.bookmarks.is_empty());
    assert!(saved.bookmark_images.is_empty());
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings_path).unwrap()).unwrap();
    assert!(json.get("bookmark_images").is_none());
    assert_eq!(std::fs::read(root.join("20.png")).unwrap(), source);
}

#[gpui_kit::test]
fn unavailable_bookmark_keeps_the_page_and_identity_in_all_locales(cx: &mut TestAppContext) {
    let mut settings = AppSettings::default();
    settings.preferences.remember_position = false;
    let fixture = fixture_with_settings(cx, settings);
    let root = folder(&fixture);
    reopen(&fixture, &root, cx);
    press(&fixture, "b", cx);
    let target = fixture
        .view
        .read_with(cx, |view, _| view.reader.settings.bookmark_images.clone());
    toggle_recursion(&fixture, cx);
    reopen(&fixture, &root, cx);
    press(&fixture, "pagedown", cx);
    for language in [
        Language::English,
        Language::Japanese,
        Language::TraditionalChinese,
    ] {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture
                .view
                .update(cx, |view, cx| view.set_language(language, window, cx));
        })
        .unwrap();
        bookmark_menu(&fixture, Text::GoToBookmark, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.reader.current_name(), Some("20.png"));
            assert_eq!(view.reader.current, 1);
            assert_eq!(view.reader.settings.bookmark_images, target);
            assert!(view.message_error);
        });
        cx.update_window(fixture.window, |_, window, _| {
            assert_eq!(window.find("bookmark").checked(), Some(false));
            assert_eq!(
                window.find("status-message").label(),
                Some(language.text(Text::BookmarkUnavailable))
            );
        })
        .unwrap();
    }
    toggle_recursion(&fixture, cx);
    reopen(&fixture, &root, cx);
    press(&fixture, "pagedown", cx);
    press(&fixture, "shift-b", cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.current, 0);
        assert!(view.reader.current_name().unwrap().ends_with("1.png"));
        assert_eq!(view.reader.settings.bookmark_images, target);
    });
}
