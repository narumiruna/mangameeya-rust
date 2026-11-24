//! Native production picker API gate. No test-support API is used.
use gpui_kit::*;
use std::{cell::RefCell, rc::Rc, time::Duration};

#[derive(Clone, Debug)]
struct Target {
    id: &'static str,
    role: &'static str,
    label: &'static str,
    source: &'static str,
    window: &'static str,
    revision: u64,
    layer: u32,
    hitbox: Hitbox,
}
#[derive(Default)]
struct State {
    revision: u64,
    targets: Vec<Target>,
    picking: bool,
    selected: Vec<&'static str>,
    actions: usize,
    intercepted: usize,
    overlay: bool,
}
struct Probe {
    name: &'static str,
    state: Rc<RefCell<State>>,
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
}
fn register(
    s: &mut State,
    window: &mut Window,
    name: &'static str,
    id: &'static str,
    bounds: Bounds<Pixels>,
    layer: u32,
) {
    let hitbox = window.insert_hitbox(
        bounds,
        if id == "region" {
            HitboxBehavior::BlockMouse
        } else {
            HitboxBehavior::Normal
        },
    );
    s.targets.push(Target {
        id,
        role: if id == "region" { "region" } else { "button" },
        label: "Safe probe control",
        source: "examples/developer_probe.rs::Probe::render",
        window: name,
        revision: s.revision,
        layer,
        hitbox,
    });
}
fn control(
    state: Rc<RefCell<State>>,
    name: &'static str,
    id: &'static str,
    width: f32,
    height: f32,
    layer: u32,
) -> impl IntoElement {
    let down = state.clone();
    let up = state.clone();
    let click = state.clone();
    div()
        .id(id)
        .w(px(width))
        .h(px(height))
        .occlude()
        .bg(rgb(0x335577))
        .on_mouse_down(MouseButton::Left, move |_, _, _| {
            down.borrow_mut().actions += 1
        })
        .on_mouse_up(MouseButton::Left, move |_, _, _| {
            up.borrow_mut().actions += 1
        })
        .on_click(move |_, _, _| click.borrow_mut().actions += 1)
        .child(
            canvas(
                move |bounds, window, _| {
                    register(&mut state.borrow_mut(), window, name, id, bounds, layer)
                },
                |_, (), _, _| {},
            )
            .size_full(),
        )
}
impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let pre = self.state.clone();
        let paint = self.state.clone();
        let name = self.name;
        let overlay = self.state.borrow().overlay;
        let controls = self.state.clone();
        div()
            .size_full()
            .relative()
            .child(
                canvas(
                    move |_, window, _| {
                        let mut s = pre.borrow_mut();
                        s.revision += 1;
                        s.targets.clear();
                        register(&mut s, window, name, "region", rect(0., 0., 400., 300.), 0);
                    },
                    move |_, (), window, _| {
                        // Root capture listener must precede every normal pointer listener.
                        let down = paint.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                            if phase == DispatchPhase::Capture && down.borrow().picking {
                                let mut s = down.borrow_mut();
                                let matches: Vec<_> = s
                                    .targets
                                    .iter()
                                    .filter(|t| t.hitbox.is_hovered_at(event.position, window))
                                    .collect();
                                assert!(matches.len() <= 1, "ambiguous ownership must not select");
                                let id = matches.first().map(|t| t.id);
                                if let Some(id) = id {
                                    s.selected.push(id);
                                }
                                s.intercepted += 1;
                                cx.stop_propagation();
                            }
                        });
                        let up = paint.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture && up.borrow().picking {
                                up.borrow_mut().intercepted += 1;
                                cx.stop_propagation();
                            }
                        });
                        let motion = paint.clone();
                        window.on_mouse_event(move |_: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture && motion.borrow().picking {
                                motion.borrow_mut().intercepted += 1;
                                cx.stop_propagation();
                            }
                        });
                        for target in &paint.borrow().targets {
                            let clipped = target
                                .hitbox
                                .bounds
                                .intersect(&target.hitbox.content_mask.bounds);
                            window.paint_quad(fill(
                                clipped,
                                rgb(if target.id == "popover" {
                                    0x993355
                                } else {
                                    0x335577
                                }),
                            ));
                        }
                        // Normal control handlers, including down/up and held-button drag initiation.
                        let action = paint.clone();
                        window.on_mouse_event(move |_: &MouseDownEvent, _, _, _| {
                            action.borrow_mut().actions += 1
                        });
                        let action = paint.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, _, _, _| {
                            action.borrow_mut().actions += 1
                        });
                        window.on_mouse_event(move |e: &MouseMoveEvent, _, _, _| {
                            if e.pressed_button.is_some() {
                                paint.borrow_mut().actions += 1;
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(div().absolute().left(px(20.)).top(px(20.)).child(control(
                controls.clone(),
                name,
                "base",
                180.,
                140.,
                1,
            )))
            .child(
                div()
                    .absolute()
                    .left(px(220.))
                    .top(px(20.))
                    .w(px(50.))
                    .h(px(60.))
                    .overflow_hidden()
                    .child(control(controls.clone(), name, "clipped", 130., 100., 2)),
            )
            .children(overlay.then(|| {
                deferred(
                    div()
                        .absolute()
                        .left(px(80.))
                        .top(px(60.))
                        .child(control(controls, name, "popover", 120., 80., 3)),
                )
                .priority(10)
            }))
            .children(overlay.then(|| {
                deferred(
                    div()
                        .absolute()
                        .left(px(160.))
                        .top(px(100.))
                        .w(px(25.))
                        .h(px(25.))
                        .occlude()
                        .bg(rgb(0x995533)),
                )
                .priority(20)
            }))
    }
}
fn sequence(window: &mut Window, cx: &mut App, x: f32, y: f32, count: usize) {
    let position = point(px(x), px(y));
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            ..Default::default()
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            position,
            click_count: count,
            ..Default::default()
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position: position + point(px(12.), px(8.)),
            pressed_button: Some(MouseButton::Left),
            ..Default::default()
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            position,
            click_count: count,
            ..Default::default()
        }),
        cx,
    );
}
fn verify(handle: AnyWindowHandle, state: Rc<RefCell<State>>, cx: &mut App, log: &mut String) {
    handle.update(cx, |_, window, cx| {
        let _ = window.draw(cx);
        for (x, y, expected) in [(30.,30.,"base"), (100.,80.,"popover"), (230.,30.,"clipped"), (300.,30.,"region")] {
            let before = state.borrow().selected.len();
            sequence(window, cx, x, y, 1);
            sequence(window, cx, x, y, 2);
            let s = state.borrow();
            assert_eq!(&s.selected[before..], &[expected, expected]);
            assert_eq!(s.actions, 0, "normal action or drag ran during picking");
        }
        let selected_before = state.borrow().selected.len();
        sequence(window,cx,170.,110.,1);
        assert_eq!(state.borrow().selected.len(), selected_before, "unregistered occluder must not select underlying owner");
        assert_eq!(state.borrow().actions, 0);
        let s = state.borrow();
        for t in &s.targets {
            log.push_str(&format!("target={} role={} label={} source={} window={} revision={} layer={} bounds={:?} clipped={:?} scale={}\n", t.id,t.role,t.label,t.source,t.window,t.revision,t.layer,t.hitbox.bounds,t.hitbox.bounds.intersect(&t.hitbox.content_mask.bounds),window.scale_factor()));
        }
        drop(s);
        let old_revision = state.borrow().revision;
        state.borrow_mut().overlay = false;
        window.refresh();
        let _ = window.draw(cx);
        assert!(state.borrow().revision > old_revision);
        assert!(!state.borrow().targets.iter().any(|t| t.id == "popover"));
        sequence(window, cx, 100.,80.,1);
        assert_eq!(state.borrow().selected.last(), Some(&"base"));
        assert_eq!(state.borrow().actions, 0);
        state.borrow_mut().picking = false;
        sequence(window, cx, 30.,30.,1);
        assert!(state.borrow().actions > 0, "positive control did not run");
        log.push_str(&format!("PASS window={:?} intercepted={} positive_actions={} stale_overlay_removed=true unknown_occluder_rejected=true\n", window.window_handle(), state.borrow().intercepted, state.borrow().actions));
    }).unwrap();
}
fn main() {
    std::fs::create_dir_all("target").unwrap();
    gpui_kit::application().run(|cx| {
        let mut probes = Vec::new();
        for name in ["main", "preferences"] {
            let state = Rc::new(RefCell::new(State { picking: true, overlay: true, ..Default::default() }));
            let view_state = state.clone();
            let handle = gpui_kit::open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(rect(if name == "main" { 50. } else { 500. }, 100., 400., 300.))), titlebar: Some(TitlebarOptions { title: Some(format!("Developer probe {name}").into()), ..Default::default() }), ..Default::default() }, cx, move |_, cx| cx.new(|_| Probe { name, state: view_state })).unwrap();
            probes.push((handle.0, state));
        }
        cx.activate(true);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            cx.update(|cx| {
                let mut log = String::from("Native release GPUI Kit 0.9.0 production pointer gate\n");
                for (handle,state) in probes { verify(handle,state,cx,&mut log); }
                log.push_str("PASS all assertions; no test-support, inspector, or simulated test window APIs called\n");
                std::fs::write("target/developer-probe-runtime.log",log).unwrap();
                cx.quit();
            });
        }).detach();
    });
}
