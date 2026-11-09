use super::*;
use crate::release_check::{NotificationPreferences, RELEASES_URL, Release};
use crate::ui::release_notifications::{CheckStatus, ReleaseNotifications};

fn newer_release() -> Result<Option<Release>, String> {
    Ok(Some(Release {
        version: "99.0.0".to_owned(),
    }))
}

fn no_release() -> Result<Option<Release>, String> {
    Ok(None)
}

fn failed_check() -> Result<Option<Release>, String> {
    Err("Network unavailable".to_owned())
}

fn scroll_to_releases(fixture: &Fixture, cx: &mut TestAppContext) -> AnyWindowHandle {
    let preferences = open_preferences(fixture, cx);
    cx.update_window(preferences, |_, window, cx| {
        window.scroll(
            "preferences-content",
            gpui_kit::ScrollDelta::Lines(point(0., -100.)),
            cx,
        );
    })
    .unwrap();
    render_windows(fixture, cx);
    preferences
}

#[gpui_kit::test]
fn startup_check_can_be_disabled_without_disabling_manual_checks(cx: &mut TestAppContext) {
    let fixture = fixture(cx);
    fixture.view.update(cx, |view, cx| {
        assert!(matches!(
            view.release_notifications.status,
            CheckStatus::Idle
        ));
        view.release_notifications.check = newer_release;
        view.set_release_startup_check(false, cx);
        view.check_for_updates_on_startup(cx);
        assert!(matches!(
            view.release_notifications.status,
            CheckStatus::Idle
        ));
    });
    let path = fixture.directory.path().join("release-notifications.json");
    assert!(
        !NotificationPreferences::load_from(&path)
            .unwrap()
            .check_on_startup
    );
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.view.update(cx, |view, cx| {
            view.perform(Command::CheckForUpdates, window, cx);
        });
    })
    .unwrap();
    pump(&fixture, cx);
    fixture.view.read_with(cx, |view, _| {
        assert!(view.release_notifications.available());
        assert!(!view.release_notifications.preferences.check_on_startup);
        assert!(!view.dirty);
    });
    assert!(cx.opened_url().is_none(), "checks must not open a browser");
}

#[gpui_kit::test]
fn startup_check_shows_a_non_modal_notice_and_only_user_action_opens_releases(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let path = book(&fixture, "reading", 2);
    fixture.sender.send(vec![path]).unwrap();
    pump(&fixture, cx);
    let settings = fixture
        .view
        .read_with(cx, |view, _| view.reader.settings.clone());
    fixture.view.update(cx, |view, cx| {
        view.release_notifications.check = newer_release;
        view.check_for_updates_on_startup(cx);
        assert!(view.release_notifications.checking());
        // A second request cannot start while the first request is in flight.
        view.release_notifications.check = failed_check;
        view.check_for_updates(cx);
    });
    pump(&fixture, cx);
    assert!(cx.opened_url().is_none());
    fixture.view.read_with(cx, |view, _| {
        assert!(view.release_notifications.available());
        assert_eq!(view.reader.settings, settings);
        assert!(view.preferences_window.is_none());
    });
    cx.update_window(fixture.window, |_, window, cx| {
        let expected = Language::TraditionalChinese.format(
            Text::NewReleaseAvailable,
            &[("version", "99.0.0".to_owned())],
        );
        assert_eq!(
            window.find("release-notice").label(),
            Some(expected.as_str())
        );
        window.click("release-notice-open", cx);
        window.click("next", cx);
    })
    .unwrap();
    assert_eq!(cx.opened_url().as_deref(), Some(RELEASES_URL));
    fixture
        .view
        .read_with(cx, |view, _| assert_eq!(view.reader.current, 1));
    cx.update_window(fixture.window, |_, window, cx| {
        window.click("release-notice-dismiss", cx);
    })
    .unwrap();
    render_windows(&fixture, cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert!(window.try_find("release-notice").is_none());
    })
    .unwrap();
    // A new explicit check can show the notice again.
    fixture.view.update(cx, |view, cx| {
        view.release_notifications.check = newer_release;
        view.check_for_updates(cx);
    });
    pump(&fixture, cx);
    cx.update_window(fixture.window, |_, window, _| {
        assert!(window.find("release-notice").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn preferences_controls_persist_separately_and_release_notices_translate_live(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    fixture.view.update(cx, |view, _| {
        view.reader.settings.save_to(&view.settings_path).unwrap();
    });
    let settings_path = fixture.directory.path().join("settings.json");
    let original = std::fs::read(&settings_path).unwrap();
    let preferences = scroll_to_releases(&fixture, cx);
    click_preference(&fixture, "release-check-on-startup", cx);
    let path = fixture.directory.path().join("release-notifications.json");
    assert!(
        !NotificationPreferences::load_from(&path)
            .unwrap()
            .check_on_startup
    );
    assert_eq!(std::fs::read(&settings_path).unwrap(), original);
    fixture.view.update(cx, |view, _| {
        view.release_notifications.check = newer_release;
    });
    click_preference(&fixture, "check-for-updates", cx);
    pump(&fixture, cx);
    for language in Language::ALL {
        cx.update_window(fixture.window, |_, window, cx| {
            fixture
                .view
                .update(cx, |view, cx| view.set_language(language, window, cx));
        })
        .unwrap();
        render_windows(&fixture, cx);
        let expected = language.format(
            Text::NewReleaseAvailable,
            &[("version", "99.0.0".to_owned())],
        );
        cx.update_window(fixture.window, |_, window, _| {
            assert_eq!(
                window.find("release-notice").label(),
                Some(expected.as_str())
            );
            assert_eq!(
                window.find("release-notice-open").label(),
                Some(language.text(Text::OpenReleases))
            );
        })
        .unwrap();
        cx.update_window(preferences, |_, window, _| {
            assert_eq!(
                window.find("release-check-status").label(),
                Some(expected.as_str())
            );
            assert_eq!(
                window.find("check-for-updates").label(),
                Some(language.text(Text::CheckForUpdates))
            );
            assert_eq!(
                window.find("release-check-on-startup").checked(),
                Some(false)
            );
        })
        .unwrap();
    }
    click_preference(&fixture, "release-preferences-open", cx);
    assert_eq!(cx.opened_url().as_deref(), Some(RELEASES_URL));
}

#[gpui_kit::test]
fn no_update_and_network_failure_leave_reading_unchanged_and_show_preferences_status(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let path = book(&fixture, "still-reading", 2);
    fixture.sender.send(vec![path]).unwrap();
    pump(&fixture, cx);
    let preferences = scroll_to_releases(&fixture, cx);
    let before = fixture.view.read_with(cx, |view, _| {
        (
            view.reader.settings.clone(),
            view.message.clone(),
            view.content_mode,
        )
    });
    for (check, text) in [
        (
            no_release as fn() -> Result<Option<Release>, String>,
            Text::NoNewRelease,
        ),
        (failed_check, Text::ReleaseCheckFailed),
    ] {
        fixture
            .view
            .update(cx, |view, _| view.release_notifications.check = check);
        click_preference(&fixture, "check-for-updates", cx);
        pump(&fixture, cx);
        fixture.view.read_with(cx, |view, _| {
            assert_eq!(view.reader.settings, before.0);
            assert_eq!(view.message, before.1);
            assert_eq!(view.content_mode, before.2);
        });
        let expected = Language::TraditionalChinese
            .format(text, &[("error", "Network unavailable".to_owned())]);
        cx.update_window(preferences, |_, window, _| {
            assert_eq!(
                window.find("release-check-status").label(),
                Some(expected.as_str())
            );
            assert_ne!(window.find("check-for-updates").disabled(), Some(true));
        })
        .unwrap();
        cx.update_window(fixture.window, |_, window, _| {
            assert!(window.try_find("release-notice").is_none());
        })
        .unwrap();
    }
    assert!(cx.opened_url().is_none());
}

#[gpui_kit::test]
fn malformed_notification_preferences_disable_startup_and_can_be_replaced_explicitly(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let path = fixture.directory.path().join("release-notifications.json");
    std::fs::write(&path, b"invalid").unwrap();
    fixture.view.update(cx, |view, cx| {
        view.release_notifications = ReleaseNotifications::new(&view.settings_path);
        view.release_notifications.check = newer_release;
        view.check_for_updates_on_startup(cx);
        assert!(!view.release_notifications.preferences.check_on_startup);
        assert!(matches!(
            view.release_notifications.status,
            CheckStatus::Idle
        ));
    });
    let preferences = scroll_to_releases(&fixture, cx);
    cx.update_window(preferences, |_, window, _| {
        assert!(window.find("release-preferences-error").label().is_some());
    })
    .unwrap();
    click_preference(&fixture, "release-check-on-startup", cx);
    assert!(
        NotificationPreferences::load_from(&path)
            .unwrap()
            .check_on_startup
    );
    fixture.view.read_with(cx, |view, _| {
        assert!(view.release_notifications.preference_error.is_none());
        assert!(matches!(
            view.release_notifications.status,
            CheckStatus::Idle
        ));
    });
}

#[gpui_kit::test]
fn preference_save_failure_and_late_check_result_do_not_change_reader_data(
    cx: &mut TestAppContext,
) {
    let fixture = fixture(cx);
    let blocker = fixture.directory.path().join("not-a-directory");
    std::fs::write(&blocker, b"blocker").unwrap();
    fixture.view.update(cx, |view, cx| {
        let settings = view.reader.settings.clone();
        view.settings_path = blocker.join("settings.json");
        view.set_release_startup_check(false, cx);
        assert!(view.release_notifications.preferences.check_on_startup);
        assert!(view.release_notifications.preference_error.is_some());
        assert_eq!(view.reader.settings, settings);
        view.release_notifications.status = CheckStatus::Checking;
        view.closing = true;
        view.release_check_finished(newer_release(), cx);
        assert!(view.release_notifications.checking());
    });
    render_windows(&fixture, cx);
    assert!(cx.opened_url().is_none());
}
