use super::developer::{enable, read_frame, wait_for, write_frame};
use super::*;
use std::net::TcpStream;

const SESSION: &str = "requested-pi-session";
const WORKSPACE: &str = "C:/workspace/requested-project";

fn request(f: &Fixture, cx: &mut TestAppContext) -> TcpStream {
    let (port, instance) = f.view.read_with(cx, |view, _| {
        let service = view.developer.service.as_ref().unwrap();
        let command = service.connection_command().unwrap();
        (
            command
                .split_whitespace()
                .nth(1)
                .unwrap()
                .parse::<u16>()
                .unwrap(),
            service.status().capture_id,
        )
    });
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write_frame(
        &mut stream,
        serde_json::json!({
            "v":1,"id":crate::developer::random_id().unwrap(),"type":"pair_request",
            "instance_id":instance,"session_id":SESSION,"workspace":WORKSPACE
        }),
    );
    wait_for(f, cx, |view| {
        view.developer
            .service
            .as_ref()
            .unwrap()
            .pending_pairing()
            .is_some()
    });
    stream
}

#[gpui_kit::test]
fn developer_pairing_requires_visible_approval_across_locales_and_sizes(cx: &mut TestAppContext) {
    for language in Language::ALL {
        for (width, height) in [(1280., 800.), (960., 640.), (800., 540.)] {
            let mut settings = AppSettings::default();
            settings.preferences.language = language;
            let f = fixture_with_settings(cx, settings);
            enable(&f, cx);
            cx.update_window(f.window, |_, window, cx| {
                window.resize(size(px(width), px(height)));
                window.bounds_changed(cx);
            })
            .unwrap();
            let mut connection = request(&f, cx);
            cx.update_window(f.window, |_, window, _| {
                let content = window.find("developer-pairing-request");
                assert!(content.visible());
                let viewport = Bounds::new(Default::default(), window.viewport_size());
                assert!(content.bounds().left() >= viewport.left());
                assert!(content.bounds().right() <= viewport.right());
                assert!(content.bounds().bottom() <= viewport.bottom());
                assert_eq!(
                    window.find("developer-pairing-workspace").label(),
                    Some(
                        language
                            .format(
                                Text::DeveloperPairWorkspace,
                                &[("workspace", WORKSPACE.into())]
                            )
                            .as_str()
                    )
                );
                assert_eq!(
                    window.find("developer-pairing-session").label(),
                    Some(
                        language
                            .format(Text::DeveloperPairSession, &[("session", SESSION.into())])
                            .as_str()
                    )
                );
                assert_eq!(
                    window.find("ok").label(),
                    Some(language.text(Text::DeveloperPairAllow))
                );
                assert_eq!(
                    window.find("cancel").label(),
                    Some(language.text(Text::DeveloperPairDeny))
                );
            })
            .unwrap();
            f.view.read_with(cx, |view, _| {
                assert_eq!(view.developer.state_text(), Text::DeveloperWaiting);
                assert!(view.developer.busy());
            });
            cx.update_window(f.window, |_, window, cx| window.click("ok", cx))
                .unwrap();
            wait_for(&f, cx, |view| {
                view.developer.state_text() == Text::DeveloperConnected
            });
            assert_eq!(read_frame(&mut connection)["type"], "hello_ok");
            f.view.read_with(cx, |view, _| {
                assert!(!view.developer.busy());
                let status = view.developer.service.as_ref().unwrap().status();
                assert_eq!(status.session_id.as_deref(), Some(SESSION));
                assert_eq!(status.workspace.as_deref(), Some(WORKSPACE));
            });
            cx.update_window(f.window, |_, window, _| window.remove_window())
                .unwrap();
        }
    }
}

#[gpui_kit::test]
fn developer_pairing_deny_and_escape_do_not_authorize(cx: &mut TestAppContext) {
    for escape in [false, true] {
        let f = fixture(cx);
        enable(&f, cx);
        let mut connection = request(&f, cx);
        cx.update_window(f.window, |_, window, cx| {
            if escape {
                window.press("escape", cx);
            } else {
                window.click("cancel", cx);
            }
        })
        .unwrap();
        pump(&f, cx);
        let result = read_frame(&mut connection);
        assert_eq!(result["type"], "error");
        assert_eq!(result["code"], "canceled");
        f.view.read_with(cx, |view, _| {
            assert_eq!(view.developer.state_text(), Text::DeveloperWaiting);
            assert!(!view.developer.busy());
        });
        cx.update_window(f.window, |_, window, _| {
            assert!(window.try_find("developer-pairing-request").is_none());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn developer_pairing_client_close_and_disable_remove_the_prompt(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    let connection = request(&f, cx);
    drop(connection);
    wait_for(&f, cx, |view| {
        view.developer
            .service
            .as_ref()
            .unwrap()
            .pending_pairing()
            .is_none()
            && !view.developer.busy()
    });
    cx.update_window(f.window, |_, window, _| {
        assert!(window.try_find("developer-pairing-request").is_none());
    })
    .unwrap();
    let _connection = request(&f, cx);
    f.view
        .update(cx, |view, cx| view.apply_developer_enabled(false, cx));
    pump(&f, cx);
    cx.update_window(f.window, |_, window, _| {
        assert!(window.try_find("developer-pairing-request").is_none());
    })
    .unwrap();
    f.view
        .read_with(cx, |view, _| assert!(!view.developer.busy()));
}

#[gpui_kit::test]
fn developer_pairing_waits_for_an_existing_dialog(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    cx.update_window(f.window, |_, window, cx| {
        window.open_alert_dialog(cx, |dialog, _, _| {
            dialog.confirm().title("Existing dialog").child(
                div()
                    .id("existing-dialog")
                    .test_support()
                    .child("Keep this dialog"),
            )
        });
    })
    .unwrap();
    let connection = request(&f, cx);
    cx.update_window(f.window, |_, window, cx| {
        assert!(window.find("existing-dialog").visible());
        assert!(window.try_find("developer-pairing-request").is_none());
        window.press("escape", cx);
    })
    .unwrap();
    pump(&f, cx);
    cx.update_window(f.window, |_, window, _| {
        assert!(window.find("developer-pairing-request").visible());
        assert!(window.try_find("existing-dialog").is_none());
    })
    .unwrap();
    drop(connection);
    wait_for(&f, cx, |view| !view.developer.busy());
}
