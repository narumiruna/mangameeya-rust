//! Non-modal release notices. Downloads and installation remain manual.

use super::*;
use crate::release_check::{self, NotificationPreferences, Release};

#[derive(Clone)]
pub(super) enum CheckStatus {
    Idle,
    Checking,
    Current,
    Available(Release),
    Failed(String),
}

#[derive(Clone)]
pub(super) struct ReleaseNotifications {
    pub preferences: NotificationPreferences,
    pub preference_error: Option<String>,
    pub status: CheckStatus,
    pub dismissed: bool,
    pub preferences_scroll: gpui_kit::ScrollHandle,
    // Keep network access separate from UI work and deterministic UI tests.
    pub check: fn() -> Result<Option<Release>, String>,
}

impl ReleaseNotifications {
    pub fn new(settings_path: &Path) -> Self {
        let (preferences, preference_error) = match NotificationPreferences::load_from(
            &settings_path.with_file_name("release-notifications.json"),
        ) {
            Ok(preferences) => (preferences, None),
            Err(error) => (
                NotificationPreferences {
                    check_on_startup: false,
                },
                Some(error),
            ),
        };
        Self {
            preferences,
            preference_error,
            status: CheckStatus::Idle,
            dismissed: false,
            preferences_scroll: Default::default(),
            check: release_check::check_latest,
        }
    }

    pub fn status_text(&self, language: Language) -> String {
        match &self.status {
            CheckStatus::Idle => language.text(Text::ReleaseCheckNotRun).to_owned(),
            CheckStatus::Checking => language.text(Text::CheckingForUpdates).to_owned(),
            CheckStatus::Current => language.text(Text::NoNewRelease).to_owned(),
            CheckStatus::Available(release) => language.format(
                Text::NewReleaseAvailable,
                &[("version", release.version.clone())],
            ),
            CheckStatus::Failed(error) => {
                language.format(Text::ReleaseCheckFailed, &[("error", error.clone())])
            }
        }
    }

    pub fn available(&self) -> bool {
        matches!(self.status, CheckStatus::Available(_))
    }

    pub fn checking(&self) -> bool {
        matches!(self.status, CheckStatus::Checking)
    }
}

impl ReaderView {
    pub(super) fn check_for_updates_on_startup(&mut self, cx: &mut Context<Self>) {
        if self.release_notifications.preferences.check_on_startup {
            self.check_for_updates(cx);
        }
    }

    pub(super) fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        if self.closing || self.release_notifications.checking() {
            return;
        }
        self.release_notifications.status = CheckStatus::Checking;
        cx.notify();
        let check = self.release_notifications.check;
        let task = cx.background_spawn(async move { check() });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            _ = view.update(cx, |view, cx| view.release_check_finished(result, cx));
        })
        .detach();
    }

    pub(super) fn release_check_finished(
        &mut self,
        result: Result<Option<Release>, String>,
        cx: &mut Context<Self>,
    ) {
        if self.closing {
            return;
        }
        self.release_notifications.status = match result {
            Ok(Some(release)) => CheckStatus::Available(release),
            Ok(None) => CheckStatus::Current,
            Err(error) => CheckStatus::Failed(error),
        };
        self.release_notifications.dismissed = false;
        cx.notify();
    }

    pub(super) fn set_release_startup_check(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let preferences = NotificationPreferences {
            check_on_startup: enabled,
        };
        match preferences.save_to(
            &self
                .settings_path
                .with_file_name("release-notifications.json"),
        ) {
            Ok(()) => {
                self.release_notifications.preferences = preferences;
                self.release_notifications.preference_error = None;
            }
            Err(error) => self.release_notifications.preference_error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn render_release_notice(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let state = &self.release_notifications;
        if !state.available() || state.dismissed {
            return div().into_any_element();
        }
        let language = self.language();
        let label = state.status_text(language);
        div()
            .id("release-notice")
            .test_support()
            .role(gpui_kit::Role::Status)
            .aria_label(label.clone())
            .flex()
            .flex_wrap()
            .flex_none()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().primary.opacity(0.1))
            .child(div().flex_1().min_w(px(180.)).child(label))
            .child(
                Button::new("release-notice-open")
                    .small()
                    .outline()
                    .label(language.text(Text::OpenReleases))
                    .accessibility_label(language.text(Text::OpenReleases))
                    .on_click(|_, _, cx| cx.open_url(release_check::RELEASES_URL)),
            )
            .child(
                Button::new("release-notice-dismiss")
                    .small()
                    .ghost()
                    .label(language.text(Text::DismissReleaseNotice))
                    .accessibility_label(language.text(Text::DismissReleaseNotice))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.release_notifications.dismissed = true;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}
