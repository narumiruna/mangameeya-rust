//! Independent, session-only Files sidebar and bounded directory workers.

use super::*;
use crate::file_tree::{FileTree, RowKind, ScrollAnchor, display_path, load_directory};

const ROW_HEIGHT: f32 = 32.;

pub(super) fn entry_id(path: &Path) -> SharedString {
    format!("files-entry:{:?}", path.as_os_str()).into()
}

pub(super) fn disclosure_id(path: &Path) -> SharedString {
    format!("files-toggle:{:?}", path.as_os_str()).into()
}

pub(super) struct FilesUi {
    pub(super) model: FileTree,
    pub(super) list: ListState,
    pub(super) focus: FocusHandle,
    pub(super) search: Entity<InputState>,
    pub(super) source: Option<PathBuf>,
    pub(super) manually_navigated: bool,
    last_navigation: Option<PathBuf>,
    reveal_target: Option<PathBuf>,
    focus_reveal: bool,
    auto_reveal: bool,
    pub(super) rendered: Rc<RefCell<HashSet<usize>>>,
    _subscriptions: Vec<Subscription>,
}

impl FilesUi {
    pub(super) fn new(window: &mut Window, cx: &mut Context<ReaderView>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(&search, window, |view, input, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                if view.files_ui.model.query() != query {
                    view.files_cancel_reveal(cx);
                    view.files_remember_scroll();
                    view.files_ui.model.set_query(query);
                    view.files_rebuild(cx);
                }
            }
        });
        Self {
            model: FileTree::new(),
            list: ListState::new(0, ListAlignment::Top, px(0.)),
            focus: cx.focus_handle(),
            search,
            source: None,
            manually_navigated: false,
            last_navigation: None,
            reveal_target: None,
            focus_reveal: false,
            auto_reveal: false,
            rendered: Rc::default(),
            _subscriptions: vec![subscription],
        }
    }
}

#[derive(Clone, Copy)]
enum FilesAction {
    Back,
    Parent,
    Locate,
    Hidden,
}

impl ReaderView {
    pub(super) fn files_cancel_reveal(&mut self, cx: &mut Context<Self>) {
        if self.files_ui.reveal_target.is_some() {
            self.files_ui.model.clear_error();
        }
        if self.files_ui.auto_reveal {
            self.files_ui.model.cancel_navigation();
        }
        self.files_ui.model.cancel_locate();
        self.files_ui.auto_reveal = false;
        self.files_ui.reveal_target = None;
        self.files_ui.focus_reveal = false;
        cx.notify();
    }

    pub(super) fn files_close(&mut self, cx: &mut Context<Self>) {
        self.files_ui.source = None;
        self.files_ui.reveal_target = None;
        self.files_ui.auto_reveal = false;
        self.files_ui.focus_reveal = false;
        self.files_ui.model.close();
        self.files_rebuild(cx);
    }

    fn files_remember_scroll(&mut self) {
        let top = self.files_ui.list.logical_scroll_top();
        let path = self
            .files_ui
            .model
            .visible_rows()
            .get(top.item_ix)
            .map(|row| row.path.clone());
        self.files_ui.model.set_scroll_anchor(ScrollAnchor {
            path,
            offset: f32::from(top.offset_in_item),
        });
    }

    pub(super) fn files_rebuild(&mut self, cx: &mut Context<Self>) {
        let rows = self.files_ui.model.visible_rows();
        self.files_ui
            .list
            .reset_with_uniform_height(rows.len(), px(ROW_HEIGHT));
        let anchor = self.files_ui.model.scroll_anchor();
        if let Some(index) = anchor
            .path
            .as_ref()
            .and_then(|path| rows.iter().position(|row| &row.path == path))
        {
            self.files_ui.list.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(anchor.offset),
            });
        }
        cx.notify();
    }

    fn files_reveal_focus(&mut self) {
        if let Some(index) = self.files_ui.model.focused().and_then(|path| {
            self.files_ui
                .model
                .visible_rows()
                .iter()
                .position(|row| row.kind == RowKind::Entry && row.path == path)
        }) {
            self.files_ui.list.scroll_to_reveal_item(index);
        }
    }

    fn files_sync_inputs(
        &mut self,
        location_changed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if location_changed {
            let value = self
                .files_ui
                .model
                .location()
                .map(display_path)
                .unwrap_or_default();
            self.path_input
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
        let query = self.files_ui.model.query().to_owned();
        if self.files_ui.search.read(cx).value().as_ref() != query {
            self.files_ui
                .search
                .update(cx, |input, cx| input.set_value(query, window, cx));
        }
    }

    fn files_schedule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        for request in self.files_ui.model.take_requests() {
            let task = cx.background_spawn(async move { load_directory(&request.path) });
            cx.spawn_in(window, async move |view, cx| {
                let result = task.await;
                _ = view.update_in(cx, |view, window, cx| {
                    if view.closing {
                        return;
                    }
                    view.files_remember_scroll();
                    let old_location = view.files_ui.model.location().map(Path::to_path_buf);
                    let was_hidden = view.files_ui.model.show_hidden();
                    let accepted = view.files_ui.model.complete(request.token, result);
                    if accepted {
                        if view.files_ui.auto_reveal
                            && let Some(target) = view.files_ui.reveal_target.clone()
                            && view.files_ui.model.location() == target.parent()
                            && view.files_ui.model.node(&target).is_some()
                        {
                            view.files_ui.model.reveal_loaded(&target);
                            view.files_ui.auto_reveal = false;
                        }
                        view.files_rebuild(cx);
                        let changed = old_location.as_deref() != view.files_ui.model.location();
                        view.files_sync_inputs(changed, window, cx);
                        let revealed =
                            view.files_ui
                                .reveal_target
                                .as_deref()
                                .is_some_and(|target| {
                                    view.files_ui.model.selected() == Some(target)
                                        && view.files_ui.model.visible_rows().iter().any(|row| {
                                            row.path == target && row.kind == RowKind::Entry
                                        })
                                });
                        if revealed {
                            view.files_reveal_focus();
                            if view.files_ui.focus_reveal
                                && view.files_ui.focus.is_focused(window)
                                && view.reader.settings.preferences.sidebar
                                && view.reader.settings.preferences.sidebar_mode
                                    == SidebarMode::Files
                            {
                                view.files_ui.focus.focus(window, cx);
                            }
                            view.files_ui.reveal_target = None;
                        }
                        if !was_hidden
                            && view.files_ui.model.show_hidden()
                            && view.files_ui.focus_reveal
                        {
                            view.set_localized_message(
                                Text::FilesHiddenRevealed,
                                vec![],
                                false,
                                cx,
                            );
                        }
                        if revealed {
                            view.files_ui.focus_reveal = false;
                        }
                    }
                    // Obsolete synchronous work still releases its slot here.
                    view.files_schedule(window, cx);
                });
            })
            .detach();
        }
    }

    pub(super) fn files_browse(
        &mut self,
        path: PathBuf,
        manual: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.has_active_drag() {
            return;
        }
        self.files_remember_scroll();
        self.files_ui.manually_navigated |= manual;
        self.files_ui.last_navigation = Some(path.clone());
        self.files_ui.reveal_target = None;
        self.files_ui.auto_reveal = false;
        self.files_ui.focus_reveal = false;
        self.files_ui.model.request_navigation(path);
        self.files_rebuild(cx);
        if manual {
            self.files_ui.focus.focus(window, cx);
        }
        self.files_schedule(window, cx);
    }

    pub(super) fn files_submit_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.path_input.read(cx).value().to_string();
        // Unedited display text must not replace a lossless native path.
        let target = self
            .files_ui
            .model
            .location()
            .filter(|path| display_path(path) == value)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(value));
        self.files_browse(target, true, window, cx);
    }

    pub(super) fn files_after_open(
        &mut self,
        source: PathBuf,
        external: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.files_ui.source = Some(source.clone());
        if external && !self.files_ui.manually_navigated {
            // Establish a useful initial root without enumerating the whole home-to-book chain.
            let parent = source
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or(&source)
                .to_path_buf();
            self.files_browse(parent, false, window, cx);
            self.files_ui.reveal_target = Some(source);
            self.files_ui.auto_reveal = true;
            self.files_ui.focus_reveal = false;
        } else {
            cx.notify();
        }
    }

    pub(super) fn files_locate(
        &mut self,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.files_ui.source.clone() else {
            return;
        };
        self.files_remember_scroll();
        let was_hidden = self.files_ui.model.show_hidden();
        self.files_ui.reveal_target = Some(source.clone());
        self.files_ui.focus_reveal = focus;
        self.files_ui.manually_navigated |= focus;
        self.files_ui.auto_reveal = false;
        self.files_ui.model.locate(source);
        self.files_rebuild(cx);
        self.files_sync_inputs(true, window, cx);
        self.files_reveal_focus();
        if focus {
            self.files_ui.focus.focus(window, cx);
        }
        if !was_hidden && self.files_ui.model.show_hidden() && focus {
            self.set_localized_message(Text::FilesHiddenRevealed, vec![], false, cx);
        }
        if self
            .files_ui
            .reveal_target
            .as_deref()
            .is_some_and(|target| {
                self.files_ui.model.selected() == Some(target)
                    && self
                        .files_ui
                        .model
                        .visible_rows()
                        .iter()
                        .any(|row| row.path == target && row.kind == RowKind::Entry)
            })
        {
            self.files_ui.reveal_target = None;
            self.files_ui.focus_reveal = false;
        }
        self.files_schedule(window, cx);
    }

    fn files_action(&mut self, action: FilesAction, window: &mut Window, cx: &mut Context<Self>) {
        if cx.has_active_drag() {
            return;
        }
        self.files_remember_scroll();
        match action {
            FilesAction::Locate => {
                self.files_locate(true, window, cx);
                return;
            }
            FilesAction::Back => {
                self.files_ui.auto_reveal = false;
                self.files_ui.focus_reveal = false;
                self.files_ui.manually_navigated = true;
                self.files_ui.reveal_target = None;
                self.files_ui.model.back();
                self.files_sync_inputs(true, window, cx);
            }
            FilesAction::Parent => {
                if self.files_ui.model.can_parent() {
                    self.files_ui.auto_reveal = false;
                    self.files_ui.focus_reveal = false;
                    self.files_ui.manually_navigated = true;
                    self.files_ui.reveal_target = None;
                    self.files_ui.last_navigation = self
                        .files_ui
                        .model
                        .location()
                        .and_then(Path::parent)
                        .map(Path::to_path_buf);
                    self.files_ui.model.request_parent();
                }
            }
            FilesAction::Hidden => {
                self.files_cancel_reveal(cx);
                self.files_ui
                    .model
                    .set_show_hidden(!self.files_ui.model.show_hidden());
            }
        }
        self.files_rebuild(cx);
        self.files_ui.focus.focus(window, cx);
        self.files_schedule(window, cx);
    }

    fn files_read_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry = self
            .files_ui
            .model
            .selected_visible_entry()
            .filter(|entry| !entry.is_directory)
            .cloned();
        if let Some(entry) = entry {
            self.open_path_from(entry.entry_path, OpenSource::Sidebar, window, cx);
        }
    }

    fn files_can_read_folder(&self) -> bool {
        self.files_ui.model.location().is_some_and(|path| {
            !self.files_ui.model.is_loading(path)
                && self.files_ui.model.node_error(path).is_none()
                && self.files_ui.model.children(path).any(|node| {
                    node.entry.kind == EntryKind::Image
                        || (self.reader.settings.preferences.recursive_folder
                            && node.entry.is_directory
                            && !node.cycle)
                })
        })
    }

    fn files_read_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.files_can_read_folder()
            && let Some(path) = self.files_ui.model.location().map(Path::to_path_buf)
        {
            self.open_path_from(path, OpenSource::Sidebar, window, cx);
        }
    }

    fn files_key_down(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let path_focused = self
            .path_input
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        let search_focused = self
            .files_ui
            .search
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        if path_focused || search_focused {
            if path_focused && key == "escape" {
                self.files_sync_inputs(true, window, cx);
                self.files_ui.focus.focus(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        if cx.has_active_drag() {
            return;
        }
        if key == "backspace" || (key == "up" && event.keystroke.modifiers.alt) {
            self.files_action(FilesAction::Parent, window, cx);
            cx.stop_propagation();
            return;
        }
        if !self.files_ui.focus.is_focused(window) {
            return;
        }
        if matches!(
            key,
            "up" | "down" | "home" | "end" | "right" | "left" | "enter" | "escape"
        ) {
            self.files_cancel_reveal(cx);
        }
        // Read the cached rows by reference; do not clone a large tree for each key.
        let rows = self.files_ui.model.visible_rows();
        let index = self
            .files_ui
            .model
            .focused()
            .and_then(|path| {
                rows.iter()
                    .position(|row| row.kind == RowKind::Entry && row.path == path)
            })
            .unwrap_or(0);
        let current = rows
            .get(index)
            .filter(|row| row.kind == RowKind::Entry)
            .cloned();
        match key {
            "up" | "down" | "home" | "end" => {
                let next = match key {
                    "up" => rows[..index]
                        .iter()
                        .rev()
                        .find(|row| row.kind == RowKind::Entry),
                    "down" => rows
                        .iter()
                        .skip(index + 1)
                        .find(|row| row.kind == RowKind::Entry),
                    "home" => rows.iter().find(|row| row.kind == RowKind::Entry),
                    _ => rows.iter().rev().find(|row| row.kind == RowKind::Entry),
                }
                .or(current.as_ref())
                .map(|row| row.path.clone());
                if let Some(path) = next {
                    self.files_ui.model.focus(&path);
                }
            }
            "right" => {
                if let Some(row) = current.filter(|row| {
                    row.entry.as_ref().is_some_and(|entry| entry.is_directory) && !row.cycle
                }) {
                    if row.expanded {
                        let child = rows
                            .iter()
                            .skip(index + 1)
                            .find(|child| child.kind == RowKind::Entry)
                            .filter(|child| child.depth > row.depth)
                            .map(|child| child.path.clone());
                        if let Some(path) = child {
                            self.files_ui.model.focus(&path);
                        }
                    } else {
                        self.files_ui.model.expand(&row.path);
                    }
                }
            }
            "left" => {
                if let Some(row) = current {
                    if row.expanded && self.files_ui.model.query().trim().is_empty() {
                        self.files_ui.model.collapse(&row.path);
                    } else if let Some(parent) = row.path.parent() {
                        self.files_ui.model.focus(parent);
                    }
                }
            }
            "enter" => {
                if let Some(row) = current {
                    if row.entry.as_ref().is_some_and(|entry| entry.is_directory) {
                        if !row.cycle {
                            self.files_browse(row.path, true, window, cx);
                        }
                    } else {
                        self.files_ui.model.select(&row.path);
                        self.files_read_selected(window, cx);
                    }
                }
            }
            "escape" => {
                self.focus.focus(window, cx);
            }
            _ => return,
        }
        self.files_rebuild(cx);
        self.files_reveal_focus();
        self.files_schedule(window, cx);
        cx.stop_propagation();
    }

    fn files_icon_button(
        &self,
        id: &'static str,
        icon: IconName,
        text: Text,
        enabled: bool,
        action: FilesAction,
        cx: &mut Context<Self>,
    ) -> Button {
        let label = self.language().text(text);
        Button::new(id)
            .small()
            .ghost()
            .icon(icon)
            .accessibility_label(label)
            .tooltip(label)
            .w(px(28.))
            .h(px(28.))
            .disabled(!enabled)
            .when(matches!(action, FilesAction::Hidden), |button| {
                button
                    .selected(self.files_ui.model.show_hidden())
                    .toggled(self.files_ui.model.show_hidden())
            })
            .on_click(cx.listener(move |view, _, window, cx| view.files_action(action, window, cx)))
    }

    pub(super) fn render_files(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
        self.files_ui.rendered.borrow_mut().clear();
        let view = cx.entity().downgrade();
        let mut controls = div()
            .flex_none()
            .p_2()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(self.files_icon_button(
                        "files-back",
                        IconName::ArrowLeft,
                        Text::FilesBack,
                        self.files_ui.model.can_back(),
                        FilesAction::Back,
                        cx,
                    ))
                    .child(self.files_icon_button(
                        "files-parent",
                        IconName::ArrowUp,
                        Text::ParentFolder,
                        self.files_ui.model.can_parent(),
                        FilesAction::Parent,
                        cx,
                    ))
                    .child(self.files_icon_button(
                        "files-locate",
                        IconName::LocateFixed,
                        Text::FilesLocate,
                        self.files_ui.source.is_some(),
                        FilesAction::Locate,
                        cx,
                    ))
                    .child(self.files_icon_button(
                        "files-hidden",
                        IconName::Eye,
                        Text::FilesHidden,
                        true,
                        FilesAction::Hidden,
                        cx,
                    )),
            )
            .child(
                div()
                    .id("files-path-control")
                    .test_support()
                    .role(gpui_kit::Role::Group)
                    .aria_label(language.text(Text::FolderPath))
                    .aria_description(language.text(Text::FilesPathTip))
                    .child(
                        Input::new(&self.path_input)
                            .id("path-input")
                            .aria_label(language.text(Text::FolderPath))
                            .small(),
                    ),
            )
            .child(
                Input::new(&self.files_ui.search)
                    .id("files-search")
                    .aria_label(language.text(Text::FilesSearchLoaded))
                    .small(),
            )
            .child(
                div()
                    .id("files-search-hint")
                    .test_support()
                    .aria_label(language.text(Text::FilesSearchHint))
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(language.text(Text::FilesSearchHint)),
            );
        if let Some(error) = self.files_ui.model.error() {
            let target = self.files_ui.last_navigation.clone();
            let locate = self.files_ui.reveal_target.is_some();
            controls = controls.child(
                div()
                    .id("files-navigation-error")
                    .test_support()
                    .role(gpui_kit::Role::Alert)
                    .aria_label(language.format(Text::FolderError, &[("error", error.to_owned())]))
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .line_clamp(2)
                            .child(
                                language.format(Text::FolderError, &[("error", error.to_owned())]),
                            ),
                    )
                    .child(
                        Button::new("files-retry-navigation")
                            .small()
                            .ghost()
                            .label(language.text(Text::Retry))
                            .disabled(target.is_none() && !locate)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                if locate {
                                    view.files_locate(true, window, cx);
                                } else if let Some(path) = &target {
                                    view.files_browse(path.clone(), true, window, cx);
                                }
                            })),
                    ),
            );
        }
        let empty = self.files_ui.model.visible_rows().is_empty();
        let mut tree = div()
            .id("files-tree")
            .test_support()
            .role(gpui_kit::Role::Tree)
            .aria_label(language.text(Text::FilesTree))
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(
                list(self.files_ui.list.clone(), move |index, _, cx| {
                    view.update(cx, |view, cx| view.file_tree_row(index, cx))
                        .unwrap_or_else(|_| div().into_any_element())
                })
                .size_full(),
            )
            .child(Scrollbar::vertical(&self.files_ui.list).mode(ScrollbarMode::Always));
        if empty {
            let text = if self.files_ui.model.pending_navigation().is_some() {
                Text::FolderLoading
            } else if !self.files_ui.model.query().trim().is_empty() {
                Text::BrowserNoMatches
            } else {
                Text::BrowserEmpty
            };
            tree = tree.child(
                div()
                    .id("files-empty")
                    .test_support()
                    .absolute()
                    .inset_0()
                    .p_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(language.text(text)),
            );
        }
        let selected = self
            .files_ui
            .model
            .selected_visible_entry()
            .is_some_and(|entry| !entry.is_directory);
        div()
            .id("files-panel")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::FilesTree))
            .key_context("FilesTree")
            .track_focus(&self.files_ui.focus)
            .capture_key_down(
                cx.listener(|view, event, window, cx| view.files_key_down(event, window, cx)),
            )
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .child(controls)
            .child(tree)
            .child(
                div()
                    .flex_none()
                    .p_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        Button::new("files-read-selected")
                            .small()
                            .primary()
                            .label(language.text(Text::FilesReadSelected))
                            .tooltip(
                                self.files_ui
                                    .model
                                    .selected_entry()
                                    .map(|entry| display_path(&entry.entry_path))
                                    .unwrap_or_else(|| {
                                        language.text(Text::FilesReadSelected).to_owned()
                                    }),
                            )
                            .w_full()
                            .h(px(28.))
                            .disabled(!selected)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.files_read_selected(window, cx)
                            })),
                    )
                    .child(
                        Button::new("files-read-folder")
                            .small()
                            .ghost()
                            .label(language.text(Text::FilesReadFolder))
                            .w_full()
                            .h(px(24.))
                            .disabled(!self.files_can_read_folder())
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.files_read_folder(window, cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    fn file_tree_row(&self, index: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(row) = self.files_ui.model.visible_rows().get(index) else {
            return div().into_any_element();
        };
        self.files_ui.rendered.borrow_mut().insert(index);
        let language = self.language();
        let path = row.path.clone();
        let indent = (row.depth.min(5) * 10) as f32;
        if row.kind != RowKind::Entry {
            let error = row.error.clone();
            let mut status = div()
                .h(px(ROW_HEIGHT))
                .w_full()
                .min_w_0()
                .pl(px(indent + 24.))
                .pr_2()
                .flex()
                .items_center()
                .gap_1();
            if let Some(error) = error {
                status = status
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "files-error:{:?}",
                                path.as_os_str()
                            )))
                            .test_support()
                            .role(gpui_kit::Role::Alert)
                            .aria_label(error.clone())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(language.text(Text::ReadFailed)),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "files-retry:{:?}",
                            path.as_os_str()
                        )))
                        .small()
                        .ghost()
                        .label(language.text(Text::Retry))
                        .tooltip(error)
                        .on_click(cx.listener(
                            move |view, _, window, cx| {
                                view.files_remember_scroll();
                                view.files_ui.model.retry(&path);
                                view.files_rebuild(cx);
                                view.files_schedule(window, cx);
                            },
                        )),
                    );
            } else {
                status = status.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(language.text(Text::FolderLoading)),
                );
            }
            return status.into_any_element();
        }
        let entry = row.entry.as_ref().expect("entry row");
        let directory = entry.is_directory;
        let cycle = row.cycle;
        let selected = self.files_ui.model.selected() == Some(path.as_path());
        let focused = self.files_ui.model.focused() == Some(path.as_path());
        let current = self.files_ui.source.as_deref() == Some(path.as_path());
        let expanded = row.expanded;
        let toggle_path = path.clone();
        let mut content = div()
            .id(SharedString::from(format!(
                "files-row:{:?}",
                path.as_os_str()
            )))
            .test_support()
            .role(gpui_kit::Role::TreeItem)
            .aria_label(entry.name.clone())
            .aria_level(row.depth + 1)
            .aria_selected(selected)
            .when(directory, |item| item.aria_expanded(expanded))
            .when(focused, |item| item.aria_active_descendant())
            .h(px(ROW_HEIGHT))
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .pr_1()
            .children((0..row.depth.min(5)).map(|_| {
                div()
                    .w(px(10.))
                    .h_full()
                    .flex_none()
                    .border_l_1()
                    .border_color(cx.theme().border)
            }));
        if directory {
            content = content.child(
                Button::new(disclosure_id(&path))
                    .small()
                    .ghost()
                    .icon(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .w(px(22.))
                    .h(px(28.))
                    .flex_none()
                    .tab_stop(false)
                    .accessibility_label(language.text(if expanded {
                        Text::FilesCollapse
                    } else {
                        Text::FilesExpand
                    }))
                    .disabled(cycle || !self.files_ui.model.query().trim().is_empty())
                    .on_click(cx.listener(move |view, _, window, cx| {
                        if cx.has_active_drag() {
                            return;
                        }
                        view.files_cancel_reveal(cx);
                        view.files_remember_scroll();
                        view.files_ui.model.focus(&toggle_path);
                        view.files_ui.model.toggle_expanded(&toggle_path);
                        view.files_rebuild(cx);
                        view.files_ui.focus.focus(window, cx);
                        view.files_reveal_focus();
                        view.files_schedule(window, cx);
                    })),
            );
        } else {
            content = content.child(div().w(px(22.)).flex_none());
        }
        let icon = match entry.kind {
            EntryKind::Folder => IconName::Folder,
            EntryKind::Archive => IconName::FileArchive,
            EntryKind::Image => IconName::FileImage,
        };
        let tip = if cycle {
            format!(
                "{}\n{}",
                display_path(&entry.entry_path),
                language.text(Text::FilesCycle)
            )
        } else {
            display_path(&entry.entry_path)
        };
        content
            .child(
                Button::new(entry_id(&path))
                    .small()
                    .ghost()
                    .tab_stop(false)
                    .flex_1()
                    .min_w_0()
                    .h(px(30.))
                    .px_1()
                    .accessibility_label(entry.name.clone())
                    .tooltip(tip)
                    .selected(selected)
                    .toggled(selected)
                    .border_l_2()
                    .border_color(if current {
                        cx.theme().primary
                    } else {
                        cx.theme().transparent
                    })
                    .when(focused, |button| {
                        button.bg(cx.theme().primary.opacity(0.08))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .w_full()
                            .min_w_0()
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "files-icon:{:?}",
                                        path.as_os_str()
                                    )))
                                    .test_support()
                                    .w(px(18.))
                                    .flex_none()
                                    .text_color(if directory {
                                        gpui_kit::rgb(0xa78545).into()
                                    } else {
                                        cx.theme().muted_foreground
                                    })
                                    .child(Icon::new(icon).size(px(15.))),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_left()
                                    .truncate()
                                    .text_size(px(12.))
                                    .child(entry.name.clone()),
                            )
                            .when(current, |label| {
                                label.child(
                                    div()
                                        .id("files-current-marker")
                                        .test_support()
                                        .text_size(px(9.))
                                        .flex_none()
                                        .text_color(cx.theme().primary)
                                        .child(language.text(Text::FilesReading)),
                                )
                            }),
                    )
                    .on_click(cx.listener(
                        move |view, event: &gpui_kit::ClickEvent, window, cx| {
                            if cx.has_active_drag() {
                                return;
                            }
                            view.files_ui.focus.focus(window, cx);
                            if directory && !cycle {
                                view.files_browse(path.clone(), true, window, cx);
                            } else {
                                view.files_cancel_reveal(cx);
                                view.files_ui.model.select(&path);
                                if !directory && event.click_count() >= 2 {
                                    view.files_read_selected(window, cx);
                                }
                                cx.notify();
                            }
                        },
                    )),
            )
            .into_any_element()
    }
}
