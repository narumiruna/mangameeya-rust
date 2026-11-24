use super::*;
use crate::book::{DirectoryEntry, DirectoryListing};

pub(super) const DETAIL_ROW_HEIGHT: f32 = 30.;
const FOLDER_ROW_HEIGHT: f32 = 28.;

pub(super) struct BrowserFolders {
    list: ListState,
    roots: Vec<PathBuf>,
    collapsed: HashSet<PathBuf>,
    rows: Vec<(PathBuf, usize, bool)>,
    siblings: Option<(PathBuf, Vec<PathBuf>)>,
    loading: bool,
}

impl BrowserFolders {
    pub(super) fn new() -> Self {
        Self {
            list: ListState::new(0, ListAlignment::Top, px(0.)),
            roots: computer_roots(),
            collapsed: HashSet::new(),
            rows: Vec::new(),
            siblings: None,
            loading: false,
        }
    }

    pub(super) fn location_changed(&mut self, listing: Option<&DirectoryListing>) {
        if let Some(listing) = listing {
            for ancestor in listing.path.ancestors() {
                self.collapsed.remove(ancestor);
            }
        }
        self.rebuild(listing);
    }

    fn rebuild(&mut self, listing: Option<&DirectoryListing>) {
        self.rows.clear();
        let Some(listing) = listing else {
            self.list.reset(0);
            return;
        };
        let chain: Vec<_> = listing
            .path
            .ancestors()
            .filter(|path| path.is_absolute())
            .collect();
        if let Some((&root, _)) = chain.split_last() {
            let mut roots = self.roots.clone();
            if !roots.iter().any(|path| path == root) {
                roots.push(root.to_path_buf());
            }
            for path in roots {
                let on_branch = path == root;
                if on_branch {
                    self.append_branch(&chain, listing, 0);
                } else {
                    self.rows.push((path, 0, false));
                }
            }
        }
        self.list
            .reset_with_uniform_height(self.rows.len(), px(FOLDER_ROW_HEIGHT));
        if let Some(index) = self
            .rows
            .iter()
            .position(|(path, ..)| *path == listing.path)
        {
            self.list.scroll_to_reveal_item(index);
        }
    }

    fn append_branch(&mut self, chain: &[&Path], listing: &DirectoryListing, depth: usize) {
        let mut branches = chain.iter().rev().peekable();
        let mut level = depth;
        while let Some(&path) = branches.next() {
            let expanded = !self.collapsed.contains(path);
            self.rows.push((path.to_path_buf(), level, expanded));
            if !expanded {
                return;
            }
            if branches.peek().copied() == Some(&listing.path.as_path())
                && let Some((_, siblings)) =
                    self.siblings.as_ref().filter(|(parent, _)| parent == path)
            {
                let split = siblings
                    .partition_point(|sibling| folder_order(sibling, &listing.path).is_lt());
                self.rows.extend(
                    siblings[..split]
                        .iter()
                        .cloned()
                        .map(|path| (path, level + 1, false)),
                );
                let expanded = !self.collapsed.contains(&listing.path);
                self.rows.push((listing.path.clone(), level + 1, expanded));
                if expanded {
                    self.rows.extend(child_folders(listing, level + 2));
                }
                self.rows.extend(
                    siblings[split..]
                        .iter()
                        .filter(|path| **path != listing.path)
                        .cloned()
                        .map(|path| (path, level + 1, false)),
                );
                return;
            }
            level += 1;
        }
        self.rows.extend(child_folders(listing, level));
    }

    pub(super) fn refresh(&mut self) {
        self.siblings = None;
    }
}

fn child_folders(
    listing: &DirectoryListing,
    depth: usize,
) -> impl Iterator<Item = (PathBuf, usize, bool)> {
    listing
        .entries
        .iter()
        .filter(|entry| entry.is_directory)
        .map(move |entry| (entry.entry_path.clone(), depth, false))
}

#[cfg(windows)]
fn computer_roots() -> Vec<PathBuf> {
    // The mask lists drive letters without opening any drive.
    let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
    (0..26)
        .filter(|index| mask & (1 << index) != 0)
        .map(|index| PathBuf::from(format!(r"\\?\{}:\", char::from(b'A' + index))))
        .collect()
}

#[cfg(not(windows))]
fn computer_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

fn folder_order(a: &Path, b: &Path) -> std::cmp::Ordering {
    natord::compare_ignore_case(&browser_folder_name(a), &browser_folder_name(b))
        .then_with(|| a.cmp(b))
}

fn sibling_folders(parent: &Path) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if let Ok(kind) = entry.file_type()
                && (kind.is_dir() || (kind.is_symlink() && entry.path().is_dir()))
            {
                folders.push(entry.path());
            }
        }
    }
    folders.sort_unstable_by(|a, b| folder_order(a, b));
    folders
}

fn entry_icon(entry: &DirectoryEntry) -> IconName {
    match entry.kind {
        EntryKind::Folder => IconName::Folder,
        EntryKind::Archive => IconName::FileArchive,
        EntryKind::Image => IconName::FileImage,
    }
}

fn entry_format(entry: &DirectoryEntry) -> String {
    entry
        .entry_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_uppercase()
}

fn entry_type(entry: &DirectoryEntry, language: Language) -> String {
    let text = match entry.kind {
        EntryKind::Folder => return language.text(Text::BrowserFolderType).into(),
        EntryKind::Archive => Text::BrowserArchiveType,
        EntryKind::Image => Text::BrowserImageType,
    };
    language.format(text, &[("format", entry_format(entry))])
}

fn file_size(bytes: u64) -> String {
    let mut size = bytes as f64;
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if size < 1000. || unit == "TB" {
            return if unit == "B" {
                format!("{bytes} B")
            } else {
                format!("{size:.1} {unit}")
            };
        }
        size /= 1000.;
    }
    unreachable!()
}

fn detail_cell(text: impl Into<SharedString>) -> gpui_kit::Div {
    div().min_w_0().truncate().child(text.into())
}

fn detail_columns(
    name: impl IntoElement,
    modified: String,
    kind: String,
    size: String,
) -> gpui_kit::Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .gap_3()
        .px_3()
        .child(div().flex_1().min_w_0().child(name))
        .child(detail_cell(modified).w(px(112.)).flex_none())
        .child(detail_cell(kind).w(px(82.)).flex_none())
        .child(detail_cell(size).w(px(74.)).flex_none().text_right())
}

impl ReaderView {
    pub(in crate::ui) fn render_browser_status(
        &self,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let language = self.language();
        let text = if self.message_error {
            self.message.clone()
        } else if let Some(entry) = self.browser_selection() {
            let selected = language.text(Text::BrowserSelected);
            let size = entry.size.map(file_size);
            size.map_or_else(
                || selected.to_owned(),
                |size| format!("{selected} ({size})"),
            )
        } else {
            self.message.clone()
        };
        div()
            .id("reader-status")
            .test_support()
            .flex()
            .flex_none()
            .h(px(30.))
            .items_center()
            .gap_4()
            .px_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .text_xs()
            .child(
                div()
                    .id("status-message")
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .aria_label(text.clone())
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .when(self.message_error, |bar| bar.text_color(cx.theme().danger))
                    .child(text),
            )
            .when_some(self.listing.as_ref(), |bar, listing| {
                let path = crate::file_tree::display_path(&listing.path);
                bar.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .max_w(gpui_kit::relative(0.5))
                        .min_w_0()
                        .child(Icon::new(IconName::Folder).small().flex_none())
                        .child(
                            div()
                                .id("browser-status-path")
                                .test_support()
                                .aria_label(path.clone())
                                .truncate()
                                .child(path),
                        ),
                )
            })
            .into_any_element()
    }

    pub(super) fn browser_folders_tick(&mut self, cx: &mut Context<Self>) {
        let Some(parent) = self
            .listing
            .as_ref()
            .and_then(|listing| listing.parent.clone())
        else {
            return;
        };
        let folders = &mut self.browser.folders;
        if folders.loading
            || folders
                .siblings
                .as_ref()
                .is_some_and(|(cached, _)| *cached == parent)
        {
            return;
        }
        folders.loading = true;
        let revision = self.listing_revision;
        let task = cx.background_spawn(async move {
            let folders = sibling_folders(&parent);
            (parent, folders)
        });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            _ = view.update(cx, |view, cx| {
                view.browser.folders.loading = false;
                if !view.closing && view.listing_revision == revision {
                    view.browser.folders.siblings = Some(result);
                    view.browser.folders.rebuild(view.listing.as_ref());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn browser_folder_row(&self, index: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some((path, depth, expanded)) = self.browser.folders.rows.get(index).cloned() else {
            return div().into_any_element();
        };
        let language = self.language();
        let current = self
            .listing
            .as_ref()
            .is_some_and(|listing| listing.path == path);
        let in_branch = self
            .listing
            .as_ref()
            .is_some_and(|listing| listing.path.starts_with(&path));
        let name = browser_folder_name(&path);
        let label = crate::file_tree::display_path(&path);
        let target = path.clone();
        let key = format!("{:?}", path.as_os_str());
        div()
            .id(SharedString::from(format!("browser-folder-row:{key}")))
            .h(px(FOLDER_ROW_HEIGHT))
            .w_full()
            .pr_1()
            .rounded_md()
            .flex()
            .min_w_0()
            .items_center()
            .pl(px(4. + (depth.min(5) * 12) as f32))
            .when(current, |row| row.bg(cx.theme().primary.opacity(0.16)))
            .child(
                Button::new(SharedString::from(format!(
                    "browser-folder-disclosure:{key}"
                )))
                .small()
                .ghost()
                .flex_none()
                .w(px(22.))
                .h(px(26.))
                .icon(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .accessibility_label(language.text(if expanded {
                    Text::FilesCollapse
                } else {
                    Text::FilesExpand
                }))
                .on_click(cx.listener(move |view, _, window, cx| {
                    if in_branch {
                        if expanded {
                            view.browser.folders.collapsed.insert(target.clone());
                        } else {
                            view.browser.folders.collapsed.remove(&target);
                        }
                        view.browser.folders.rebuild(view.listing.as_ref());
                        cx.notify();
                    } else {
                        view.browse(target.clone(), window, cx);
                    }
                })),
            )
            .child(
                Button::new(SharedString::from(format!("browser-folder:{key}")))
                    .small()
                    .ghost()
                    .flex_1()
                    .min_w_0()
                    .h(px(26.))
                    .px_1()
                    .selected(current)
                    .toggled(current)
                    .when(current, |button| button.bg(cx.theme().transparent))
                    .accessibility_label(name.clone())
                    .tooltip(label)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .w_full()
                            .min_w_0()
                            .child(
                                Icon::new(if depth == 0 {
                                    IconName::HardDrive
                                } else {
                                    IconName::Folder
                                })
                                .small()
                                .flex_none()
                                .text_color(if depth == 0 {
                                    cx.theme().muted_foreground
                                } else {
                                    gpui_kit::rgb(0xc28a20).into()
                                }),
                            )
                            .child(detail_cell(name).flex_1()),
                    )
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.browse(path.clone(), window, cx)
                    })),
            )
            .into_any_element()
    }

    pub(super) fn render_browser_folders(
        &self,
        width: f32,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let language = self.language();
        let owner = cx.entity().downgrade();
        div()
            .id("browser-folders")
            .test_support()
            .aria_label(language.text(Text::FilesTree))
            .w(px((width * 0.17).clamp(148., 270.)))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .h(px(38.))
                    .flex_none()
                    .gap_2()
                    .px_3()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(
                        Icon::new(IconName::Monitor)
                            .small()
                            .text_color(cx.theme().primary),
                    )
                    .child(language.text(Text::BrowserComputer)),
            )
            .child(
                div()
                    .relative()
                    .min_h_0()
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        list(self.browser.folders.list.clone(), move |index, _, cx| {
                            owner
                                .update(cx, |view, cx| view.browser_folder_row(index, cx))
                                .unwrap_or_else(|_| div().into_any_element())
                        })
                        .size_full(),
                    )
                    .child(
                        Scrollbar::vertical(&self.browser.folders.list)
                            .mode(ScrollbarMode::Scrolling),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn browser_column_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let language = self.language();
        div()
            .id("browser-columns")
            .test_support()
            .flex_none()
            .h(px(34.))
            .bg(cx.theme().sidebar)
            .border_b_1()
            .border_color(cx.theme().border)
            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
            .child(detail_columns(
                detail_cell(language.text(Text::BrowserName)),
                language.text(Text::BrowserSortModified).to_owned(),
                language.text(Text::BrowserSortType).to_owned(),
                language.text(Text::BrowserSize).to_owned(),
            ))
    }

    pub(super) fn browser_detail_row(
        &self,
        position: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let Some(entry) = self.browser_entry(position) else {
            return div().into_any_element();
        };
        let selected = self.browser.model.selected.as_ref() == Some(&entry.entry_path);
        let language = self.language();
        let modified = entry
            .modified
            .and_then(|modified| {
                // Convert fallibly before formatting: filesystem timestamps can be out of range.
                let since_epoch = modified.duration_since(SystemTime::UNIX_EPOCH);
                let seconds = match since_epoch {
                    Ok(duration) => i64::try_from(duration.as_secs()).ok()?,
                    Err(error) => {
                        let duration = error.duration();
                        -i64::try_from(duration.as_secs()).ok()?
                            - i64::from(duration.subsec_nanos() != 0)
                    }
                };
                chrono::DateTime::from_timestamp_secs(seconds)
            })
            .map(|utc| {
                utc.with_timezone(&chrono::Local)
                    .format("%Y/%m/%d %H:%M")
                    .to_string()
            });
        let path = entry.entry_path.clone();
        div()
            .w_full()
            .h(px(DETAIL_ROW_HEIGHT))
            .px_0p5()
            .child(
                Button::new(browser_entry_id(&path))
                    .small()
                    .ghost()
                    .w_full()
                    .h_full()
                    .p_0()
                    .accessibility_label(entry.name.clone())
                    .tooltip(crate::file_tree::display_path(&path))
                    .selected(selected)
                    .toggled(selected)
                    .tab_stop(false)
                    .when(selected, |button| {
                        button.bg(cx.theme().primary.opacity(0.16))
                    })
                    .when(!selected && !position.is_multiple_of(2), |button| {
                        button.bg(cx.theme().sidebar.opacity(0.45))
                    })
                    .child(detail_columns(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w_0()
                            .child(
                                Icon::new(entry_icon(entry))
                                    .small()
                                    .flex_none()
                                    .text_color(cx.theme().primary),
                            )
                            .child(detail_cell(entry.name.clone())),
                        modified.unwrap_or_default(),
                        entry_type(entry, language),
                        entry.size.map(file_size).unwrap_or_default(),
                    ))
                    .on_click(cx.listener(
                        move |view, event: &gpui_kit::ClickEvent, window, cx| {
                            view.browser.model.selected = Some(path.clone());
                            view.focus.focus(window, cx);
                            if event.click_count() >= 2 {
                                view.browser_open(window, cx);
                            }
                            cx.notify();
                        },
                    )),
            )
            .into_any_element()
    }

    pub(super) fn render_browser_preview(
        &self,
        width: f32,
        cx: &Context<Self>,
    ) -> gpui_kit::AnyElement {
        let language = self.language();
        let pane_width = (width * 0.265).clamp(212., 420.);
        let mut preview = div()
            .id("browser-preview")
            .test_support()
            .aria_label(language.text(Text::BrowserPreview))
            .w(px(pane_width))
            .flex_none()
            .min_h_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .border_l_1()
            .border_color(cx.theme().border);
        if let Some(entry) = self.browser_selection() {
            let key = self.browser.cover_key(entry);
            let status = self.browser.cover_status(&key);
            let status_text = language.text(status.text());
            let failure = self
                .browser
                .failures
                .get(&key.source())
                .map(|failure| failure.error.to_string());
            let cover = self.browser.covers.get(&key);
            let aspect = cover.map_or(1.4, |cover| {
                cover.page.height as f32 / cover.page.width as f32
            });
            let picture = if let Some(cover) = cover {
                img(cover.page.tiles[0].image.clone())
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .into_any_element()
            } else {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .bg(cx.theme().sidebar)
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(entry_icon(entry)).size_6())
                    .child(status_text)
                    .into_any_element()
            };
            let mut metadata = if entry.is_directory {
                entry_type(entry, language)
            } else {
                entry_format(entry)
            };
            if let Some(count) = self.browser.model.page_count(entry, &self.reader.settings) {
                metadata.push_str(" · ");
                metadata.push_str(
                    &language.format(Text::BrowserPageCount, &[("count", count.to_string())]),
                );
            }
            let title = if entry.is_directory {
                entry.name.clone()
            } else {
                entry
                    .entry_path
                    .file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| entry.name.clone())
            };
            preview = preview
                .child(
                    div()
                        .id("browser-preview-cover")
                        .test_support()
                        .aria_label(status_text)
                        .when_some(failure, |image, reason| {
                            image
                                .aria_description(reason.clone())
                                .tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(reason.clone())
                                        .build(window, cx)
                                })
                        })
                        .w_full()
                        .h(px((pane_width - 33.) * aspect))
                        .flex_shrink(1.)
                        .min_h_0()
                        .max_h(px(550.))
                        .child(picture),
                )
                .child(
                    div()
                        .id("browser-preview-title")
                        .test_support()
                        .aria_label(title.clone())
                        .text_lg()
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .line_clamp(2)
                        .flex_none()
                        .child(title),
                )
                .child(
                    div()
                        .id("browser-preview-metadata")
                        .test_support()
                        .aria_label(metadata.clone())
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .flex_none()
                        .child(metadata),
                )
                .when_some(
                    self.browser.model.progress(entry, &self.reader.settings),
                    |pane, progress| {
                        let label = language.format(
                            Text::ReadProgress,
                            &[("percent", format!("{:.0}", progress * 100.))],
                        );
                        pane.child(
                            div()
                                .id("browser-preview-progress")
                                .test_support()
                                .aria_label(label.clone())
                                .flex_none()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(label)
                                .child(
                                    div()
                                        .w_full()
                                        .h(px(7.))
                                        .rounded_full()
                                        .bg(cx.theme().border)
                                        .child(
                                            div()
                                                .h_full()
                                                .w(gpui_kit::relative(progress as f32))
                                                .rounded_full()
                                                .bg(cx.theme().primary),
                                        ),
                                ),
                        )
                    },
                );
        }
        preview.into_any_element()
    }
}
