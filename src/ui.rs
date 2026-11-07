//! Native reader window, commands, and asynchronous image loading.

use crate::{
    book::{self, Book, DirectoryListing, EntryKind},
    imaging::{self, Filters},
    instance::{self, Instance, PrimaryInstance},
    reader::Reader,
    settings::{AppSettings, Background, FitMode, ReadingMode, SidebarMode},
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenu},
    scroll::{Scrollbar, ScrollbarMode},
    slider::{Slider, SliderEvent, SliderState},
};
use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, ExternalPaths, FocusHandle, InteractiveElement,
    IntoElement, KeyBinding, ListAlignment, ListOffset, ListState, Menu, MenuItem, MouseButton,
    ObjectFit, ParentElement, Pixels, Render, RenderImage, SharedString, Size,
    StatefulInteractiveElement, Styled, StyledImage, Subscription, TestSupportExt, Window,
    WindowBounds, WindowOptions, canvas, div, img, list, point, prelude::FluentBuilder, px, size,
};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, gpui_kit::Action)]
#[action(namespace = reader, no_json)]
enum Command {
    OpenFile,
    OpenFolder,
    SavePage,
    Close,
    Quit,
    Sidebar,
    Thumbnails,
    Spread,
    Continuous,
    Direction,
    Fullscreen,
    FitWindow,
    FitWidth,
    FitHeight,
    Original,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    First,
    Previous,
    Next,
    Last,
    Left,
    Right,
    ScrollUp,
    ScrollDown,
    Slideshow,
    Bookmark,
    Resume,
    ClearBookmark,
    RotateLeft,
    RotateRight,
    ResetFilters,
    Settings,
    About,
    ClearHistory,
    Escape,
    OpenListedFolder,
}

const PAGE_CACHE_BYTES: usize = 256 * 1024 * 1024;
const THUMB_CACHE_BYTES: usize = 24 * 1024 * 1024;
const MAX_TEXTURE_SIDE: u32 = 8192;
const MAX_PAGE_REQUESTS: usize = 2;
const MAX_THUMB_REQUESTS: usize = 2;
const TICK: Duration = Duration::from_millis(80);

type RequestKey = (u64, u64, usize, bool);
type VisibleRequests = Rc<RefCell<BTreeSet<(usize, bool)>>>;

struct RenderedTile {
    image: Arc<RenderImage>,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

struct CachedPage {
    tiles: Vec<RenderedTile>,
    width: u32,
    height: u32,
    source_bytes: usize,
    allocation: usize,
    revision: u64,
}

impl CachedPage {
    fn release(self, mut window: Option<&mut Window>, cx: &mut App) {
        for tile in self.tiles {
            cx.drop_image(tile.image, window.as_deref_mut());
        }
    }
}

// Split long source pages into GPU-sized textures without changing their
// display geometry or source resolution. All tiles retain the same frame delays.
fn cached_page(
    decoded: Vec<(imaging::DecodedPage, image::Delay)>,
    source_bytes: usize,
    revision: u64,
) -> Result<CachedPage, String> {
    let first = decoded.first().ok_or("Image contains no frames")?;
    let (width, height) = (first.0.width, first.0.height);
    let mut coordinates = Vec::new();
    for y in (0..height).step_by(MAX_TEXTURE_SIDE as usize) {
        for x in (0..width).step_by(MAX_TEXTURE_SIDE as usize) {
            coordinates.push((
                x,
                y,
                (width - x).min(MAX_TEXTURE_SIDE),
                (height - y).min(MAX_TEXTURE_SIDE),
            ));
        }
    }
    let mut tile_frames: Vec<Vec<image::Frame>> = coordinates
        .iter()
        .map(|_| Vec::with_capacity(decoded.len()))
        .collect();
    let mut allocation = 0;
    for (page, delay) in decoded {
        if (page.width, page.height) != (width, height) {
            return Err("Animation frame canvas changed size".into());
        }
        allocation += page.pixels.len();
        let buffer = image::RgbaImage::from_raw(width, height, page.into_bgra())
            .ok_or("Invalid decoded image")?;
        if coordinates.len() == 1 {
            tile_frames[0].push(image::Frame::from_parts(buffer, 0, 0, delay));
        } else {
            for ((x, y, width, height), frames) in coordinates.iter().zip(&mut tile_frames) {
                let tile = image::imageops::crop_imm(&buffer, *x, *y, *width, *height).to_image();
                frames.push(image::Frame::from_parts(tile, 0, 0, delay));
            }
        }
    }
    let tiles = coordinates
        .into_iter()
        .zip(tile_frames)
        .map(|((x, y, width, height), frames)| RenderedTile {
            image: Arc::new(RenderImage::new(frames)),
            x,
            y,
            width,
            height,
        })
        .collect();
    Ok(CachedPage {
        tiles,
        width,
        height,
        source_bytes,
        allocation,
        revision,
    })
}

#[derive(Clone, Copy)]
struct PageGeometry {
    width: u32,
    height: u32,
    rotation: i32,
}

pub struct ReaderView {
    reader: Reader,
    settings_path: PathBuf,
    focus: FocusHandle,
    pages: HashMap<usize, CachedPage>,
    thumbs: HashMap<usize, CachedPage>,
    geometry: HashMap<usize, PageGeometry>,
    visible: Rc<RefCell<HashSet<usize>>>,
    visible_thumbnails: Rc<RefCell<HashSet<usize>>>,
    errors: HashMap<(usize, bool), String>,
    inflight: HashSet<RequestKey>,
    wanted: VisibleRequests,
    book_revision: u64,
    image_revision: u64,
    open_revision: u64,
    listing_revision: u64,
    opening: bool,
    listing: Option<DirectoryListing>,
    listing_error: Option<String>,
    continuous: ListState,
    continuous_target: Option<usize>,
    thumbnails: ListState,
    files: ListState,
    paged_scroll: gpui_kit::ScrollHandle,
    horizontal_scroll: gpui_kit::ScrollHandle,
    viewer_size: Size<Pixels>,
    viewer_bounds: Bounds<Pixels>,
    controls_pending: bool,
    path_pending: bool,
    page_input: Entity<InputState>,
    zoom_input: Entity<InputState>,
    path_input: Entity<InputState>,
    interval_input: Entity<InputState>,
    page_slider: Entity<SliderState>,
    filter_sliders: [Entity<SliderState>; 3],
    inspector: bool,
    preferences_open: bool,
    message: String,
    message_error: bool,
    dirty: bool,
    saving: bool,
    save_lock: Arc<Mutex<()>>,
    save_epoch: Arc<AtomicU64>,
    save_error: Option<String>,
    filter_changed: Option<Instant>,
    next_slide: Instant,
    instance: Option<PrimaryInstance>,
    launches: mpsc::Receiver<Vec<PathBuf>>,
    pending_drop: Option<PathBuf>,
    _subscriptions: Vec<Subscription>,
}

impl ReaderView {
    pub fn new(
        settings: AppSettings,
        settings_path: PathBuf,
        warning: Option<String>,
        instance: Option<PrimaryInstance>,
        launches: mpsc::Receiver<Vec<PathBuf>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let page_input = cx.new(|cx| InputState::new(window, cx).default_value("1"));
        let zoom_input = cx.new(|cx| InputState::new(window, cx).default_value("100"));
        let path_input = cx.new(|cx| InputState::new(window, cx).placeholder("資料夾路徑"));
        let interval_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.preferences.interval.to_string())
        });
        let page_slider = cx.new(|_| SliderState::new().min(0.).max(100.).step(0.01));
        let filter_sliders = [
            cx.new(|_| SliderState::new().min(50.).max(150.).default_value(100.)),
            cx.new(|_| SliderState::new().min(50.).max(150.).default_value(100.)),
            cx.new(|_| SliderState::new().min(0.).max(100.).default_value(0.)),
        ];
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &page_input,
            window,
            |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    match input.read(cx).value().trim().parse::<usize>() {
                        Ok(page) => this.navigate(page.saturating_sub(1), window, cx),
                        Err(_) => this.set_message("頁碼必須是正整數。", true, cx),
                    }
                    this.focus.focus(window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &zoom_input,
            window,
            |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let value = input.read(cx).value();
                    match value.trim().trim_end_matches('%').parse::<f32>() {
                        Ok(percent) if percent.is_finite() => {
                            this.reader.set_zoom(percent / 100.);
                            this.relayout(cx);
                            this.sync_controls(window, cx);
                        }
                        _ => this.set_message("縮放必須是 10–800%。", true, cx),
                    }
                    this.focus.focus(window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &path_input,
            window,
            |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let path = PathBuf::from(input.read(cx).value().as_ref());
                    this.browse(path, window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &interval_input,
            window,
            |this, input, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    match input.read(cx).value().trim().parse::<u64>() {
                        Ok(seconds) => {
                            this.reader.settings.preferences.interval = seconds.clamp(1, 120);
                            this.next_slide =
                                Instant::now() + Duration::from_secs(seconds.clamp(1, 120));
                            this.dirty = true;
                            cx.notify();
                        }
                        Err(_) => this.set_message("投影片間隔必須是 1–120 秒。", true, cx),
                    }
                }
            },
        ));
        subscriptions.push(
            cx.subscribe_in(&page_slider, window, |this, _, event, window, cx| {
                if let SliderEvent::Change(value) = event {
                    let count = this.page_count().saturating_sub(1);
                    this.navigate(
                        (value.start() / 100. * count as f32).round() as usize,
                        window,
                        cx,
                    );
                }
            }),
        );
        for (filter, slider) in filter_sliders.iter().enumerate() {
            subscriptions.push(
                cx.subscribe_in(slider, window, move |this, _, event, _, cx| {
                    if let SliderEvent::Change(value) = event {
                        match filter {
                            0 => this.reader.filters.brightness = value.start(),
                            1 => this.reader.filters.contrast = value.start(),
                            _ => this.reader.filters.grayscale = value.start(),
                        }
                        this.filter_changed = Some(Instant::now());
                        cx.notify();
                    }
                }),
            );
        }
        subscriptions.push(cx.on_app_quit(|this, _| {
            let result = this.flush_settings();
            async move {
                if let Err(error) = result {
                    eprintln!("{error}");
                }
            }
        }));
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            if let Some(view) = weak.upgrade() {
                view.update(cx, |this, _| {
                    if let Err(error) = this.flush_settings() {
                        eprintln!("{error}");
                    }
                });
            }
            window.remove_window();
            cx.quit();
            false
        });
        let continuous = ListState::new(0, ListAlignment::Top, px(500.));
        let thumbnails = ListState::new(0, ListAlignment::Top, px(300.));
        let mut view = Self {
            reader: Reader::new(settings),
            settings_path,
            focus,
            pages: HashMap::new(),
            thumbs: HashMap::new(),
            geometry: HashMap::new(),
            visible: Rc::default(),
            visible_thumbnails: Rc::default(),
            errors: HashMap::new(),
            inflight: HashSet::new(),
            wanted: Rc::default(),
            book_revision: 0,
            image_revision: 0,
            open_revision: 0,
            listing_revision: 0,
            opening: false,
            listing: None,
            listing_error: None,
            continuous,
            continuous_target: None,
            thumbnails,
            files: ListState::new(0, ListAlignment::Top, px(200.)),
            paged_scroll: Default::default(),
            horizontal_scroll: Default::default(),
            viewer_size: size(px(800.), px(600.)),
            viewer_bounds: Bounds::default(),
            controls_pending: false,
            path_pending: false,
            page_input,
            zoom_input,
            path_input,
            interval_input,
            page_slider,
            filter_sliders,
            inspector: true,
            preferences_open: false,
            message: warning.clone().unwrap_or_else(|| "就緒".into()),
            message_error: warning.is_some(),
            dirty: false,
            saving: false,
            save_lock: Arc::default(),
            save_epoch: Arc::new(AtomicU64::new(0)),
            save_error: warning,
            filter_changed: None,
            next_slide: Instant::now(),
            instance,
            launches,
            pending_drop: None,
            _subscriptions: subscriptions,
        };
        let home = directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        view.browse(home, window, cx);
        cx.spawn_in(window, async move |view, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if view
                    .update_in(cx, |view, window, cx| view.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        view
    }

    fn clear_images(&mut self, mut window: Option<&mut Window>, cx: &mut Context<Self>) {
        for (_, page) in self.pages.drain().chain(self.thumbs.drain()) {
            page.release(window.as_deref_mut(), cx);
        }
    }

    fn page_count(&self) -> usize {
        self.reader.book.as_ref().map_or(0, |book| book.pages.len())
    }

    fn set_message(&mut self, message: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        self.message = message.into();
        self.message_error = error;
        cx.notify();
    }

    fn sync_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = (self.reader.current + 1).to_string();
        self.page_input
            .update(cx, |input, cx| input.set_value(page, window, cx));
        let zoom = format!("{:.0}", self.reader.zoom * 100.);
        self.zoom_input
            .update(cx, |input, cx| input.set_value(zoom, window, cx));
        let maximum = self.page_count().saturating_sub(1).max(1);
        let progress = self.reader.current as f32 / maximum as f32 * 100.;
        self.page_slider
            .update(cx, |slider, cx| slider.set_value(progress, window, cx));
        self.thumbnails.scroll_to_reveal_item(self.reader.current);
        self.dirty = true;
        cx.notify();
    }

    fn flush_settings(&self) -> Result<(), String> {
        let revision = self.save_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        save_snapshot(
            &self.reader.settings,
            &self.settings_path,
            &self.save_lock,
            &self.save_epoch,
            revision,
        )
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.has_active_drag() && self.pending_drop.take().is_some() {
            cx.notify();
        }
        if self.controls_pending {
            self.controls_pending = false;
            self.sync_controls(window, cx);
            if let Some(book) = &self.reader.book {
                window.set_window_title(&format!("{} — MangaMeeya Rust", book.title));
            }
        }
        if self.path_pending {
            self.path_pending = false;
            if let Some(listing) = &self.listing {
                let path = listing.path.to_string_lossy().into_owned();
                self.path_input
                    .update(cx, |input, cx| input.set_value(path, window, cx));
            }
        }
        let mut requested_path = None;
        while let Some(launch) = self.instance.as_ref().and_then(PrimaryInstance::try_recv) {
            window.activate_window();
            cx.activate(true);
            if let Some(path) = launch.into_iter().next() {
                requested_path = Some(path);
            }
        }
        for launch in self.launches.try_iter() {
            if let Some(path) = launch.into_iter().next() {
                requested_path = Some(path);
            }
        }
        if let Some(path) = requested_path {
            window.activate_window();
            self.open_path(path, window, cx);
        }
        if self
            .filter_changed
            .is_some_and(|time| time.elapsed() >= Duration::from_millis(120))
        {
            self.filter_changed = None;
            self.image_revision += 1;
            self.errors.retain(|(_, thumb), _| *thumb);
            self.relayout(cx);
        }
        if self.reader.slideshow && Instant::now() >= self.next_slide {
            if self.reader.current + 1 < self.page_count() {
                self.reader.next();
                self.after_navigation(window, cx);
                self.next_slide =
                    Instant::now() + Duration::from_secs(self.reader.settings.preferences.interval);
            } else {
                self.reader.slideshow = false;
                self.set_message("投影片已結束。", false, cx);
            }
        }
        if self.reader.settings.preferences.mode == ReadingMode::Continuous
            && self.reader.book.is_some()
            && self.continuous_target.is_none()
        {
            let top = self.continuous.logical_scroll_top().item_ix;
            let center = self.viewer_bounds.origin.y + self.viewer_bounds.size.height / 2.;
            let current = (top..self.page_count())
                .take(128)
                .find(|index| {
                    self.continuous
                        .bounds_for_item(*index)
                        .is_some_and(|bounds| bounds.bottom() >= center)
                })
                .unwrap_or(top)
                .min(self.page_count().saturating_sub(1));
            if self.reader.current != current {
                self.reader.navigate(current);
                self.sync_controls(window, cx);
            }
        }
        if self.dirty && !self.saving {
            self.dirty = false;
            self.saving = true;
            let settings = self.reader.settings.clone();
            let path = self.settings_path.clone();
            let lock = self.save_lock.clone();
            let epoch = self.save_epoch.clone();
            let revision = epoch.fetch_add(1, Ordering::SeqCst) + 1;
            let save = cx.background_spawn(async move {
                save_snapshot(&settings, &path, &lock, &epoch, revision)
            });
            cx.spawn(async move |view, cx| {
                let result = save.await;
                _ = view.update(cx, |view, cx| {
                    view.saving = false;
                    if let Err(error) = result {
                        if view.save_error.as_deref() != Some(&error) {
                            view.save_error = Some(error.clone());
                            view.set_message(error, true, cx);
                        }
                    } else if view.save_error.take().is_some() {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        self.load_requested(cx);
        self.evict_cache(true, Some(window), cx);
    }

    fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.open_revision += 1;
        let revision = self.open_revision;
        let recursive = self.reader.settings.preferences.recursive_folder;
        let explicit_image = path.is_file()
            && book::IMAGE_EXTENSIONS.contains(
                &path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str(),
            );
        self.opening = true;
        self.reader.slideshow = false;
        self.set_message(format!("正在開啟 {}…", path.display()), false, cx);
        let task = cx.background_spawn(async move { Book::open(&path, recursive) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            _ = view.update(cx, |view, cx| {
                if revision != view.open_revision {
                    return;
                }
                view.opening = false;
                match result {
                    Ok(book) => {
                        let explicit = explicit_image.then_some(book.initial_page);
                        view.reader.set_book(Arc::new(book), explicit);
                        view.book_revision += 1;
                        view.image_revision = 0;
                        view.clear_images(None, cx);
                        view.geometry.clear();
                        view.visible.borrow_mut().clear();
                        view.visible_thumbnails.borrow_mut().clear();
                        view.errors.clear();
                        view.wanted.borrow_mut().clear();
                        let count = view.page_count();
                        view.continuous
                            .reset_with_uniform_height(count, view.viewer_size.height);
                        view.thumbnails.reset_with_uniform_height(count, px(174.));
                        view.continuous.scroll_to(ListOffset {
                            item_ix: view.reader.current,
                            offset_in_item: px(0.),
                        });
                        view.continuous_target = Some(view.reader.current);
                        view.paged_scroll.set_offset(point(px(0.), px(0.)));
                        view.dirty = true;
                        view.controls_pending = true;
                        view.message = format!(
                            "已開啟 {}（{} 頁）",
                            view.reader.book.as_ref().unwrap().title,
                            count
                        );
                        view.message_error = false;
                        let current = view.reader.current;
                        for index in current.saturating_sub(1)..(current + 3).min(count) {
                            view.wanted.borrow_mut().insert((index, false));
                        }
                        cx.notify();
                    }
                    Err(error) => view.set_message(error, true, cx),
                }
            });
        })
        .detach();
    }

    fn browse(&mut self, path: PathBuf, _: &mut Window, cx: &mut Context<Self>) {
        self.listing_revision += 1;
        let revision = self.listing_revision;
        self.listing_error = None;
        let task = cx.background_spawn(async move { book::browse_directory(&path) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            _ = view.update(cx, |view, cx| {
                if revision != view.listing_revision {
                    return;
                }
                match result {
                    Ok(listing) => {
                        view.files
                            .reset_with_uniform_height(listing.entries.len(), px(32.));
                        view.listing = Some(listing);
                        view.path_pending = true;
                    }
                    Err(error) => view.listing_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn navigate(&mut self, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.reader.navigate(page);
        self.after_navigation(window, cx);
    }

    fn after_navigation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reader.settings.preferences.mode == ReadingMode::Continuous {
            self.continuous.scroll_to(ListOffset {
                item_ix: self.reader.current,
                offset_in_item: px(0.),
            });
            self.continuous_target = Some(self.reader.current);
        } else {
            self.paged_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.horizontal_scroll.set_offset(point(px(0.), px(0.)));
        self.sync_controls(window, cx);
        let current = self.reader.current;
        let count = self.page_count();
        for index in current..(current + 3).min(count) {
            self.wanted.borrow_mut().insert((index, false));
        }
        self.load_requested(cx);
    }

    fn relayout(&mut self, cx: &mut Context<Self>) {
        self.continuous.remeasure();
        self.dirty = true;
        cx.notify();
    }

    fn request_dialog(&mut self, directory: bool, window: &mut Window, cx: &mut Context<Self>) {
        let choice = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: false,
            prompt: Some(
                if directory {
                    "開啟圖片資料夾"
                } else {
                    "開啟漫畫或圖片"
                }
                .into(),
            ),
        });
        cx.spawn_in(window, async move |view, cx| match choice.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    _ = view.update_in(cx, |view, window, cx| view.open_path(path, window, cx));
                }
            }
            Ok(Ok(None)) => {}
            error => {
                _ = view.update_in(cx, |view, _, cx| {
                    view.set_message(format!("無法開啟選擇視窗：{error:?}"), true, cx)
                });
            }
        })
        .detach();
    }

    fn save_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(book) = self.reader.book.clone() else {
            return;
        };
        let index = self.reader.current;
        let name = Path::new(&book.pages[index].name)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let directory = book.path.parent().unwrap_or_else(|| Path::new("."));
        let choice = cx.prompt_for_new_path(directory, Some(&name));
        cx.spawn_in(window, async move |view, cx| {
            let destination = match choice.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) => return,
                error => {
                    _ = view.update_in(cx, |view, _, cx| {
                        view.set_message(format!("無法選擇儲存位置：{error:?}"), true, cx)
                    });
                    return;
                }
            };
            let task = cx.background_spawn(async move { book.save_page(index, &destination) });
            let result = task.await;
            _ = view.update_in(cx, |view, _, cx| match result {
                Ok(()) => view.set_message("已儲存原始圖片。", false, cx),
                Err(error) => view.set_message(error, true, cx),
            });
        })
        .detach();
    }

    fn perform(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        use Command::*;
        match command {
            OpenFile => self.request_dialog(false, window, cx),
            OpenFolder => self.request_dialog(true, window, cx),
            SavePage => self.save_page(window, cx),
            Quit => cx.quit(),
            Close => {
                self.open_revision += 1;
                self.book_revision += 1;
                self.reader.close();
                self.clear_images(Some(window), cx);
                self.geometry.clear();
                self.visible.borrow_mut().clear();
                self.visible_thumbnails.borrow_mut().clear();
                self.errors.clear();
                self.wanted.borrow_mut().clear();
                self.continuous.reset(0);
                self.thumbnails.reset(0);
                self.opening = false;
                self.sync_controls(window, cx);
                window.set_window_title("MangaMeeya Rust");
                self.set_message("已關閉漫畫。", false, cx);
            }
            Sidebar => {
                self.reader.settings.preferences.sidebar =
                    !self.reader.settings.preferences.sidebar;
                self.relayout(cx);
            }
            Thumbnails => {
                self.reader.settings.preferences.sidebar = true;
                self.reader.settings.preferences.sidebar_mode = SidebarMode::Thumbnails;
                self.relayout(cx);
            }
            Spread => {
                let mode = if self.reader.settings.preferences.mode == ReadingMode::Spread {
                    ReadingMode::Single
                } else {
                    ReadingMode::Spread
                };
                self.reader.set_mode(mode);
                self.relayout(cx);
                self.after_navigation(window, cx);
            }
            Continuous => {
                let mode = if self.reader.settings.preferences.mode == ReadingMode::Continuous {
                    ReadingMode::Single
                } else {
                    ReadingMode::Continuous
                };
                self.reader.set_mode(mode);
                self.relayout(cx);
                self.after_navigation(window, cx);
            }
            Direction => {
                self.reader.settings.preferences.rtl = !self.reader.settings.preferences.rtl;
                self.dirty = true;
                cx.notify();
            }
            Fullscreen => window.toggle_fullscreen(),
            Escape => {
                if window.is_fullscreen() {
                    window.toggle_fullscreen();
                }
                self.focus.focus(window, cx);
            }
            FitWindow | FitWidth | FitHeight | Original => {
                let fit = match command {
                    FitWidth => FitMode::Width,
                    FitHeight => FitMode::Height,
                    Original => FitMode::Original,
                    _ => FitMode::Window,
                };
                self.reader.set_fit(fit);
                self.relayout(cx);
                self.sync_controls(window, cx);
            }
            ZoomIn | ZoomOut | ZoomReset => {
                if command == ZoomReset {
                    self.reader.set_zoom(1.);
                } else {
                    self.reader
                        .zoom_by(if command == ZoomIn { 1.15 } else { 1. / 1.15 });
                }
                self.relayout(cx);
                self.sync_controls(window, cx);
            }
            First | Last | Next | Previous | Left | Right => {
                match command {
                    First => self.reader.navigate(0),
                    Last => self.reader.navigate(self.page_count().saturating_sub(1)),
                    Next => self.reader.next(),
                    Previous => self.reader.previous(),
                    Left if self.reader.settings.preferences.rtl => self.reader.next(),
                    Right if !self.reader.settings.preferences.rtl => self.reader.next(),
                    _ => self.reader.previous(),
                };
                self.after_navigation(window, cx);
            }
            ScrollUp | ScrollDown => {
                self.continuous_target = None;
                let amount = if command == ScrollUp { -80. } else { 80. };
                if self.reader.settings.preferences.mode == ReadingMode::Continuous {
                    self.continuous.scroll_by(px(amount));
                } else {
                    let offset = self.paged_scroll.offset();
                    self.paged_scroll
                        .set_offset(point(offset.x, offset.y - px(amount)));
                }
                cx.notify();
            }
            Slideshow => {
                if self.reader.book.is_some() {
                    self.reader.slideshow = !self.reader.slideshow;
                    self.next_slide = Instant::now()
                        + Duration::from_secs(self.reader.settings.preferences.interval);
                    cx.notify();
                }
            }
            Bookmark | Resume | ClearBookmark => {
                if let Some(book) = &self.reader.book {
                    let path = book.path.clone();
                    match command {
                        Bookmark => {
                            self.reader
                                .settings
                                .bookmarks
                                .insert(path, self.reader.current);
                            self.set_message(
                                format!("書籤：第 {} 頁", self.reader.current + 1),
                                false,
                                cx,
                            );
                        }
                        Resume => match self.reader.settings.bookmarks.get(&path).copied() {
                            Some(page) => self.navigate(page, window, cx),
                            None => self.set_message("本書尚無書籤。", true, cx),
                        },
                        _ => {
                            self.reader.settings.bookmarks.remove(&path);
                            self.set_message("已刪除本書書籤。", false, cx);
                        }
                    }
                    self.dirty = true;
                }
            }
            RotateLeft | RotateRight => {
                self.reader.rotation = (self.reader.rotation
                    + if command == RotateLeft { -90 } else { 90 })
                .rem_euclid(360);
                self.image_revision += 1;
                self.errors.retain(|(_, thumb), _| *thumb);
                self.relayout(cx);
            }
            ResetFilters => {
                self.reader.filters = Filters::default();
                self.image_revision += 1;
                self.filter_changed = None;
                self.errors.retain(|(_, thumb), _| *thumb);
                for (slider, value) in self.filter_sliders.iter().zip([100., 100., 0.]) {
                    slider.update(cx, |slider, cx| slider.set_value(value, window, cx));
                }
                self.relayout(cx);
            }
            Settings => {
                self.preferences_open = !self.preferences_open;
                self.inspector = true;
                cx.notify();
            }
            About => window.open_dialog(cx, |dialog, _, _| {
                dialog
                    .title("關於 MangaMeeya Rust")
                    .child(concat!("MangaMeeya Rust ", env!("CARGO_PKG_VERSION")))
                    .child("Rust · GPUI Kit · Windows / macOS")
                    .child("離線閱讀圖片資料夾、ZIP/CBZ 與 RAR/CBR。原始漫畫不會被修改。")
                    .child("AGPL-3.0-only")
            }),
            ClearHistory => {
                self.reader.settings.recent.clear();
                self.dirty = true;
                cx.notify();
            }
            OpenListedFolder => {
                if let Some(listing) = &self.listing {
                    self.open_path(listing.path.clone(), window, cx);
                }
            }
        }
    }

    fn load_requested(&mut self, cx: &mut Context<Self>) {
        let Some(book) = self.reader.book.clone() else {
            return;
        };
        let thumb_top = self.thumbnails.logical_scroll_top().item_ix;
        self.wanted.borrow_mut().retain(|(index, thumb)| {
            let visible = if *thumb {
                &self.visible_thumbnails
            } else {
                &self.visible
            };
            visible.borrow().contains(index)
                || index.abs_diff(if *thumb {
                    thumb_top
                } else {
                    self.reader.current
                }) <= if *thumb { 20 } else { 8 }
        });
        if self.reader.settings.preferences.mode != ReadingMode::Continuous {
            for index in self.reader.visible_indices() {
                self.wanted.borrow_mut().insert((index, false));
            }
        }
        let mut requests: Vec<_> = self.wanted.borrow().iter().copied().collect();
        requests.sort_by_key(|(index, thumb)| (*thumb, index.abs_diff(self.reader.current)));
        for (index, thumb) in requests {
            if index >= book.pages.len() {
                self.wanted.borrow_mut().remove(&(index, thumb));
                continue;
            }
            let revision = if thumb { 0 } else { self.image_revision };
            let cache = if thumb { &self.thumbs } else { &self.pages };
            if cache
                .get(&index)
                .is_some_and(|page| page.revision == revision)
                || self.errors.contains_key(&(index, thumb))
            {
                self.wanted.borrow_mut().remove(&(index, thumb));
                continue;
            }
            let key = (self.book_revision, revision, index, thumb);
            if self.inflight.contains(&key) {
                continue;
            }
            let limit = if thumb {
                MAX_THUMB_REQUESTS
            } else {
                MAX_PAGE_REQUESTS
            };
            if self.inflight.len() >= MAX_PAGE_REQUESTS
                || self.inflight.iter().filter(|key| key.3 == thumb).count() >= limit
            {
                continue;
            }
            self.inflight.insert(key);
            self.wanted.borrow_mut().remove(&(index, thumb));
            let book = book.clone();
            let rotation = self.reader.rotation;
            let filters = self.reader.filters;
            let task = cx.background_spawn(async move {
                let bytes = book.read_page(index)?;
                let source_bytes = bytes.len();
                let decoded = if thumb {
                    imaging::decode_thumbnail_frames(&bytes, 144, 144)?
                } else {
                    imaging::decode_frames(&bytes, rotation, filters)?
                };
                cached_page(decoded, source_bytes, revision)
            });
            cx.spawn(async move |view, cx| {
                let result = task.await;
                _ = view.update(cx, |view, cx| {
                    view.inflight.remove(&key);
                    if key.0 != view.book_revision || (!thumb && key.1 != view.image_revision) {
                        return;
                    }
                    match result {
                        Ok(page) => {
                            if thumb {
                                if let Some(old) = view.thumbs.insert(index, page) {
                                    old.release(None, cx);
                                }
                            } else {
                                view.geometry.insert(
                                    index,
                                    PageGeometry {
                                        width: page.width,
                                        height: page.height,
                                        rotation: view.reader.rotation,
                                    },
                                );
                                if let Some(old) = view.pages.insert(index, page) {
                                    old.release(None, cx);
                                }
                                if index == view.continuous.logical_scroll_top().item_ix {
                                    view.continuous.remeasure();
                                } else {
                                    view.continuous.remeasure_items(index..index + 1);
                                }
                            }
                            view.evict_cache(thumb, None, cx);
                        }
                        Err(error) => {
                            view.errors.insert((index, thumb), error);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn evict_cache(
        &mut self,
        thumb: bool,
        mut window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let current = if thumb {
            self.thumbnails.logical_scroll_top().item_ix
        } else {
            self.reader.current
        };
        let visible = if thumb {
            self.visible_thumbnails.borrow().clone()
        } else {
            let mut visible = self.visible.borrow().clone();
            visible.extend(self.reader.visible_indices());
            visible
        };
        let cache = if thumb {
            &mut self.thumbs
        } else {
            &mut self.pages
        };
        let limit = if thumb {
            THUMB_CACHE_BYTES
        } else {
            PAGE_CACHE_BYTES
        };
        let count_limit = (if thumb { 160 } else { 12 }).max(visible.len());
        let mut bytes: usize = cache.values().map(|page| page.allocation).sum();
        // Visible working sets can exceed the budget. Keep them resident to
        // avoid decoding and evicting the same animation on every render.
        while bytes > limit || cache.len() > count_limit {
            let candidate = cache
                .keys()
                .filter(|index| !visible.contains(index))
                .max_by_key(|index| index.abs_diff(current))
                .copied();
            let Some(index) = candidate else {
                break;
            };
            let old = cache.remove(&index).unwrap();
            bytes -= old.allocation;
            old.release(window.as_deref_mut(), cx);
        }
    }

    fn layout_for(&self, width: u32, height: u32, count: usize) -> (f32, f32) {
        let width = width.max(1) as f32;
        let height = height.max(1) as f32;
        let available_width =
            (f32::from(self.viewer_size.width) - 24.).max(1.) / count.max(1) as f32;
        let available_height = (f32::from(self.viewer_size.height) - 24.).max(1.);
        let layout = crate::reader::fitted_page_layout(
            width,
            height,
            0,
            self.reader.settings.preferences.fit,
            self.reader.zoom,
            available_width,
            available_height,
        );
        (layout.width, layout.height)
    }

    fn page_element(
        &self,
        index: usize,
        count: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        self.visible.borrow_mut().insert(index);
        self.wanted.borrow_mut().insert((index, false));
        let Some(book) = &self.reader.book else {
            return div().into_any_element();
        };
        let name = book
            .pages
            .get(index)
            .map(|page| page.name.as_str())
            .unwrap_or_default();
        let mut frame = div()
            .id(SharedString::from(format!(
                "page:{}:{name}",
                book.path.display()
            )))
            .test_support()
            .role(gpui_kit::Role::Image)
            .aria_label(format!("第 {} 頁", index + 1))
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .p_1();
        if let Some(error) = self.errors.get(&(index, false)) {
            frame = frame
                .min_h_64()
                .w_80()
                .gap_3()
                .p_4()
                .bg(cx.theme().background)
                .text_color(cx.theme().foreground)
                .child(format!("第 {} 頁：{}", index + 1, name))
                .child(error.clone())
                .child(
                    Button::new(("retry", index))
                        .label("重試")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.errors.remove(&(index, false));
                            this.wanted.borrow_mut().insert((index, false));
                            cx.notify();
                        })),
                );
        } else if let Some(page) = self.pages.get(&index) {
            let (width, height) = self.layout_for(page.width, page.height, count);
            let mut picture = div().relative().w(px(width)).h(px(height)).flex_none();
            for (tile_index, tile) in page.tiles.iter().enumerate() {
                picture = picture.child(
                    div()
                        .id(("tile", tile_index))
                        .absolute()
                        .left(px(tile.x as f32 * width / page.width as f32))
                        .top(px(tile.y as f32 * height / page.height as f32))
                        .child(
                            img(tile.image.clone())
                                .id("bitmap")
                                .w(px(tile.width as f32 * width / page.width as f32))
                                .h(px(tile.height as f32 * height / page.height as f32)),
                        ),
                );
            }
            frame = frame.child(picture);
        } else if let Some(geometry) = self.geometry.get(&index) {
            let (width, height) =
                if (self.reader.rotation - geometry.rotation).rem_euclid(180) == 90 {
                    (geometry.height, geometry.width)
                } else {
                    (geometry.width, geometry.height)
                };
            let (width, height) = self.layout_for(width, height, count);
            frame = frame
                .w(px(width + 8.))
                .h(px(height + 8.))
                .text_color(cx.theme().muted_foreground)
                .child(format!("正在讀取第 {} 頁…", index + 1));
        } else {
            frame = frame
                .w(px((f32::from(self.viewer_size.width)
                    / count.max(1) as f32
                    - 16.)
                    .max(1.)))
                .h(self.viewer_size.height - px(16.))
                .text_color(cx.theme().muted_foreground)
                .child(format!("正在讀取第 {} 頁…", index + 1));
        }
        frame.into_any_element()
    }

    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        command: Command,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .small()
            .ghost()
            .label(label)
            .accessibility_label(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.perform(command, window, cx);
                this.focus.focus(window, cx);
            }))
    }

    fn menu_items(&self, group: usize, menu: PopupMenu) -> PopupMenu {
        use Command::*;
        let opened = self.reader.book.is_some();
        match group {
            0 => menu
                .menu("開啟漫畫…", Box::new(OpenFile))
                .menu("開啟資料夾…", Box::new(OpenFolder))
                .menu_with_disabled("另存原始圖片…", Box::new(SavePage), !opened)
                .separator()
                .menu_with_disabled("關閉漫畫", Box::new(Close), !opened)
                .menu("結束", Box::new(Quit)),
            1 => menu
                .menu_with_check(
                    "顯示側欄",
                    self.reader.settings.preferences.sidebar,
                    Box::new(Sidebar),
                )
                .menu("縮圖", Box::new(Thumbnails))
                .menu_with_check(
                    "雙頁跨頁",
                    self.reader.settings.preferences.mode == ReadingMode::Spread,
                    Box::new(Spread),
                )
                .menu_with_check(
                    "連續頁面",
                    self.reader.settings.preferences.mode == ReadingMode::Continuous,
                    Box::new(Continuous),
                )
                .menu_with_check(
                    "右向左閱讀",
                    self.reader.settings.preferences.rtl,
                    Box::new(Direction),
                )
                .separator()
                .menu("全螢幕", Box::new(Fullscreen)),
            2 => menu
                .menu("適合視窗", Box::new(FitWindow))
                .menu("適合寬度", Box::new(FitWidth))
                .menu("適合高度", Box::new(FitHeight))
                .menu("原始尺寸", Box::new(Original))
                .separator()
                .menu("放大", Box::new(ZoomIn))
                .menu("縮小", Box::new(ZoomOut))
                .menu("100%", Box::new(ZoomReset)),
            3 => menu
                .menu("第一頁", Box::new(First))
                .menu("上一頁", Box::new(Previous))
                .menu("下一頁", Box::new(Next))
                .menu("最後一頁", Box::new(Last))
                .separator()
                .menu_with_check("投影片", self.reader.slideshow, Box::new(Slideshow)),
            4 => menu
                .menu_with_disabled("加入／更新書籤", Box::new(Bookmark), !opened)
                .menu_with_disabled("跳至書籤", Box::new(Resume), !opened)
                .menu_with_disabled("刪除本書書籤", Box::new(ClearBookmark), !opened),
            5 => menu
                .menu("向左旋轉", Box::new(RotateLeft))
                .menu("向右旋轉", Box::new(RotateRight))
                .menu("重設濾鏡", Box::new(ResetFilters))
                .separator()
                .menu("偏好設定", Box::new(Settings)),
            _ => menu.menu("關於 MangaMeeya Rust…", Box::new(About)),
        }
    }

    fn render_chrome(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut menus = div()
            .flex()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(cx.theme().border);
        for (group, label) in ["檔案", "檢視", "縮放", "移動", "書籤", "工具", "說明"]
            .into_iter()
            .enumerate()
        {
            let view = cx.entity().downgrade();
            menus = menus.child(
                Button::new(("menu", group))
                    .small()
                    .ghost()
                    .label(label)
                    .dropdown_menu(move |menu, _, cx| {
                        if let Some(view) = view.upgrade() {
                            view.read(cx).menu_items(group, menu)
                        } else {
                            menu
                        }
                    }),
            );
        }
        let opened = self.reader.book.is_some();
        let mode = self.reader.settings.preferences.mode;
        let toolbar = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_1()
            .px_2()
            .py_1()
            .child(self.button("open", "開啟…", Command::OpenFile, cx))
            .child(
                self.button("previous", "上一頁", Command::Previous, cx)
                    .disabled(!opened || self.reader.current == 0),
            )
            .child(
                self.button("next", "下一頁", Command::Next, cx)
                    .disabled(!opened || self.reader.current + 1 >= self.page_count()),
            )
            .child(
                self.button("spread", "雙頁", Command::Spread, cx)
                    .selected(mode == ReadingMode::Spread),
            )
            .child(
                self.button("continuous", "連續", Command::Continuous, cx)
                    .selected(mode == ReadingMode::Continuous),
            )
            .child(self.button(
                "direction",
                if self.reader.settings.preferences.rtl {
                    "R → L"
                } else {
                    "L → R"
                },
                Command::Direction,
                cx,
            ))
            .child(self.button("fit", "適合視窗", Command::FitWindow, cx))
            .child(
                self.button("zoom-out", "−", Command::ZoomOut, cx)
                    .tooltip("縮小（−）"),
            )
            .child(
                Input::new(&self.zoom_input)
                    .id("zoom-input")
                    .aria_label("縮放百分比")
                    .small()
                    .w_16(),
            )
            .child("%")
            .child(
                self.button("zoom-in", "+", Command::ZoomIn, cx)
                    .tooltip("放大（+）"),
            )
            .child(
                self.button("bookmark", "書籤", Command::Bookmark, cx)
                    .disabled(!opened),
            )
            .child(
                self.button(
                    "slideshow",
                    if self.reader.slideshow {
                        "停止"
                    } else {
                        "投影片"
                    },
                    Command::Slideshow,
                    cx,
                )
                .selected(self.reader.slideshow)
                .disabled(!opened),
            )
            .child(self.button("fullscreen", "全螢幕", Command::Fullscreen, cx))
            .child(self.button("preferences", "偏好設定", Command::Settings, cx));
        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(menus)
            .child(toolbar)
            .into_any_element()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut tabs = div().flex().gap_1().p_2().flex_none();
        for (mode, label) in [
            (SidebarMode::Files, "檔案"),
            (SidebarMode::Thumbnails, "縮圖"),
            (SidebarMode::Recent, "最近開啟"),
        ] {
            tabs = tabs.child(
                Button::new(label)
                    .small()
                    .ghost()
                    .label(label)
                    .selected(self.reader.settings.preferences.sidebar_mode == mode)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.reader.settings.preferences.sidebar_mode = mode;
                        this.dirty = true;
                        cx.notify();
                    })),
            );
        }
        let content = match self.reader.settings.preferences.sidebar_mode {
            SidebarMode::Thumbnails => {
                if self.reader.book.is_none() {
                    div().p_4().child("尚未開啟漫畫").into_any_element()
                } else {
                    let view = cx.entity().downgrade();
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .child(
                            list(self.thumbnails.clone(), move |index, _, cx| {
                                view.update(cx, |this, cx| this.thumbnail(index, cx))
                                    .unwrap_or_else(|_| div().into_any_element())
                            })
                            .size_full(),
                        )
                        .child(Scrollbar::vertical(&self.thumbnails).mode(ScrollbarMode::Always))
                        .into_any_element()
                }
            }
            SidebarMode::Files => self.render_files(cx),
            SidebarMode::Recent => {
                let mut recent = div()
                    .id("recent-list")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .p_2()
                    .overflow_y_scroll();
                if self.reader.settings.recent.is_empty() {
                    recent = recent.child("尚無開啟歷史");
                }
                for path in &self.reader.settings.recent {
                    let target = path.clone();
                    recent = recent.child(
                        Button::new(SharedString::from(format!("recent:{}", path.display())))
                            .ghost()
                            .label(
                                path.file_name()
                                    .unwrap_or(path.as_os_str())
                                    .to_string_lossy()
                                    .into_owned(),
                            )
                            .tooltip(path.to_string_lossy().into_owned())
                            .w_full()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_path(target.clone(), window, cx)
                            })),
                    );
                }
                if !self.reader.settings.recent.is_empty() {
                    recent = recent.child(self.button(
                        "clear-history",
                        "清除歷史",
                        Command::ClearHistory,
                        cx,
                    ));
                }
                recent.into_any_element()
            }
        };
        div()
            .flex()
            .flex_col()
            .w_64()
            .min_w_0()
            .h_full()
            .flex_none()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(tabs)
            .child(content)
            .into_any_element()
    }

    fn thumbnail(&self, index: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(book) = &self.reader.book else {
            return div().into_any_element();
        };
        let Some(page) = book.pages.get(index) else {
            return div().into_any_element();
        };
        self.visible_thumbnails.borrow_mut().insert(index);
        self.wanted.borrow_mut().insert((index, true));
        let picture = match self.thumbs.get(&index) {
            Some(page) => img(page.tiles[0].image.clone())
                .id("thumbnail-bitmap")
                .w_32()
                .h_32()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => div()
                .w_32()
                .h_32()
                .flex()
                .items_center()
                .justify_center()
                .child(if self.errors.contains_key(&(index, true)) {
                    "無法讀取".to_string()
                } else {
                    (index + 1).to_string()
                })
                .into_any_element(),
        };
        div()
            .p_2()
            .child(
                Button::new(SharedString::from(format!(
                    "thumb:{}:{}",
                    book.path.display(),
                    page.name
                )))
                .ghost()
                .w_full()
                .h_auto()
                .selected(index == self.reader.current)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .child(picture)
                        .child(div().w_full().truncate().text_xs().child(format!(
                            "{}. {}",
                            index + 1,
                            page.name
                        ))),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.navigate(index, window, cx))),
            )
            .into_any_element()
    }

    fn render_files(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut body = div().flex().flex_col().flex_1().min_h_0();
        if let Some(listing) = &self.listing {
            let parent = listing.parent.clone();
            let path = listing.path.clone();
            body = body.child(
                div()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                Button::new("parent-folder")
                                    .small()
                                    .label("上層")
                                    .disabled(parent.is_none())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if let Some(parent) = &parent {
                                            this.browse(parent.clone(), window, cx);
                                        }
                                    })),
                            )
                            .child(self.button(
                                "open-current-folder",
                                "開啟此資料夾",
                                Command::OpenListedFolder,
                                cx,
                            )),
                    )
                    .child(
                        Input::new(&self.path_input)
                            .id("path-input")
                            .aria_label("資料夾路徑")
                            .small(),
                    )
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .child(path.to_string_lossy().into_owned()),
                    ),
            );
            let view = cx.entity().downgrade();
            body = body.child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        list(self.files.clone(), move |index, _, cx| {
                            view.update(cx, |this, cx| this.file_entry(index, cx))
                                .unwrap_or_else(|_| div().into_any_element())
                        })
                        .size_full(),
                    )
                    .child(Scrollbar::vertical(&self.files).mode(ScrollbarMode::Always)),
            );
        } else {
            body = body.child(div().p_4().child("正在讀取資料夾…"));
        }
        if let Some(error) = &self.listing_error {
            body = body.child(
                div()
                    .p_3()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        body.into_any_element()
    }

    fn file_entry(&self, index: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(entry) = self
            .listing
            .as_ref()
            .and_then(|listing| listing.entries.get(index))
        else {
            return div().into_any_element();
        };
        let target = entry.path.clone();
        let directory = entry.is_directory;
        let prefix = match entry.kind {
            EntryKind::Folder => "▸",
            EntryKind::Archive => "▣",
            EntryKind::Image => "▧",
        };
        div()
            .px_2()
            .child(
                Button::new(SharedString::from(format!(
                    "entry:{}",
                    entry.path.display()
                )))
                .small()
                .ghost()
                .w_full()
                .label(format!("{prefix} {}", entry.name))
                .tooltip(entry.path.to_string_lossy().into_owned())
                .on_click(cx.listener(move |this, _, window, cx| {
                    if directory {
                        this.browse(target.clone(), window, cx);
                    } else {
                        this.open_path(target.clone(), window, cx);
                    }
                })),
            )
            .into_any_element()
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let mut panel = div()
            .id("inspector")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label("影像與偏好設定")
            .w(gpui_kit::rems(13.))
            .flex_none()
            .h_full()
            .p_3()
            .gap_3()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(cx.theme().border)
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("影像調整")
                    .child(
                        Button::new("hide-inspector")
                            .small()
                            .ghost()
                            .label("×")
                            .accessibility_label("隱藏影像調整")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inspector = false;
                                cx.notify();
                            })),
                    ),
            );
        for (index, label, value) in [
            (0, "亮度", self.reader.filters.brightness),
            (1, "對比", self.reader.filters.contrast),
            (2, "灰階", self.reader.filters.grayscale),
        ] {
            panel = panel.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(format!("{label}  {value:.0}%"))
                    .child(Slider::new(&self.filter_sliders[index]).accessibility_label(label)),
            );
        }
        panel = panel.child(
            div()
                .flex()
                .gap_1()
                .child(
                    self.button("rotate-left", "↶", Command::RotateLeft, cx)
                        .tooltip("向左旋轉（[）"),
                )
                .child(
                    self.button("rotate-right", "↷", Command::RotateRight, cx)
                        .tooltip("向右旋轉（]）"),
                )
                .child(self.button("reset-filters", "重設", Command::ResetFilters, cx)),
        );
        if self.preferences_open {
            let prefs = &self.reader.settings.preferences;
            panel = panel
                .child(
                    div()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .pt_3()
                        .child("偏好設定"),
                )
                .child(
                    Checkbox::new("remember-position")
                        .label("記住閱讀位置")
                        .checked(prefs.remember_position)
                        .on_change(cx.listener(|this, value, _, cx| {
                            this.reader.settings.preferences.remember_position = *value;
                            this.dirty = true;
                            cx.notify();
                        })),
                )
                .child(
                    Checkbox::new("recursive-folder")
                        .label("包含子資料夾")
                        .checked(prefs.recursive_folder)
                        .on_change(cx.listener(|this, value, _, cx| {
                            this.reader.settings.preferences.recursive_folder = *value;
                            this.dirty = true;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .text_xs()
                        .child("包含子資料夾的變更在下次開啟時生效。"),
                )
                .child("投影片間隔（1–120 秒）")
                .child(
                    Input::new(&self.interval_input)
                        .id("interval-input")
                        .aria_label("投影片間隔")
                        .small(),
                )
                .child("閱讀背景");
            for (background, label) in [
                (Background::Black, "黑色"),
                (Background::Gray, "深灰"),
                (Background::Paper, "紙色"),
            ] {
                panel = panel.child(
                    Button::new(label)
                        .small()
                        .ghost()
                        .label(label)
                        .selected(prefs.background == background)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.reader.settings.preferences.background = background;
                            this.dirty = true;
                            cx.notify();
                        })),
                );
            }
            panel = panel.child(
                div()
                    .text_xs()
                    .child("滾輪捲動；Ctrl＋滾輪縮放。所有設定自動儲存。"),
            );
            if let Some(error) = &self.save_error {
                panel = panel
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(error.clone()),
                    )
                    .child(
                        Button::new("reset-corrupt-settings")
                            .small()
                            .label("備份並重設設定")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save_epoch.fetch_add(1, Ordering::SeqCst);
                                let lock = this.save_lock.clone();
                                let result = lock
                                    .lock()
                                    .map_err(|_| "Settings writer lock failed".to_string())
                                    .and_then(|_guard| {
                                        AppSettings::reset_corrupt_at(&this.settings_path)
                                    });
                                match result {
                                    Ok(backup) => {
                                        this.reader.settings = AppSettings::default();
                                        this.save_error = None;
                                        this.interval_input.update(cx, |input, cx| {
                                            input.set_value("5", window, cx)
                                        });
                                        this.sync_controls(window, cx);
                                        this.set_message(
                                            format!("設定已重設；原始檔備份：{}", backup.display()),
                                            false,
                                            cx,
                                        );
                                    }
                                    Err(error) => this.set_message(error, true, cx),
                                }
                            })),
                    );
            }
        }
        panel.into_any_element()
    }

    fn render_viewer(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        self.visible.borrow_mut().clear();
        // These colors are document-background preferences, not UI chrome colors.
        let background = match self.reader.settings.preferences.background {
            Background::Black => gpui_kit::rgb(0x08090b),
            Background::Gray => gpui_kit::rgb(0x303238),
            Background::Paper => gpui_kit::rgb(0xeee8da),
        };
        let content = if self.reader.book.is_none() {
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_4()
                .bg(cx.theme().background)
                .text_color(cx.theme().muted_foreground)
                .child(div().text_2xl().child("MangaMeeya Rust"))
                .child("拖放漫畫，或開啟圖片資料夾與封存檔")
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(self.button("empty-open", "開啟漫畫…", Command::OpenFile, cx))
                        .child(self.button("empty-folder", "開啟資料夾…", Command::OpenFolder, cx)),
                )
                .child(
                    div()
                        .text_xs()
                        .child("JPG · PNG · GIF · BMP · WebP · ZIP/CBZ · RAR/CBR"),
                )
                .into_any_element()
        } else if self.reader.settings.preferences.mode == ReadingMode::Continuous {
            let view = cx.entity().downgrade();
            let width = self
                .geometry
                .values()
                .map(|page| {
                    let (width, height) =
                        if (self.reader.rotation - page.rotation).rem_euclid(180) == 90 {
                            (page.height, page.width)
                        } else {
                            (page.width, page.height)
                        };
                    self.layout_for(width, height, 1).0 + 16.
                })
                .fold(f32::from(self.viewer_size.width), f32::max);
            div()
                .id("continuous-horizontal")
                .size_full()
                .overflow_x_scroll()
                .track_scroll(&self.horizontal_scroll)
                .child(
                    div().w(px(width)).h_full().child(
                        list(self.continuous.clone(), move |index, _, cx| {
                            view.update(cx, |this, cx| {
                                let width = this
                                    .pages
                                    .get(&index)
                                    .map(|page| this.layout_for(page.width, page.height, 1).0 + 8.)
                                    .or_else(|| {
                                        this.geometry.get(&index).map(|page| {
                                            let (width, height) = if (this.reader.rotation
                                                - page.rotation)
                                                .rem_euclid(180)
                                                == 90
                                            {
                                                (page.height, page.width)
                                            } else {
                                                (page.width, page.height)
                                            };
                                            this.layout_for(width, height, 1).0 + 8.
                                        })
                                    })
                                    .unwrap_or(f32::from(this.viewer_size.width));
                                let inset =
                                    ((f32::from(this.viewer_size.width) - width) / 2.).max(0.);
                                div()
                                    .flex()
                                    .pl(px(inset))
                                    .w_full()
                                    .child(this.page_element(index, 1, cx))
                                    .into_any_element()
                            })
                            .unwrap_or_else(|_| div().into_any_element())
                        })
                        .size_full(),
                    ),
                )
                .child(Scrollbar::horizontal(&self.horizontal_scroll))
                .child(Scrollbar::vertical(&self.continuous).mode(ScrollbarMode::Always))
                .into_any_element()
        } else {
            let indices = self.reader.display_indices();
            let count = indices.len();
            let content_width = indices
                .iter()
                .filter_map(|index| self.pages.get(index))
                .map(|page| self.layout_for(page.width, page.height, count).0 + 8.)
                .sum::<f32>();
            let mut pages = div()
                .flex()
                .items_start()
                .justify_center()
                .w(px(content_width.max(f32::from(self.viewer_size.width))))
                .min_h_full();
            for index in indices {
                pages = pages.child(self.page_element(index, count, cx));
            }
            div()
                .id("paged-scroll")
                .size_full()
                .overflow_scroll()
                .track_scroll(&self.paged_scroll)
                .child(pages)
                .child(Scrollbar::vertical(&self.paged_scroll))
                .child(Scrollbar::horizontal(&self.paged_scroll))
                .into_any_element()
        };
        let view = cx.entity().downgrade();
        let bounds_view = view.clone();
        div()
            .id("viewer")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label("漫畫閱讀區")
            .relative()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .bg(background)
            .overflow_hidden()
            .child(content)
            .capture_any_mouse_down(cx.listener(|this, _, _, _| {
                this.continuous_target = None;
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui_kit::MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    let bounds = this.viewer_bounds;
                    let width = f32::from(this.viewer_size.width);
                    let x = f32::from(event.position.x - bounds.origin.x);
                    if this.reader.book.is_some()
                        && this.reader.settings.preferences.mode != ReadingMode::Continuous
                        && width > 0.
                    {
                        if x < width * 0.12 {
                            this.perform(Command::Left, window, cx);
                        } else if x > width * 0.88 {
                            this.perform(Command::Right, window, cx);
                        }
                    }
                }),
            )
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let view = bounds_view.clone();
                        cx.defer(move |cx| {
                            _ = view.update(cx, |this, cx| {
                                this.viewer_bounds = bounds;
                                if this.viewer_size != bounds.size {
                                    this.viewer_size = bounds.size;
                                    this.relayout(cx);
                                }
                            });
                        });
                    },
                    move |bounds, _, window, _| {
                        let view = view.clone();
                        window.on_mouse_event(
                            move |event: &gpui_kit::ScrollEvent, phase, window, cx| {
                                if phase == gpui_kit::DispatchPhase::Capture
                                    && bounds.contains(&event.position)
                                {
                                    if !event.modifiers.control {
                                        _ = view.update(cx, |this, _| {
                                            this.continuous_target = None;
                                        });
                                        return;
                                    }
                                    _ = view.update(cx, |this, cx| {
                                        let delta = event.delta.pixel_delta(px(16.)).y;
                                        this.reader.zoom_by(if delta > px(0.) {
                                            1.1
                                        } else {
                                            1. / 1.1
                                        });
                                        this.relayout(cx);
                                        this.sync_controls(window, cx);
                                    });
                                    cx.stop_propagation();
                                }
                            },
                        );
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let current = self.reader.current;
        let name = self
            .reader
            .book
            .as_ref()
            .and_then(|book| book.pages.get(current))
            .map(|page| page.name.clone())
            .unwrap_or_else(|| "尚未開啟漫畫".into());
        let detail = self
            .pages
            .get(&current)
            .map(|page| {
                format!(
                    "{}×{} · {:.1} KiB",
                    page.width,
                    page.height,
                    page.source_bytes as f32 / 1024.
                )
            })
            .unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .child(div().flex_1().min_w_0().truncate().text_xs().child(name))
                    .child("頁碼")
                    .child(
                        Input::new(&self.page_input)
                            .id("page-input")
                            .aria_label("頁碼")
                            .small()
                            .w_16()
                            .disabled(self.reader.book.is_none()),
                    )
                    .child(format!("/ {}", self.page_count()))
                    .child(
                        div().w_40().px_2().child(
                            Slider::new(&self.page_slider)
                                .disabled(self.reader.book.is_none())
                                .accessibility_label("頁面位置"),
                        ),
                    )
                    .child(div().text_xs().child(detail))
                    .child(self.button("sidebar-toggle", "側欄", Command::Sidebar, cx))
                    .child(
                        Button::new("inspector-toggle")
                            .small()
                            .ghost()
                            .label("影像調整")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.inspector = !this.inspector;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("status-message")
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .aria_label(self.message.clone())
                    .px_3()
                    .pb_1()
                    .text_xs()
                    .truncate()
                    .text_color(if self.message_error {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(self.message.clone()),
            )
            .into_any_element()
    }
}

impl Render for ReaderView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.visible_thumbnails.borrow_mut().clear();
        let workspace = div()
            .flex()
            .items_stretch()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .when(self.reader.settings.preferences.sidebar, |workspace| {
                workspace.child(self.render_sidebar(cx))
            })
            .child(self.render_viewer(cx))
            .when(self.inspector, |workspace| {
                workspace.child(self.render_inspector(cx))
            });
        let drop_view = cx.entity().downgrade();
        div()
            .id("reader-window")
            .key_context("Reader")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|this, command: &Command, window, cx| {
                    this.perform(*command, window, cx)
                }),
            )
            .on_drag_move(cx.listener(
                |this, event: &gpui_kit::DragMoveEvent<ExternalPaths>, _, cx| {
                    if event.bounds.contains(&event.event.position) {
                        let path = event.drag(cx).paths().first().cloned();
                        if this.pending_drop != path {
                            this.pending_drop = path;
                            cx.notify();
                        }
                    }
                },
            ))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .child(self.render_chrome(cx))
            .child(workspace)
            .child(self.render_status(cx))
            .when(
                self.pending_drop.is_some() && cx.has_active_drag(),
                |root| {
                    root.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(cx.theme().primary.opacity(0.9))
                            .text_color(cx.theme().primary_foreground)
                            .text_lg()
                            .child("放開以開啟漫畫"),
                    )
                },
            )
            // Capture the window drop before an opaque control can hide the
            // ancestor's drop hitbox. DragMove supplies the typed OS paths.
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let down_view = drop_view.clone();
                        window.on_mouse_event(move |_: &gpui_kit::MouseDownEvent, phase, _, cx| {
                            if phase == gpui_kit::DispatchPhase::Capture && !cx.has_active_drag() {
                                _ = down_view.update(cx, |this, _| {
                                    this.pending_drop = None;
                                });
                            }
                        });
                        let view = drop_view.clone();
                        window.on_mouse_event(
                            move |event: &gpui_kit::MouseUpEvent, phase, window, cx| {
                                if phase == gpui_kit::DispatchPhase::Capture
                                    && event.button == MouseButton::Left
                                    && bounds.contains(&event.position)
                                {
                                    _ = view.update(cx, |this, cx| {
                                        if let Some(path) = this.pending_drop.take()
                                            && cx.has_active_drag()
                                        {
                                            cx.stop_active_drag(window);
                                            this.open_path(path, window, cx);
                                            cx.stop_propagation();
                                        }
                                    });
                                }
                            },
                        );
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }
}

// The epoch check under the writer lock prevents a delayed background save from
// replacing a newer shutdown snapshot.
fn save_snapshot(
    settings: &AppSettings,
    path: &Path,
    lock: &Mutex<()>,
    epoch: &AtomicU64,
    revision: u64,
) -> Result<(), String> {
    let _guard = lock
        .lock()
        .map_err(|_| "Settings writer lock failed".to_string())?;
    if epoch.load(Ordering::SeqCst) == revision {
        settings.save_to(path)
    } else {
        Ok(())
    }
}

pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    bind_commands(cx);
}

fn bind_commands(cx: &mut App) {
    use Command::*;
    let navigation = "Reader && !Input";
    for (key, command) in [
        ("space", Next),
        ("pagedown", Next),
        ("pageup", Previous),
        ("left", Left),
        ("right", Right),
        ("up", ScrollUp),
        ("down", ScrollDown),
        ("home", First),
        ("end", Last),
        ("s", Spread),
        ("c", Continuous),
        ("d", Direction),
        ("f", FitWindow),
        ("w", FitWidth),
        ("h", FitHeight),
        ("1", Original),
        ("+", ZoomIn),
        ("=", ZoomIn),
        ("-", ZoomOut),
        ("0", ZoomReset),
        ("[", RotateLeft),
        ("]", RotateRight),
        ("b", Bookmark),
        ("shift-b", Resume),
        ("p", Slideshow),
        ("t", Thumbnails),
        ("tab", Sidebar),
    ] {
        cx.bind_keys([KeyBinding::new(key, command, Some(navigation))]);
    }
    for (key, command) in [
        ("ctrl-o", OpenFile),
        ("ctrl-shift-o", OpenFolder),
        ("ctrl-s", SavePage),
        ("ctrl-w", Close),
        ("f11", Fullscreen),
        ("escape", Escape),
    ] {
        cx.bind_keys([KeyBinding::new(key, command, Some("Reader"))]);
    }
    #[cfg(target_os = "macos")]
    for (key, command) in [
        ("cmd-o", OpenFile),
        ("cmd-shift-o", OpenFolder),
        ("cmd-s", SavePage),
        ("cmd-w", Close),
        ("cmd-q", Quit),
        ("cmd-,", Settings),
        ("ctrl-cmd-f", Fullscreen),
    ] {
        cx.bind_keys([KeyBinding::new(key, command, Some("Reader"))]);
    }
    #[cfg(target_os = "windows")]
    cx.bind_keys([KeyBinding::new("alt-f4", Quit, Some("Reader"))]);
    cx.set_menus([
        Menu::new("MangaMeeya Rust").items([
            MenuItem::action("About MangaMeeya Rust…", About),
            MenuItem::action("Settings", Settings),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("Open comic…", OpenFile),
            MenuItem::action("Open folder…", OpenFolder),
            MenuItem::action("Save original image…", SavePage),
            MenuItem::action("Close comic", Close),
        ]),
        Menu::new("View").items([
            MenuItem::action("Sidebar", Sidebar),
            MenuItem::action("Spread", Spread),
            MenuItem::action("Continuous", Continuous),
            MenuItem::action("Reading direction", Direction),
            MenuItem::action("Fit window", FitWindow),
            MenuItem::action("Fit width", FitWidth),
            MenuItem::action("Fit height", FitHeight),
            MenuItem::action("Original size", Original),
            MenuItem::action("Fullscreen", Fullscreen),
        ]),
        Menu::new("Navigate").items([
            MenuItem::action("First page", First),
            MenuItem::action("Previous page", Previous),
            MenuItem::action("Next page", Next),
            MenuItem::action("Last page", Last),
            MenuItem::action("Slideshow", Slideshow),
        ]),
        Menu::new("Bookmarks").items([
            MenuItem::action("Set bookmark", Bookmark),
            MenuItem::action("Go to bookmark", Resume),
            MenuItem::action("Delete bookmark", ClearBookmark),
        ]),
    ]);
}

pub fn run() -> anyhow::Result<()> {
    let settings_path = AppSettings::config_path().map_err(anyhow::Error::msg)?;
    let paths = std::env::args_os()
        .skip(1)
        .filter(|arg| !arg.to_string_lossy().starts_with('-'))
        .map(PathBuf::from)
        .collect();
    let instance = match instance::start(settings_path.parent().unwrap(), paths)
        .map_err(anyhow::Error::msg)?
    {
        Instance::Forwarded => return Ok(()),
        Instance::Primary(primary) => primary,
    };
    let (settings, warning) = match AppSettings::load_from(&settings_path) {
        Ok(settings) => (settings, None),
        Err(error) => (AppSettings::default(), Some(error)),
    };
    let (sender, launches) = mpsc::channel();
    let application = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    application.on_open_urls(move |urls| {
        let paths = urls
            .into_iter()
            .filter_map(|url| url::Url::parse(&url).ok()?.to_file_path().ok())
            .collect();
        _ = sender.send(paths);
    });
    application.on_reopen(|cx| {
        cx.activate(true);
        if let Some(window) = cx.windows().first() {
            _ = window.update(cx, |_, window, _| window.activate_window());
        }
    });
    application.run(move |cx| {
        init(cx);
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(800.), px(540.))),
                titlebar: Some(gpui_kit::TitlebarOptions {
                    title: Some("MangaMeeya Rust".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            cx,
            move |window, cx| {
                cx.new(|cx| {
                    ReaderView::new(
                        settings,
                        settings_path,
                        warning,
                        Some(instance),
                        launches,
                        window,
                        cx,
                    )
                })
            },
        )
        .expect("Cannot open the native GPUI window");
        cx.activate(true);
    });
    Ok(())
}

#[cfg(test)]
mod tests;
