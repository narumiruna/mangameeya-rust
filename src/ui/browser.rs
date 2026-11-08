//! Central file grid and bounded, static cover cache.

use super::*;
use crate::browser::{BrowserModel, BrowserSnapshot, SortKey, columns, row_count};
use gpui_kit::component::menu::PopupMenuItem;
use std::time::SystemTime;

const COVER_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_COVER_JOBS: usize = 2;
const TILE_GAP: f32 = 12.;

pub(super) fn browser_entry_id(path: &Path) -> SharedString {
    // OsStr Debug escapes invalid native units instead of replacing them.
    // Display strings are not unique identifiers for filesystem entries.
    format!("browser-entry:{:?}", path.as_os_str()).into()
}

#[cfg(all(test, any(unix, windows)))]
#[test]
fn native_entry_ids_do_not_merge_equal_lossy_display_names() {
    #[cfg(unix)]
    let paths = {
        use std::os::unix::ffi::OsStringExt;
        [0xfe, 0xff].map(|unit| PathBuf::from(std::ffi::OsString::from_vec(vec![unit])))
    };
    #[cfg(windows)]
    let paths = {
        use std::os::windows::ffi::OsStringExt;
        [0xd800, 0xd801].map(|unit| PathBuf::from(std::ffi::OsString::from_wide(&[unit])))
    };
    assert_eq!(paths[0].to_string_lossy(), paths[1].to_string_lossy());
    assert_ne!(browser_entry_id(&paths[0]), browser_entry_id(&paths[1]));
}

fn cover_dimensions(tile_width: f32, scale: f32, capacity: usize) -> (u32, u32) {
    // Keep the whole viewport and overscan inside the pixel budget. A dense
    // high-DPI view must not repeatedly decode and evict its own visible covers.
    let pixels = COVER_CACHE_BYTES / 4 / capacity.max(1);
    let max_width = ((pixels as f64 / 1.4).sqrt().floor() as u32).max(1);
    let width = ((tile_width * scale).ceil().max(1.) as u32).min(max_width);
    (width, (width * 14 / 10).max(1))
}

#[test]
fn dense_high_dpi_cover_working_sets_fit_the_pixel_budget() {
    for width in [140., 100., 220.] {
        for scale in [2., 1., 1.25, 3., 4.] {
            for capacity in [91, 1, 16, 32, 64, 128, 512] {
                let (w, h) = cover_dimensions(width, scale, capacity);
                assert!(w > 0 && h > 0);
                assert!(
                    u64::from(w) * u64::from(h) * 4 * capacity as u64 <= COVER_CACHE_BYTES as u64,
                    "{capacity} covers at {width}px / {scale}x exceed the cache budget: {w}x{h}"
                );
            }
        }
    }
    assert_eq!(cover_dimensions(140., 2., 16), (280, 392));
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CoverKey {
    path: PathBuf,
    size: Option<u64>,
    modified: Option<SystemTime>,
    width: u32,
    height: u32,
}

struct Cover {
    page: CachedPage,
    used: u64,
}

pub(super) struct BrowserUi {
    pub(super) model: BrowserModel,
    pub(super) grid: ListState,
    pub(super) columns: usize,
    pub(super) search: Entity<InputState>,
    pub(super) tile_slider: Entity<SliderState>,
    pub(super) snapshots: HashMap<PathBuf, BrowserSnapshot>,
    tile_width: f32,
    show_names: bool,
    name_lines: usize,
    metadata: bool,
    scale: f32,
    viewport_height: f32,
    generation: u64,
    listing_revision: u64,
    clock: u64,
    covers: HashMap<CoverKey, Cover>,
    failures: HashMap<CoverKey, String>,
    inflight: HashSet<(u64, CoverKey)>,
    visible_rows: Rc<RefCell<HashSet<usize>>>,
    search_pending: bool,
    search_language: Option<Language>,
    scroll_anchor: Option<(PathBuf, ListOffset)>,
    #[cfg(test)]
    requested_count: usize,
    _subscriptions: Vec<Subscription>,
}

impl BrowserUi {
    pub(super) fn new(window: &mut Window, cx: &mut Context<ReaderView>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx));
        let tile_slider = cx.new(|_| {
            SliderState::new()
                .min(100.)
                .max(220.)
                .step(1.)
                .default_value(140.)
        });
        let subscriptions = vec![
            cx.subscribe_in(&search, window, |this, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().to_string();
                    if this.browser.model.query != query {
                        this.browser.model.query = query;
                        this.rebuild_browser(cx);
                    }
                }
            }),
            cx.subscribe_in(&tile_slider, window, |this, _, event, _, cx| {
                if let SliderEvent::Change(value) = event {
                    let anchor = this.browser_anchor();
                    this.browser.tile_width = value.start().clamp(100., 220.);
                    this.browser.columns = columns(
                        (f32::from(this.viewer_size.width) - 24.).max(1.),
                        this.browser.tile_width + TILE_GAP,
                    );
                    this.reset_browser_grid(anchor);
                    cx.notify();
                }
            }),
        ];
        Self {
            model: BrowserModel::default(),
            grid: ListState::new(0, ListAlignment::Top, px(0.)),
            columns: 1,
            search,
            tile_slider,
            snapshots: HashMap::new(),
            tile_width: 140.,
            show_names: true,
            name_lines: 2,
            metadata: true,
            scale: window.scale_factor(),
            viewport_height: 0.,
            generation: 0,
            listing_revision: 0,
            clock: 0,
            covers: HashMap::new(),
            failures: HashMap::new(),
            inflight: HashSet::new(),
            visible_rows: Rc::default(),
            search_pending: false,
            search_language: None,
            scroll_anchor: None,
            #[cfg(test)]
            requested_count: 0,
            _subscriptions: subscriptions,
        }
    }

    fn row_height(&self) -> f32 {
        self.tile_width * 1.4
            + if self.show_names {
                self.name_lines as f32 * 18. + 8.
            } else {
                0.
            }
            + if self.metadata { 36. } else { 0. }
            + 24.
    }

    fn requested_range(&self) -> std::ops::Range<usize> {
        let height = f32::from(self.grid.viewport_bounds().size.height);
        if height <= 0. || !height.is_finite() {
            return 0..0;
        }
        let top = self.grid.logical_scroll_top();
        let visible =
            ((height + f32::from(top.offset_in_item)) / self.row_height()).ceil() as usize;
        let start = top.item_ix.saturating_sub(1).saturating_mul(self.columns);
        let end = top
            .item_ix
            .saturating_add(visible)
            .saturating_add(1)
            .saturating_mul(self.columns)
            .min(self.model.order.len());
        start.min(end)..end
    }

    fn cover_key(&self, entry: &book::DirectoryEntry) -> CoverKey {
        // Include a partial row and both overscan rows. This capacity does not
        // change at each scroll offset, so decoded resolution remains stable.
        let rows = (self.viewport_height.max(0.) / self.row_height()).ceil() as usize;
        let capacity = rows
            .saturating_add(3)
            .saturating_mul(self.columns)
            .min(self.model.order.len())
            .max(1);
        let (width, height) = cover_dimensions(self.tile_width, self.scale, capacity);
        CoverKey {
            path: entry.path.clone(),
            size: entry.size,
            modified: entry.modified,
            width,
            height,
        }
    }
}

#[cfg(test)]
impl BrowserUi {
    pub(super) fn active_jobs(&self) -> usize {
        self.inflight.len()
    }
    pub(super) fn cache_bytes(&self) -> usize {
        self.covers
            .values()
            .map(|cover| cover.page.allocation)
            .sum()
    }
    pub(super) fn cache_count(&self) -> usize {
        self.covers.len()
    }
    pub(super) fn failure_count(&self) -> usize {
        self.failures.len()
    }
    pub(super) fn requested_count(&self) -> usize {
        self.requested_count
    }
    pub(super) fn visible_row_count(&self) -> usize {
        self.visible_rows.borrow().len()
    }
    pub(super) fn rendered_tile_count(&self) -> usize {
        self.visible_rows
            .borrow()
            .iter()
            .map(|&row| {
                self.model
                    .order
                    .len()
                    .saturating_sub(row * self.columns)
                    .min(self.columns)
            })
            .sum()
    }
    pub(super) fn wanted_cover_count(&self) -> usize {
        self.requested_range().len()
    }
    pub(super) fn cover_generation(&self) -> u64 {
        self.generation
    }
    pub(super) fn cached_images(&self) -> Vec<Arc<RenderImage>> {
        self.covers
            .values()
            .flat_map(|cover| cover.page.tiles.iter().map(|tile| tile.image.clone()))
            .collect()
    }
}

fn sort_text(sort: SortKey) -> Text {
    match sort {
        SortKey::Natural => Text::BrowserSortNatural,
        SortKey::Name => Text::BrowserSortName,
        SortKey::Modified => Text::BrowserSortModified,
        SortKey::Created => Text::BrowserSortCreated,
        SortKey::Size => Text::BrowserSortSize,
        SortKey::Type => Text::BrowserSortType,
        SortKey::Pages => Text::BrowserSortPages,
        SortKey::Progress => Text::BrowserSortProgress,
        SortKey::Recent => Text::BrowserSortRecent,
        SortKey::Random => Text::BrowserSortRandom,
    }
}

impl ReaderView {
    fn browser_anchor(&self) -> (Option<PathBuf>, f32) {
        let top = self.browser.grid.logical_scroll_top();
        if let Some((path, requested)) = &self.browser.scroll_anchor
            && top.item_ix == requested.item_ix
            && (top.offset_in_item - requested.offset_in_item).abs() < px(0.5)
        {
            return (Some(path.clone()), f32::from(top.offset_in_item));
        }
        let anchor = self.listing.as_ref().and_then(|listing| {
            self.browser
                .model
                .order
                .get(top.item_ix * self.browser.columns)
                .and_then(|&index| listing.entries.get(index))
                .map(|entry| entry.entry_path.clone())
        });
        (anchor, f32::from(top.offset_in_item))
    }

    fn reset_browser_grid(&mut self, anchor: (Option<PathBuf>, f32)) {
        self.browser.grid.reset_with_uniform_height(
            row_count(self.browser.model.order.len(), self.browser.columns),
            px(self.browser.row_height()),
        );
        self.browser.visible_rows.borrow_mut().clear();
        if let Some(position) = anchor.0.as_ref().and_then(|path| {
            let listing = self.listing.as_ref()?;
            self.browser
                .model
                .order
                .iter()
                .position(|&index| listing.entries[index].entry_path == *path)
        }) {
            let requested = ListOffset {
                item_ix: position / self.browser.columns,
                offset_in_item: px(anchor.1.max(0.)),
            };
            self.browser.scroll_anchor = anchor.0.map(|path| (path, requested));
            self.browser.grid.scroll_to(requested);
        } else {
            self.browser.scroll_anchor = None;
        }
    }

    pub(super) fn remember_browser_location(&mut self) {
        if let Some(listing) = &self.listing {
            let (anchor, offset) = self.browser_anchor();
            self.browser.snapshots.insert(
                listing.path.clone(),
                self.browser.model.snapshot(anchor, offset),
            );
        }
    }

    pub(super) fn rebuild_browser(&mut self, cx: &mut Context<Self>) {
        let anchor = self.browser_anchor();
        if let Some(listing) = &self.listing {
            self.browser.model.rebuild(listing, &self.reader.settings);
        }
        self.reset_browser_grid(anchor);
        cx.notify();
    }

    pub(super) fn browser_resume(&mut self, cx: &mut Context<Self>) {
        // The listing remains valid after reading; reject only pending work
        // from the previous view intent, not new visible-cover requests.
        self.browser.listing_revision = self.listing_revision;
        cx.notify();
    }

    pub(super) fn browser_listing_changed(&mut self, cx: &mut Context<Self>) {
        self.clear_browser_covers(cx);
        self.browser.listing_revision = self.listing_revision;
        let snapshot = self
            .listing
            .as_ref()
            .and_then(|listing| self.browser.snapshots.get(&listing.path))
            .cloned();
        if let Some(snapshot) = &snapshot {
            self.browser.model.restore(snapshot);
        } else {
            self.browser.model.query.clear();
            self.browser.model.selected = None;
        }
        self.browser.search_pending = true;
        if let (Some(book), Some(listing)) = (&self.reader.book, &self.listing) {
            let directory = if listing.path == book.path {
                Some(true)
            } else {
                listing
                    .entries
                    .iter()
                    .find(|entry| entry.path == book.path)
                    .map(|entry| entry.is_directory)
            };
            if directory.is_some() {
                self.browser
                    .model
                    .known_pages
                    .insert((book.path.clone(), self.book_recursive), book.pages.len());
            }
        }
        if let Some(listing) = &self.listing {
            self.browser.model.rebuild(listing, &self.reader.settings);
        }
        self.reset_browser_grid(
            snapshot.map_or((None, 0.), |snapshot| (snapshot.anchor, snapshot.offset)),
        );
        cx.notify();
    }

    pub(super) fn clear_browser_covers(&mut self, cx: &mut Context<Self>) {
        self.browser.generation = self.browser.generation.wrapping_add(1);
        let pages: Vec<_> = self
            .browser
            .covers
            .drain()
            .map(|(_, cover)| cover.page)
            .collect();
        // Refresh/Close can run while the current Window is borrowed. Release
        // from App after that borrow ends so every window atlas is reachable.
        cx.defer(move |cx| {
            for page in pages {
                page.release(None, cx);
            }
        });
        self.browser.failures.clear();
        self.browser.visible_rows.borrow_mut().clear();
        // Running synchronous reads cannot be cancelled. Keep their slots until completion.
    }

    pub(super) fn browser_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_browser_location();
        self.clear_browser_covers(cx);
        self.browser.model.known_pages.clear();
        self.browser.model.shuffle();
        if let Some(listing) = &self.listing {
            self.browse(listing.path.clone(), window, cx);
        }
    }

    pub(super) fn reveal_browser_selection(&mut self) {
        if let Some(listing) = &self.listing
            && let Some(position) = self.browser.model.order.iter().position(|&index| {
                Some(&listing.entries[index].entry_path) == self.browser.model.selected.as_ref()
            })
        {
            let requested = ListOffset {
                item_ix: position / self.browser.columns,
                offset_in_item: px(0.),
            };
            self.browser.scroll_anchor = self
                .browser
                .model
                .selected
                .clone()
                .map(|path| (path, requested));
            self.browser.grid.scroll_to(requested);
        }
    }

    pub(super) fn browser_move(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(listing) = &self.listing else {
            return;
        };
        let order = &self.browser.model.order;
        if order.is_empty() {
            return;
        }
        let current = order
            .iter()
            .position(|&index| {
                Some(&listing.entries[index].entry_path) == self.browser.model.selected.as_ref()
            })
            .unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(order.len() - 1);
        self.browser.model.selected = Some(listing.entries[order[next]].entry_path.clone());
        self.browser
            .grid
            .scroll_to_reveal_item(next / self.browser.columns);
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn browser_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry =
            self.listing
                .as_ref()
                .and_then(|listing| {
                    listing.entries.iter().find(|entry| {
                        Some(&entry.entry_path) == self.browser.model.selected.as_ref()
                    })
                })
                .cloned();
        if let Some(entry) = entry {
            self.remember_browser_location();
            if entry.is_directory {
                self.browse(entry.entry_path, window, cx);
            } else {
                self.open_browser_path(entry.entry_path, window, cx);
            }
        }
    }

    pub(super) fn browser_tick(&mut self, cx: &mut Context<Self>) {
        if self.content_mode != ContentMode::Browsing
            || self.closing
            || self.browser.listing_revision != self.listing_revision
        {
            return;
        }
        let Some(listing) = &self.listing else {
            return;
        };
        // Row render callbacks cannot borrow ListState. Capture the viewport
        // here, outside list rendering, before selecting cover dimensions.
        let height = f32::from(self.browser.grid.viewport_bounds().size.height);
        if self.browser.viewport_height != height {
            self.browser.viewport_height = height;
            cx.notify();
        }
        // ListState may repaint rows without rebuilding the parent view.
        // Use its current viewport, not all previously rendered rows.
        let range = self.browser.requested_range();
        self.browser.clock = self.browser.clock.wrapping_add(1);
        let stamp = self.browser.clock;
        let mut requests = Vec::new();
        for position in range {
            let entry = &listing.entries[self.browser.model.order[position]];
            let key = self.browser.cover_key(entry);
            if let Some(cover) = self.browser.covers.get_mut(&key) {
                cover.used = stamp;
            } else if !self.browser.failures.contains_key(&key)
                && !self
                    .browser
                    .inflight
                    .contains(&(self.browser.generation, key.clone()))
            {
                requests.push((key, entry.clone()));
            }
        }
        for (key, entry) in requests {
            if self.browser.inflight.len() >= MAX_COVER_JOBS {
                break;
            }
            let generation = self.browser.generation;
            let listing_revision = self.listing_revision;
            if !self.browser.inflight.insert((generation, key.clone())) {
                // Different native entries may alias the same canonical source.
                // Only one worker owns a given slot until its completion.
                continue;
            }
            #[cfg(test)]
            {
                self.browser.requested_count += 1;
            }
            let task_key = key.clone();
            let task = cx.background_spawn(async move {
                let source = book::read_cover(&entry)?;
                let decoded =
                    imaging::decode_thumbnail(&source.bytes, task_key.width, task_key.height)?;
                let page = cached_page(
                    vec![(decoded, image::Delay::from_numer_denom_ms(0, 1))],
                    source.bytes.len(),
                    generation,
                )?;
                Ok::<_, String>((page, source.page_count))
            });
            cx.spawn(async move |view, cx| {
                let result = task.await;
                _ = view.update(cx, |this, cx| {
                    this.browser.inflight.remove(&(generation, key.clone()));
                    if generation != this.browser.generation
                        || this.closing
                        || listing_revision != this.listing_revision
                    {
                        if let Ok((page, _)) = result {
                            page.release(None, cx);
                        }
                        return;
                    }
                    match result {
                        Ok((page, count)) => {
                            if let Some(count) = count {
                                // Folder covers are shallow and never supply recursive counts.
                                this.browser
                                    .model
                                    .known_pages
                                    .insert((key.path.clone(), false), count);
                            }
                            if page.allocation > COVER_CACHE_BYTES {
                                page.release(None, cx);
                                this.browser
                                    .failures
                                    .insert(key, "Cover exceeds cache budget".into());
                            } else {
                                let mut bytes: usize = this
                                    .browser
                                    .covers
                                    .values()
                                    .map(|cover| cover.page.allocation)
                                    .sum();
                                while bytes + page.allocation > COVER_CACHE_BYTES {
                                    let oldest = this
                                        .browser
                                        .covers
                                        .iter()
                                        .min_by_key(|(_, cover)| cover.used)
                                        .map(|(key, _)| key.clone());
                                    let Some(oldest) = oldest else {
                                        break;
                                    };
                                    if let Some(old) = this.browser.covers.remove(&oldest) {
                                        bytes -= old.page.allocation;
                                        old.page.release(None, cx);
                                    }
                                }
                                if let Some(old) = this.browser.covers.insert(
                                    key,
                                    Cover {
                                        page,
                                        used: this.browser.clock,
                                    },
                                ) {
                                    old.page.release(None, cx);
                                }
                            }
                        }
                        Err(error) => {
                            this.browser.failures.insert(key, error);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn browser_sort_menu(
        &self,
        mut menu: PopupMenu,
        owner: gpui_kit::WeakEntity<Self>,
    ) -> PopupMenu {
        let language = self.language();
        for sort in SortKey::ALL {
            let owner = owner.clone();
            menu = menu.item(
                PopupMenuItem::new(language.text(sort_text(sort)))
                    .checked(self.browser.model.sort == sort)
                    .on_click(move |_, _, cx| {
                        _ = owner.update(cx, |this, cx| {
                            this.browser.model.sort = sort;
                            this.rebuild_browser(cx);
                        });
                    }),
            );
        }
        menu
    }

    fn browser_tile(&self, position: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(entry) = self.listing.as_ref().and_then(|listing| {
            self.browser
                .model
                .order
                .get(position)
                .and_then(|&index| listing.entries.get(index))
        }) else {
            return div().into_any_element();
        };
        let key = self.browser.cover_key(entry);
        let selected = self.browser.model.selected.as_ref() == Some(&entry.entry_path);
        let language = self.language();
        let picture = if let Some(cover) = self.browser.covers.get(&key) {
            img(cover.page.tiles[0].image.clone())
                .size_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_3xl()
                .child(match entry.kind {
                    EntryKind::Folder => "▸",
                    EntryKind::Archive => "▣",
                    EntryKind::Image => "▧",
                })
                .into_any_element()
        };
        let tooltip = self.browser.failures.get(&key).map_or_else(
            || entry.entry_path.to_string_lossy().into_owned(),
            |error| language.format(Text::ErrorDetails, &[("error", error.clone())]),
        );
        let mut content = div().flex().flex_col().w_full().min_w_0().gap_1().child(
            div()
                .w_full()
                .h(px(self.browser.tile_width * 1.4))
                .child(picture),
        );
        if self.browser.show_names {
            content = content.child(
                div()
                    .w_full()
                    .h(px(self.browser.name_lines as f32 * 18. + 4.))
                    .line_height(px(18.))
                    .text_size(px(12.))
                    .line_clamp(self.browser.name_lines)
                    .child(entry.name.clone()),
            );
        }
        if self.browser.metadata {
            let pages = self
                .browser
                .model
                .page_count(entry, &self.reader.settings)
                .map_or_else(
                    || "—".to_owned(),
                    |count| {
                        language.format(Text::BrowserPageCount, &[("count", count.to_string())])
                    },
                );
            let progress = self
                .browser
                .model
                .progress(entry, &self.reader.settings)
                .map_or_else(|| "—".to_owned(), |value| format!("{:.0}%", value * 100.));
            content = content.child(
                div()
                    .text_xs()
                    .h(px(32.))
                    .overflow_hidden()
                    .child(format!("{pages} · {progress}")),
            );
        }
        let path = entry.entry_path.clone();
        Button::new(browser_entry_id(&entry.entry_path))
            .ghost()
            .p_2()
            .w(px(self.browser.tile_width + TILE_GAP))
            .h(px(self.browser.row_height() - 8.))
            .accessibility_label(entry.name.clone())
            .tooltip(tooltip)
            .selected(selected)
            .toggled(selected)
            .text_color(gpui_kit::rgb(0xe4e4e7))
            .border_1()
            .border_color(if selected {
                cx.theme().primary
            } else {
                cx.theme().transparent
            })
            .when(selected, |button| {
                button.bg(cx.theme().primary.opacity(0.18))
            })
            .child(content)
            .on_click(
                cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                    this.browser.model.selected = Some(path.clone());
                    this.focus.focus(window, cx);
                    if event.click_count() >= 2 {
                        this.browser_open(window, cx);
                    }
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    fn browser_row(&self, row: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        self.browser.visible_rows.borrow_mut().insert(row);
        let mut content = div().flex().gap_0().px_3().h(px(self.browser.row_height()));
        let start = row * self.browser.columns;
        let end = (start + self.browser.columns).min(self.browser.model.order.len());
        for position in start..end {
            content = content.child(self.browser_tile(position, cx));
        }
        content.into_any_element()
    }

    pub(super) fn render_browser(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let language = self.language();
        self.browser.visible_rows.borrow_mut().clear();
        let owner = cx.entity().downgrade();
        let sort_owner = owner.clone();
        let parent = self
            .listing
            .as_ref()
            .and_then(|listing| listing.parent.clone());
        let path = self
            .listing
            .as_ref()
            .map(|listing| listing.path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let toolbar = div()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .flex_none()
            .child(div().text_xs().truncate().child(path))
            .child(
                Input::new(&self.browser.search)
                    .id("browser-search")
                    .aria_label(language.text(Text::BrowserSearch))
                    .small(),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .child(
                        Button::new("browser-parent")
                            .small()
                            .ghost()
                            .label(language.text(Text::ParentFolder))
                            .disabled(parent.is_none())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(parent) = &parent {
                                    this.remember_browser_location();
                                    this.browse(parent.clone(), window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("browser-sort")
                            .small()
                            .ghost()
                            .accessibility_label(language.text(Text::BrowserSort))
                            .label(language.text(sort_text(self.browser.model.sort)))
                            .dropdown_menu(move |menu, _, cx| {
                                if let Some(view) = sort_owner.upgrade() {
                                    view.read(cx).browser_sort_menu(menu, sort_owner.clone())
                                } else {
                                    menu
                                }
                            }),
                    )
                    .child(
                        Button::new("browser-reverse")
                            .small()
                            .ghost()
                            .label(language.text(Text::BrowserReverse))
                            .selected(self.browser.model.descending)
                            .toggled(self.browser.model.descending)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.browser.model.descending = !this.browser.model.descending;
                                this.rebuild_browser(cx);
                            })),
                    )
                    .child(
                        Button::new("browser-folders-first")
                            .small()
                            .ghost()
                            .label(language.text(Text::BrowserFoldersFirst))
                            .selected(self.browser.model.folders_first)
                            .toggled(self.browser.model.folders_first)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.browser.model.folders_first =
                                    !this.browser.model.folders_first;
                                this.rebuild_browser(cx);
                            })),
                    )
                    .child(
                        Button::new("browser-shuffle")
                            .small()
                            .ghost()
                            .label(language.text(Text::BrowserShuffle))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.browser.model.sort = SortKey::Random;
                                this.browser.model.shuffle();
                                this.rebuild_browser(cx);
                            })),
                    )
                    .child(self.button(
                        "browser-refresh",
                        language.text(Text::BrowserRefresh),
                        Command::BrowserRefresh,
                        cx,
                    ))
                    .child(
                        self.button(
                            "browser-open",
                            language.text(Text::BrowserOpen),
                            Command::BrowserOpen,
                            cx,
                        )
                        .disabled(self.browser.model.selected.is_none()),
                    )
                    .child(
                        self.button(
                            "browser-resume",
                            language.text(Text::ContinueReading),
                            Command::ContinueReading,
                            cx,
                        )
                        .disabled(self.reader.book.is_none()),
                    ),
            )
            .when(
                matches!(self.browser.model.sort, SortKey::Pages | SortKey::Progress),
                |toolbar| {
                    toolbar.child(div().text_xs().child(language.text(Text::BrowserKnownHint)))
                },
            );
        let view = owner.clone();
        let mut body = div()
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                div().id("browser-grid").test_support().size_full().child(
                    list(self.browser.grid.clone(), move |row, _, cx| {
                        view.update(cx, |this, cx| this.browser_row(row, cx))
                            .unwrap_or_else(|_| div().into_any_element())
                    })
                    .size_full(),
                ),
            )
            .child(Scrollbar::vertical(&self.browser.grid).mode(ScrollbarMode::Always));
        if self.browser.model.order.is_empty() {
            let text = if self.listing.is_none() {
                Text::FolderLoading
            } else if self.browser.model.query.trim().is_empty() {
                Text::BrowserEmpty
            } else {
                Text::BrowserNoMatches
            };
            body = body.child(div().absolute().inset_0().p_4().child(language.text(text)));
        }
        let mut content = div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(body);
        if let Some(error) = &self.listing_error {
            content = content.child(
                div()
                    .p_3()
                    .text_color(cx.theme().danger)
                    .child(language.format(Text::FolderError, &[("error", error.clone())])),
            );
        }
        let scale = window.scale_factor();
        div()
            .id("viewer")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::BrowserArea))
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .bg(gpui_kit::rgb(0x000000))
            .text_color(gpui_kit::rgb(0xe4e4e7))
            .overflow_hidden()
            .child(content)
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let owner = owner.clone();
                        window.defer(cx, move |window, cx| {
                            _ = owner.update(cx, |this, cx| {
                                if this.content_mode != ContentMode::Browsing {
                                    return;
                                }
                                this.viewer_bounds = bounds;
                                this.viewer_size = bounds.size;
                                let new_columns = columns(
                                    (f32::from(bounds.size.width) - 24.).max(1.),
                                    this.browser.tile_width + TILE_GAP,
                                );
                                if new_columns != this.browser.columns {
                                    let anchor = this.browser_anchor();
                                    this.browser.columns = new_columns;
                                    this.reset_browser_grid(anchor);
                                    cx.notify();
                                }
                                if this.browser.scale != scale {
                                    this.browser.scale = scale;
                                    cx.notify();
                                }
                                let placeholder = this.language().text(Text::BrowserSearch);
                                if this.browser.search_language != Some(this.language()) {
                                    this.browser.search_language = Some(this.language());
                                    this.browser.search.update(cx, |input, cx| {
                                        input.set_placeholder(placeholder, window, cx)
                                    });
                                }
                                if this.browser.search_pending {
                                    this.browser.search_pending = false;
                                    let value = this.browser.model.query.clone();
                                    this.browser
                                        .search
                                        .update(cx, |input, cx| input.set_value(value, window, cx));
                                }
                            });
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }

    pub(super) fn render_browser_tools(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
        div()
            .id("browser-tools")
            .capture_any_mouse_down(cx.listener(|this, _, _, _| {
                // A new control gesture starts from the current first visible
                // item. Preserve that item across the gesture's change events.
                this.browser.scroll_anchor = None;
            }))
            .test_support()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .child(div().text_sm().child(language.text(Text::BrowserTab)))
            .child(div().text_xs().child(format!(
                "{}: {:.0}",
                language.text(Text::BrowserTileWidth),
                self.browser.tile_width
            )))
            .child(
                div().id("browser-tile-width").test_support().child(
                    Slider::new(&self.browser.tile_slider)
                        .accessibility_label(language.text(Text::BrowserTileWidth)),
                ),
            )
            .child(
                Button::new("browser-show-names")
                    .small()
                    .ghost()
                    .label(language.text(Text::BrowserShowNames))
                    .selected(self.browser.show_names)
                    .toggled(self.browser.show_names)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let anchor = this.browser_anchor();
                        this.browser.show_names = !this.browser.show_names;
                        this.reset_browser_grid(anchor);
                        cx.notify();
                    })),
            )
            .child(div().text_xs().child(language.text(Text::BrowserNameLines)))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .children([2, 3].into_iter().map(|lines| {
                        Button::new(("browser-name-lines", lines))
                            .small()
                            .ghost()
                            .label(lines.to_string())
                            .accessibility_label(format!(
                                "{}: {lines}",
                                language.text(Text::BrowserNameLines)
                            ))
                            .selected(self.browser.name_lines == lines)
                            .toggled(self.browser.name_lines == lines)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let anchor = this.browser_anchor();
                                this.browser.name_lines = lines;
                                this.reset_browser_grid(anchor);
                                cx.notify();
                            }))
                    })),
            )
            .child(
                Button::new("browser-metadata")
                    .small()
                    .ghost()
                    .label(language.text(Text::BrowserMetadata))
                    .selected(self.browser.metadata)
                    .toggled(self.browser.metadata)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let anchor = this.browser_anchor();
                        this.browser.metadata = !this.browser.metadata;
                        this.reset_browser_grid(anchor);
                        cx.notify();
                    })),
            )
            .child(div().text_xs().child(language.text(Text::BrowserKnownHint)))
            .into_any_element()
    }
}
