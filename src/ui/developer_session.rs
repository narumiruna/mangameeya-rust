use super::{
    developer::{Metadata, Picker, Selected},
    *,
};
use crate::developer as dev;
use gpui_kit::component::input::{Textarea, TextareaState};

pub(super) struct DeveloperUi {
    reader_window: AnyWindowHandle,
    pub enabled: bool,
    pub preference_error: bool,
    pub preference_durability_warning: bool,
    pub service: Option<dev::DeveloperService>,
    pub reader_picker: Option<Picker>,
    pub preferences_picker: Option<Picker>,
    draft: Option<Rc<RefCell<Draft>>>,
    pub status: Option<Text>,
}
#[derive(Clone, Debug)]
pub(super) struct FrozenContext {
    pub(super) identity: u64,
    pub(super) state: dev::FeedbackState,
    pub(super) destination: dev::ServiceStatus,
}
struct Draft {
    destination_generation: u64,
    feedback: dev::Feedback,
    input: Entity<TextareaState>,
    window: AnyWindowHandle,
    message: Option<Text>,
    pending: bool,
    unknown: bool,
}
impl DeveloperUi {
    pub fn new(path: &Path, reader_window: AnyWindowHandle) -> Self {
        let loaded =
            dev::DeveloperPreferences::load_from(&path.with_file_name("developer-mode.json"));
        let mut this = Self {
            reader_window,
            enabled: loaded.as_ref().is_ok_and(|p| p.enabled),
            preference_error: loaded.is_err(),
            preference_durability_warning: false,
            service: None,
            reader_picker: None,
            preferences_picker: None,
            draft: None,
            status: None,
        };
        if this.enabled {
            this.start();
        }
        this
    }
    fn start(&mut self) {
        match dev::DeveloperService::start() {
            Ok(service) => {
                self.service = Some(service);
                self.reader_picker = Some(Picker::new("reader"));
                self.preferences_picker = Some(Picker::new("preferences"));
                self.status = None;
            }
            Err(_) => self.status = Some(Text::DeveloperFailed),
        }
    }
    #[cfg(test)]
    pub(super) fn selected_target(&self) -> Option<dev::FeedbackTarget> {
        self.draft
            .as_ref()
            .map(|d| d.borrow().feedback.target.clone())
    }
    #[cfg(test)]
    pub(super) fn draft_text(&self, cx: &App) -> Option<String> {
        self.draft
            .as_ref()
            .map(|d| d.borrow().input.read(cx).value().to_string())
    }
    pub fn busy(&self) -> bool {
        self.draft.is_some()
            || self.reader_picker.as_ref().is_some_and(Picker::active)
            || self.preferences_picker.as_ref().is_some_and(Picker::active)
    }
    pub fn state_text(&self) -> Text {
        if !self.enabled {
            return Text::DeveloperDisabled;
        }
        match self.service.as_ref().map(|s| s.status().state) {
            Some(dev::ConnectionState::Connected) => Text::DeveloperConnected,
            Some(dev::ConnectionState::WaitingForPi) => Text::DeveloperWaiting,
            _ => Text::DeveloperFailed,
        }
    }
    pub fn marker(
        &self,
        id: impl Into<SharedString>,
        label: &'static str,
        source: &'static str,
    ) -> Option<gpui_kit::AnyElement> {
        self.reader_picker
            .as_ref()
            .map(|p| p.mark(metadata(id, label, source, dev::TargetKind::Element)))
    }
}
pub(super) fn metadata(
    id: impl Into<SharedString>,
    label: &'static str,
    source: &'static str,
    kind: dev::TargetKind,
) -> Metadata {
    Metadata {
        id: id.into(),
        role: if kind == dev::TargetKind::Region {
            "region"
        } else {
            "control"
        },
        label,
        source,
        target_kind: kind,
        layer: 0,
    }
}
pub(super) fn frame(
    picker: &Picker,
    reader: gpui_kit::WeakEntity<ReaderView>,
    frozen: Option<FrozenContext>,
) -> gpui_kit::AnyElement {
    picker.frame(frozen, move |selected, window, cx| {
        if let Some(frozen) = selected.context.clone() {
            show_feedback(reader.clone(), selected, frozen, window, cx);
        }
    })
}
pub(super) fn begin(
    reader: gpui_kit::WeakEntity<ReaderView>,
    picker: Picker,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(view) = reader.upgrade() else {
        return;
    };
    if view.read(cx).developer.busy() {
        return;
    }
    picker.start(window, cx);
    view.update(cx, |view, cx| {
        view.developer.status = Some(Text::DeveloperPickHint);
        cx.notify();
    });
}
pub(super) fn confirm_preferences_close(
    reader: gpui_kit::WeakEntity<ReaderView>,
    reader_window: AnyWindowHandle,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some(entity) = reader.upgrade() else {
        return false;
    };
    let handle = window.window_handle();
    let picker = entity.read(cx).developer.preferences_picker.clone();
    if let Some(picker) = picker.filter(Picker::active) {
        picker.cancel(window, cx);
    }
    let view = entity.read(cx);
    if !view
        .developer
        .draft
        .as_ref()
        .is_some_and(|d| d.borrow().window == handle)
    {
        return false;
    }
    let language = view.language();
    window.open_alert_dialog(cx, move |dialog, _, _| {
        let reader = reader.clone();
        dialog
            .confirm()
            .title(language.text(Text::DeveloperDiscard))
            .child(language.text(Text::DeveloperDiscardDescription))
            .on_ok(move |_, window, cx| {
                if let Some(entity) = reader.upgrade() {
                    entity.update(cx, |v, cx| {
                        v.developer.draft = None;
                        cx.notify();
                    });
                }
                super::preferences::close(reader.clone(), reader_window, window, cx);
                true
            })
    });
    true
}
fn show_feedback(
    reader: gpui_kit::WeakEntity<ReaderView>,
    selected: Selected,
    frozen: FrozenContext,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(entity) = reader.upgrade() else {
        return;
    };
    let view = entity.read(cx);
    if !view.developer.enabled || view.closing {
        return;
    }
    let language = view.language();
    let Some(current) = view.developer_context(selected.window == "preferences") else {
        return;
    };
    if current.identity != frozen.identity || current.state != frozen.state {
        return;
    }
    let destination = frozen.destination;
    let destination_generation = destination.generation;
    let feedback = dev::Feedback {
        feedback_id: match dev::Feedback::new_id() {
            Ok(id) => id,
            Err(_) => return,
        },
        capture_id: destination.capture_id,
        session_id: destination.session_id.unwrap_or_default(),
        captured_at_ms: selected.captured_at_ms,
        target: selected.feedback_target(),
        state: frozen.state,
        text: String::new(),
    };
    let input = cx.new(|cx| {
        TextareaState::new(window, cx)
            .placeholder(language.text(Text::DeveloperRequest))
            .rows(4)
    });
    let draft = Rc::new(RefCell::new(Draft {
        destination_generation,
        feedback,
        input: input.clone(),
        window: window.window_handle(),
        message: None,
        pending: false,
        unknown: false,
    }));
    entity.update(cx, |view, cx| {
        view.developer.draft = Some(draft.clone());
        view.developer.status = None;
        cx.notify();
    });
    open_feedback(reader, draft, language, window, cx);
    let handle = window.window_handle();
    cx.defer(move |cx| {
        _ = handle.update(cx, |_, window, cx| {
            input.update(cx, |input, cx| input.focus(window, cx))
        });
    });
}
fn open_feedback(
    reader: gpui_kit::WeakEntity<ReaderView>,
    draft: Rc<RefCell<Draft>>,
    language: Language,
    window: &mut Window,
    cx: &mut App,
) {
    let closing_reader = reader.clone();
    window.open_dialog(cx, move |dialog, window, _cx| {
        let d = draft.borrow();
        let preview = serde_json::to_string_pretty(&(&d.feedback.target, &d.feedback.state))
            .unwrap_or_default();
        let destination = language.format(
            Text::DeveloperDestination,
            &[("session", d.feedback.session_id.clone())],
        );
        let send_reader = reader.clone();
        let send_draft = draft.clone();
        let key_reader = reader.clone();
        let key_draft = draft.clone();
        let input = d.input.clone();
        let pending = d.pending || d.unknown;
        let content = div()
            .id("developer-feedback-content")
            .test_support()
            .flex()
            .flex_col()
            .gap_2()
            .max_h((window.viewport_size().height - px(180.)).max(px(100.)))
            .overflow_y_scroll()
            .child(div().text_xs().child(destination))
            .child(
                div()
                    .id("developer-context-preview")
                    .test_support()
                    .text_xs()
                    .max_h((window.viewport_size().height / 8.).clamp(px(32.), px(96.)))
                    .overflow_y_scroll()
                    .child(preview),
            )
            .child(
                div().id("developer-feedback-input").test_support().child(
                    Textarea::new(&input)
                        .readonly(pending)
                        .accessibility_id("developer-feedback-editor")
                        .aria_label(language.text(Text::DeveloperRequest))
                        .h((window.viewport_size().height / 4.).clamp(px(48.), px(100.))),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .child(language.text(Text::DeveloperSendHint)),
            )
            .children(d.message.map(|m| {
                div()
                    .id("developer-feedback-status")
                    .test_support()
                    .text_xs()
                    .child(language.text(m))
            }))
            .capture_key_down(move |event, window, cx| {
                if event.keystroke.key == "enter"
                    && (event.keystroke.modifiers.control || event.keystroke.modifiers.platform)
                {
                    submit(key_reader.clone(), key_draft.clone(), language, window, cx);
                    cx.stop_propagation();
                }
            });
        let close_reader = closing_reader.clone();
        dialog
            .title(language.text(Text::DeveloperFeedback))
            .w(px(520.))
            .overlay_closable(false)
            .on_ok(|_, _, _| false)
            .on_close(move |_, _, cx| {
                if let Some(e) = close_reader.upgrade() {
                    e.update(cx, |v, cx| {
                        v.developer.draft = None;
                        cx.notify();
                    });
                }
            })
            .child(content)
            .footer(
                Button::new("developer-send")
                    .label(language.text(Text::DeveloperSend))
                    .disabled(pending)
                    .on_click(move |_, window, cx| {
                        submit(
                            send_reader.clone(),
                            send_draft.clone(),
                            language,
                            window,
                            cx,
                        )
                    }),
            )
    });
}
fn submit(
    reader: gpui_kit::WeakEntity<ReaderView>,
    draft: Rc<RefCell<Draft>>,
    language: Language,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(entity) = reader.upgrade() else {
        return;
    };
    if draft.borrow().pending || draft.borrow().unknown {
        return;
    }
    let text = draft.borrow().input.read(cx).value().to_string();
    if text.trim().is_empty() || text.len() > 8192 {
        draft.borrow_mut().message = Some(Text::DeveloperInputLimit);
        window.refresh();
        return;
    }
    let destination = entity
        .read(cx)
        .developer
        .service
        .as_ref()
        .map(|s| s.status());
    let Some(destination) = destination.filter(|s| s.session_id.is_some()) else {
        draft.borrow_mut().message = Some(Text::DeveloperSendFailed);
        window.refresh();
        return;
    };
    if draft.borrow().destination_generation != destination.generation
        || draft.borrow().feedback.session_id != destination.session_id.clone().unwrap_or_default()
        || draft.borrow().feedback.capture_id != destination.capture_id
    {
        let title = language.format(
            Text::DeveloperDestination,
            &[(
                "session",
                destination.session_id.clone().unwrap_or_default(),
            )],
        );
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let draft = draft.clone();
            let dest = destination.clone();
            dialog
                .confirm()
                .title(title.clone())
                .child(language.text(Text::DeveloperSendFailed))
                .on_ok(move |_, _, _| {
                    let mut d = draft.borrow_mut();
                    d.destination_generation = dest.generation;
                    d.feedback.session_id = dest.session_id.clone().unwrap_or_default();
                    d.feedback.capture_id = dest.capture_id.clone();
                    d.message = None;
                    true
                })
        });
        return;
    }
    let mut d = draft.borrow_mut();
    let Ok(feedback_id) = dev::Feedback::new_id() else {
        d.message = Some(Text::DeveloperSendFailed);
        window.refresh();
        return;
    };
    d.feedback.feedback_id = feedback_id;
    d.feedback.text = text;
    let result = entity
        .read(cx)
        .developer
        .service
        .as_ref()
        .unwrap()
        .try_send_feedback(d.feedback.clone());
    match result {
        Ok(()) => {
            d.pending = true;
            d.message = None;
            d.input.update(cx, |input, cx| input.set_readonly(true, cx));
        }
        Err(_) => d.message = Some(Text::DeveloperSendFailed),
    }
    window.refresh();
}
impl ReaderView {
    pub(super) fn developer_context(&self, preferences: bool) -> Option<FrozenContext> {
        use std::hash::{Hash, Hasher};
        let mut identity = std::collections::hash_map::DefaultHasher::new();
        (
            self.open_revision,
            self.book_revision,
            self.listing_revision,
            self.image_revision,
        )
            .hash(&mut identity);
        self.browser.model.order.hash(&mut identity);
        self.reader.settings.recent.hash(&mut identity);
        for row in self.files_ui.model.visible_rows() {
            row.path.hash(&mut identity);
        }
        Some(FrozenContext {
            identity: identity.finish(),
            destination: self.developer.service.as_ref()?.status(),
            state: dev::FeedbackState {
                ui_mode: if preferences {
                    dev::UiMode::Preferences
                } else if self.content_mode == ContentMode::Reading {
                    dev::UiMode::Reading
                } else {
                    dev::UiMode::Browsing
                },
                reading_mode: match self.reader.settings.preferences.mode {
                    ReadingMode::Single => dev::ReadingMode::Single,
                    ReadingMode::Spread => dev::ReadingMode::Spread,
                    ReadingMode::Continuous => dev::ReadingMode::Continuous,
                },
                page_index: self
                    .reader
                    .book
                    .as_ref()
                    .map(|_| self.reader.current as u64),
                fit_mode: match self.reader.settings.preferences.fit {
                    FitMode::Window => dev::FitMode::Window,
                    FitMode::Width => dev::FitMode::Width,
                    FitMode::Height => dev::FitMode::Height,
                    FitMode::Original => dev::FitMode::Original,
                    FitMode::Custom => dev::FitMode::Custom,
                },
                app_version: env!("CARGO_PKG_VERSION").into(),
            },
        })
    }
    pub(super) fn confirm_developer_quit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.developer.draft.is_none() {
            return false;
        }
        let reader = cx.entity().downgrade();
        let language = self.language();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let reader = reader.clone();
            dialog
                .confirm()
                .title(language.text(Text::DeveloperDiscard))
                .child(language.text(Text::DeveloperDiscardDescription))
                .on_ok(move |_, window, cx| {
                    if let Some(entity) = reader.upgrade() {
                        entity.update(cx, |view, cx| {
                            view.developer.draft = None;
                            view.developer.service = None;
                            view.closing = true;
                            view.close_preferences(cx);
                            if let Err(error) = view.flush_settings() {
                                eprintln!("{error}");
                            }
                        });
                    }
                    window.remove_window();
                    cx.quit();
                    true
                })
        });
        true
    }
    pub(super) fn set_developer_mode(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !enabled && self.developer.draft.is_some() {
            let weak = cx.entity().downgrade();
            let language = self.language();
            window.open_alert_dialog(cx, move |dialog, _, _| {
                let weak = weak.clone();
                dialog
                    .confirm()
                    .title(language.text(Text::DeveloperDiscard))
                    .child(language.text(Text::DeveloperDiscardDescription))
                    .on_ok(move |_, _, cx| {
                        if let Some(e) = weak.upgrade() {
                            e.update(cx, |v, cx| {
                                v.apply_developer_enabled(false, cx);
                            });
                        }
                        true
                    })
            });
        } else {
            self.apply_developer_enabled(enabled, cx);
        }
    }
    pub(super) fn apply_developer_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled == self.developer.enabled
            && !self.developer.preference_error
            && !self.developer.preference_durability_warning
        {
            return;
        }
        if !enabled {
            let reader_window = self.developer.reader_window;
            let preferences_window = self.preferences_window;
            let reader_picker = self.developer.reader_picker.clone();
            let preferences_picker = self.developer.preferences_picker.clone();
            cx.defer(move |cx| {
                if let Some(picker) = reader_picker {
                    _ = reader_window.update(cx, |_, window, cx| picker.cancel(window, cx));
                }
                if let (Some(handle), Some(picker)) = (preferences_window, preferences_picker) {
                    _ = handle.update(cx, |_, window, cx| picker.cancel(window, cx));
                }
            });
            self.developer.service = None;
            self.developer.reader_picker = None;
            self.developer.preferences_picker = None;
            if let Some(draft) = self.developer.draft.take() {
                let handle = draft.borrow().window;
                cx.defer(move |cx| {
                    _ = handle.update(cx, |_, window, cx| window.close_dialog(cx));
                });
            }
            self.developer.enabled = false;
            self.developer.status = None;
        }
        let saved = (dev::DeveloperPreferences { enabled })
            .save_to(&self.settings_path.with_file_name("developer-mode.json"));
        self.developer_preference_saved(enabled, saved, cx);
    }
    pub(super) fn developer_preference_saved(
        &mut self,
        enabled: bool,
        saved: Result<dev::PreferenceSave, String>,
        cx: &mut Context<Self>,
    ) {
        match saved {
            Ok(outcome) => {
                self.developer.preference_error = false;
                self.developer.preference_durability_warning =
                    outcome == dev::PreferenceSave::DurabilityUnconfirmed;
                if enabled && !self.developer.enabled {
                    self.developer.enabled = true;
                    self.developer.start();
                }
            }
            Err(_) => {
                self.developer.preference_error = true;
                self.developer.preference_durability_warning = false;
            }
        }
        cx.notify();
    }
    pub(super) fn developer_copy(&mut self, cx: &mut Context<Self>) {
        if let Some(service) = &self.developer.service {
            match service.connection_command() {
                Ok(command) => {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(command));
                    self.developer.status = Some(Text::DeveloperCopied);
                }
                Err(_) => self.developer.status = Some(Text::DeveloperFailed),
            }
        }
        cx.notify();
    }
    pub(super) fn developer_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.developer.busy() {
            self.next_slide =
                Instant::now() + Duration::from_secs(self.reader.settings.preferences.interval);
        }
        let Some(service) = &self.developer.service else {
            return;
        };
        service.set_ui_mode(if self.content_mode == ContentMode::Reading {
            dev::UiMode::Reading
        } else {
            dev::UiMode::Browsing
        });
        let mut events = Vec::new();
        while let Some(e) = service.try_recv_event() {
            events.push(e);
        }
        for event in events {
            match &event {
                dev::ServiceEvent::Connected | dev::ServiceEvent::Disconnected => {}
                dev::ServiceEvent::FeedbackHandedOff(id)
                | dev::ServiceEvent::FeedbackFailed(id)
                | dev::ServiceEvent::DeliveryUnknown(id) => {
                    if let Some(draft) = &self.developer.draft {
                        let mut d = draft.borrow_mut();
                        if &d.feedback.feedback_id == id {
                            d.pending = false;
                            let message = match event {
                                dev::ServiceEvent::FeedbackHandedOff(_) => Text::DeveloperHandedOff,
                                dev::ServiceEvent::DeliveryUnknown(_) => {
                                    d.unknown = true;
                                    Text::DeveloperDeliveryUnknown
                                }
                                _ => Text::DeveloperSendFailed,
                            };
                            d.message = Some(message);
                            if message == Text::DeveloperHandedOff {
                                d.unknown = true;
                            }
                            let readonly = d.unknown;
                            d.input
                                .update(cx, |input, cx| input.set_readonly(readonly, cx));
                            _ = d.window.update(cx, |_, w, _| w.refresh());
                        }
                    }
                }
            }
            cx.notify();
        }
        let _ = window;
    }
    pub(super) fn developer_log(&self, target: dev::Target, code: dev::EventCode, success: bool) {
        self.developer_log_duration(target, code, success, 0);
    }
    pub(super) fn developer_log_elapsed(
        &self,
        target: dev::Target,
        code: dev::EventCode,
        success: bool,
        started: Instant,
    ) {
        self.developer_log_duration(
            target,
            code,
            success,
            started.elapsed().as_millis().min(9_007_199_254_740_991) as u64,
        );
    }
    fn developer_log_duration(
        &self,
        target: dev::Target,
        code: dev::EventCode,
        success: bool,
        duration_ms: u64,
    ) {
        if let Some(service) = &self.developer.service {
            service.logs().record(dev::DiagnosticEvent {
                level: if success {
                    dev::Level::Info
                } else {
                    dev::Level::Error
                },
                target,
                code,
                operation_id: self.open_revision,
                duration_ms,
                fields: vec![
                    (dev::Field::Success, dev::Scalar::Bool(success)),
                    (
                        dev::Field::PageIndex,
                        dev::Scalar::Number(self.reader.current as u64),
                    ),
                    (
                        dev::Field::Count,
                        dev::Scalar::Number(self.page_count() as u64),
                    ),
                ],
            });
        }
    }
}
