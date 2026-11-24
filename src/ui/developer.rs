//! Opt-in native picker primitives. Each window must own a separate `Picker`.
//!
//! Paint `frame` before all other listeners. Put `mark` inside the actual occluding
//! owner, not around it. Instrumented subtrees must not reuse cached paint output.
use gpui_kit::*;
use std::{cell::RefCell, rc::Rc};

const MAX_TARGETS: usize = 512;

pub use crate::developer::TargetKind;

/// Only reviewed static labels belong here, never document names or input text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub id: SharedString,
    pub role: &'static str,
    pub label: &'static str,
    pub source: &'static str,
    pub target_kind: TargetKind,
    /// Application hint only. Native hit testing determines actual eligibility.
    pub layer: u32,
}

/// Frozen on mouse-down, delivered only after the selecting release is consumed.
#[derive(Clone, Debug)]
pub struct Selected {
    pub metadata: Metadata,
    pub bounds: Bounds<Pixels>,
    pub click: Point<Pixels>,
    pub scale: f32,
    pub revision: u64,
    pub captured_at_ms: u64,
    pub(super) context: Option<super::developer_session::FrozenContext>,
    pub window: &'static str,
    pub window_id: WindowId,
}

impl Selected {
    pub fn feedback_target(&self) -> crate::developer::FeedbackTarget {
        crate::developer::FeedbackTarget {
            id: self.metadata.id.to_string(),
            role: self.metadata.role.into(),
            label: self.metadata.label.into(),
            source: self.metadata.source.into(),
            window_id: format!("{}:{}", self.window, self.window_id.as_u64()),
            view_revision: self.revision,
            target_kind: self.metadata.target_kind,
            bounds: crate::developer::Bounds {
                x: f32::from(self.bounds.origin.x).into(),
                y: f32::from(self.bounds.origin.y).into(),
                width: f32::from(self.bounds.size.width).into(),
                height: f32::from(self.bounds.size.height).into(),
            },
            click: crate::developer::Point {
                x: f32::from(self.click.x).into(),
                y: f32::from(self.click.y).into(),
            },
            scale_factor: self.scale.into(),
        }
    }
}

struct Target {
    metadata: Metadata,
    hitbox: Hitbox,
}

#[derive(Default)]
struct State {
    active: bool,
    captured_buttons: Vec<MouseButton>,
    release_button: Option<MouseButton>,
    suppress_repeat: bool,
    revision: u64,
    targets: Vec<Target>,
    overflow: bool,
    pending: Option<Selected>,
    hovered: Option<SharedString>,
    previous_focus: Option<FocusHandle>,
}

/// Store this behind `Option` in the view. `None` installs no picker elements.
#[derive(Clone)]
pub struct Picker {
    window: &'static str,
    state: Rc<RefCell<State>>,
}

impl Picker {
    pub fn new(window: &'static str) -> Self {
        Self {
            window,
            state: Rc::new(RefCell::new(State::default())),
        }
    }

    pub fn active(&self) -> bool {
        self.state.borrow().active
    }

    pub fn start(&self, window: &mut Window, cx: &mut App) {
        let mut state = self.state.borrow_mut();
        if !state.active {
            state.previous_focus = window.focused(cx);
        }
        state.active = true;
        state.pending = None;
        state.hovered = None;
        window.refresh();
    }

    /// Keeps an in-progress pointer latch until release, even after Escape.
    pub fn cancel(&self, window: &mut Window, cx: &mut App) {
        let focus = {
            let mut state = self.state.borrow_mut();
            state.active = false;
            state.pending = None;
            state.hovered = None;
            state.previous_focus.take()
        };
        if let Some(focus) = focus {
            window.focus(&focus, cx);
        }
        window.refresh();
    }

    /// First root child. Its paint callback must precede application listeners.
    /// The callback runs in this window after release dispatch has completed.
    pub fn frame(
        &self,
        context: Option<super::developer_session::FrozenContext>,
        selected: impl Fn(Selected, &mut Window, &mut App) + 'static,
    ) -> AnyElement {
        let pre = self.clone();
        let paint = self.clone();
        let selected = Rc::new(selected);
        canvas(
            move |_, _, _| {
                let mut state = pre.state.borrow_mut();
                state.revision = state.revision.wrapping_add(1);
                state.targets.clear();
                state.overflow = false;
            },
            move |_, (), window, _| {
                let down = paint.clone();
                let down_context = context.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let mut state = down.state.borrow_mut();
                    let repeat = state.suppress_repeat && event.click_count > 1;
                    if !state.active && state.captured_buttons.is_empty() && !repeat {
                        state.suppress_repeat = false;
                        return;
                    }
                    if state.active
                        && state.captured_buttons.is_empty()
                        && event.button == MouseButton::Left
                    {
                        state.pending =
                            resolve(&state, event.position, window).map(|target| Selected {
                                metadata: target.metadata.clone(),
                                bounds: target
                                    .hitbox
                                    .bounds
                                    .intersect(&target.hitbox.content_mask.bounds),
                                click: event.position,
                                scale: window.scale_factor(),
                                revision: state.revision,
                                captured_at_ms: crate::developer::now_ms(),
                                context: down_context.clone(),
                                window: down.window,
                                window_id: window.window_handle().window_id(),
                            });
                    }
                    if state.captured_buttons.is_empty() {
                        state.release_button = Some(event.button);
                    }
                    if !state.captured_buttons.contains(&event.button) {
                        state.captured_buttons.push(event.button);
                    }
                    cx.stop_propagation();
                });
                let up = paint.clone();
                let callback = selected.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let selection = {
                        let mut state = up.state.borrow_mut();
                        if !state.active && state.captured_buttons.is_empty() {
                            return;
                        }
                        cx.stop_propagation();
                        state
                            .captured_buttons
                            .retain(|button| *button != event.button);
                        if state
                            .release_button
                            .is_some_and(|button| button != event.button)
                        {
                            return;
                        }
                        state.release_button = None;
                        state.suppress_repeat = true;
                        let selection = state.pending.take().filter(|pending| {
                            let same_context = match (&pending.context, &context) {
                                (Some(old), Some(current)) => {
                                    old.identity == current.identity
                                        && old.state == current.state
                                        && old.destination.generation
                                            == current.destination.generation
                                }
                                (None, None) => true,
                                _ => false,
                            };
                            same_context
                                && resolve(&state, pending.click, window).is_some_and(|target| {
                                    target.metadata == pending.metadata
                                        && target
                                            .hitbox
                                            .bounds
                                            .intersect(&target.hitbox.content_mask.bounds)
                                            == pending.bounds
                                })
                                && window.scale_factor() == pending.scale
                        });
                        if selection.is_some() {
                            state.active = false;
                            state.hovered = None;
                        }
                        selection
                    };
                    window.refresh();
                    if let Some(selection) = selection {
                        let callback = callback.clone();
                        window.defer(cx, move |window, cx| callback(selection, window, cx));
                    }
                });
                let motion = paint.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let mut state = motion.state.borrow_mut();
                    if !state.active && state.captured_buttons.is_empty() {
                        return;
                    }
                    let hovered =
                        resolve(&state, event.position, window).map(|t| t.metadata.id.clone());
                    if state.hovered != hovered {
                        state.hovered = hovered;
                        window.refresh();
                    }
                    cx.stop_propagation();
                });
                let escape = paint.clone();
                window.on_key_event(move |event: &KeyDownEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture
                        && event.keystroke.key == "escape"
                        && escape.active()
                    {
                        escape.cancel(window, cx);
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .size_full()
        .into_any_element()
    }

    /// Absolute full-owner marker: does not take space in the owner's flex row.
    /// Use `relative().block_mouse_except_scroll()` on scrolling owners so
    /// ancestor scroll handlers still run. True overlays can use `occlude()`.
    /// Do not put a new occluding
    /// descendant over this marker. Unknown occluders intentionally select none.
    /// For regions with child controls, insert the marker before those children.
    pub fn mark(&self, metadata: Metadata) -> AnyElement {
        let pre = self.clone();
        let paint = self.clone();
        let paint_metadata = metadata.clone();
        canvas(
            move |bounds, window, _| {
                let mut state = pre.state.borrow_mut();
                if !state.active {
                    return;
                }
                if state.targets.len() >= MAX_TARGETS {
                    state.overflow = true;
                    return;
                }
                let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                state.targets.push(Target {
                    metadata: metadata.clone(),
                    hitbox,
                });
            },
            move |_, (), window, _| {
                let state = paint.state.borrow();
                if state.active
                    && !state.overflow
                    && state.hovered.as_ref() == Some(&paint_metadata.id)
                    && let Some(target) =
                        state.targets.iter().find(|t| t.metadata == paint_metadata)
                {
                    let bounds = target
                        .hitbox
                        .bounds
                        .intersect(&target.hitbox.content_mask.bounds);
                    window.paint_quad(fill(bounds, rgba(0x3388ff44)));
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }
}

/// Never infer ownership from rectangle size or a declared semantic layer.
fn resolve<'a>(state: &'a State, position: Point<Pixels>, window: &Window) -> Option<&'a Target> {
    if state.overflow {
        return None;
    }
    let mut matches = state
        .targets
        .iter()
        .filter(|t| t.hitbox.is_hovered_at(position, window));
    let target = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::{Metadata, Picker, Selected, TargetKind};
    use gpui_kit::{Bounds, WindowId, point, px, size};

    #[test]
    fn independent_windows_start_empty_and_inactive() {
        let reader = Picker::new("reader");
        let preferences = Picker::new("preferences");
        reader.state.borrow_mut().active = true;
        assert!(!preferences.active());
        assert!(preferences.state.borrow().targets.is_empty());
        assert_eq!(preferences.state.borrow().revision, 0);
    }

    #[test]
    fn feedback_snapshot_uses_only_reviewed_fields() {
        let selected = Selected {
            metadata: Metadata {
                id: "reader.next".into(),
                role: "button",
                label: "Next page",
                source: "src/ui.rs::command_button",
                target_kind: TargetKind::Element,
                layer: 2,
            },
            bounds: Bounds::new(point(px(10.), px(20.)), size(px(30.), px(40.))),
            click: point(px(15.), px(25.)),
            scale: 1.25,
            revision: 3,
            captured_at_ms: 1,
            context: None,
            window: "reader",
            window_id: WindowId::from(7),
        };
        let target = selected.feedback_target();
        assert_eq!(target.id, "reader.next");
        assert_eq!(
            target.window_id,
            format!("reader:{}", selected.window_id.as_u64())
        );
        assert_eq!(target.view_revision, 3);
        assert_eq!(target.target_kind, crate::developer::TargetKind::Element);
        assert_eq!((target.bounds.x, target.bounds.y), (10., 20.));
        assert_eq!((target.bounds.width, target.bounds.height), (30., 40.));
        assert_eq!((target.click.x, target.click.y), (15., 25.));
        assert_eq!(target.scale_factor, 1.25);
    }

    #[test]
    fn clones_share_the_window_state() {
        let picker = Picker::new("reader");
        picker.clone().state.borrow_mut().active = true;
        assert!(picker.active());
    }
}
