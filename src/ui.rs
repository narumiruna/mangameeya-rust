//! Native reader window, commands, and asynchronous image loading.

mod assets;
mod browser;
mod files;
mod preferences;
pub use assets::ReaderAssets;

use crate::{
    book::{self, Book, DirectoryListing, EntryKind},
    i18n::{Language, Text},
    imaging::{self, Filters},
    instance::{self, Instance, PrimaryInstance},
    reader::Reader,
    settings::{AppSettings, Background, FitMode, ReadingMode, SidebarMode},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenu},
    popover::Popover,
    resizable::{ResizableState, h_resizable, resizable_panel},
    scroll::{Scrollbar, ScrollbarMode},
    slider::{Slider, SliderEvent, SliderState},
    theme::Theme,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, Entity, ExternalPaths, FocusHandle,
    Focusable, InteractiveElement, IntoElement, KeyBinding, ListAlignment, ListOffset, ListState,
    Menu, MenuItem, MouseButton, ObjectFit, ParentElement, Pixels, Render, RenderImage,
    SharedString, Size, StatefulInteractiveElement, Styled, StyledImage, Subscription,
    TestSupportExt, Window, WindowBounds, WindowOptions, canvas, div, img, list, point,
    prelude::FluentBuilder, px, size,
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
    Inspector,
    ToggleRecursiveFolder,
    Single,
    SetSpread,
    SetContinuous,
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
    Browse,
    ContinueReading,
    BrowserBack,
    BrowserOpen,
    BrowserRefresh,
}

impl Command {
    fn reading_only(self) -> bool {
        use Command::*;
        matches!(
            self,
            SavePage
                | Single
                | SetSpread
                | SetContinuous
                | Spread
                | Continuous
                | Direction
                | FitWindow
                | FitWidth
                | FitHeight
                | Original
                | ZoomIn
                | ZoomOut
                | ZoomReset
                | First
                | Previous
                | Next
                | Last
                | Left
                | Right
                | ScrollUp
                | ScrollDown
                | Slideshow
                | Bookmark
                | Resume
                | ClearBookmark
                | RotateLeft
                | RotateRight
                | ResetFilters
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ContentMode {
    Reading,
    Browsing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OpenSource {
    External,
    CentralBrowser,
    Sidebar,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum InspectorTab {
    #[default]
    Image,
    Playback,
    Browser,
}

// Resize handles straddle panel boundaries. Reserve a band as well as the reader minimum.
fn panel_widths(width: f32, left: Option<u32>, right: Option<u32>) -> (f32, f32) {
    let mut left = left.map_or(0., |value| value.clamp(216, 320) as f32);
    let mut right = right.map_or(0., |value| value.clamp(240, 360) as f32);
    let mut excess = (left + right + 288. - width).max(0.);
    let reduction = excess.min((right - 240.).max(0.));
    right -= reduction;
    excess -= reduction;
    left -= excess.min((left - 216.).max(0.));
    (left, right)
}

const CONTROL_HEIGHT: f32 = 34.;
const TOOLBAR_HEIGHT: f32 = 59.;
const MENU_HEIGHT: f32 = 30.;
const STATUS_HEIGHT: f32 = 47.;

fn background_color(background: Background) -> gpui_kit::Hsla {
    gpui_kit::rgb(match background {
        Background::Black => 0x08090b,
        Background::Gray => 0x303238,
        Background::Paper => 0xeee8da,
    })
    .into()
}

fn direction_label(language: Language, rtl: bool) -> &'static str {
    language.text(if rtl {
        Text::DirectionRtl
    } else {
        Text::DirectionLtr
    })
}

const PAGE_CACHE_BYTES: usize = 256 * 1024 * 1024;
const THUMB_CACHE_BYTES: usize = 24 * 1024 * 1024;
const MAX_TEXTURE_SIDE: u32 = 8192;
const MAX_PAGE_REQUESTS: usize = 2;
const MAX_THUMB_REQUESTS: usize = 2;
const TICK: Duration = Duration::from_millis(80);

fn initial_directory() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn fit_label(language: Language, fit: FitMode) -> &'static str {
    language.text(match fit {
        FitMode::Window => Text::FitWindow,
        FitMode::Width => Text::FitWidth,
        FitMode::Height => Text::FitHeight,
        FitMode::Original => Text::OriginalSize,
        FitMode::Custom => Text::CustomZoom,
    })
}

fn zoom_percentage(scale: Option<f32>) -> String {
    match scale.filter(|scale| scale.is_finite() && *scale > 0.) {
        Some(scale) if scale < 0.01 => "<1%".into(),
        Some(scale) => format!("{:.0}%", f64::from(scale) * 100.),
        None => "—".into(),
    }
}

fn zoom_input_value(scale: f32) -> String {
    let percent = f64::from(scale) * 100.;
    if percent < 0.01 {
        percent.to_string()
    } else {
        format!("{percent:.2}")
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    }
}

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
    content_mode: ContentMode,
    paused_viewer_size: Option<Size<Pixels>>,
    book_recursive: bool,
    browser: browser::BrowserUi,
    reading_origin: Option<PathBuf>,
    sidebar_folder_origin: Option<PathBuf>,
    pending_browser_selection: Option<PathBuf>,
    settings_path: PathBuf,
    focus: FocusHandle,
    preferences_window: Option<AnyWindowHandle>,
    closing: bool,
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
    files_ui: files::FilesUi,
    paged_scroll: gpui_kit::ScrollHandle,
    horizontal_scroll: gpui_kit::ScrollHandle,
    viewer_size: Size<Pixels>,
    viewer_bounds: Bounds<Pixels>,
    controls_pending: bool,
    page_input: Entity<InputState>,
    zoom_input: Entity<InputState>,
    zoom_open: bool,
    path_input: Entity<InputState>,
    interval_input: Entity<InputState>,
    page_slider: Entity<SliderState>,
    filter_sliders: [Entity<SliderState>; 3],
    inspector_tab: InspectorTab,
    info_open: bool,
    panel_state: Entity<ResizableState>,
    panel_layout_key: (u32, bool, bool),
    panel_drag_start: Option<(bool, Pixels)>,
    message: String,
    message_translation: (Text, Vec<(&'static str, String)>),
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
        let language = settings.preferences.language;
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let page_input = cx.new(|cx| InputState::new(window, cx).default_value("—"));
        let zoom_input = cx.new(|cx| InputState::new(window, cx).placeholder("—"));
        let path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(language.text(Text::FolderPath)));
        let interval_input = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.preferences.interval.to_string())
        });
        let page_slider = cx.new(|_| SliderState::new().min(0.).max(100.).step(0.01));
        let filter_sliders = [
            cx.new(|_| SliderState::new().min(50.).max(150.).default_value(100.)),
            cx.new(|_| SliderState::new().min(50.).max(150.).default_value(100.)),
            cx.new(|_| SliderState::new().min(0.).max(100.).default_value(0.)),
        ];
        let panel_state = cx.new(|_| ResizableState::default());
        let mut subscriptions = vec![cx.observe(&panel_state, |_, _, cx| cx.notify())];
        subscriptions.push(cx.subscribe_in(
            &page_input,
            window,
            |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && this.content_mode == ContentMode::Reading
                {
                    match input.read(cx).value().trim().parse::<usize>() {
                        Ok(page) => this.navigate(page.saturating_sub(1), window, cx),
                        Err(_) => this.set_localized_message(Text::InvalidPage, vec![], true, cx),
                    }
                    this.focus.focus(window, cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &zoom_input,
            window,
            |this, input, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && this.zoom_open
                    && this.effective_scale().is_some()
                {
                    let value = input.read(cx).value();
                    let value = value.trim();
                    match value.strip_suffix('%').unwrap_or(value).parse::<f32>() {
                        Ok(percent) if percent.is_finite() => {
                            this.apply_zoom(percent / 100., window, cx);
                            this.zoom_open = false;
                            this.set_localized_message(Text::ZoomApplied, vec![], false, cx);
                        }
                        _ => this.set_localized_message(Text::InvalidZoom, vec![], true, cx),
                    }
                }
            },
        ));
        subscriptions.push(
            cx.subscribe_in(&path_input, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.files_submit_path(window, cx);
                }
            }),
        );
        subscriptions.push(
            cx.subscribe_in(&interval_input, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.apply_interval(cx);
                }
            }),
        );
        subscriptions.push(
            cx.subscribe_in(&page_slider, window, |this, _, event, window, cx| {
                if let SliderEvent::Change(value) = event
                    && this.content_mode == ContentMode::Reading
                {
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
                    if let SliderEvent::Change(value) = event
                        && this.content_mode == ContentMode::Reading
                    {
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
        subscriptions.push(cx.on_app_quit(|this, cx| {
            this.closing = true;
            this.close_preferences(cx);
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
                view.update(cx, |this, cx| {
                    this.closing = true;
                    this.close_preferences(cx);
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
        let browser = browser::BrowserUi::new(window, cx);
        let files_ui = files::FilesUi::new(window, cx);
        let mut view = Self {
            reader: Reader::new(settings),
            content_mode: ContentMode::Browsing,
            paused_viewer_size: None,
            book_recursive: false,
            browser,
            reading_origin: None,
            sidebar_folder_origin: None,
            pending_browser_selection: None,
            settings_path,
            focus,
            preferences_window: None,
            closing: false,
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
            files_ui,
            paged_scroll: Default::default(),
            horizontal_scroll: Default::default(),
            viewer_size: size(px(800.), px(600.)),
            viewer_bounds: Bounds::default(),
            controls_pending: false,
            page_input,
            zoom_input,
            zoom_open: false,
            path_input,
            interval_input,
            page_slider,
            filter_sliders,
            inspector_tab: InspectorTab::Image,
            info_open: false,
            panel_state,
            panel_layout_key: (0, false, false),
            panel_drag_start: None,
            message: String::new(),
            message_translation: warning.clone().map_or((Text::Ready, vec![]), |error| {
                (Text::SettingsError, vec![("error", error)])
            }),
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
        view.update_language(window, cx);
        let home = initial_directory();
        view.browse(home.clone(), window, cx);
        view.files_browse(home, false, window, cx);
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

    /// Last opened Preferences handle. A closed window is recreated on the next request.
    pub fn preferences_window(&self) -> Option<AnyWindowHandle> {
        self.preferences_window
    }

    fn open_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.commit_focused_interval(window, cx);
        self.focus.focus(window, cx);
        let reader = cx.entity().downgrade();
        let reader_window = window.window_handle();
        let display = window.display(cx).map(|display| display.id());
        // GPUI draws a new window before open_window returns. Do not hold a
        // mutable ReaderView borrow while Preferences reads its settings.
        cx.defer(move |cx| {
            let Some(entity) = reader.upgrade() else {
                return;
            };
            if entity.read(cx).closing || !cx.windows().contains(&reader_window) {
                return;
            }
            if let Some(handle) = entity.read(cx).preferences_window
                && handle
                    .update(cx, |_, window, _| window.activate_window())
                    .is_ok()
            {
                return;
            }
            let language = entity.read(cx).language();
            let result = preferences::open(reader, reader_window, language, display, cx);
            entity.update(cx, |reader, cx| reader.preferences_opened(result, cx));
        });
    }

    fn preferences_opened(
        &mut self,
        result: anyhow::Result<AnyWindowHandle>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(handle) => self.preferences_window = Some(handle),
            Err(error) => {
                self.preferences_window = None;
                self.set_localized_message(
                    Text::PreferencesOpenFailed,
                    vec![("error", error.to_string())],
                    true,
                    cx,
                );
            }
        }
    }

    fn close_preferences(&mut self, cx: &mut App) {
        if let Some(handle) = self.preferences_window.take() {
            _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    fn apply_preference(
        &mut self,
        change: preferences::Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use preferences::Change;
        match change {
            Change::Language(language) => self.set_language(language, window, cx),
            Change::Reset => self.reset_corrupt_settings(window, cx),
            Change::Remember(value)
                if self.reader.settings.preferences.remember_position != value =>
            {
                self.reader.settings.preferences.remember_position = value;
                self.dirty = true;
                cx.notify();
            }
            Change::Direction(value) if self.reader.settings.preferences.rtl != value => {
                self.reader.settings.preferences.rtl = value;
                self.dirty = true;
                self.relayout(cx);
            }
            Change::Background(value) if self.reader.settings.preferences.background != value => {
                self.reader.settings.preferences.background = value;
                self.dirty = true;
                cx.notify();
            }
            _ => {}
        }
    }

    fn clear_images(&mut self, mut window: Option<&mut Window>, cx: &mut Context<Self>) {
        for (_, page) in self.pages.drain().chain(self.thumbs.drain()) {
            page.release(window.as_deref_mut(), cx);
        }
    }

    fn page_count(&self) -> usize {
        self.reader.book.as_ref().map_or(0, |book| book.pages.len())
    }

    fn set_error(&mut self, error: impl Into<String>, cx: &mut Context<Self>) {
        self.set_localized_message(Text::ErrorDetails, vec![("error", error.into())], true, cx);
    }

    fn language(&self) -> Language {
        self.reader.settings.preferences.language
    }

    fn set_localized_message(
        &mut self,
        text: Text,
        args: Vec<(&'static str, String)>,
        error: bool,
        cx: &mut Context<Self>,
    ) {
        self.message = self.language().format(text, &args);
        self.message_translation = (text, args);
        self.message_error = error;
        cx.notify();
    }

    fn set_language(&mut self, language: Language, window: &mut Window, cx: &mut Context<Self>) {
        if self.language() == language {
            return;
        }
        self.reader.settings.preferences.language = language;
        self.update_language(window, cx);
        self.dirty = true;
        cx.notify();
    }

    fn update_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let language = self.language();
        gpui_kit::component::set_locale(language.component_locale());
        #[cfg(target_os = "windows")]
        {
            // Segoe UI's implicit CJK fallback can select a serif family.
            // Resolve a language-appropriate UI family, with GPUI's fallback
            // when the optional Windows language font is not installed.
            let requested = match language {
                Language::English => "Segoe UI",
                Language::Japanese => "Yu Gothic UI",
                Language::TraditionalChinese => "Microsoft JhengHei UI",
            };
            let text = cx.text_system();
            let family = text
                .get_font_for_id(text.resolve_font(&gpui_kit::font(requested)))
                .map_or_else(|| ".SystemUIFont".into(), |font| font.family);
            Theme::update(cx, |theme| theme.font_family = family);
        }
        set_native_menus(language, cx);
        self.path_input.update(cx, |input, cx| {
            input.set_placeholder(language.text(Text::FolderPath), window, cx);
        });
        self.files_ui.search.update(cx, |input, cx| {
            input.set_placeholder(language.text(Text::FilesSearchLoaded), window, cx);
        });
        self.message = language.format(self.message_translation.0, &self.message_translation.1);
    }

    fn sync_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let page = if self.page_count() == 0 {
            "—".to_owned()
        } else {
            (self.reader.current + 1).to_string()
        };
        self.page_input
            .update(cx, |input, cx| input.set_value(page, window, cx));
        self.sync_zoom_input(false, window, cx);
        let maximum = self.page_count().saturating_sub(1).max(1);
        let progress = self.reader.current as f32 / maximum as f32 * 100.;
        self.page_slider
            .update(cx, |slider, cx| slider.set_value(progress, window, cx));
        if self
            .thumbnails
            .bounds_for_item(self.reader.current)
            .is_none()
        {
            // Unmeasured virtual rows have no height to reveal against yet.
            self.thumbnails.scroll_to(ListOffset {
                item_ix: self.reader.current,
                offset_in_item: px(0.),
            });
        } else {
            self.thumbnails.scroll_to_reveal_item(self.reader.current);
        }
        self.dirty = true;
        cx.notify();
    }

    fn sync_zoom_input(&self, force: bool, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.zoom_input.read(cx);
        if !force && input.focus_handle(cx).is_focused(window) {
            return;
        }
        let value = self
            .effective_scale()
            .map(zoom_input_value)
            .unwrap_or_default();
        if input.value().as_ref() != value {
            self.zoom_input
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }

    fn apply_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.effective_scale().is_none() {
            return;
        }
        self.reader.set_zoom(zoom);
        self.relayout(cx);
        self.sync_zoom_input(true, window, cx);
        self.sync_controls(window, cx);
    }

    fn zoom_by(&mut self, factor: f32, window: &mut Window, cx: &mut Context<Self>) {
        if factor.is_finite()
            && factor > 0.
            && let Some(scale) = self.effective_scale()
        {
            let zoom = (f64::from(scale) * f64::from(factor)).clamp(0.1, 8.) as f32;
            self.apply_zoom(zoom, window, cx);
        }
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
        if self.content_mode == ContentMode::Reading
            && self.reader.slideshow
            && Instant::now() >= self.next_slide
        {
            if self.reader.current + 1 < self.page_count() {
                self.reader.next();
                self.after_navigation(window, cx);
                self.next_slide =
                    Instant::now() + Duration::from_secs(self.reader.settings.preferences.interval);
            } else {
                self.reader.slideshow = false;
                self.set_localized_message(Text::SlideshowEnded, vec![], false, cx);
            }
        }
        if self.content_mode == ContentMode::Reading
            && self.reader.settings.preferences.mode == ReadingMode::Continuous
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
                            view.set_localized_message(
                                Text::SettingsError,
                                vec![("error", error)],
                                true,
                                cx,
                            );
                        }
                    } else if view.save_error.take().is_some() {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        if self.content_mode == ContentMode::Browsing {
            self.browser_tick(cx);
        } else {
            self.load_requested(cx);
        }
        self.evict_cache(true, Some(window), cx);
    }

    fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_path_from(path, OpenSource::External, window, cx);
    }

    fn open_browser_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_browser_location();
        self.open_path_from(path, OpenSource::CentralBrowser, window, cx);
    }

    fn open_path_from(
        &mut self,
        path: PathBuf,
        source: OpenSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.info_open = false;
        self.zoom_open = false;
        self.focus.focus(window, cx);
        self.open_revision += 1;
        self.files_cancel_reveal(cx);
        // A newer open intent owns the current location even before it succeeds.
        // Do not let an older directory enumeration replace its return snapshot.
        self.listing_revision += 1;
        self.pending_browser_selection = None;
        self.browser_resume(cx);
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
        self.set_localized_message(
            Text::BookOpening,
            vec![("path", path.display().to_string())],
            false,
            cx,
        );
        let task = cx.background_spawn(async move {
            let book = Book::open(&path, recursive)?;
            // Keep the entry's own name even when it is a symbolic link. Listing
            // canonicalizes its parent but retains this native child identity.
            let origin = path
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .and_then(|parent| path.file_name().map(|name| parent.join(name)))
                .unwrap_or_else(|| book.path.clone());
            let folder_book = book.path.is_dir();
            let source_entry = std::path::absolute(&path).map_err(|error| error.to_string())?;
            Ok::<_, String>((book, origin, folder_book, source_entry))
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = task.await;
            _ = view.update_in(cx, |view, window, cx| {
                if revision != view.open_revision || view.closing {
                    return;
                }
                view.opening = false;
                match result {
                    Ok((book, origin, folder_book, source_entry)) => {
                        view.content_mode = ContentMode::Reading;
                        view.paused_viewer_size = None;
                        view.listing_revision += 1;
                        view.reading_origin =
                            (source != OpenSource::CentralBrowser).then_some(origin);
                        view.sidebar_folder_origin =
                            (source == OpenSource::Sidebar && folder_book && !explicit_image)
                                .then(|| source_entry.clone());
                        view.book_recursive = folder_book && recursive;
                        let key = (book.path.clone(), view.book_recursive);
                        view.browser.model.known_pages.insert(key, book.pages.len());
                        view.zoom_open = false;
                        view.info_open = false;
                        let explicit = explicit_image.then_some(book.initial_page);
                        view.reader.set_book(Arc::new(book), explicit);
                        view.book_revision += 1;
                        view.image_revision = 0;
                        view.clear_images(Some(window), cx);
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
                        view.files_after_open(
                            source_entry,
                            source == OpenSource::External,
                            window,
                            cx,
                        );
                        view.set_localized_message(
                            Text::BookOpened,
                            vec![
                                ("title", view.reader.book.as_ref().unwrap().title.clone()),
                                ("count", count.to_string()),
                            ],
                            false,
                            cx,
                        );
                        let current = view.reader.current;
                        for index in current.saturating_sub(1)..(current + 3).min(count) {
                            view.wanted.borrow_mut().insert((index, false));
                        }
                        cx.notify();
                    }
                    Err(error) => {
                        if view.reader.book.is_none() && view.listing.is_none() {
                            view.load_browser_listing(initial_directory(), cx);
                        }
                        view.set_error(error, cx);
                    }
                }
            });
        })
        .detach();
    }

    fn browse(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_browser_location();
        self.enter_browser(window, cx);
        self.reading_origin = None;
        self.pending_browser_selection = None;
        self.load_browser_listing(path, cx);
    }

    fn load_browser_listing(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.listing_revision += 1;
        let revision = self.listing_revision;
        self.listing_error = None;
        let task = cx.background_spawn(async move { book::browse_directory(&path) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            _ = view.update(cx, |view, cx| {
                if revision != view.listing_revision || view.closing {
                    return;
                }
                match result {
                    Ok(listing) => {
                        view.listing = Some(listing);
                        view.browser_listing_changed(cx);
                        if let Some(target) = view.pending_browser_selection.take() {
                            let visible = view.listing.as_ref().is_some_and(|listing| {
                                view.browser
                                    .model
                                    .order
                                    .iter()
                                    .any(|&index| listing.entries[index].entry_path == target)
                            });
                            if !visible {
                                view.browser.model.query.clear();
                            }
                            view.browser.model.selected = Some(target);
                            view.rebuild_browser(cx);
                            view.reveal_browser_selection();
                        }
                    }
                    Err(error) => {
                        view.listing_error = Some(error);
                        view.pending_browser_selection = None;
                        // The previous listing is still visible. A failed newer
                        // intent must not leave its visible-cover dispatcher paused.
                        view.browser_resume(cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn navigate(&mut self, page: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.content_mode != ContentMode::Reading {
            return;
        }
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

    fn apply_interval(&mut self, cx: &mut Context<Self>) {
        let value = self.interval_input.read(cx).value();
        match value.trim().parse::<u64>() {
            Ok(seconds) => {
                let seconds = seconds.clamp(1, 120);
                self.reader.settings.preferences.interval = seconds;
                self.next_slide = Instant::now() + Duration::from_secs(seconds);
                self.dirty = true;
                cx.notify();
            }
            Err(_) => self.set_localized_message(Text::InvalidInterval, vec![], true, cx),
        }
    }

    fn commit_focused_interval(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.inspector_tab == InspectorTab::Playback
            && self
                .interval_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            // Native menu actions can unmount the input before its deferred Blur.
            self.apply_interval(cx);
        }
    }

    fn relayout(&mut self, cx: &mut Context<Self>) {
        self.continuous.remeasure();
        cx.notify();
    }

    fn request_dialog(&mut self, directory: bool, window: &mut Window, cx: &mut Context<Self>) {
        let language = self.language();
        let choice = cx.prompt_for_paths(gpui_kit::PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: false,
            prompt: Some(
                if directory {
                    language.text(Text::OpenImageFolder)
                } else {
                    language.text(Text::OpenComicOrImage)
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
                    view.set_localized_message(
                        Text::OpenDialogFailed,
                        vec![("error", format!("{error:?}"))],
                        true,
                        cx,
                    )
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
                        view.set_localized_message(
                            Text::SaveDialogFailed,
                            vec![("error", format!("{error:?}"))],
                            true,
                            cx,
                        )
                    });
                    return;
                }
            };
            let task = cx.background_spawn(async move { book.save_page(index, &destination) });
            let result = task.await;
            _ = view.update_in(cx, |view, _, cx| match result {
                Ok(()) => view.set_localized_message(Text::ImageSaved, vec![], false, cx),
                Err(error) => view.set_error(error, cx),
            });
        })
        .detach();
    }

    fn enter_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.content_mode == ContentMode::Reading {
            self.paused_viewer_size = Some(self.viewer_size);
        }
        self.open_revision += 1;
        self.opening = false;
        self.reader.slideshow = false;
        self.zoom_open = false;
        self.info_open = false;
        self.content_mode = ContentMode::Browsing;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn show_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let origin = self.reading_origin.take();
        let folder = self.sidebar_folder_origin.take();
        self.enter_browser(window, cx);
        if let Some(folder) = folder {
            self.browse(folder, window, cx);
        } else if let Some(origin) = origin {
            let parent = origin.parent().unwrap_or(&origin).to_path_buf();
            self.browse(parent, window, cx);
            self.pending_browser_selection = Some(origin);
        } else {
            self.browser_resume(cx);
        }
    }

    fn resume_reading(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reader.book.is_none() {
            return;
        }
        self.remember_browser_location();
        self.open_revision += 1;
        self.listing_revision += 1;
        self.opening = false;
        if self
            .paused_viewer_size
            .take()
            .is_some_and(|size| size != self.viewer_size)
        {
            self.continuous.remeasure();
        }
        self.content_mode = ContentMode::Reading;
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn reading_command_blocked(&self, command: Command) -> bool {
        self.content_mode == ContentMode::Browsing && command.reading_only()
            // Direction is also an application preference. Keep its existing
            // empty-window setup behavior without changing a retained book.
            && !(command == Command::Direction && self.reader.book.is_none())
    }

    fn perform(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        let language = self.language();
        use Command::*;
        if self.info_open
            && matches!(
                command,
                FitWindow | FitWidth | FitHeight | Original | ZoomIn | ZoomOut | ZoomReset
            )
        {
            return;
        }
        if cx.has_active_drag()
            && matches!(
                command,
                Sidebar
                    | Inspector
                    | Thumbnails
                    | Settings
                    | Browse
                    | ContinueReading
                    | BrowserBack
                    | BrowserOpen
                    | BrowserRefresh
                    | Escape
                    | Left
                    | Right
                    | ScrollUp
                    | ScrollDown
                    | First
                    | Last
            )
        {
            return;
        }
        if self.content_mode == ContentMode::Browsing {
            let step = self.browser.columns.max(1) as isize;
            match command {
                Left => {
                    self.browser_move(-1, window, cx);
                    return;
                }
                Right => {
                    self.browser_move(1, window, cx);
                    return;
                }
                ScrollUp => {
                    self.browser_move(-step, window, cx);
                    return;
                }
                ScrollDown => {
                    self.browser_move(step, window, cx);
                    return;
                }
                First => {
                    self.browser_move(-isize::MAX, window, cx);
                    return;
                }
                Last => {
                    self.browser_move(isize::MAX, window, cx);
                    return;
                }
                _ if self.reading_command_blocked(command) => return,
                _ => {}
            }
        }
        match command {
            Browse => self.show_browser(window, cx),
            ContinueReading => self.resume_reading(window, cx),
            BrowserBack => {
                if self.content_mode == ContentMode::Reading {
                    self.show_browser(window, cx);
                } else if let Some(parent) = self
                    .listing
                    .as_ref()
                    .and_then(|listing| listing.parent.clone())
                {
                    self.browse(parent, window, cx);
                }
            }
            BrowserOpen => {
                if self.content_mode == ContentMode::Browsing {
                    self.browser_open(window, cx);
                }
            }
            BrowserRefresh => {
                if self.content_mode == ContentMode::Browsing {
                    self.browser_refresh(window, cx);
                }
            }
            OpenFile => self.request_dialog(false, window, cx),
            OpenFolder => self.request_dialog(true, window, cx),
            SavePage => self.save_page(window, cx),
            Quit => {
                self.closing = true;
                self.close_preferences(cx);
                cx.quit();
            }
            Close => {
                self.info_open = false;
                self.zoom_open = false;
                self.focus.focus(window, cx);
                self.open_revision += 1;
                self.book_revision += 1;
                self.reader.close();
                self.content_mode = ContentMode::Browsing;
                self.paused_viewer_size = None;
                self.book_recursive = false;
                self.reading_origin = None;
                self.sidebar_folder_origin = None;
                self.files_close(cx);
                self.pending_browser_selection = None;
                self.listing_revision += 1;
                self.clear_browser_covers(cx);
                self.browser_resume(cx);
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
                self.set_localized_message(Text::BookClosed, vec![], false, cx);
                if self.listing.is_none() {
                    self.load_browser_listing(initial_directory(), cx);
                }
            }
            Sidebar => {
                self.reader.settings.preferences.sidebar =
                    !self.reader.settings.preferences.sidebar;
                if !self.reader.settings.preferences.sidebar
                    && self.files_ui.focus.contains_focused(window, cx)
                {
                    self.focus.focus(window, cx);
                }
                self.dirty = true;
                self.relayout(cx);
            }
            Thumbnails => {
                self.reader.settings.preferences.sidebar = true;
                self.reader.settings.preferences.sidebar_mode = SidebarMode::Thumbnails;
                self.dirty = true;
                self.relayout(cx);
            }
            Inspector => {
                self.commit_focused_interval(window, cx);
                self.reader
                    .settings
                    .preferences
                    .panel_layout
                    .inspector_visible = !self
                    .reader
                    .settings
                    .preferences
                    .panel_layout
                    .inspector_visible;
                self.dirty = true;
                self.focus.focus(window, cx);
                self.relayout(cx);
            }
            ToggleRecursiveFolder => {
                self.reader.settings.preferences.recursive_folder =
                    !self.reader.settings.preferences.recursive_folder;
                self.dirty = true;
                cx.notify();
            }
            Single | SetSpread | SetContinuous => {
                let mode = match command {
                    SetSpread => ReadingMode::Spread,
                    SetContinuous => ReadingMode::Continuous,
                    _ => ReadingMode::Single,
                };
                self.set_reading_mode(mode, window, cx);
            }
            Spread => {
                let mode = if self.reader.settings.preferences.mode == ReadingMode::Spread {
                    ReadingMode::Single
                } else {
                    ReadingMode::Spread
                };
                self.set_reading_mode(mode, window, cx);
            }
            Continuous => {
                let mode = if self.reader.settings.preferences.mode == ReadingMode::Continuous {
                    ReadingMode::Single
                } else {
                    ReadingMode::Continuous
                };
                self.set_reading_mode(mode, window, cx);
            }
            Direction => {
                self.reader.settings.preferences.rtl = !self.reader.settings.preferences.rtl;
                self.dirty = true;
                cx.notify();
            }
            Fullscreen => window.toggle_fullscreen(),
            Escape => {
                if self.info_open || self.zoom_open {
                    self.info_open = false;
                    self.zoom_open = false;
                    self.focus.focus(window, cx);
                    cx.notify();
                    return;
                }
                if self.content_mode == ContentMode::Browsing {
                    self.resume_reading(window, cx);
                    return;
                }
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
                self.zoom_open = false;
                self.relayout(cx);
                self.sync_zoom_input(true, window, cx);
                self.sync_controls(window, cx);
            }
            ZoomIn | ZoomOut | ZoomReset => {
                if command == ZoomReset {
                    self.apply_zoom(1., window, cx);
                } else {
                    self.zoom_by(if command == ZoomIn { 1.15 } else { 1. / 1.15 }, window, cx);
                }
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
                            self.set_localized_message(
                                Text::BookmarkSet,
                                vec![("page", (self.reader.current + 1).to_string())],
                                false,
                                cx,
                            );
                        }
                        Resume => match self.reader.settings.bookmarks.get(&path).copied() {
                            Some(page) => self.navigate(page, window, cx),
                            None => self.set_localized_message(Text::NoBookmark, vec![], true, cx),
                        },
                        _ => {
                            self.reader.settings.bookmarks.remove(&path);
                            self.set_localized_message(Text::BookmarkDeleted, vec![], false, cx);
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
            Settings => self.open_preferences(window, cx),
            About => window.open_dialog(cx, move |dialog, _, _| {
                dialog
                    .title(language.text(Text::AboutTitle))
                    .child(concat!("MangaMeeya Rust ", env!("CARGO_PKG_VERSION")))
                    .child("Rust · GPUI Kit · Windows / macOS")
                    .child(language.text(Text::AboutDescription))
                    .child("AGPL-3.0-only")
            }),
            ClearHistory => {
                self.reader.settings.recent.clear();
                self.dirty = true;
                cx.notify();
            }
            OpenListedFolder => {
                if self.content_mode == ContentMode::Browsing
                    && let Some(listing) = &self.listing
                {
                    self.open_browser_path(listing.path.clone(), window, cx);
                }
            }
        }
    }

    fn load_requested(&mut self, cx: &mut Context<Self>) {
        if self.content_mode != ContentMode::Reading || self.closing {
            return;
        }
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

    fn page_dimensions(&self, index: usize) -> Option<(u32, u32)> {
        if let Some(page) = self.pages.get(&index) {
            return Some((page.width, page.height));
        }
        self.geometry.get(&index).map(|page| {
            if (self.reader.rotation - page.rotation).rem_euclid(180) == 90 {
                (page.height, page.width)
            } else {
                (page.width, page.height)
            }
        })
    }

    fn effective_scale(&self) -> Option<f32> {
        if self.content_mode != ContentMode::Reading
            || self.reader.book.is_none()
            || self.viewer_bounds.size.width <= px(0.)
            || self.viewer_bounds.size.height <= px(0.)
            || self.errors.contains_key(&(self.reader.current, false))
        {
            return None;
        }
        let (width, height) = self.page_dimensions(self.reader.current)?;
        let count = self.reader.visible_indices().len();
        Some(self.page_layout(width, height, count).scale)
    }

    fn layout_for(&self, width: u32, height: u32, count: usize) -> (f32, f32) {
        let layout = self.page_layout(width, height, count);
        (layout.width, layout.height)
    }

    fn page_layout(&self, width: u32, height: u32, count: usize) -> crate::reader::PageLayout {
        let width = width.max(1) as f32;
        let height = height.max(1) as f32;
        let available_width =
            (f32::from(self.viewer_size.width) - 24.).max(1.) / count.max(1) as f32;
        let available_height = (f32::from(self.viewer_size.height) - 24.).max(1.);
        crate::reader::fitted_page_layout(
            width,
            height,
            0,
            self.reader.settings.preferences.fit,
            self.reader.zoom,
            available_width,
            available_height,
        )
    }

    fn page_element(
        &self,
        index: usize,
        count: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let language = self.language();
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
            .aria_label(language.format(Text::PageLabel, &[("page", (index + 1).to_string())]))
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
                .child(language.format(
                    Text::PageTitle,
                    &[("page", (index + 1).to_string()), ("name", name.into())],
                ))
                .child(language.format(Text::PageError, &[("error", error.clone())]))
                .child(
                    Button::new(("retry", index))
                        .label(language.text(Text::Retry))
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
        } else if let Some((width, height)) = self.page_dimensions(index) {
            let (width, height) = self.layout_for(width, height, count);
            frame = frame
                .w(px(width + 8.))
                .h(px(height + 8.))
                .text_color(cx.theme().muted_foreground)
                .child(language.format(Text::PageLoading, &[("page", (index + 1).to_string())]));
        } else {
            frame = frame
                .w(px((f32::from(self.viewer_size.width)
                    / count.max(1) as f32
                    - 16.)
                    .max(1.)))
                .h(self.viewer_size.height - px(16.))
                .text_color(cx.theme().muted_foreground)
                .child(language.format(Text::PageLoading, &[("page", (index + 1).to_string())]));
        }
        frame.into_any_element()
    }

    fn icon_button(
        &self,
        id: &'static str,
        label: &'static str,
        icon: IconName,
        command: Command,
        cx: &mut Context<Self>,
    ) -> Button {
        // An empty label is still a text button in GPUI Kit. Do not add one:
        // normal text padding and the empty-label gap can clip the icon.
        self.command_button(id, label, command, cx)
            .icon(icon)
            .tooltip(label)
            .w(px(CONTROL_HEIGHT))
            .h(px(CONTROL_HEIGHT))
    }

    fn set_reading_mode(&mut self, mode: ReadingMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.reader.settings.preferences.mode == mode {
            return;
        }
        self.reader.set_mode(mode);
        self.dirty = true;
        self.relayout(cx);
        self.after_navigation(window, cx);
    }

    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        command: Command,
        cx: &mut Context<Self>,
    ) -> Button {
        self.command_button(id, label, command, cx).label(label)
    }

    fn command_button(
        &self,
        id: &'static str,
        label: &'static str,
        command: Command,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .small()
            .ghost()
            .accessibility_label(label)
            .disabled(self.reading_command_blocked(command))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.perform(command, window, cx);
                if command != Command::Settings {
                    this.focus.focus(window, cx);
                }
            }))
    }

    fn menu_items(&self, group: usize, menu: PopupMenu) -> PopupMenu {
        let language = self.language();
        use Command::*;
        let opened = self.reader.book.is_some() && self.content_mode == ContentMode::Reading;
        match group {
            0 => menu
                .menu(language.text(Text::OpenComic), Box::new(OpenFile))
                .menu(language.text(Text::OpenFolder), Box::new(OpenFolder))
                .menu_with_disabled(
                    language.text(Text::FilesReadFolder),
                    Box::new(OpenListedFolder),
                    self.content_mode != ContentMode::Browsing || self.listing.is_none(),
                )
                .separator()
                .label(language.text(Text::OpenOptions))
                .menu_with_check(
                    language.text(Text::IncludeSubfolders),
                    self.reader.settings.preferences.recursive_folder,
                    Box::new(ToggleRecursiveFolder),
                )
                .separator()
                .menu_with_disabled(language.text(Text::SaveImage), Box::new(SavePage), !opened)
                .separator()
                .menu_with_disabled(
                    language.text(Text::CloseComic),
                    Box::new(Close),
                    self.reader.book.is_none(),
                )
                .menu(language.text(Text::Quit), Box::new(Quit)),
            1 => menu
                .menu(language.text(Text::BrowseFiles), Box::new(Browse))
                .menu_with_disabled(
                    language.text(Text::ContinueReading),
                    Box::new(ContinueReading),
                    self.reader.book.is_none(),
                )
                .separator()
                .menu_with_check(
                    language.text(Text::ShowSidebar),
                    self.reader.settings.preferences.sidebar,
                    Box::new(Sidebar),
                )
                .menu(language.text(Text::Thumbnails), Box::new(Thumbnails))
                .menu_with_check(
                    language.text(Text::ShowInspector),
                    self.reader
                        .settings
                        .preferences
                        .panel_layout
                        .inspector_visible,
                    Box::new(Inspector),
                )
                .menu_with_check(
                    language.text(Text::Spread),
                    self.reader.settings.preferences.mode == ReadingMode::Spread,
                    Box::new(Spread),
                )
                .menu_with_check(
                    language.text(Text::ContinuousPages),
                    self.reader.settings.preferences.mode == ReadingMode::Continuous,
                    Box::new(Continuous),
                )
                .menu_with_check(
                    language.text(Text::RightToLeft),
                    self.reader.settings.preferences.rtl,
                    Box::new(Direction),
                )
                .separator()
                .menu(language.text(Text::Fullscreen), Box::new(Fullscreen)),
            2 => menu
                .menu(language.text(Text::FitWindow), Box::new(FitWindow))
                .menu(language.text(Text::FitWidth), Box::new(FitWidth))
                .menu(language.text(Text::FitHeight), Box::new(FitHeight))
                .menu(language.text(Text::OriginalSize), Box::new(Original))
                .separator()
                .menu(language.text(Text::ZoomIn), Box::new(ZoomIn))
                .menu(language.text(Text::ZoomOut), Box::new(ZoomOut))
                .menu("100%", Box::new(ZoomReset)),
            3 => menu
                .menu(language.text(Text::FirstPage), Box::new(First))
                .menu(language.text(Text::PreviousPage), Box::new(Previous))
                .menu(language.text(Text::NextPage), Box::new(Next))
                .menu(language.text(Text::LastPage), Box::new(Last))
                .separator()
                .menu_with_check(
                    language.text(Text::Slideshow),
                    self.reader.slideshow,
                    Box::new(Slideshow),
                ),
            4 => menu
                .menu_with_disabled(
                    language.text(Text::SetBookmark),
                    Box::new(Bookmark),
                    !opened,
                )
                .menu_with_disabled(language.text(Text::GoToBookmark), Box::new(Resume), !opened)
                .menu_with_disabled(
                    language.text(Text::DeleteBookmark),
                    Box::new(ClearBookmark),
                    !opened,
                ),
            5 => menu
                .menu(language.text(Text::RotateLeft), Box::new(RotateLeft))
                .menu(language.text(Text::RotateRight), Box::new(RotateRight))
                .menu(language.text(Text::ResetFilters), Box::new(ResetFilters))
                .separator()
                .menu(language.text(Text::Preferences), Box::new(Settings)),
            _ => menu.menu(language.text(Text::AboutMenu), Box::new(About)),
        }
    }

    fn render_zoom_control(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
        let scale = self.effective_scale();
        let percent = zoom_percentage(scale);
        let fit = self.reader.settings.preferences.fit;
        let mode = fit_label(language, fit);
        let enabled = scale.is_some();
        let input = self.zoom_input.clone();
        let owner = cx.entity().downgrade();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .child(
                Popover::new("zoom-popover")
                    .open(
                        self.zoom_open
                            && self.reader.book.is_some()
                            && self.content_mode == ContentMode::Reading,
                    )
                    .on_open_change(cx.listener(|this, open: &bool, window, cx| {
                        this.zoom_open = *open
                            && this.reader.book.is_some()
                            && this.content_mode == ContentMode::Reading;
                        if this.zoom_open {
                            this.info_open = false;
                        }
                        this.sync_zoom_input(true, window, cx);
                        cx.notify();
                    }))
                    .trigger(
                        Button::new("zoom-trigger")
                            .small()
                            .outline()
                            .h(px(CONTROL_HEIGHT))
                            .label(format!("{mode}  |  {percent}"))
                            .child(Icon::new(IconName::ChevronDown).small())
                            .accessibility_label(language.format(
                                Text::ZoomSummary,
                                &[("mode", mode.into()), ("percent", percent.clone())],
                            ))
                            .disabled(
                                self.reader.book.is_none()
                                    || self.content_mode != ContentMode::Reading,
                            )
                            .tooltip(language.text(Text::ZoomTip)),
                    )
                    .content(move |_, _, cx| {
                        let mut stepper = div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(language.text(Text::ZoomLevel));
                        for (id, label, command) in [
                            ("zoom-out", "−", Command::ZoomOut),
                            ("zoom-in", "+", Command::ZoomIn),
                        ] {
                            if command == Command::ZoomIn {
                                stepper = stepper
                                    .child(
                                        Input::new(&input)
                                            .id("zoom-input")
                                            .aria_label(language.text(Text::ZoomPercentage))
                                            .small()
                                            .w_16()
                                            .disabled(!enabled),
                                    )
                                    .child("%");
                            }
                            let owner = owner.clone();
                            stepper = stepper.child(
                                Button::new(id)
                                    .small()
                                    .outline()
                                    .label(label)
                                    .accessibility_label(if command == Command::ZoomIn {
                                        language.text(Text::ZoomIn)
                                    } else {
                                        language.text(Text::ZoomOut)
                                    })
                                    .disabled(!enabled)
                                    .on_click(move |_, window, cx| {
                                        _ = owner.update(cx, |this, cx| {
                                            this.perform(command, window, cx);
                                        });
                                    }),
                            );
                        }
                        let mut choices = div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .pt_3()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(language.text(Text::FitMode)),
                            );
                        for (id, value, shortcut, description, command) in [
                            (
                                "zoom-fit-window",
                                FitMode::Window,
                                "F",
                                language.text(Text::FitWindowDescription),
                                Command::FitWindow,
                            ),
                            (
                                "zoom-fit-width",
                                FitMode::Width,
                                "W",
                                language.text(Text::FitWidthDescription),
                                Command::FitWidth,
                            ),
                            (
                                "zoom-fit-height",
                                FitMode::Height,
                                "H",
                                language.text(Text::FitHeightDescription),
                                Command::FitHeight,
                            ),
                            (
                                "zoom-original",
                                FitMode::Original,
                                "1",
                                language.text(Text::OriginalDescription),
                                Command::Original,
                            ),
                        ] {
                            let selected = fit == value;
                            let owner = owner.clone();
                            choices = choices.child(
                                Button::new(id)
                                    .small()
                                    .ghost()
                                    .w_full()
                                    .h_auto()
                                    .py_2()
                                    .selected(selected)
                                    .toggled(selected)
                                    .accessibility_label(fit_label(language, value))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .w_full()
                                            .child(div().w_4().when(selected, |this| {
                                                this.child(Icon::new(IconName::Check).small())
                                            }))
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .items_start()
                                                    .flex_1()
                                                    .child(
                                                        div()
                                                            .text_sm()
                                                            .child(fit_label(language, value)),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child(description),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(shortcut),
                                            ),
                                    )
                                    .on_click(move |_, window, cx| {
                                        _ = owner.update(cx, |this, cx| {
                                            this.perform(command, window, cx);
                                        });
                                    }),
                            );
                        }
                        div()
                            .id("zoom-panel")
                            .test_support()
                            .role(gpui_kit::Role::Group)
                            .aria_label(language.text(Text::ZoomSettings))
                            .flex()
                            .flex_col()
                            .w_80()
                            .gap_3()
                            .child(stepper)
                            .child(choices)
                    }),
            )
            .into_any_element()
    }

    fn render_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let width = f32::from(window.viewport_size().width);
        let prefs = &self.reader.settings.preferences;
        let left_visible = prefs.sidebar;
        let right_visible = prefs.panel_layout.inspector_visible;
        let key = (width.round() as u32, left_visible, right_visible);
        // Retain the entity and the document/input entities. Only reset the layout slots
        // when their topology or viewport changes, never during an active handle drag.
        if self.panel_layout_key != key && !cx.has_active_drag() {
            self.panel_state.update(cx, |state, _| state.clear());
            self.panel_layout_key = key;
        }
        let (left, right) = panel_widths(
            width,
            left_visible.then_some(prefs.panel_layout.sidebar_width),
            right_visible.then_some(prefs.panel_layout.inspector_width),
        );
        let left_max = (width - 288. - right).clamp(216., 320.);
        let right_max = (width - 288. - left).clamp(240., 360.);
        let owner = cx.entity().downgrade();
        let mut group = h_resizable("reader-panels")
            .with_state(&self.panel_state)
            .on_resize(move |state, _, cx| {
                let sizes = state.read(cx).sizes().clone();
                _ = owner.update(cx, |this, cx| {
                    if let Some((is_left, before)) = this.panel_drag_start.take() {
                        let size = if is_left { sizes.first() } else { sizes.last() };
                        if let Some(size) = size {
                            let (minimum, maximum) = if is_left {
                                (216., left_max)
                            } else {
                                (240., right_max)
                            };
                            let visible = f32::from(*size).clamp(minimum, maximum);
                            let before = f32::from(before).clamp(minimum, maximum);
                            if visible.is_finite()
                                && before.is_finite()
                                && (visible - before).abs() >= 0.5
                            {
                                let layout = &mut this.reader.settings.preferences.panel_layout;
                                let saved = if is_left {
                                    &mut layout.sidebar_width
                                } else {
                                    &mut layout.inspector_width
                                };
                                let value = visible.round() as u32;
                                if *saved != value {
                                    *saved = value;
                                    this.dirty = true;
                                }
                            }
                        }
                    }
                    // The component can retain an overdrag's raw size while painting
                    // its clamped size. Reconcile slots from preferences after release.
                    this.panel_layout_key = (0, false, false);
                    cx.notify();
                });
            });
        if left_visible {
            group = group.child(
                resizable_panel()
                    .size(px(left))
                    .size_range(px(216.)..px(left_max))
                    .fixed()
                    .child(self.render_sidebar(cx)),
            );
        }
        group = group.child(resizable_panel().size_range(px(280.)..Pixels::MAX).child(
            match self.content_mode {
                ContentMode::Reading => self.render_viewer(cx),
                ContentMode::Browsing => self.render_browser(window, cx),
            },
        ));
        if right_visible {
            group = group.child(
                resizable_panel()
                    .size(px(right))
                    .size_range(px(240.)..px(right_max))
                    .fixed()
                    .child(self.render_inspector(cx)),
            );
        }
        let capture_owner = cx.entity().downgrade();
        div()
            .id("reader-workspace")
            .test_support()
            .role(gpui_kit::Role::Group)
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(group)
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let owner = capture_owner.clone();
                        // The resize handle has its own hitbox. Capture before it consumes
                        // the pointer; do not use an occluded parent hitbox for this baseline.
                        window.on_mouse_event(
                            move |event: &gpui_kit::MouseDownEvent, phase, _, cx| {
                                if phase != gpui_kit::DispatchPhase::Capture
                                    || event.button != MouseButton::Left
                                    || !bounds.contains(&event.position)
                                {
                                    return;
                                }
                                _ = owner.update(cx, |this, cx| {
                                    let sizes = this.panel_state.read(cx).sizes();
                                    this.panel_drag_start =
                                        if this.reader.settings.preferences.sidebar
                                            && (event.position.x - this.viewer_bounds.left()).abs()
                                                <= px(8.)
                                        {
                                            sizes.first().copied().map(|size| (true, size))
                                        } else if this
                                            .reader
                                            .settings
                                            .preferences
                                            .panel_layout
                                            .inspector_visible
                                            && (event.position.x - this.viewer_bounds.right()).abs()
                                                <= px(8.)
                                        {
                                            sizes.last().copied().map(|size| (false, size))
                                        } else {
                                            None
                                        };
                                });
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

    fn render_chrome(&self, compact: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
        let mut menus = div()
            .flex()
            .items_center()
            .h(px(MENU_HEIGHT))
            .gap_1()
            .px_3()
            .border_b_1()
            .border_color(cx.theme().border);
        for (group, label) in [
            Text::File,
            Text::View,
            Text::Zoom,
            Text::Navigate,
            Text::Bookmarks,
            Text::Tools,
            Text::Help,
        ]
        .into_iter()
        .enumerate()
        {
            let view = cx.entity().downgrade();
            menus = menus.child(
                Button::new(("menu", group))
                    .small()
                    .ghost()
                    .label(language.text(label))
                    .dropdown_menu(move |menu, _, cx| {
                        if let Some(view) = view.upgrade() {
                            view.read(cx).menu_items(group, menu)
                        } else {
                            menu
                        }
                    }),
            );
        }
        let opened = self.reader.book.is_some() && self.content_mode == ContentMode::Reading;
        let mut modes = div()
            .id("reading-modes")
            .test_support()
            .role(gpui_kit::Role::TabList)
            .aria_label(language.text(Text::ReadingMode))
            .flex()
            .flex_none()
            .gap_1()
            .p_1()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar);
        for (id, mode, text, icon, command) in [
            (
                "mode-single",
                ReadingMode::Single,
                Text::SinglePage,
                IconName::FileText,
                Command::Single,
            ),
            (
                "mode-spread",
                ReadingMode::Spread,
                Text::TwoPages,
                IconName::Columns2,
                Command::SetSpread,
            ),
            (
                "mode-continuous",
                ReadingMode::Continuous,
                Text::Continuous,
                IconName::Rows3,
                Command::SetContinuous,
            ),
        ] {
            let selected = self.reader.settings.preferences.mode == mode;
            modes = modes.child(
                self.command_button(id, language.text(text), command, cx)
                    .icon(icon)
                    .when(!compact, |button| button.label(language.text(text)))
                    .when(compact, |button| button.w(px(CONTROL_HEIGHT)))
                    .role(gpui_kit::Role::Tab)
                    .selected(selected)
                    .toggled(selected)
                    .tooltip(language.text(text))
                    .disabled(!opened)
                    .h(px(30.))
                    .when(selected, |button| {
                        button
                            .bg(cx.theme().background)
                            .text_color(cx.theme().primary)
                            .shadow_xs()
                    }),
            );
        }
        let direction = direction_label(language, self.reader.settings.preferences.rtl);
        let marked = self.reader.book.as_ref().is_some_and(|book| {
            self.reader.settings.bookmarks.get(&book.path) == Some(&self.reader.current)
        });
        let left_visible = self.reader.settings.preferences.sidebar;
        let right_visible = self
            .reader
            .settings
            .preferences
            .panel_layout
            .inspector_visible;
        let border = cx.theme().border;
        let separator = || div().w(px(1.)).h_6().flex_none().bg(border);
        let toolbar = div()
            .id("reader-toolbar")
            .test_support()
            .role(gpui_kit::Role::Group)
            .flex()
            .items_center()
            .h(px(TOOLBAR_HEIGHT))
            .flex_none()
            .gap_3()
            .px_4()
            .when(compact, |bar| bar.gap_2().px_2())
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .child(
                        self.icon_button(
                            "open",
                            language.text(Text::Open),
                            IconName::FolderOpen,
                            Command::OpenFile,
                            cx,
                        )
                        .when(!compact, |button| {
                            button.w_auto().px_3().label(language.text(Text::Open))
                        }),
                    )
                    .child(
                        self.icon_button(
                            "sidebar-toggle",
                            language.text(Text::Sidebar),
                            IconName::PanelLeft,
                            Command::Sidebar,
                            cx,
                        )
                        .selected(left_visible)
                        .toggled(left_visible)
                        .when(left_visible, |button| {
                            button
                                .bg(cx.theme().primary.opacity(0.1))
                                .text_color(cx.theme().primary)
                        }),
                    ),
            )
            .child(
                self.icon_button(
                    "browser-toggle",
                    language.text(if self.content_mode == ContentMode::Reading {
                        Text::BrowseFiles
                    } else {
                        Text::ContinueReading
                    }),
                    if self.content_mode == ContentMode::Reading {
                        IconName::FolderOpen
                    } else {
                        IconName::FileText
                    },
                    if self.content_mode == ContentMode::Reading {
                        Command::Browse
                    } else {
                        Command::ContinueReading
                    },
                    cx,
                )
                .disabled(self.content_mode == ContentMode::Browsing && self.reader.book.is_none()),
            )
            .child(separator())
            .child(modes)
            .child(separator())
            .child(self.render_zoom_control(cx))
            .child(
                self.icon_button(
                    "direction",
                    language.text(Text::ReadingDirection),
                    IconName::ArrowLeftRight,
                    Command::Direction,
                    cx,
                )
                .when(!compact, |button| {
                    button.w_auto().px_3().child(direction.test_text(direction))
                })
                .disabled(self.reading_command_blocked(Command::Direction))
                .tooltip(format!(
                    "{}: {direction}",
                    language.text(Text::ReadingDirection)
                )),
            )
            .child(div().flex_1().min_w_0())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .child(
                        self.icon_button(
                            "bookmark",
                            language.text(Text::Bookmark),
                            IconName::Bookmark,
                            Command::Bookmark,
                            cx,
                        )
                        .selected(marked)
                        .toggled(marked)
                        .disabled(!opened)
                        .when(marked, |button| {
                            button
                                .bg(cx.theme().primary.opacity(0.1))
                                .text_color(cx.theme().primary)
                        }),
                    )
                    .child(
                        self.icon_button(
                            "inspector-toggle",
                            language.text(Text::ReadingTools),
                            IconName::SlidersHorizontal,
                            Command::Inspector,
                            cx,
                        )
                        .selected(right_visible)
                        .toggled(right_visible)
                        .when(right_visible, |button| {
                            button
                                .bg(cx.theme().primary.opacity(0.1))
                                .text_color(cx.theme().primary)
                        }),
                    )
                    .child(self.icon_button(
                        "fullscreen",
                        language.text(Text::Fullscreen),
                        IconName::Maximize,
                        Command::Fullscreen,
                        cx,
                    ))
                    .child(self.icon_button(
                        "preferences",
                        language.text(Text::Preferences),
                        IconName::Ellipsis,
                        Command::Settings,
                        cx,
                    )),
            );
        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(menus)
            .child(toolbar)
            .into_any_element()
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
        let mut tabs = div()
            .id("sidebar-tabs")
            .test_support()
            .role(gpui_kit::Role::TabList)
            .aria_label(language.text(Text::Sidebar))
            .flex()
            .gap_1()
            .px_2()
            .pb_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .flex_none();
        for (mode, id, label) in [
            (SidebarMode::Files, "檔案", language.text(Text::File)),
            (
                SidebarMode::Thumbnails,
                "縮圖",
                language.text(Text::Thumbnails),
            ),
            (SidebarMode::Recent, "最近開啟", language.text(Text::Recent)),
        ] {
            tabs = tabs.child(
                Button::new(id)
                    .small()
                    .ghost()
                    .label(label)
                    .tooltip(label)
                    .flex_1()
                    .min_w_0()
                    .h(px(32.))
                    .role(gpui_kit::Role::Tab)
                    .selected(self.reader.settings.preferences.sidebar_mode == mode)
                    .toggled(self.reader.settings.preferences.sidebar_mode == mode)
                    .when(
                        self.reader.settings.preferences.sidebar_mode == mode,
                        |tab| {
                            tab.bg(cx.theme().primary.opacity(0.1))
                                .text_color(cx.theme().primary)
                        },
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.focus.focus(window, cx);
                        this.reader.settings.preferences.sidebar_mode = mode;
                        this.dirty = true;
                        cx.notify();
                    })),
            );
        }
        let content = match self.reader.settings.preferences.sidebar_mode {
            SidebarMode::Thumbnails => {
                if self.reader.book.is_none() {
                    div()
                        .p_4()
                        .child(language.text(Text::NoBook))
                        .into_any_element()
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
                    recent = recent.child(language.text(Text::NoHistory));
                }
                for path in &self.reader.settings.recent {
                    let target = path.clone();
                    recent = recent.child(
                        Button::new(SharedString::from(format!("recent:{}", path.display())))
                            .ghost()
                            .accessibility_label(
                                path.file_name()
                                    .unwrap_or(path.as_os_str())
                                    .to_string_lossy()
                                    .into_owned(),
                            )
                            .child(
                                div().w_full().min_w_0().text_left().truncate().child(
                                    path.file_name()
                                        .unwrap_or(path.as_os_str())
                                        .to_string_lossy()
                                        .into_owned(),
                                ),
                            )
                            .tooltip(path.to_string_lossy().into_owned())
                            .w_full()
                            .h(px(CONTROL_HEIGHT))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_path(target.clone(), window, cx)
                            })),
                    );
                }
                if !self.reader.settings.recent.is_empty() {
                    recent = recent.child(self.button(
                        "clear-history",
                        language.text(Text::ClearHistory),
                        Command::ClearHistory,
                        cx,
                    ));
                }
                recent.into_any_element()
            }
        };
        div()
            .id("sidebar")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::Sidebar))
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .h_full()
            .flex_none()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .id("sidebar-heading")
                    .test_support()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .h(px(48.))
                    .px_4()
                    .flex_none()
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(
                                language
                                    .text(Text::ContentNavigation)
                                    .test_text(language.text(Text::ContentNavigation)),
                            ),
                    )
                    .when(
                        self.reader.settings.preferences.sidebar_mode == SidebarMode::Thumbnails,
                        |heading| {
                            heading.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(language.format(
                                        Text::PageCount,
                                        &[("count", self.page_count().to_string())],
                                    )),
                            )
                        },
                    ),
            )
            .child(tabs)
            .child(content)
            .when_some(self.reader.book.as_ref(), |sidebar, book| {
                sidebar.child(
                    div()
                        .id("sidebar-book-title")
                        .test_support()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .px_4()
                        .py_3()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .flex_none()
                        .truncate()
                        .child(book.title.clone()),
                )
            })
            .into_any_element()
    }

    fn thumbnail(&self, index: usize, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
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
                .w(px(110.))
                .h(px(156.))
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            None => div()
                .w(px(110.))
                .h(px(156.))
                .flex()
                .items_center()
                .justify_center()
                .child(if self.errors.contains_key(&(index, true)) {
                    language.text(Text::ReadFailed).to_owned()
                } else {
                    (index + 1).to_string()
                })
                .into_any_element(),
        };
        let selected = index == self.reader.current;
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
                .h(px(180.))
                .border_1()
                .rounded_md()
                .border_color(if selected {
                    cx.theme().primary.opacity(0.25)
                } else {
                    cx.theme().transparent
                })
                .when(selected, |button| {
                    button.bg(cx.theme().primary.opacity(0.1))
                })
                .selected(selected)
                .toggled(selected)
                .tooltip(page.name.clone())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .w_full()
                        .min_w_0()
                        .border_l_2()
                        .border_color(if selected {
                            cx.theme().primary
                        } else {
                            gpui_kit::hsla(0., 0., 0., 0.)
                        })
                        .pl_1()
                        .child(
                            div()
                                .id("thumbnail-container")
                                .test_support()
                                .w(px(110.))
                                .h(px(156.))
                                .flex_none()
                                .child(picture),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .id(("thumbnail-page", index))
                                        .test_support()
                                        .aria_label((index + 1).to_string())
                                        .text_size(px(13.))
                                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                        .when(selected, |label| {
                                            label.text_color(cx.theme().primary)
                                        })
                                        .child(language.format(
                                            Text::PageLabel,
                                            &[("page", (index + 1).to_string())],
                                        )),
                                )
                                .child(
                                    div()
                                        .id(("thumbnail-name", index))
                                        .test_support()
                                        .aria_label(page.name.clone())
                                        .w_full()
                                        .truncate()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(page.name.clone()),
                                ),
                        ),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.navigate(index, window, cx))),
            )
            .into_any_element()
    }

    fn render_image_tools(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
        let mut content = div().flex().flex_col().gap_6().flex_none();
        content = content.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(language.text(Text::ImageAdjustments)),
        );
        for (index, text, value) in [
            (0, Text::Brightness, self.reader.filters.brightness),
            (1, Text::Contrast, self.reader.filters.contrast),
            (2, Text::Grayscale, self.reader.filters.grayscale),
        ] {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(language.text(text))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{value:.0}%")),
                            ),
                    )
                    .child(
                        Slider::new(&self.filter_sliders[index])
                            .disabled(self.content_mode != ContentMode::Reading)
                            .accessibility_label(language.text(text)),
                    ),
            );
        }
        content = content
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(language.text(Text::Rotation))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                self.icon_button(
                                    "rotate-left",
                                    language.text(Text::RotateLeft),
                                    IconName::RotateCcw,
                                    Command::RotateLeft,
                                    cx,
                                )
                                .tooltip(language.text(Text::RotateLeftTip)),
                            )
                            .child(
                                self.icon_button(
                                    "rotate-right",
                                    language.text(Text::RotateRight),
                                    IconName::RotateCw,
                                    Command::RotateRight,
                                    cx,
                                )
                                .tooltip(language.text(Text::RotateRightTip)),
                            ),
                    ),
            )
            .child(
                self.button(
                    "reset-filters",
                    language.text(Text::ResetFilters),
                    Command::ResetFilters,
                    cx,
                )
                .outline()
                .border_color(cx.theme().border)
                .icon(IconName::RotateCcw)
                .h(px(CONTROL_HEIGHT))
                .w_full(),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        language
                            .text(Text::FiltersScope)
                            .test_text(language.text(Text::FiltersScope)),
                    ),
            );
        content.into_any_element()
    }

    fn render_playback_tools(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
        let mut content = div().flex().flex_col().gap_6().flex_none();
        content = content
            .child(
                self.button(
                    "slideshow",
                    language.text(if self.reader.slideshow {
                        Text::Stop
                    } else {
                        Text::Slideshow
                    }),
                    Command::Slideshow,
                    cx,
                )
                .selected(self.reader.slideshow)
                .toggled(self.reader.slideshow)
                .disabled(self.reader.book.is_none() || self.content_mode != ContentMode::Reading),
            )
            .child(language.text(Text::IntervalLabel))
            .child(
                Input::new(&self.interval_input)
                    .id("interval-input")
                    .aria_label(language.text(Text::SlideshowInterval))
                    .small(),
            );
        content.into_any_element()
    }

    fn render_inspector(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
        let mut tabs = div()
            .flex()
            .gap_4()
            .px_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .flex_none()
            .id("inspector-tabs")
            .test_support()
            .role(gpui_kit::Role::TabList)
            .aria_label(language.text(Text::ReadingTools));
        for (id, tab, text) in [
            ("inspector-image", InspectorTab::Image, Text::ImageTab),
            ("inspector-browser", InspectorTab::Browser, Text::BrowserTab),
            (
                "inspector-playback",
                InspectorTab::Playback,
                Text::PlaybackTab,
            ),
        ] {
            let selected = self.inspector_tab == tab;
            tabs = tabs.child(
                Button::new(id)
                    .small()
                    .ghost()
                    .label(language.text(text))
                    .tooltip(language.text(text))
                    .flex_1()
                    .min_w_0()
                    .h(px(40.))
                    .px_1()
                    .rounded_none()
                    .border_b_2()
                    .border_color(if selected {
                        cx.theme().primary
                    } else {
                        cx.theme().transparent
                    })
                    .bg(cx.theme().transparent)
                    .text_color(if selected {
                        cx.theme().primary
                    } else {
                        cx.theme().muted_foreground
                    })
                    .role(gpui_kit::Role::Tab)
                    .selected(selected)
                    .toggled(selected)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.commit_focused_interval(window, cx);
                        this.inspector_tab = tab;
                        this.focus.focus(window, cx);
                        cx.notify();
                    })),
            );
        }
        let content = div()
            .id("inspector-content")
            .test_support()
            .role(gpui_kit::Role::Group)
            .flex()
            .flex_col()
            .gap_6()
            .p_5()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(match self.inspector_tab {
                InspectorTab::Image => self.render_image_tools(cx),
                InspectorTab::Playback => self.render_playback_tools(cx),
                InspectorTab::Browser => self.render_browser_tools(cx),
            });
        div()
            .id("inspector")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::ReadingTools))
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(57.))
                    .px_4()
                    .flex_none()
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(language.text(Text::ReadingTools)),
                    )
                    .child(
                        self.icon_button(
                            "hide-inspector",
                            language.text(Text::HideInspector),
                            IconName::X,
                            Command::Inspector,
                            cx,
                        )
                        .bg(cx.theme().primary.opacity(0.1))
                        .text_color(cx.theme().primary),
                    ),
            )
            .child(tabs)
            .child(content)
            .child(
                div()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .px_5()
                    .py_3()
                    .flex_none()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(language.text(Text::SourceUnchanged)),
            )
            .into_any_element()
    }

    fn reset_corrupt_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_epoch.fetch_add(1, Ordering::SeqCst);
        let lock = self.save_lock.clone();
        let result = lock
            .lock()
            .map_err(|_| "Settings writer lock failed".to_string())
            .and_then(|_guard| AppSettings::reset_corrupt_at(&self.settings_path));
        match result {
            Ok(backup) => {
                self.reader.settings = AppSettings::default();
                self.panel_layout_key = (0, false, false);
                self.update_language(window, cx);
                self.save_error = None;
                self.interval_input
                    .update(cx, |input, cx| input.set_value("5", window, cx));
                self.sync_controls(window, cx);
                self.set_localized_message(
                    Text::SettingsReset,
                    vec![("path", backup.display().to_string())],
                    false,
                    cx,
                );
            }
            Err(error) => self.set_error(error, cx),
        }
    }

    fn render_viewer(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
        self.visible.borrow_mut().clear();
        // These colors are document-background preferences, not UI chrome colors.
        let background = background_color(self.reader.settings.preferences.background);
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
                .child(language.text(Text::EmptyHint))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(self.button(
                            "empty-open",
                            language.text(Text::OpenComic),
                            Command::OpenFile,
                            cx,
                        ))
                        .child(self.button(
                            "empty-folder",
                            language.text(Text::OpenFolder),
                            Command::OpenFolder,
                            cx,
                        )),
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
                .items_center()
                .justify_center()
                .w(px(content_width.max(f32::from(self.viewer_size.width))))
                .min_h_full();
            for index in indices {
                pages = pages.child(self.page_element(index, count, cx));
            }
            div()
                .relative()
                .size_full()
                .child(
                    div()
                        .id("paged-scroll")
                        .size_full()
                        .overflow_scroll()
                        .track_scroll(&self.paged_scroll)
                        .child(pages),
                )
                // Scrollbar overlays must not increase the measured content size.
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(Scrollbar::vertical(&self.paged_scroll))
                        .child(Scrollbar::horizontal(&self.paged_scroll)),
                )
                .into_any_element()
        };
        let view = cx.entity().downgrade();
        let bounds_view = view.clone();
        div()
            .id("viewer")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::ReadingArea))
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
                                        // A panel over the viewer must not pass wheel zoom through.
                                        if this.zoom_open
                                            || this.info_open
                                            || delta == px(0.)
                                            || !f32::from(delta).is_finite()
                                        {
                                            return;
                                        }
                                        this.zoom_by(
                                            if delta > px(0.) { 1.1 } else { 1. / 1.1 },
                                            window,
                                            cx,
                                        );
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

    fn render_page_information(&self) -> gpui_kit::AnyElement {
        let language = self.language();
        let current = self.reader.current;
        let name = self
            .reader
            .current_name()
            .unwrap_or(language.text(Text::NoBook));
        let unavailable = language.text(Text::NotAvailable);
        let dimensions = self
            .pages
            .get(&current)
            .map(|page| format!("{}×{}", page.width, page.height))
            .unwrap_or_else(|| unavailable.to_owned());
        let bytes = self
            .pages
            .get(&current)
            .map(|page| format!("{:.1} KiB", page.source_bytes as f64 / 1024.))
            .unwrap_or_else(|| unavailable.to_owned());
        div()
            .id("info-panel")
            .test_support()
            .role(gpui_kit::Role::Group)
            .aria_label(language.text(Text::PageInformation))
            .w_80()
            .flex()
            .flex_col()
            .gap_3()
            .child(language.text(Text::PageInformation))
            .child(
                div()
                    .id("info-name")
                    .test_support()
                    .aria_label(name.to_owned())
                    .w_full()
                    .overflow_x_scroll()
                    .child(name.to_owned()),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .child(language.text(Text::Dimensions))
                    .child(
                        div()
                            .id("info-dimensions")
                            .test_support()
                            .aria_label(dimensions.clone())
                            .child(dimensions),
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .child(language.text(Text::FileSize))
                    .child(
                        div()
                            .id("info-size")
                            .test_support()
                            .aria_label(bytes.clone())
                            .child(bytes),
                    ),
            )
            .into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let language = self.language();
        let count = self.page_count();
        let opened = count > 0 && self.content_mode == ContentMode::Reading;
        let percent = if count > 0 {
            ((self.reader.current + 1) as f64 / count as f64 * 100.).round() as u32
        } else {
            0
        };
        let progress = if count > 0 {
            language.format(Text::ReadProgress, &[("percent", percent.to_string())])
        } else {
            "—".to_owned()
        };
        let owner = cx.entity().downgrade();
        let info = Popover::new("info-popover")
            .open(self.info_open && opened)
            .on_open_change(cx.listener(|this, open: &bool, _, cx| {
                this.info_open = *open
                    && this.reader.book.is_some()
                    && this.content_mode == ContentMode::Reading;
                if this.info_open {
                    this.zoom_open = false;
                }
                cx.notify();
            }))
            .trigger(
                Button::new("info-trigger")
                    .small()
                    .ghost()
                    .icon(IconName::Info)
                    .w(px(CONTROL_HEIGHT))
                    .h(px(CONTROL_HEIGHT))
                    .accessibility_label(language.text(Text::PageInformation))
                    .tooltip(language.text(Text::PageInformation))
                    .disabled(!opened),
            )
            .content(move |_, _, cx| {
                owner.upgrade().map_or_else(
                    || div().into_any_element(),
                    |view| view.read(cx).render_page_information(),
                )
            });
        let message = div()
            .id("status-message")
            .test_support()
            .role(gpui_kit::Role::Status)
            .aria_label(self.message.clone())
            .flex_1()
            .min_w_0()
            .truncate()
            .text_xs()
            .text_color(if self.message_error {
                cx.theme().danger
            } else {
                cx.theme().muted_foreground
            })
            .child(self.message.clone());
        div()
            .id("reader-status")
            .test_support()
            .role(gpui_kit::Role::Group)
            .flex()
            .flex_col()
            .flex_none()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_4()
                    .h(px(STATUS_HEIGHT))
                    .child(message)
                    .child(
                        div()
                            .id("status-navigation")
                            .test_support()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap_2()
                            .child(
                                self.icon_button(
                                    "previous",
                                    language.text(Text::PreviousPage),
                                    IconName::ChevronLeft,
                                    Command::Previous,
                                    cx,
                                )
                                .disabled(!opened || self.reader.current == 0),
                            )
                            .child(
                                Input::new(&self.page_input)
                                    .id("page-input")
                                    .aria_label(language.text(Text::PageNumber))
                                    .small()
                                    .w_12()
                                    .disabled(!opened),
                            )
                            .child(format!("/ {count}"))
                            .child(
                                self.icon_button(
                                    "next",
                                    language.text(Text::NextPage),
                                    IconName::ChevronRight,
                                    Command::Next,
                                    cx,
                                )
                                .disabled(!opened || self.reader.current + 1 >= count),
                            )
                            .child(
                                div().w(px(145.)).px_1().child(
                                    Slider::new(&self.page_slider)
                                        .disabled(!opened)
                                        .accessibility_label(language.text(Text::PagePosition)),
                                ),
                            )
                            .child(
                                div()
                                    .id("reading-progress")
                                    .test_support()
                                    .aria_label(progress.clone())
                                    .text_xs()
                                    .child(progress),
                            ),
                    )
                    .child(div().flex().flex_1().min_w_0().justify_end().child(info)),
            )
            .when(self.message_error, |bar| {
                bar.child(
                    div()
                        .id("status-error-detail")
                        .text_xs()
                        .px_3()
                        .pb_2()
                        .text_color(cx.theme().danger)
                        .w_full()
                        .overflow_x_scroll()
                        .child(self.message.clone()),
                )
            })
            .into_any_element()
    }
}

impl Render for ReaderView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.language();
        self.sync_zoom_input(false, window, cx);
        self.visible_thumbnails.borrow_mut().clear();
        let workspace = self.render_workspace(window, cx);
        let drop_view = cx.entity().downgrade();
        div()
            .id("reader-window")
            .key_context(if self.content_mode == ContentMode::Browsing {
                "Reader FileBrowser"
            } else {
                "Reader"
            })
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
            .text_size(px(13.))
            .child(self.render_chrome(window.viewport_size().width < px(1200.), cx))
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
                            .child(language.text(Text::DropHint)),
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
    Theme::update(cx, |theme| {
        let accent = gpui_kit::rgb(0x376bc5).into();
        theme.primary = accent;
        theme.ring = accent;
        theme.slider_bar = accent;
        theme.slider_thumb = accent;
        if !theme.is_dark() {
            theme.background = gpui_kit::rgb(0xffffff).into();
            theme.foreground = gpui_kit::rgb(0x344054).into();
            theme.muted_foreground = gpui_kit::rgb(0x667085).into();
            theme.sidebar = gpui_kit::rgb(0xfbfcfe).into();
            theme.border = gpui_kit::rgb(0xe1e5eb).into();
        }
        #[cfg(target_os = "windows")]
        {
            // Use the installed Windows UI family. CJK uses the native fallback.
            theme.font_family = "Segoe UI".into();
        }
    });
    bind_commands(cx);
}

fn bind_commands(cx: &mut App) {
    use Command::*;
    let navigation = "Reader && !Input && !PopoverContent && !PopupMenu && !Dialog && !FilesTree";
    let reader = "Reader";
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
        ("[", RotateLeft),
        ("]", RotateRight),
        ("b", Bookmark),
        ("shift-b", Resume),
        ("p", Slideshow),
        ("t", Thumbnails),
        ("tab", Sidebar),
    ] {
        let context = if key == "tab" {
            "Reader && !Input && !PopoverContent && !PopupMenu && !Dialog && !FileBrowser && !FilesTree"
        } else {
            navigation
        };
        cx.bind_keys([KeyBinding::new(key, command, Some(context))]);
    }
    for (key, command) in [
        ("backspace", BrowserBack),
        ("enter", BrowserOpen),
        ("f5", BrowserRefresh),
        ("escape", Escape),
    ] {
        cx.bind_keys([KeyBinding::new(
            key,
            command,
            Some("Reader && !Input && !PopoverContent && !PopupMenu && !Dialog && !FilesTree"),
        )]);
    }
    for (key, command) in [
        ("f", FitWindow),
        ("w", FitWidth),
        ("h", FitHeight),
        ("1", Original),
        ("+", ZoomIn),
        ("=", ZoomIn),
        ("-", ZoomOut),
        ("0", ZoomReset),
    ] {
        cx.bind_keys([KeyBinding::new(
            key,
            command,
            Some("Reader && !Input && !FilesTree"),
        )]);
    }
    for (key, command) in [
        ("ctrl-o", OpenFile),
        ("ctrl-shift-o", OpenFolder),
        ("ctrl-s", SavePage),
        ("ctrl-w", Close),
        ("f11", Fullscreen),
    ] {
        cx.bind_keys([KeyBinding::new(key, command, Some(reader))]);
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
        cx.bind_keys([KeyBinding::new(key, command, Some(reader))]);
    }
    #[cfg(target_os = "windows")]
    cx.bind_keys([KeyBinding::new("alt-f4", Quit, Some(reader))]);
    set_native_menus(Language::default(), cx);
}

fn set_native_menus(language: Language, cx: &mut App) {
    use Command::*;
    cx.set_menus([
        Menu::new("MangaMeeya Rust").items([
            MenuItem::action(language.text(Text::AboutMenu), About),
            MenuItem::action(language.text(Text::Preferences), Settings),
            MenuItem::separator(),
            MenuItem::action(language.text(Text::Quit), Quit),
        ]),
        Menu::new(language.text(Text::File)).items([
            MenuItem::action(language.text(Text::OpenComic), OpenFile),
            MenuItem::action(language.text(Text::OpenFolder), OpenFolder),
            MenuItem::action(language.text(Text::FilesReadFolder), OpenListedFolder),
            MenuItem::action(
                language.text(Text::IncludeSubfolders),
                ToggleRecursiveFolder,
            ),
            MenuItem::action(language.text(Text::SaveImage), SavePage),
            MenuItem::action(language.text(Text::CloseComic), Close),
        ]),
        Menu::new(language.text(Text::View)).items([
            MenuItem::action(language.text(Text::BrowseFiles), Browse),
            MenuItem::action(language.text(Text::ContinueReading), ContinueReading),
            MenuItem::action(language.text(Text::Sidebar), Sidebar),
            MenuItem::action(language.text(Text::ShowInspector), Inspector),
            MenuItem::action(language.text(Text::SinglePage), Single),
            MenuItem::action(language.text(Text::Spread), Spread),
            MenuItem::action(language.text(Text::ContinuousPages), Continuous),
            MenuItem::action(language.text(Text::ReadingDirection), Direction),
            MenuItem::action(language.text(Text::FitWindow), FitWindow),
            MenuItem::action(language.text(Text::FitWidth), FitWidth),
            MenuItem::action(language.text(Text::FitHeight), FitHeight),
            MenuItem::action(language.text(Text::OriginalSize), Original),
            MenuItem::action(language.text(Text::Fullscreen), Fullscreen),
        ]),
        Menu::new(language.text(Text::Navigate)).items([
            MenuItem::action(language.text(Text::FirstPage), First),
            MenuItem::action(language.text(Text::PreviousPage), Previous),
            MenuItem::action(language.text(Text::NextPage), Next),
            MenuItem::action(language.text(Text::LastPage), Last),
            MenuItem::action(language.text(Text::Slideshow), Slideshow),
        ]),
        Menu::new(language.text(Text::Bookmarks)).items([
            MenuItem::action(language.text(Text::SetBookmark), Bookmark),
            MenuItem::action(language.text(Text::GoToBookmark), Resume),
            MenuItem::action(language.text(Text::DeleteBookmark), ClearBookmark),
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
    let application = gpui_kit::application().with_assets(ReaderAssets);
    application.on_open_urls(move |urls| {
        let paths = urls
            .into_iter()
            .filter_map(|url| url::Url::parse(&url).ok()?.to_file_path().ok())
            .collect();
        _ = sender.send(paths);
    });
    let reader_window = Rc::new(RefCell::new(None::<AnyWindowHandle>));
    let reopen_window = reader_window.clone();
    application.on_reopen(move |cx| {
        cx.activate(true);
        if let Some(window) = *reopen_window.borrow() {
            _ = window.update(cx, |_, window, _| window.activate_window());
        }
    });
    application.run(move |cx| {
        init(cx);
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        let (handle, _) = gpui_kit::open_window(
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
        *reader_window.borrow_mut() = Some(handle);
        cx.activate(true);
    });
    Ok(())
}

#[cfg(test)]
mod tests;
