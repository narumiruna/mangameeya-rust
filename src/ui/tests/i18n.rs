use super::*;
use crate::i18n::{Language, Text};
use gpui_kit::OwnedMenuItem;

const LANGUAGES: [(Language, &str, &str); 3] = [
    (Language::English, "language-en", "English"),
    (Language::Japanese, "language-ja", "日本語"),
    (Language::TraditionalChinese, "language-zh-Hant", "正體中文"),
];

fn assert_language_choices(window: &Window, selected: Language) {
    for (language, id, label) in LANGUAGES {
        let button = window.find(id);
        assert_eq!(button.label(), Some(label), "{id}");
        assert_eq!(button.checked(), Some(language == selected), "{id}");
    }
}

fn assert_controls(window: &Window, language: Language) {
    assert_eq!(
        window.find("preferences").label(),
        Some(language.text(Text::Preferences))
    );
    assert_eq!(
        window.find("page-input").label(),
        Some(language.text(Text::PageNumber))
    );
    let trigger = window.find("zoom-trigger");
    let label = trigger.label().unwrap();
    assert!(
        label.starts_with(language.text(Text::ZoomSettings)),
        "{label}"
    );
    assert!(label.contains(language.text(Text::FitWindow)), "{label}");
}

fn assert_native_menus(cx: &App, language: Language) {
    let menus = cx
        .get_menus()
        .expect("test platform must expose native menus");
    for (command, text) in [
        (Command::Settings, Text::Preferences),
        (Command::FitWindow, Text::FitWindow),
    ] {
        let label = menus
            .iter()
            .flat_map(|menu| &menu.items)
            .find_map(|item| match item {
                OwnedMenuItem::Action { name, action, .. }
                    if action.as_any().downcast_ref::<Command>() == Some(&command) =>
                {
                    Some(name.as_str())
                }
                _ => None,
            })
            .expect("localized reader action must be in the native menu");
        assert_eq!(label, language.text(text));
    }
}

#[gpui_kit::test]
fn new_reader_defaults_to_english_controls_and_ready_status(cx: &mut TestAppContext) {
    let fixture = fixture_with_settings(cx, AppSettings::default());
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.settings.preferences.language, Language::English);
    });
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        assert_controls(window, Language::English);
        assert_eq!(window.find("preferences").label(), Some("Preferences"));
        assert_eq!(window.find("status-message").label(), Some("Ready"));
        assert_eq!(window.find("zoom-trigger").disabled(), Some(true));
        window.click("preferences", cx);
        assert_language_choices(window, Language::English);
        assert_native_menus(cx, Language::English);
        for (language, id, _) in [LANGUAGES[1], LANGUAGES[2], LANGUAGES[0]] {
            window.click(id, cx);
            assert_controls(window, language);
            assert_eq!(
                window.find("status-message").label(),
                Some(language.text(Text::Ready))
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn preference_clicks_translate_controls_status_and_both_menus_immediately(cx: &mut TestAppContext) {
    let mut settings = AppSettings::default();
    settings.preferences.sidebar_mode = SidebarMode::Files;
    let fixture = fixture_with_settings(cx, settings);
    let path = book(&fixture, "localized", 3);
    fixture.sender.send(vec![path]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("preferences", cx)
    })
    .unwrap();
    for (language, id, _) in [LANGUAGES[1], LANGUAGES[2], LANGUAGES[0]] {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture.view.update(cx, |view, _| view.dirty = false);
            window.click(id, cx);
            fixture.view.read_with(cx, |view, _| {
                assert_eq!(view.reader.settings.preferences.language, language);
                assert!(view.dirty, "language changes must be saved");
            });
            assert_language_choices(window, language);
            assert_controls(window, language);
            assert_eq!(
                window.find("path-input").label(),
                Some(language.text(Text::FolderPath))
            );
            assert_eq!(
                window.find("path-input").placeholder(),
                Some(language.text(Text::FolderPath))
            );
            let opened = language.format(
                Text::BookOpened,
                &[("title", "localized".into()), ("count", "3".into())],
            );
            assert_eq!(window.find("status-message").label(), Some(opened.as_str()));
            assert_native_menus(cx, language);

            // The reader menus are accessible even without a native menu bar.
            window.click(("menu", 2usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(fixture.window, |_, window, cx| {
            window.render_frame(cx);
            assert!(
                window
                    .within("popup-menu")
                    .find_by_label(language.text(Text::FitWindow))
                    .visible()
            );
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(fixture.window, |_, window, cx| {
            window.click("zoom-trigger", cx);
            assert_eq!(
                window.find("zoom-panel").label(),
                Some(language.text(Text::ZoomSettings))
            );
            assert_eq!(
                window.find("zoom-fit-window").label(),
                Some(language.text(Text::FitWindow))
            );
            assert_eq!(window.find("zoom-fit-window").checked(), Some(true));
            assert_eq!(window.find("zoom-input").value(), Some("100"));
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn native_open_dialog_prompts_follow_the_selected_language(cx: &mut TestAppContext) {
    let fixture = fixture_with_settings(cx, AppSettings::default());
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("preferences", cx)
    })
    .unwrap();
    for (language, id, _) in [LANGUAGES[1], LANGUAGES[2], LANGUAGES[0]] {
        cx.update_window(fixture.window, |_, window, cx| {
            window.click(id, cx);
            window.click("open", cx);
        })
        .unwrap();
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|options| {
            assert_eq!(
                options.prompt.as_deref(),
                Some(language.text(Text::OpenComicOrImage))
            );
            assert!(options.files && !options.directories && !options.multiple);
            None
        });
        pump(&fixture, cx);
        cx.update_window(fixture.window, |_, window, cx| {
            window.press("ctrl-shift-o", cx);
        })
        .unwrap();
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|options| {
            assert_eq!(
                options.prompt.as_deref(),
                Some(language.text(Text::OpenImageFolder))
            );
            assert!(!options.files && options.directories && !options.multiple);
            None
        });
        pump(&fixture, cx);
    }
    fixture.view.read_with(cx, |view, _| {
        assert!(view.reader.book.is_none());
        assert!(view.reader.settings.recent.is_empty());
    });
}

#[gpui_kit::test]
fn switching_language_reformats_an_existing_bookmark_message(cx: &mut TestAppContext) {
    let fixture = fixture_with_settings(cx, AppSettings::default());
    let path = book(&fixture, "bookmark", 4);
    fixture.sender.send(vec![path]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.navigate(2, window, cx);
            view.perform(Command::Bookmark, window, cx);
        });
        window.render_frame(cx);
        let original = window.find("status-message").label().unwrap().to_string();
        for (language, _, _) in [LANGUAGES[1], LANGUAGES[2], LANGUAGES[0]] {
            fixture.view.update(cx, |view, cx| {
                view.set_language(language, window, cx);
                assert!(!view.message_error);
            });
            window.render_frame(cx);
            let expected = language.format(Text::BookmarkSet, &[("page", "3".into())]);
            assert_eq!(
                window.find("status-message").label(),
                Some(expected.as_str())
            );
            if language != Language::English {
                assert_ne!(expected, original);
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn translated_error_status_preserves_the_original_details(cx: &mut TestAppContext) {
    let fixture = fixture_with_settings(cx, AppSettings::default());
    let details = "Cannot read 日本語/正體中文/{error}.cbz: OS error 13";
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.set_error(details, cx);
        });
        for (language, _, _) in [LANGUAGES[1], LANGUAGES[2], LANGUAGES[0]] {
            fixture.view.update(cx, |view, cx| {
                view.set_language(language, window, cx);
                assert!(view.message_error);
            });
            window.render_frame(cx);
            let expected = language.format(Text::ErrorDetails, &[("error", details.into())]);
            let status = window.find("status-message");
            assert_eq!(status.label(), Some(expected.as_str()));
            assert!(status.label().unwrap().contains(details));
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn language_changes_preserve_reader_state_history_and_input_drafts(cx: &mut TestAppContext) {
    let mut settings = AppSettings::default();
    let historical = PathBuf::from("previous-book.cbz");
    settings.recent.push(historical.clone());
    settings.positions.insert(historical.clone(), 7);
    settings.bookmarks.insert(historical, 4);
    let fixture = fixture_with_settings(cx, settings);
    let path = book(&fixture, "preserved", 5);
    fixture.sender.send(vec![path]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.navigate(2, window, cx);
            view.reader.set_mode(ReadingMode::Spread);
            view.reader.settings.preferences.rtl = false;
            view.reader.set_zoom(1.75);
            view.reader.rotation = 90;
            view.reader.filters = Filters {
                brightness: 115.,
                contrast: 85.,
                grayscale: 30.,
            };
            view.reader.set_bookmark();
            view.inspector_tab = InspectorTab::Reading;
            view.reader
                .settings
                .preferences
                .panel_layout
                .inspector_visible = true;
            view.zoom_open = true;
            view.controls_pending = false;
            view.path_pending = false;
            for (input, draft) in [
                (&view.page_input, "004"),
                (&view.path_input, "/draft/日本語/正體中文"),
                (&view.interval_input, "17"),
                (&view.zoom_input, "62."),
            ] {
                input.update(cx, |input, cx| input.set_value(draft, window, cx));
            }
            view.zoom_input.read(cx).focus_handle(cx).focus(window, cx);
            let book = view.reader.book.clone().unwrap();
            let filters = view.reader.filters;
            let revision = (view.book_revision, view.image_revision, view.open_revision);
            let mut expected = view.reader.settings.clone();
            for (language, _, _) in [LANGUAGES[1], LANGUAGES[2], LANGUAGES[0]] {
                view.dirty = false;
                view.set_language(language, window, cx);
                expected.preferences.language = language;
                assert_eq!(view.reader.settings, expected);
                assert!(Arc::ptr_eq(view.reader.book.as_ref().unwrap(), &book));
                assert_eq!(view.reader.current, 2);
                assert_eq!(view.reader.zoom, 1.75);
                assert_eq!(view.reader.rotation, 90);
                assert_eq!(view.reader.filters, filters);
                assert_eq!(
                    (view.book_revision, view.image_revision, view.open_revision),
                    revision
                );
                assert!(view.dirty);
                assert_eq!(view.inspector_tab, InspectorTab::Reading);
                assert!(
                    view.reader
                        .settings
                        .preferences
                        .panel_layout
                        .inspector_visible
                );
                assert!(view.zoom_open);
                assert!(!view.controls_pending && !view.path_pending);
                for (input, draft) in [
                    (&view.page_input, "004"),
                    (&view.path_input, "/draft/日本語/正體中文"),
                    (&view.interval_input, "17"),
                    (&view.zoom_input, "62."),
                ] {
                    assert_eq!(input.read(cx).value().as_ref(), draft);
                }
            }
        });
        window.render_frame(cx);
        assert_eq!(window.find("page-input").value(), Some("004"));
        assert_eq!(window.find("zoom-input").value(), Some("62."));
    })
    .unwrap();
}

#[gpui_kit::test]
fn language_selection_persists_and_is_applied_when_reopening(cx: &mut TestAppContext) {
    let mut fixture = fixture_with_settings(cx, AppSettings::default());
    let folder = book(&fixture, "reopened", 4);
    fixture.sender.send(vec![folder.clone()]).unwrap();
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.navigate(2, window, cx);
            view.perform(Command::Bookmark, window, cx);
        });
        window.click("preferences", cx);
        window.click("language-ja", cx);
    })
    .unwrap();
    pump(&fixture, cx);
    let settings_path = fixture.directory.path().join("settings.json");
    let saved = AppSettings::load_from(&settings_path).unwrap();
    assert_eq!(saved.preferences.language, Language::Japanese);
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings_path).unwrap()).unwrap();
    assert_eq!(json["preferences"]["language"], "ja");
    let original = saved.clone();
    cx.update_window(fixture.window, |_, window, _| window.remove_window())
        .unwrap();
    let (sender, launches) = mpsc::channel();
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| ReaderView::new(saved, settings_path, None, None, launches, window, cx))
        })
        .unwrap()
    });
    fixture.window = window;
    fixture.view = view;
    fixture.sender = sender;
    pump(&fixture, cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_controls(window, Language::Japanese);
        assert_eq!(
            window.find("status-message").label(),
            Some(Language::Japanese.text(Text::Ready))
        );
        assert_native_menus(cx, Language::Japanese);
    })
    .unwrap();
    fixture.sender.send(vec![folder]).unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert_eq!(view.reader.current, 2);
        assert_eq!(view.reader.settings, original);
    });
}
