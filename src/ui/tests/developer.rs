use super::*;
use crate::developer::{DeveloperPreferences, TargetKind};
use gpui_kit::{MouseDownEvent, MouseUpEvent, PlatformInput, ScrollDelta};
use std::{
    io::{Read, Write},
    net::TcpStream,
};

fn enable(f: &Fixture, cx: &mut TestAppContext) {
    f.view
        .update(cx, |v, cx| v.apply_developer_enabled(true, cx));
    pump(f, cx);
}
fn pick_open(f: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(f.window, |_, w, cx| w.click("developer-picker", cx))
        .unwrap();
    pump(f, cx);
    cx.update_window(f.window, |_, w, cx| w.click("open", cx))
        .unwrap();
    pump(f, cx);
}
fn wait_for(f: &Fixture, cx: &mut TestAppContext, predicate: impl Fn(&ReaderView) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        pump(f, cx);
        if f.view.read_with(cx, |v, _| predicate(v)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "developer worker state did not settle"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn write_frame(stream: &mut TcpStream, value: serde_json::Value) {
    let bytes = serde_json::to_vec(&value).unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    stream.write_all(&bytes).unwrap();
}
fn read_frame(stream: &mut TcpStream) -> serde_json::Value {
    let mut length = [0; 4];
    stream.read_exact(&mut length).unwrap();
    let mut bytes = vec![0; u32::from_be_bytes(length) as usize];
    assert!(bytes.len() <= 65536);
    stream.read_exact(&mut bytes).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
fn pair(f: &Fixture, cx: &mut TestAppContext) -> TcpStream {
    let command = f.view.read_with(cx, |v, _| {
        v.developer
            .service
            .as_ref()
            .unwrap()
            .connection_command()
            .unwrap()
    });
    let parts: Vec<_> = command.split_whitespace().collect();
    let mut stream = TcpStream::connect(("127.0.0.1", parts[1].parse::<u16>().unwrap())).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write_frame(
        &mut stream,
        serde_json::json!({"v":1,"id":"pair","type":"hello","token":parts[2],"session_id":"test-session","workspace":"test-workspace"}),
    );
    assert_eq!(read_frame(&mut stream)["type"], "hello_ok");
    stream
}
#[gpui_kit::test]
fn developer_held_pointer_rejects_changed_state_identity_and_bounds(cx: &mut TestAppContext) {
    for change in 0..3 {
        let f = fixture(cx);
        enable(&f, cx);
        cx.update_window(f.window, |_, w, cx| w.click("developer-picker", cx))
            .unwrap();
        pump(&f, cx);
        let position = cx
            .update_window(f.window, |_, w, cx| {
                let position = w
                    .find(if change == 2 { "preferences" } else { "open" })
                    .bounds()
                    .center();
                w.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent {
                        button: MouseButton::Left,
                        position,
                        ..Default::default()
                    }),
                    cx,
                );
                position
            })
            .unwrap();
        match change {
            0 => f.view.update(cx, |v, cx| {
                v.open_revision += 1;
                cx.notify();
            }),
            1 => f.view.update(cx, |v, cx| {
                v.reader.settings.preferences.mode = ReadingMode::Spread;
                cx.notify();
            }),
            _ => {
                cx.update_window(f.window, |_, w, cx| {
                    w.resize(size(px(800.), px(540.)));
                    w.bounds_changed(cx);
                })
                .unwrap();
            }
        }
        pump(&f, cx);
        cx.update_window(f.window, |_, w, cx| {
            w.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    ..Default::default()
                }),
                cx,
            )
        })
        .unwrap();
        pump(&f, cx);
        f.view.read_with(cx, |v, _| {
            assert!(
                v.developer.selected_target().is_none(),
                "changed snapshot must not open feedback: {change}"
            );
            assert!(v.developer.reader_picker.as_ref().unwrap().active());
            assert!(v.reader.book.is_none());
        });
        cx.update_window(f.window, |_, w, cx| w.press("escape", cx))
            .unwrap();
        pump(&f, cx);
    }
}

#[gpui_kit::test]
fn developer_same_session_reconnect_requires_confirmation(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    let first = pair(&f, cx);
    let generation = f.view.read_with(cx, |v, _| {
        v.developer.service.as_ref().unwrap().status().generation
    });
    pick_open(&f, cx);
    cx.update_window(f.window, |_, w, cx| {
        w.click("developer-feedback-input", cx);
        w.input("Keep this draft", cx);
    })
    .unwrap();
    assert_eq!(
        f.view
            .read_with(cx, |v, cx| v.developer.draft_text(cx))
            .as_deref(),
        Some("Keep this draft")
    );
    drop(first);
    wait_for(&f, cx, |v| {
        v.developer
            .service
            .as_ref()
            .unwrap()
            .status()
            .session_id
            .is_none()
    });
    let mut second = pair(&f, cx);
    assert_ne!(
        f.view.read_with(cx, |v, _| v
            .developer
            .service
            .as_ref()
            .unwrap()
            .status()
            .generation),
        generation
    );
    cx.update_window(f.window, |_, w, cx| w.click("developer-send", cx))
        .unwrap();
    pump(&f, cx);
    second.set_nonblocking(true).unwrap();
    let mut byte = [0];
    assert_eq!(
        second.read(&mut byte).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "reconnect must not send before confirmation"
    );
    second.set_nonblocking(false).unwrap();
    cx.update_window(f.window, |_, w, cx| w.click("ok", cx))
        .unwrap();
    pump(&f, cx);
    second.set_nonblocking(true).unwrap();
    assert_eq!(
        second.read(&mut byte).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "confirmation must not submit automatically"
    );
    second.set_nonblocking(false).unwrap();
    cx.update_window(f.window, |_, w, cx| w.click("developer-send", cx))
        .unwrap();
    let feedback = read_frame(&mut second);
    assert_eq!(feedback["feedback"]["text"], "Keep this draft");
    assert_eq!(feedback["feedback"]["session_id"], "test-session");
}

#[gpui_kit::test]
fn developer_preferences_close_cancels_picker_without_stopping_service(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    let pref = open_preferences(&f, cx);
    cx.update_window(pref, |_, w, cx| {
        w.scroll(
            "preferences-content",
            ScrollDelta::Pixels(point(px(0.), px(-3000.))),
            cx,
        )
    })
    .unwrap();
    pump(&f, cx);
    click_preference(&f, "developer-select", cx);
    assert!(f.view.read_with(cx, |v, _| v.developer.busy()));
    cx.update_window(pref, |_, w, cx| {
        w.press("escape", cx);
        w.press("escape", cx);
    })
    .unwrap();
    pump(&f, cx);
    assert!(cx.update_window(pref, |_, _, _| ()).is_err());
    f.view.read_with(cx, |v, _| {
        assert!(!v.developer.busy());
        assert!(v.developer.service.is_some());
    });
}

#[gpui_kit::test]
fn developer_disable_clears_capture_and_restores_focus(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    f.view.read_with(cx, |v, _| {
        v.developer_log(
            crate::developer::Target::Navigation,
            crate::developer::EventCode::Changed,
            true,
        )
    });
    let logs = f
        .view
        .read_with(cx, |v, _| v.developer.service.as_ref().unwrap().logs());
    assert!(!logs.query(&Default::default()).unwrap().events.is_empty());
    let focus = cx
        .update_window(f.window, |_, w, cx| w.focused(cx))
        .unwrap();
    cx.update_window(f.window, |_, w, cx| w.click("developer-picker", cx))
        .unwrap();
    pump(&f, cx);
    f.view
        .update(cx, |v, cx| v.apply_developer_enabled(false, cx));
    pump(&f, cx);
    f.view.read_with(cx, |v, _| {
        assert!(!v.developer.busy());
        assert!(!v.developer.enabled);
        assert!(v.developer.service.is_none());
        assert!(v.developer.reader_picker.is_none());
        assert!(v.developer.preferences_picker.is_none());
    });
    assert!(logs.query(&Default::default()).unwrap().events.is_empty());
    cx.update_window(f.window, |_, w, cx| {
        assert!(w.try_find("developer-picker").is_none());
        assert_eq!(w.focused(cx), focus);
    })
    .unwrap();
}

#[gpui_kit::test]
fn developer_preferences_native_close_warns_and_cancel_preserves_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    let pref = open_preferences(&f, cx);
    cx.update_window(pref, |_, w, cx| {
        w.scroll(
            "preferences-content",
            ScrollDelta::Pixels(point(px(0.), px(-3000.))),
            cx,
        )
    })
    .unwrap();
    pump(&f, cx);
    click_preference(&f, "developer-select", cx);
    cx.update_window(pref, |_, w, cx| {
        w.scroll(
            "preferences-content",
            ScrollDelta::Pixels(point(px(0.), px(3000.))),
            cx,
        )
    })
    .unwrap();
    pump(&f, cx);
    click_preference(&f, "reading-direction-ltr", cx);
    cx.update_window(pref, |_, w, cx| {
        w.click("developer-feedback-input", cx);
        w.input("Preferences draft", cx);
    })
    .unwrap();
    assert!(!gpui_kit::VisualTestContext::from_window(pref, cx).simulate_close());
    pump(&f, cx);
    cx.update_window(pref, |_, w, cx| {
        assert_eq!(w.find_all("dialog").len(), 2);
        w.press("escape", cx);
    })
    .unwrap();
    pump(&f, cx);
    f.view.read_with(cx, |v, cx| {
        assert!(v.developer.enabled);
        assert_eq!(
            v.developer.draft_text(cx).as_deref(),
            Some("Preferences draft")
        );
    });
    cx.update_window(pref, |_, w, cx| w.press("escape", cx))
        .unwrap();
    pump(&f, cx);
    f.view.read_with(cx, |v, cx| {
        assert!(!v.developer.busy());
        assert!(v.developer.draft_text(cx).is_none());
    });
}

#[gpui_kit::test]
fn developer_reader_close_warns_and_cancel_preserves_draft(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    pick_open(&f, cx);
    cx.update_window(f.window, |_, w, cx| {
        w.click("developer-feedback-input", cx);
        w.input("Unsent request", cx);
    })
    .unwrap();
    assert!(!gpui_kit::VisualTestContext::from_window(f.window, cx).simulate_close());
    pump(&f, cx);
    cx.update_window(f.window, |_, w, cx| {
        assert_eq!(
            w.find_all("dialog").len(),
            2,
            "close must warn above the draft"
        );
        w.press("escape", cx);
    })
    .unwrap();
    pump(&f, cx);
    f.view.read_with(cx, |v, cx| {
        assert!(!v.closing);
        assert!(v.developer.service.is_some());
        assert_eq!(
            v.developer.draft_text(cx).as_deref(),
            Some("Unsent request")
        );
    });
    cx.update_window(f.window, |_, w, cx| w.press("escape", cx))
        .unwrap();
    pump(&f, cx);
    f.view.read_with(cx, |v, cx| {
        assert!(!v.developer.busy());
        assert!(v.developer.draft_text(cx).is_none());
    });
}

#[gpui_kit::test]
fn developer_defaults_off_and_preserves_reader_settings(cx: &mut TestAppContext) {
    let f = fixture(cx);
    pump(&f, cx);
    f.view.read_with(cx, |v, _| {
        assert!(!v.developer.enabled);
        assert!(v.developer.service.is_none());
        assert!(v.developer.reader_picker.is_none());
    });
    cx.update_window(f.window, |_, w, _| {
        assert!(w.try_find("developer-picker").is_none())
    })
    .unwrap();
    let path = f.directory.path().join("settings.json");
    let before = std::fs::read(&path).ok();
    enable(&f, cx);
    f.view
        .update(cx, |v, cx| v.apply_developer_enabled(false, cx));
    pump(&f, cx);
    assert_eq!(std::fs::read(&path).ok(), before);
    assert!(
        !DeveloperPreferences::load_from(&path.with_file_name("developer-mode.json"))
            .unwrap()
            .enabled
    );
}
#[gpui_kit::test]
fn developer_failed_opt_out_can_retry_without_restarting_service(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    let path = f.directory.path().join("developer-mode.json");
    let backup = f.directory.path().join("developer-mode.backup");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    f.view
        .update(cx, |v, cx| v.apply_developer_enabled(false, cx));
    pump(&f, cx);
    f.view.read_with(cx, |v, _| {
        assert!(!v.developer.enabled);
        assert!(v.developer.service.is_none());
        assert!(v.developer.preference_error);
    });
    assert!(DeveloperPreferences::load_from(&backup).unwrap().enabled);
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    let pref = open_preferences(&f, cx);
    cx.update_window(pref, |_, w, cx| {
        w.scroll(
            "preferences-content",
            ScrollDelta::Pixels(point(px(0.), px(-3000.))),
            cx,
        );
    })
    .unwrap();
    pump(&f, cx);
    click_preference(&f, "developer-preference-retry", cx);
    pump(&f, cx);
    assert!(!DeveloperPreferences::load_from(&path).unwrap().enabled);
    f.view.read_with(cx, |v, _| {
        assert!(!v.developer.enabled);
        assert!(v.developer.service.is_none());
        assert!(!v.developer.preference_error);
    });
}

#[gpui_kit::test]
fn developer_applied_preference_with_durability_warning_starts_once(cx: &mut TestAppContext) {
    let f = fixture(cx);
    f.view.update(cx, |v, cx| {
        v.developer_preference_saved(
            true,
            Ok(crate::developer::PreferenceSave::DurabilityUnconfirmed),
            cx,
        );
    });
    pump(&f, cx);
    let capture = f.view.read_with(cx, |v, _| {
        assert!(v.developer.enabled);
        assert!(!v.developer.preference_error);
        assert!(v.developer.preference_durability_warning);
        v.developer.service.as_ref().unwrap().status().capture_id
    });
    let pref = open_preferences(&f, cx);
    cx.update_window(pref, |_, w, cx| {
        w.scroll(
            "preferences-content",
            ScrollDelta::Pixels(point(px(0.), px(-3000.))),
            cx,
        );
    })
    .unwrap();
    pump(&f, cx);
    cx.update_window(pref, |_, w, _| {
        assert!(w.find("developer-preference-durability").visible());
    })
    .unwrap();
    click_preference(&f, "developer-preference-retry", cx);
    pump(&f, cx);
    f.view.read_with(cx, |v, _| {
        assert!(v.developer.enabled);
        assert!(!v.developer.preference_durability_warning);
        assert_eq!(
            v.developer.service.as_ref().unwrap().status().capture_id,
            capture
        );
    });
    assert!(
        DeveloperPreferences::load_from(&f.directory.path().join("developer-mode.json"))
            .unwrap()
            .enabled
    );
}

#[gpui_kit::test]
fn developer_submitted_text_is_frozen_until_definite_failure(cx: &mut TestAppContext) {
    let select_all = if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    };
    for outcome in ["failed", "handed_off", "unknown"] {
        let f = fixture(cx);
        enable(&f, cx);
        let mut stream = pair(&f, cx);
        pick_open(&f, cx);
        cx.update_window(f.window, |_, w, cx| {
            w.click("developer-feedback-input", cx);
            w.input("Request A", cx);
            w.click("developer-send", cx);
            w.click("developer-feedback-input", cx);
            w.press(select_all, cx);
            w.input("Request B", cx);
            w.press("backspace", cx);
        })
        .unwrap();
        let feedback = read_frame(&mut stream);
        assert_eq!(feedback["feedback"]["text"], "Request A");
        assert_eq!(
            f.view
                .read_with(cx, |v, cx| v.developer.draft_text(cx))
                .as_deref(),
            Some("Request A")
        );
        if outcome == "unknown" {
            drop(stream);
        } else {
            write_frame(
                &mut stream,
                serde_json::json!({"v":1,"id":feedback["id"],"type":"ack","feedback_id":feedback["id"],"outcome":outcome}),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            pump(&f, cx);
            if cx
                .update_window(f.window, |_, w, _| {
                    w.try_find("developer-feedback-status").is_some()
                })
                .unwrap()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "delivery status did not reach editor"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        cx.update_window(f.window, |_, w, cx| {
            w.click("developer-feedback-input", cx);
            w.press(select_all, cx);
            w.input("Request B", cx);
        })
        .unwrap();
        assert_eq!(
            f.view
                .read_with(cx, |v, cx| v.developer.draft_text(cx))
                .as_deref(),
            Some(if outcome == "failed" {
                "Request B"
            } else {
                "Request A"
            })
        );
    }
}

#[gpui_kit::test]
fn developer_picker_freezes_safe_target_without_original_action(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    pick_open(&f, cx);
    f.view.read_with(cx, |v, _| {
        let target = v
            .developer
            .selected_target()
            .expect("picker must select Open");
        assert_eq!(target.id, "open");
        assert_eq!(target.target_kind, TargetKind::Element);
        assert!(target.window_id.starts_with("reader:"));
        assert!(target.bounds.width > 0.);
        assert!(v.reader.book.is_none());
    });
    cx.update_window(f.window, |_, w, cx| {
        assert!(w.try_find("developer-feedback-input").is_some());
        w.press("escape", cx);
    })
    .unwrap();
    pump(&f, cx);
    f.view.read_with(cx, |v, _| assert!(!v.developer.busy()));
}
#[gpui_kit::test]
fn developer_preferences_selects_without_changing_preference(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    let pref = open_preferences(&f, cx);
    cx.update_window(pref, |_, w, cx| {
        w.scroll(
            "preferences-content",
            ScrollDelta::Pixels(point(px(0.), px(-3000.))),
            cx,
        )
    })
    .unwrap();
    pump(&f, cx);
    click_preference(&f, "developer-select", cx);
    cx.update_window(pref, |_, w, cx| {
        w.scroll(
            "preferences-content",
            ScrollDelta::Pixels(point(px(0.), px(3000.))),
            cx,
        )
    })
    .unwrap();
    pump(&f, cx);
    click_preference(&f, "reading-direction-ltr", cx);
    pump(&f, cx);
    f.view.read_with(cx, |v, _| {
        assert!(v.reader.settings.preferences.rtl);
        let target = v.developer.selected_target().unwrap();
        assert_eq!(target.id, "reading-direction-ltr");
        assert!(target.window_id.starts_with("preferences:"));
    });
}
#[gpui_kit::test]
fn developer_bridge_feedback_handoff_is_not_reported_as_completion(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    let mut stream = pair(&f, cx);
    pick_open(&f, cx);
    cx.update_window(f.window, |_, w, cx| {
        w.click("developer-feedback-input", cx);
        w.input("Make this easier to find", cx);
        w.click("developer-send", cx);
    })
    .unwrap();
    let feedback = read_frame(&mut stream);
    assert_eq!(feedback["type"], "feedback");
    assert_eq!(feedback["feedback"]["session_id"], "test-session");
    assert_eq!(feedback["feedback"]["target"]["id"], "open");
    assert_eq!(feedback["feedback"]["text"], "Make this easier to find");
    write_frame(
        &mut stream,
        serde_json::json!({"v":1,"id":feedback["id"],"type":"ack","feedback_id":feedback["id"],"outcome":"handed_off"}),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        pump(&f, cx);
        let visible = cx
            .update_window(f.window, |_, w, _| {
                w.try_find("developer-feedback-status").is_some()
            })
            .unwrap();
        if visible {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "acknowledgment must reach the dialog"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    cx.update_window(f.window, |_, w, _| {
        let node = w.find("developer-feedback-status");
        assert!(node.visible());
    })
    .unwrap();
}
#[gpui_kit::test]
fn developer_unpaired_send_keeps_draft_and_enter_adds_line(cx: &mut TestAppContext) {
    let f = fixture(cx);
    enable(&f, cx);
    pick_open(&f, cx);
    cx.update_window(f.window, |_, w, cx| {
        w.click("developer-feedback-input", cx);
        w.input("First", cx);
        w.press("enter", cx);
        w.input("Second", cx);
        w.click("developer-send", cx);
    })
    .unwrap();
    pump(&f, cx);
    f.view.read_with(cx, |v, cx| {
        assert!(v.developer.draft_text(cx).unwrap().contains('\n'))
    });
    cx.update_window(f.window, |_, w, _| {
        assert!(w.find("developer-feedback-status").visible())
    })
    .unwrap();
    f.view
        .read_with(cx, |v, _| assert!(v.developer.selected_target().is_some()));
}
