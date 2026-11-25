//! Independent Preferences window. The reader owns all settings and save operations.

use super::{Command, ReaderView, background_color, release_notifications::ReleaseNotifications};
use crate::{
    i18n::{Language, Text},
    settings::{Background, BrowserSortScroll},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, button::Button, checkbox::Checkbox,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, DisplayId, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, TestSupportExt, TitlebarOptions, WeakEntity,
    Window, WindowBounds, WindowKind, WindowOptions, div, prelude::FluentBuilder, px, size,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Change {
    Remember(bool),
    Direction(bool),
    Background(Background),
    BrowserSortScroll(BrowserSortScroll),
    Language(Language),
    ReleaseStartupCheck(bool),
    DeveloperMode(bool),
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, gpui_kit::Action)]
#[action(namespace = preferences, no_json)]
enum Action {
    Close,
}

struct BindingsInstalled;
impl gpui_kit::Global for BindingsInstalled {}

fn init(cx: &mut App) {
    if cx.has_global::<BindingsInstalled>() {
        return;
    }
    cx.set_global(BindingsInstalled);
    for key in ["escape", "ctrl-w"] {
        cx.bind_keys([KeyBinding::new(
            key,
            Action::Close,
            Some("Preferences && !Dialog"),
        )]);
    }
    #[cfg(target_os = "macos")]
    {
        cx.bind_keys([KeyBinding::new(
            "cmd-w",
            Action::Close,
            Some("Preferences && !Dialog"),
        )]);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Command::Quit, Some("Preferences")),
            KeyBinding::new("cmd-,", Command::Settings, Some("Preferences")),
        ]);
    }
    #[cfg(target_os = "windows")]
    cx.bind_keys([KeyBinding::new(
        "alt-f4",
        Action::Close,
        Some("Preferences && !Dialog"),
    )]);
}

pub(super) fn open(
    reader: WeakEntity<ReaderView>,
    reader_window: AnyWindowHandle,
    language: Language,
    display: Option<DisplayId>,
    cx: &mut App,
) -> anyhow::Result<AnyWindowHandle> {
    init(cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            display,
            size(px(560.), px(520.)),
            cx,
        ))),
        window_min_size: Some(size(px(440.), px(360.))),
        titlebar: Some(TitlebarOptions {
            title: Some(language.text(Text::Preferences).into()),
            appears_transparent: false,
            ..Default::default()
        }),
        kind: WindowKind::Normal,
        display_id: display,
        is_resizable: true,
        ..Default::default()
    };
    let (window, _) = gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| PreferencesView::new(reader, reader_window, window, cx))
    })?;
    Ok(window)
}

pub(super) fn close(
    reader: WeakEntity<ReaderView>,
    reader_window: AnyWindowHandle,
    window: &mut Window,
    cx: &mut App,
) {
    if super::developer_session::confirm_preferences_close(
        reader.clone(),
        reader_window,
        window,
        cx,
    ) {
        return;
    }
    if let Some(entity) = reader.upgrade() {
        entity.update(cx, |reader, cx| {
            if reader.developer.preferences_picker.is_some() {
                reader.developer.preferences_picker =
                    Some(super::developer::Picker::new("preferences"));
                cx.notify();
            }
        });
    }
    window.remove_window();
    cx.defer(move |cx| {
        _ = reader_window.update(cx, |_, window, cx| {
            window.activate_window();
            _ = reader.update(cx, |reader, cx| reader.focus.focus(window, cx));
        });
    });
}

struct PreferencesView {
    reader: WeakEntity<ReaderView>,
    reader_window: AnyWindowHandle,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl PreferencesView {
    fn new(
        reader: WeakEntity<ReaderView>,
        reader_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        // ReaderView defers opening because GPUI draws new windows immediately.
        // Keep observations weak and update the title in this window's context.
        let mut subscriptions = Vec::new();
        if let Some(entity) = reader.upgrade() {
            subscriptions.push(cx.observe_in(&entity, window, |_, reader, window, cx| {
                window.set_window_title(reader.read(cx).language().text(Text::Preferences));
                cx.notify();
            }));
        }
        let close_reader = reader.clone();
        window.on_window_should_close(cx, move |window, cx| {
            close(close_reader.clone(), reader_window, window, cx);
            false
        });
        Self {
            reader,
            reader_window,
            focus,
            _subscriptions: subscriptions,
        }
    }

    fn change(&self, change: Change, cx: &mut App) {
        let reader = self.reader.clone();
        let reader_window = self.reader_window;
        cx.defer(move |cx| {
            _ = reader_window.update(cx, |_, window, cx| {
                _ = reader.update(cx, |reader, cx| {
                    reader.apply_preference(change, window, cx);
                });
            });
        });
    }

    fn forward(&self, command: Command, cx: &mut App) {
        let reader = self.reader.clone();
        let reader_window = self.reader_window;
        cx.defer(move |cx| {
            _ = reader_window.update(cx, |_, window, cx| {
                _ = reader.update(cx, |reader, cx| reader.perform(command, window, cx));
            });
        });
    }

    fn picker(&self, cx: &App) -> Option<super::developer::Picker> {
        self.reader
            .upgrade()
            .and_then(|reader| reader.read(cx).developer.preferences_picker.clone())
    }
    fn marker(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &App,
    ) -> Option<gpui_kit::AnyElement> {
        self.picker(cx).map(|picker| {
            picker.mark(super::developer_session::metadata(
                id,
                label,
                "src/ui/preferences.rs::PreferencesView",
                crate::developer::TargetKind::Element,
            ))
        })
    }
    fn label(language: Language, title: Text, description: Text, cx: &App) -> gpui_kit::Div {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(180.))
            .gap_1()
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .child(language.text(title)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(language.text(description)),
            )
    }

    fn option(
        id: impl Into<gpui_kit::ElementId>,
        label: &'static str,
        selected: bool,
        cx: &App,
    ) -> Button {
        Button::new(id)
            .small()
            .outline()
            .child(label.test_text(label))
            .accessibility_label(label)
            .selected(selected)
            .toggled(selected)
            .when(selected, |button| {
                button
                    .border_color(cx.theme().primary)
                    .bg(cx.theme().primary.opacity(0.1))
                    .text_color(cx.theme().primary)
            })
    }

    fn reading(
        &self,
        language: Language,
        remember: bool,
        rtl: bool,
        background: Background,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut directions = div().flex().flex_wrap().gap_2();
        for (value, id, text) in [
            (true, "reading-direction-rtl", Text::DirectionRtl),
            (false, "reading-direction-ltr", Text::DirectionLtr),
        ] {
            directions = directions.child(
                Self::option(id, language.text(text), rtl == value, cx)
                    .when(self.picker(cx).is_some(), |button| {
                        button.relative().block_mouse_except_scroll()
                    })
                    .children(self.marker(id, language.text(text), cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(Change::Direction(value), cx);
                    })),
            );
        }
        let mut backgrounds = div().flex().flex_wrap().gap_2();
        for (value, id, text) in [
            (Background::Black, "黑色", Text::Black),
            (Background::Gray, "深灰", Text::Gray),
            (Background::Paper, "紙色", Text::Paper),
        ] {
            let selected = background == value;
            backgrounds = backgrounds.child(
                Button::new(id)
                    .when(self.picker(cx).is_some(), |button| {
                        button.relative().block_mouse_except_scroll()
                    })
                    .children(self.marker(
                        match value {
                            Background::Black => "background-black",
                            Background::Gray => "background-gray",
                            Background::Paper => "background-paper",
                        },
                        language.text(text),
                        cx,
                    ))
                    .small()
                    .outline()
                    .h_auto()
                    .py_2()
                    .min_w(px(76.))
                    .accessibility_label(language.text(text))
                    .selected(selected)
                    .toggled(selected)
                    .when(selected, |button| {
                        button
                            .border_color(cx.theme().primary)
                            .bg(cx.theme().primary.opacity(0.1))
                            .text_color(cx.theme().primary)
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .w(px(44.))
                                    .h(px(24.))
                                    .rounded_sm()
                                    .bg(background_color(value))
                                    .border_1()
                                    .border_color(cx.theme().border),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(language.text(text).test_text(language.text(text)))
                                    .child(if selected {
                                        Icon::new(IconName::Check).size_3().into_any_element()
                                    } else {
                                        div().size(px(12.)).into_any_element()
                                    }),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(Change::Background(value), cx);
                    })),
            );
        }
        div()
            .id("reading-preferences")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::ReadingTab))
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(language.text(Text::ReadingTab)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(Self::label(
                        language,
                        Text::RememberPosition,
                        Text::RememberPositionDescription,
                        cx,
                    ))
                    .child(
                        Checkbox::new("remember-position")
                            .when(self.picker(cx).is_some(), |checkbox| {
                                checkbox.relative().block_mouse_except_scroll()
                            })
                            .children(self.marker(
                                "remember-position",
                                language.text(Text::RememberPosition),
                                cx,
                            ))
                            .accessibility_label(language.text(Text::RememberPosition))
                            .checked(remember)
                            .on_change(cx.listener(|this, value, _, cx| {
                                this.change(Change::Remember(*value), cx);
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(Self::label(
                        language,
                        Text::ReadingDirection,
                        Text::ReadingDirectionDescription,
                        cx,
                    ))
                    .child(directions),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(Self::label(
                        language,
                        Text::ReadingBackground,
                        Text::ReadingBackgroundDescription,
                        cx,
                    ))
                    .child(backgrounds),
            )
    }

    fn releases(
        &self,
        language: Language,
        state: &ReleaseNotifications,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let status = state.status_text(language);
        let mut section = div()
            .id("release-preferences")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::ReleaseNotifications))
            .flex()
            .flex_col()
            .gap_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .pt_5()
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(language.text(Text::ReleaseNotifications)),
            )
            .child(language.format(
                Text::CurrentVersion,
                &[("version", env!("CARGO_PKG_VERSION").to_owned())],
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(language.text(Text::ReleaseCheckDescription)),
            )
            .child(
                Checkbox::new("release-check-on-startup")
                    .when(self.picker(cx).is_some(), |checkbox| {
                        checkbox.relative().block_mouse_except_scroll()
                    })
                    .children(self.marker(
                        "release-check-on-startup",
                        language.text(Text::ReleaseCheckOnStartup),
                        cx,
                    ))
                    .label(language.text(Text::ReleaseCheckOnStartup))
                    .accessibility_label(language.text(Text::ReleaseCheckOnStartup))
                    .checked(state.preferences.check_on_startup)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.change(Change::ReleaseStartupCheck(*value), cx);
                    })),
            )
            .child(
                div()
                    .id("release-check-status")
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .aria_label(status.clone())
                    .text_xs()
                    .child(status),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("check-for-updates")
                            .small()
                            .label(language.text(Text::CheckForUpdates))
                            .accessibility_label(language.text(Text::CheckForUpdates))
                            .disabled(state.checking())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.forward(Command::CheckForUpdates, cx);
                            })),
                    )
                    .when(state.available(), |buttons| {
                        buttons.child(
                            Button::new("release-preferences-open")
                                .small()
                                .outline()
                                .label(language.text(Text::OpenReleases))
                                .accessibility_label(language.text(Text::OpenReleases))
                                .on_click(|_, _, cx| {
                                    cx.open_url(crate::release_check::RELEASES_URL);
                                }),
                        )
                    }),
            );
        if let Some(error) = &state.preference_error {
            let label = language.format(Text::ReleasePreferencesError, &[("error", error.clone())]);
            section = section.child(
                div()
                    .id("release-preferences-error")
                    .test_support()
                    .role(gpui_kit::Role::Alert)
                    .aria_label(label.clone())
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(label),
            );
        }
        section
    }

    fn developer(&self, language: Language, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(entity) = self.reader.upgrade() else {
            return div().into_any_element();
        };
        let reader = entity.read(cx);
        let enabled = reader.developer.enabled;
        let status = reader.developer.state_text();
        let message = reader.developer.status;
        let error = reader.developer.preference_error;
        let durability_warning = reader.developer.preference_durability_warning;
        let picker = reader.developer.preferences_picker.clone();
        let service = reader.developer.service.is_some();
        let destination = reader
            .developer
            .service
            .as_ref()
            .map(|s| s.status())
            .map(|s| {
                format!(
                    "{}\n{}",
                    s.workspace.unwrap_or_default(),
                    s.session_id.unwrap_or_default()
                )
            })
            .unwrap_or_default();
        let weak = self.reader.clone();
        div()
            .id("developer-preferences")
            .test_support()
            .flex()
            .flex_col()
            .gap_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .pt_5()
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(language.text(Text::DeveloperMode)),
            )
            .child(
                div()
                    .text_xs()
                    .child(language.text(Text::DeveloperDescription)),
            )
            .child(
                Checkbox::new("developer-mode")
                    .label(language.text(Text::DeveloperMode))
                    .accessibility_label(language.text(Text::DeveloperMode))
                    .checked(enabled)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.change(Change::DeveloperMode(*value), cx)
                    })),
            )
            .child(
                div()
                    .id("developer-state")
                    .test_support()
                    .child(language.text(status)),
            )
            .children(enabled.then(|| div().text_xs().child(destination)))
            .children(enabled.then(|| {
                div()
                    .text_xs()
                    .child(language.text(Text::DeveloperPairHint))
            }))
            .when(enabled, |section| {
                section.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("developer-copy")
                                .small()
                                .label(language.text(Text::DeveloperCopy))
                                .disabled(!service || status == Text::DeveloperConnected)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.forward(Command::DeveloperCopy, cx)
                                })),
                        )
                        .child(
                            Button::new("developer-select")
                                .small()
                                .label(language.text(Text::DeveloperSelect))
                                .disabled(picker.is_none())
                                .on_click(move |_, window, cx| {
                                    if let Some(picker) = &picker {
                                        super::developer_session::begin(
                                            weak.clone(),
                                            picker.clone(),
                                            window,
                                            cx,
                                        );
                                    }
                                }),
                        )
                        .child(
                            Button::new("developer-disconnect")
                                .small()
                                .label(language.text(Text::DeveloperDisconnect))
                                .disabled(!service)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.forward(Command::DeveloperDisconnect, cx)
                                })),
                        ),
                )
            })
            .children(message.map(|m| div().text_xs().child(language.text(m))))
            .children(error.then(|| {
                div()
                    .id("developer-preference-error")
                    .test_support()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(language.text(Text::DeveloperPreferenceError))
            }))
            .children(durability_warning.then(|| {
                div()
                    .id("developer-preference-durability")
                    .test_support()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(language.text(Text::DeveloperPreferenceDurability))
            }))
            .children((error || durability_warning).then(|| {
                Button::new("developer-preference-retry")
                    .small()
                    .label(language.text(Text::DeveloperPreferenceRetry))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(Change::DeveloperMode(enabled), cx);
                    }))
            }))
            .into_any_element()
    }

    fn file_browser(
        &self,
        language: Language,
        selected: BrowserSortScroll,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut options = div().flex().flex_wrap().gap_3();
        for (value, id, title, description) in [
            (
                BrowserSortScroll::Anchor,
                "browser-sort-scroll-anchor",
                Text::BrowserSortScrollAnchor,
                Text::BrowserSortScrollAnchorDescription,
            ),
            (
                BrowserSortScroll::Top,
                "browser-sort-scroll-top",
                Text::BrowserSortScrollTop,
                Text::BrowserSortScrollTopDescription,
            ),
            (
                BrowserSortScroll::Offset,
                "browser-sort-scroll-offset",
                Text::BrowserSortScrollOffset,
                Text::BrowserSortScrollOffsetDescription,
            ),
        ] {
            options = options.child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .w_full()
                    .gap_1()
                    .child(
                        div().flex().flex_wrap().child(
                            Self::option(id, language.text(title), selected == value, cx)
                                .h_auto()
                                .when(self.picker(cx).is_some(), |button| {
                                    button.relative().block_mouse_except_scroll()
                                })
                                .children(self.marker(id, language.text(title), cx))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.change(Change::BrowserSortScroll(value), cx);
                                })),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(language.text(description)),
                    ),
            );
        }
        div()
            .id("file-browser-preferences")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::FileBrowser))
            .flex()
            .flex_col()
            .min_w_0()
            .gap_3()
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::MEDIUM)
                    .child(language.text(Text::FileBrowser)),
            )
            .child(Self::label(
                language,
                Text::BrowserSortScroll,
                Text::BrowserSortScrollDescription,
                cx,
            ))
            .child(options)
    }

    fn general(
        &self,
        language: Language,
        browser_sort_scroll: BrowserSortScroll,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut languages = div().flex().flex_wrap().gap_2();
        for choice in Language::ALL {
            languages = languages.child(
                Self::option(
                    SharedString::from(format!("language-{}", choice.code())),
                    choice.native_name(),
                    language == choice,
                    cx,
                )
                .when(self.picker(cx).is_some(), |button| {
                    button.relative().block_mouse_except_scroll()
                })
                .children(self.picker(cx).map(|picker| {
                    picker.mark(super::developer_session::metadata(
                        format!("language-{}", choice.code()),
                        choice.native_name(),
                        "src/ui/preferences.rs::general",
                        crate::developer::TargetKind::Element,
                    ))
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change(Change::Language(choice), cx);
                })),
            );
        }
        div()
            .id("general-preferences")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::GeneralSettings))
            .flex()
            .flex_col()
            .gap_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .pt_5()
            .child(
                div()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(language.text(Text::GeneralSettings)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(Self::label(
                        language,
                        Text::Language,
                        Text::LanguageDescription,
                        cx,
                    ))
                    .child(languages),
            )
            .child(self.file_browser(language, browser_sort_scroll, cx))
    }
}

impl Focusable for PreferencesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PreferencesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(reader) = self.reader.upgrade() else {
            window.remove_window();
            return div().into_any_element();
        };
        let reader = reader.read(cx);
        let prefs = &reader.reader.settings.preferences;
        let language = prefs.language;
        let remember = prefs.remember_position;
        let rtl = prefs.rtl;
        let background = prefs.background;
        let browser_sort_scroll = prefs.browser_sort_scroll;
        let error = reader.save_error.clone();
        let release_notifications = reader.release_notifications.clone();
        let developer_picker = reader.developer.preferences_picker.clone();
        let developer_reader = self.reader.clone();
        let developer_context = reader.developer_context(true);
        window.set_window_title(language.text(Text::Preferences));

        let mut content = div()
            .id("preferences-content")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::Preferences))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&release_notifications.preferences_scroll)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_6()
                    .px(px(28.))
                    .py(px(24.))
                    .child(self.reading(language, remember, rtl, background, cx))
                    .child(self.general(language, browser_sort_scroll, cx))
                    .child(self.releases(language, &release_notifications, cx))
                    .child(self.developer(language, cx)),
            );
        if let Some(error) = error {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .px(px(28.))
                    .pb(px(24.))
                    .child(
                        div()
                            .id("settings-save-error")
                            .test_support()
                            .role(gpui_kit::Role::Alert)
                            .aria_label(
                                language.format(Text::SettingsError, &[("error", error.clone())]),
                            )
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(language.format(Text::SettingsError, &[("error", error)])),
                    )
                    .child(
                        Button::new("reset-corrupt-settings")
                            .small()
                            .h_auto()
                            .label(language.text(Text::BackupReset))
                            .accessibility_label(language.text(Text::BackupReset))
                            .on_click(cx.listener(|this, _, _, cx| this.change(Change::Reset, cx))),
                    ),
            );
        }
        div()
            .id("preferences-window")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::Preferences))
            .key_context("Preferences")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Action, window, cx| {
                let picker = this
                    .reader
                    .upgrade()
                    .and_then(|entity| entity.read(cx).developer.preferences_picker.clone());
                if let Some(picker) = picker
                    && picker.active()
                {
                    picker.cancel(window, cx);
                    return;
                }
                close(this.reader.clone(), this.reader_window, window, cx)
            }))
            // These are explicit native menu actions, not Reader key bindings.
            .on_action(cx.listener(|this, command: &Command, _, cx| this.forward(*command, cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_size(px(13.))
            .children(developer_picker.as_ref().map(|picker| {
                super::developer_session::frame(picker, developer_reader, developer_context)
            }))
            .child(content)
            .child(
                div()
                    .id("settings-save-tip")
                    .test_support()
                    .role(gpui_kit::Role::Note)
                    .aria_label(language.text(Text::PreferencesAutosave))
                    .flex_none()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .px(px(28.))
                    .py_4()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(language.text(Text::PreferencesAutosave)),
            )
            .into_any_element()
    }
}
