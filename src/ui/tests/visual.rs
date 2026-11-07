//! Visual geometry and painted-text checks, separate from accessibility labels.

use super::*;

fn painted_text(window: &Window, id: &'static str, expected: &str) {
    let id: gpui_kit::ElementId = id.into();
    assert!(
        gpui_kit::base::test_support::texts(window)
            .iter()
            .any(|text| { text.path().contains(&id) && text.visible() && text.text() == expected }),
        "{id:?} must paint {expected:?}, not only expose an accessible label"
    );
}

#[gpui_kit::test]
fn visual_chrome_dimensions_and_status_center_survive_sizes_and_locales(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "visual-layout", 3)])
        .unwrap();
    pump(&fixture, cx);
    for language in Language::ALL {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture
                .view
                .update(cx, |view, cx| view.set_language(language, window, cx));
        })
        .unwrap();
        for (width, height) in [(1280., 800.), (960., 640.), (800., 540.)] {
            cx.update_window(fixture.window, |_, window, cx| {
                window.resize(size(px(width), px(height)));
                window.bounds_changed(cx);
            })
            .unwrap();
            pump(&fixture, cx);
            cx.update_window(fixture.window, |_, window, _| {
                let toolbar = window.find("reader-toolbar").bounds();
                let status = window.find("reader-status").bounds();
                let navigation = window.find("status-navigation").bounds();
                assert_eq!(toolbar.top(), px(MENU_HEIGHT));
                assert_eq!(toolbar.size.height, px(TOOLBAR_HEIGHT));
                assert_eq!(status.size.height, px(STATUS_HEIGHT + 1.));
                assert!((navigation.center().x - px(width / 2.)).abs() < px(1.));
                assert!(navigation.left() >= status.left() && navigation.right() <= status.right());
                for id in [
                    "sidebar-toggle",
                    "inspector-toggle",
                    "bookmark",
                    "fullscreen",
                    "preferences",
                ] {
                    assert_eq!(
                        window.find(id).bounds().size,
                        size(px(CONTROL_HEIGHT), px(CONTROL_HEIGHT))
                    );
                }
                if width < 1200. {
                    for id in ["mode-single", "mode-spread", "mode-continuous"] {
                        assert_eq!(window.find(id).bounds().size.width, px(CONTROL_HEIGHT));
                    }
                } else {
                    painted_text(window, "direction", direction_label(language, true));
                }
                painted_text(
                    window,
                    "sidebar-heading",
                    language.text(Text::ContentNavigation),
                );
            })
            .unwrap();
        }
        cx.update_window(fixture.window, |_, window, cx| {
            window.resize(size(px(1280.), px(800.)));
            window.bounds_changed(cx);
        })
        .unwrap();
        pump(&fixture, cx);
    }
}

#[gpui_kit::test]
fn visual_direction_values_and_background_choices_are_visible_and_do_not_reset_preferences(
    cx: &mut TestAppContext,
) {
    let mut settings = AppSettings::default();
    settings.preferences.sidebar_mode = SidebarMode::Recent;
    settings.preferences.background = Background::Paper;
    let fixture = fixture_with_settings(cx, settings);
    pump(&fixture, cx);
    for language in Language::ALL {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture
                .view
                .update(cx, |view, cx| view.set_language(language, window, cx));
            window.click("preferences", cx);
            for _ in 0..2 {
                let rtl = fixture.view.read(cx).reader.settings.preferences.rtl;
                painted_text(window, "reading-direction", direction_label(language, rtl));
                painted_text(window, "direction", direction_label(language, rtl));
                window.click("reading-direction", cx);
                assert_ne!(fixture.view.read(cx).reader.settings.preferences.rtl, rtl);
            }
            assert_eq!(window.find("紙色").checked(), Some(true));
            assert_eq!(window.find("黑色").checked(), Some(false));
            window.click("inspector-image", cx);
            painted_text(
                window,
                "inspector-content",
                language.text(Text::FiltersScope),
            );
            let prefs = &fixture.view.read(cx).reader.settings.preferences;
            assert_eq!(prefs.background, Background::Paper);
            assert_eq!(prefs.sidebar_mode, SidebarMode::Recent);
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn visual_original_pages_center_without_changing_scale(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture
        .sender
        .send(vec![book(&fixture, "visual-centering", 1)])
        .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.press("1", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        let view = fixture.view.read(cx);
        let book = view.reader.book.as_ref().unwrap();
        let page = window
            .find(SharedString::from(format!(
                "page:{}:{}",
                book.path.display(),
                book.pages[0].name
            )))
            .bounds();
        let viewer = window.find("viewer").bounds();
        assert_eq!(page.size, size(px(128.), px(208.)));
        assert!((page.center().x - viewer.center().x).abs() < px(1.));
        assert!((page.center().y - viewer.center().y).abs() < px(1.));
        assert_eq!(view.effective_scale(), Some(1.));
    })
    .unwrap();
}
