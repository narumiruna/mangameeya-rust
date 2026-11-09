//! Reader state and layout calculations, independent of the window toolkit.

use crate::book::Book;
use crate::imaging::Filters;
use crate::settings::BookmarkImage;
pub use crate::settings::{AppSettings, Background, FitMode, ReadingMode, SidebarMode};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Left,
    Right,
}

pub struct Reader {
    pub settings: AppSettings,
    pub book: Option<Arc<Book>>,
    /// Zero-based logical page index, independent of reading direction.
    pub current: usize,
    pub zoom: f32,
    /// Clockwise degrees, normalized to 0, 90, 180, or 270 by rotation methods.
    pub rotation: i32,
    pub filters: Filters,
    /// Timer scheduling belongs to the application, not this model.
    pub slideshow: bool,
}

impl Default for Reader {
    fn default() -> Self {
        Self::new(AppSettings::default())
    }
}

impl Reader {
    pub fn new(mut settings: AppSettings) -> Self {
        settings.validate();
        Self {
            settings,
            book: None,
            current: 0,
            zoom: 1.0,
            rotation: 0,
            filters: Filters::default(),
            slideshow: false,
        }
    }

    /// An explicit image/page selection takes priority over a remembered page.
    /// Pass None when opening a folder/archive without an explicit page selection.
    pub fn set_book(&mut self, book: Arc<Book>, explicit_page: Option<usize>) {
        self.remember_position();
        let remembered = if self.settings.preferences.remember_position {
            self.settings.positions.get(&book.path).copied()
        } else {
            None
        };
        self.current = explicit_page
            .or(remembered)
            .unwrap_or(book.initial_page)
            .min(book.pages.len().saturating_sub(1));
        self.settings.add_recent(&book.path);
        self.book = Some(book);
        self.rebase_bookmark();
        self.zoom = 1.0;
        self.rotation = 0;
        self.slideshow = false;
        self.remember_position();
    }

    pub fn close(&mut self) {
        self.remember_position();
        self.book = None;
        self.current = 0;
        self.zoom = 1.0;
        self.rotation = 0;
        self.slideshow = false;
    }

    pub fn page_count(&self) -> usize {
        self.book.as_ref().map_or(0, |book| book.pages.len())
    }

    pub fn current_name(&self) -> Option<&str> {
        self.book
            .as_ref()?
            .pages
            .get(self.current)
            .map(|page| page.name.as_str())
    }

    /// Clamp the target to the book. Return true only if the page changes.
    pub fn navigate(&mut self, target: usize) -> bool {
        let count = self.page_count();
        if count == 0 {
            return false;
        }
        let target = target.min(count - 1);
        let changed = self.current != target;
        self.current = target;
        self.remember_position();
        changed
    }

    #[allow(
        clippy::should_implement_trait,
        reason = "Reader navigation is not an iterator"
    )]
    pub fn next(&mut self) -> bool {
        self.navigate(self.current.saturating_add(self.navigation_step()))
    }

    pub fn previous(&mut self) -> bool {
        self.navigate(self.current.saturating_sub(self.navigation_step()))
    }

    /// Apply a screen/arrow direction. "Next" itself always increases the index.
    pub fn navigate_direction(&mut self, direction: Direction) -> bool {
        if (direction == Direction::Left) == self.settings.preferences.rtl {
            self.next()
        } else {
            self.previous()
        }
    }

    /// Pages in a logical paged view, or the active page in continuous mode.
    /// A continuous renderer owns the full scroll list and loads pages on demand.
    pub fn visible_indices(&self) -> Vec<usize> {
        let count = self.page_count();
        if self.current >= count {
            return Vec::new();
        }
        let mut indices = vec![self.current];
        if self.settings.preferences.mode == ReadingMode::Spread && self.current < count - 1 {
            indices.push(self.current + 1);
        }
        indices
    }

    /// Left-to-right screen order for a paged renderer.
    pub fn display_indices(&self) -> Vec<usize> {
        let mut indices = self.visible_indices();
        if self.settings.preferences.rtl {
            indices.reverse();
        }
        indices
    }

    /// Set an absolute scale. Use zoom_by for multiplicative zoom controls.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.zoom = normalized_zoom(zoom);
        self.settings.preferences.fit = FitMode::Custom;
    }

    pub fn zoom_by(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            let zoom = (f64::from(self.zoom) * f64::from(factor)).clamp(0.1, 8.0);
            self.set_zoom(zoom as f32);
        }
    }

    pub fn set_fit(&mut self, fit: FitMode) {
        self.settings.preferences.fit = fit;
        if fit != FitMode::Custom {
            self.zoom = 1.0;
        }
    }

    pub fn set_mode(&mut self, mode: ReadingMode) {
        self.settings.preferences.mode = mode;
    }

    pub fn rotate_by(&mut self, degrees: i32) -> bool {
        if degrees % 90 != 0 {
            return false;
        }
        self.rotation = (self.rotation.rem_euclid(360) + degrees.rem_euclid(360)).rem_euclid(360);
        true
    }

    pub fn remember_position(&mut self) {
        if self.settings.preferences.remember_position
            && let Some(book) = &self.book
            && self.current < book.pages.len()
        {
            self.settings
                .positions
                .insert(book.path.clone(), self.current);
        }
    }

    pub fn set_bookmark(&mut self) -> bool {
        if let Some(book) = &self.book
            && self.current < book.pages.len()
        {
            self.settings
                .bookmarks
                .insert(book.path.clone(), self.current);
            if let Some(image) = book.image_page_path(self.current) {
                self.settings.bookmark_images.insert(
                    book.path.clone(),
                    BookmarkImage {
                        page: self.current,
                        image: image.to_path_buf(),
                    },
                );
            } else {
                self.settings.bookmark_images.remove(&book.path);
            }
            return true;
        }
        false
    }

    /// Resolve cached page identity without reading image data. Legacy indices still clamp.
    pub fn bookmark_index(&self) -> Option<usize> {
        let book = self.book.as_ref()?;
        let page = *self.settings.bookmarks.get(&book.path)?;
        if let Some(image) = self
            .settings
            .bookmark_images
            .get(&book.path)
            .filter(|image| image.page == page)
        {
            book.image_page_index(&image.image, page)
        } else if book.pages.is_empty() {
            None
        } else {
            Some(page.min(book.pages.len() - 1))
        }
    }

    fn rebase_bookmark(&mut self) {
        let Some(page) = self.bookmark_index() else {
            return;
        };
        let path = &self.book.as_ref().expect("resolved bookmark").path;
        if let Some(image) = self.settings.bookmark_images.get_mut(path)
            && self.settings.bookmarks.get(path) == Some(&image.page)
        {
            image.page = page;
            self.settings.bookmarks.insert(path.clone(), page);
        }
    }

    /// Return whether the bookmark can be reached, including the current page.
    pub fn resume_bookmark(&mut self) -> bool {
        let Some(index) = self.bookmark_index() else {
            return false;
        };
        self.navigate(index);
        self.rebase_bookmark();
        true
    }

    pub fn clear_bookmark(&mut self) -> bool {
        let Some(book) = &self.book else {
            return false;
        };
        self.settings.bookmark_images.remove(&book.path);
        self.settings.bookmarks.remove(&book.path).is_some()
    }

    pub fn toggle_slideshow(&mut self) -> bool {
        self.slideshow = !self.slideshow && self.page_count() > 0;
        self.slideshow
    }

    /// Advance one timer interval. Stop at the last page, without wrapping.
    pub fn slideshow_tick(&mut self) -> bool {
        if !self.slideshow {
            return false;
        }
        let changed = self.next();
        if !changed || self.current >= self.page_count().saturating_sub(1) {
            self.slideshow = false;
        }
        changed
    }

    fn navigation_step(&self) -> usize {
        if self.settings.preferences.mode == ReadingMode::Spread {
            2
        } else {
            1
        }
    }
}

fn normalized_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() {
        (zoom.clamp(0.1, 8.0) * 100.0).round() / 100.0
    } else {
        1.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageLayout {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
}

pub fn transformed_page_size(width: f32, height: f32, rotation: i32, scale: f32) -> PageSize {
    let width = nonnegative_dimension(width);
    let height = nonnegative_dimension(height);
    let scale = if scale.is_finite() && scale >= 0.0 {
        scale
    } else {
        1.0
    };
    let (width, height) = if rotation.rem_euclid(180) == 90 {
        (height, width)
    } else {
        (width, height)
    };
    PageSize {
        width: (width * scale).min(f32::MAX),
        height: (height * scale).min(f32::MAX),
    }
}

/// Fit uses the rotated dimensions. Window fit never enlarges a source image;
/// width and height fits can enlarge it. Zero viewport dimensions do not constrain it.
pub fn fitted_page_layout(
    natural_width: f32,
    natural_height: f32,
    rotation: i32,
    fit: FitMode,
    zoom: f32,
    available_width: f32,
    available_height: f32,
) -> PageLayout {
    let rotated = transformed_page_size(natural_width, natural_height, rotation, 1.0);
    let ratio = |available: f32, natural: f32| {
        if available.is_finite() && available > 0.0 && natural > 0.0 {
            (available / natural).min(f32::MAX)
        } else {
            1.0
        }
    };
    let scale = match fit {
        FitMode::Window => 1.0_f32
            .min(ratio(available_width, rotated.width))
            .min(ratio(available_height, rotated.height)),
        FitMode::Width => ratio(available_width, rotated.width),
        FitMode::Height => ratio(available_height, rotated.height),
        FitMode::Original => 1.0,
        FitMode::Custom => normalized_zoom(zoom),
    };
    let size = transformed_page_size(natural_width, natural_height, rotation, scale);
    PageLayout {
        width: size.width,
        height: size.height,
        scale,
    }
}

pub fn uses_responsive_page_size(fit: FitMode) -> bool {
    matches!(fit, FitMode::Window | FitMode::Width | FitMode::Height)
}

/// Progress is relative to the full page height, not its scrollable remainder.
pub fn page_scroll_progress(scroll_top: f32, page_top: f32, page_height: f32) -> f32 {
    if !scroll_top.is_finite()
        || !page_top.is_finite()
        || !page_height.is_finite()
        || page_height <= 0.0
    {
        return 0.0;
    }
    ((scroll_top - page_top) / page_height).clamp(0.0, 1.0)
}

pub fn page_scroll_top(page_top: f32, page_height: f32, progress: f32) -> f32 {
    let progress = if progress.is_finite() {
        progress.clamp(0.0, 1.0)
    } else {
        0.0
    };
    finite_or_zero(page_top) + nonnegative_dimension(page_height) * progress
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContinuousScrollAnchor {
    pub index: usize,
    pub progress: f32,
    pub left: f32,
}

impl ContinuousScrollAnchor {
    pub fn capture(
        index: usize,
        scroll_top: f32,
        page_top: f32,
        page_height: f32,
        left: f32,
    ) -> Self {
        Self {
            index,
            progress: page_scroll_progress(scroll_top, page_top, page_height),
            left: finite_or_zero(left),
        }
    }

    /// Return (top, left) after the same page is laid out at a new size.
    pub fn restore(&self, page_top: f32, page_height: f32) -> (f32, f32) {
        (
            page_scroll_top(page_top, page_height, self.progress),
            finite_or_zero(self.left),
        )
    }
}

/// Select the first page whose bottom reaches the viewport center. This matches
/// the original continuous reader, including a center that falls in a page gap.
/// Frames must be in page order and contain (top, height) in content coordinates.
pub fn continuous_page_index(
    scroll_top: f32,
    viewport_height: f32,
    frames: &[(f32, f32)],
) -> Option<usize> {
    if frames.is_empty() {
        return None;
    }
    let center = finite_or_zero(scroll_top) + nonnegative_dimension(viewport_height) / 2.0;
    let mut low = 0;
    let mut high = frames.len() - 1;
    while low < high {
        let middle = low + (high - low) / 2;
        let (top, height) = frames[middle];
        if center > top + height {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    Some(low)
}

pub fn distant_page_indices(
    indices: impl IntoIterator<Item = usize>,
    current: usize,
    retention: usize,
) -> Vec<usize> {
    indices
        .into_iter()
        .filter(|index| index.abs_diff(current) > retention)
        .collect()
}

pub fn is_page_navigation_key(key: &str) -> bool {
    matches!(
        key,
        "ArrowRight" | "ArrowLeft" | "PageDown" | "PageUp" | " " | "Home" | "End"
    )
}

pub fn is_native_scroll_key(key: &str) -> bool {
    matches!(key, "ArrowUp" | "ArrowDown")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContinuousPageLoadState {
    Error,
    Ready,
    Loading,
}

pub fn continuous_page_load_state(failed: bool, cached: bool) -> ContinuousPageLoadState {
    if failed {
        ContinuousPageLoadState::Error
    } else if cached {
        ContinuousPageLoadState::Ready
    } else {
        ContinuousPageLoadState::Loading
    }
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn nonnegative_dimension(value: f32) -> f32 {
    finite_or_zero(value).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn book(count: usize) -> (TempDir, Arc<Book>) {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..count {
            // Book opening enumerates image paths; decoding is tested in imaging.rs.
            fs::write(directory.path().join(format!("{index}.png")), []).unwrap();
        }
        let book = Book::open(directory.path(), false).unwrap();
        (directory, Arc::new(book))
    }

    #[test]
    fn empty_reader_operations_are_safe() {
        let mut reader = Reader::default();
        assert_eq!(reader.page_count(), 0);
        assert_eq!(reader.current_name(), None);
        assert!(reader.visible_indices().is_empty());
        assert!(!reader.navigate(usize::MAX));
        assert!(!reader.next());
        assert!(!reader.previous());
        assert!(!reader.set_bookmark());
        assert!(!reader.resume_bookmark());
        assert!(!reader.clear_bookmark());
        assert!(!reader.toggle_slideshow());
        assert!(!reader.slideshow_tick());
        reader.close();
        assert!(reader.settings.positions.is_empty());
    }

    #[test]
    fn opening_updates_history_and_remembers_navigation() {
        let (_directory, book) = book(5);
        let path = book.path.clone();
        let mut reader = Reader::default();
        reader.set_book(book.clone(), None);
        assert_eq!(reader.current, 0);
        assert_eq!(reader.current_name(), Some("0.png"));
        assert_eq!(
            reader.settings.recent.as_slice(),
            std::slice::from_ref(&path)
        );
        assert_eq!(reader.settings.positions[&path], 0);
        assert!(reader.navigate(3));
        assert_eq!(reader.settings.positions[&path], 3);
        reader.close();
        assert!(reader.book.is_none());
        assert_eq!(reader.current, 0);
        reader.set_book(book, None);
        assert_eq!(reader.current, 3);
        assert_eq!(reader.settings.recent, [path]);
    }

    #[test]
    fn explicit_selection_overrides_remembered_position_even_for_first_image() {
        let (_directory, book) = book(5);
        let path = book.path.clone();
        let mut settings = AppSettings::default();
        settings.positions.insert(path.clone(), 3);
        let mut reader = Reader::new(settings);
        reader.set_book(book.clone(), Some(0));
        assert_eq!(reader.current, 0);
        reader.navigate(3);
        reader.set_book(book.clone(), Some(1));
        assert_eq!(reader.current, 1);
        assert_eq!(reader.settings.positions[&path], 1);
        reader.set_book(book, Some(usize::MAX));
        assert_eq!(reader.current, 4);
    }

    #[test]
    fn stale_positions_and_initial_pages_are_clamped() {
        let (directory, book) = book(4);
        let mut settings = AppSettings::default();
        settings.positions.insert(book.path.clone(), usize::MAX);
        let mut reader = Reader::new(settings);
        reader.set_book(book, None);
        assert_eq!(reader.current, 3);
        let initial = Book::open(&directory.path().join("2.png"), false).unwrap();
        reader.settings.preferences.remember_position = false;
        reader.set_book(Arc::new(initial), None);
        assert_eq!(reader.current, 2);
        reader.navigate(0);
        assert_eq!(
            reader
                .settings
                .positions
                .values()
                .copied()
                .collect::<Vec<_>>(),
            [3]
        );
    }

    #[test]
    fn spread_navigation_keeps_logical_order_and_handles_odd_page_counts() {
        let (_directory, book) = book(5);
        let mut reader = Reader::default();
        reader.set_book(book, None);
        reader.set_mode(ReadingMode::Spread);
        assert_eq!(reader.visible_indices(), [0, 1]);
        assert_eq!(reader.display_indices(), [1, 0]);
        assert!(reader.next());
        assert_eq!(reader.visible_indices(), [2, 3]);
        assert!(reader.next());
        assert_eq!(reader.visible_indices(), [4]);
        assert!(!reader.next());
        assert!(reader.previous());
        assert_eq!(reader.current, 2);
        reader.settings.preferences.rtl = false;
        assert_eq!(reader.display_indices(), [2, 3]);
        reader.navigate(usize::MAX);
        assert_eq!(reader.current, 4);
        reader.navigate(0);
        assert!(!reader.previous());
    }

    #[test]
    fn direction_navigation_and_continuous_mode_use_correct_steps() {
        let (_directory, book) = book(6);
        let mut reader = Reader::default();
        reader.set_book(book, None);
        assert!(reader.navigate_direction(Direction::Left));
        assert_eq!(reader.current, 1);
        assert!(reader.navigate_direction(Direction::Right));
        assert_eq!(reader.current, 0);
        reader.settings.preferences.rtl = false;
        assert!(reader.navigate_direction(Direction::Right));
        assert_eq!(reader.current, 1);
        assert!(reader.navigate_direction(Direction::Left));
        assert_eq!(reader.current, 0);
        reader.set_mode(ReadingMode::Spread);
        reader.next();
        assert_eq!(reader.current, 2);
        reader.set_mode(ReadingMode::Continuous);
        assert_eq!(reader.visible_indices(), [2]);
        reader.next();
        assert_eq!(reader.current, 3);
        reader.previous();
        assert_eq!(reader.current, 2);
    }

    #[test]
    fn zoom_fit_and_rotation_are_validated_runtime_state() {
        let mut reader = Reader::default();
        reader.set_zoom(1.234);
        assert_eq!(reader.zoom, 1.23);
        assert_eq!(reader.settings.preferences.fit, FitMode::Custom);
        reader.zoom_by(2.0);
        assert_eq!(reader.zoom, 2.46);
        reader.set_zoom(-1.0);
        assert_eq!(reader.zoom, 0.1);
        reader.set_zoom(100.0);
        assert_eq!(reader.zoom, 8.0);
        reader.zoom_by(f32::MAX);
        assert_eq!(reader.zoom, 8.0);
        reader.zoom_by(f32::NAN);
        assert_eq!(reader.zoom, 8.0);
        reader.set_fit(FitMode::Width);
        assert_eq!(reader.zoom, 1.0);
        reader.set_zoom(f32::NAN);
        assert_eq!(reader.zoom, 1.0);
        assert!(reader.rotate_by(-90));
        assert_eq!(reader.rotation, 270);
        assert!(reader.rotate_by(450));
        assert_eq!(reader.rotation, 0);
        assert!(!reader.rotate_by(45));
        assert_eq!(reader.rotation, 0);
        let (_directory, book) = book(2);
        reader.set_zoom(2.0);
        reader.rotate_by(90);
        reader.slideshow = true;
        reader.set_book(book, None);
        assert_eq!(reader.zoom, 1.0);
        assert_eq!(reader.rotation, 0);
        assert!(!reader.slideshow);
    }

    #[test]
    fn bookmarks_are_per_book_and_do_not_require_position_memory() {
        let (_first_directory, first) = book(4);
        let (_second_directory, second) = book(2);
        let mut reader = Reader::default();
        reader.settings.preferences.remember_position = false;
        reader.set_book(first.clone(), None);
        reader.navigate(2);
        assert!(reader.set_bookmark());
        reader.navigate(0);
        assert!(reader.resume_bookmark());
        assert_eq!(reader.current, 2);
        assert!(reader.resume_bookmark());
        reader.set_book(second, None);
        assert!(!reader.resume_bookmark());
        reader.set_book(first, None);
        assert_eq!(reader.current, 0);
        assert!(reader.resume_bookmark());
        assert_eq!(reader.current, 2);
        assert!(reader.clear_bookmark());
        assert!(!reader.resume_bookmark());
        assert!(reader.settings.positions.is_empty());
    }

    #[test]
    fn folder_bookmark_keeps_image_across_recursion_order_changes_and_reload() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("00")).unwrap();
        for name in ["10.png", "20.png", "00/1.png"] {
            std::fs::write(directory.path().join(name), b"image").unwrap();
        }
        let mut reader = Reader::default();
        reader.settings.preferences.remember_position = false;
        reader.set_book(Arc::new(Book::open(directory.path(), false).unwrap()), None);
        reader.navigate(1);
        assert_eq!(reader.current_name(), Some("20.png"));
        assert!(reader.set_bookmark());
        let key = reader.book.as_ref().unwrap().path.clone();
        let image = std::fs::canonicalize(directory.path().join("20.png")).unwrap();
        assert_eq!(reader.settings.bookmark_images[&key].image, image);
        let settings_path = directory.path().join("settings.json");
        reader.settings.save_to(&settings_path).unwrap();
        reader = Reader::new(AppSettings::load_from(&settings_path).unwrap());
        reader.set_book(Arc::new(Book::open(directory.path(), true).unwrap()), None);
        assert_eq!(reader.bookmark_index(), Some(2));
        assert_eq!(reader.settings.bookmarks[&key], 2);
        assert_eq!(reader.settings.bookmark_images[&key].page, 2);
        assert!(reader.resume_bookmark());
        assert_eq!(reader.current_name(), Some("20.png"));
        assert!(
            reader.resume_bookmark(),
            "the current bookmarked page is reachable"
        );
        std::fs::remove_file(directory.path().join("10.png")).unwrap();
        reader.set_book(Arc::new(Book::open(directory.path(), false).unwrap()), None);
        assert_eq!(reader.bookmark_index(), Some(0));
        assert!(reader.resume_bookmark());
        assert_eq!(reader.current_name(), Some("20.png"));
        std::fs::write(directory.path().join("00.png"), b"earlier image").unwrap();
        reader.set_book(Arc::new(Book::open(directory.path(), false).unwrap()), None);
        assert_eq!(reader.bookmark_index(), Some(1));
        assert!(reader.resume_bookmark());
        assert_eq!(reader.current_name(), Some("20.png"));
        assert!(reader.clear_bookmark());
        assert!(!reader.settings.bookmarks.contains_key(&key));
        assert!(!reader.settings.bookmark_images.contains_key(&key));
    }

    #[test]
    fn excluded_or_deleted_bookmark_image_does_not_fall_back_to_another_page() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("00")).unwrap();
        for name in ["10.png", "20.png", "00/1.png"] {
            std::fs::write(directory.path().join(name), b"image").unwrap();
        }
        let mut reader = Reader::default();
        reader.settings.preferences.remember_position = false;
        reader.set_book(Arc::new(Book::open(directory.path(), true).unwrap()), None);
        assert!(reader.current_name().unwrap().ends_with("1.png"));
        assert!(reader.set_bookmark());
        let stored = reader.settings.bookmark_images.clone();
        for recursive in [false, true] {
            if recursive {
                std::fs::remove_file(directory.path().join("00/1.png")).unwrap();
            }
            reader.set_book(
                Arc::new(Book::open(directory.path(), recursive).unwrap()),
                None,
            );
            reader.navigate(1);
            assert_eq!(reader.bookmark_index(), None);
            assert!(!reader.resume_bookmark());
            assert_eq!(reader.current_name(), Some("20.png"));
            assert_eq!(reader.settings.bookmark_images, stored);
        }
        std::fs::write(directory.path().join("00/1.png"), b"restored image").unwrap();
        reader.set_book(Arc::new(Book::open(directory.path(), true).unwrap()), None);
        assert!(reader.resume_bookmark());
        assert!(reader.current_name().unwrap().ends_with("1.png"));
    }

    #[test]
    fn legacy_bookmarks_keep_numeric_behavior_until_set_again() {
        let (_directory, book) = book(3);
        let key = book.path.clone();
        let mut settings = AppSettings::default();
        settings.bookmarks.insert(key.clone(), usize::MAX);
        let mut reader = Reader::new(settings);
        reader.set_book(book, None);
        assert!(reader.settings.bookmark_images.is_empty());
        assert!(reader.resume_bookmark());
        assert_eq!(reader.current, 2);
        assert_eq!(reader.settings.bookmarks[&key], usize::MAX);
        assert!(reader.set_bookmark());
        assert_eq!(reader.settings.bookmark_images[&key].page, 2);
        reader.settings.bookmarks.insert(key.clone(), 0);
        assert_eq!(
            reader.bookmark_index(),
            Some(0),
            "a changed numeric entry must not use stale image metadata"
        );
        assert!(reader.clear_bookmark());
        assert!(reader.settings.bookmark_images.is_empty());
    }

    #[test]
    fn archive_bookmarks_keep_numeric_storage_without_image_metadata() {
        use std::io::Write;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("book.cbz");
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for name in ["1.png", "2.png"] {
            archive
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            archive.write_all(b"image").unwrap();
        }
        archive.finish().unwrap();
        let mut reader = Reader::default();
        reader.set_book(Arc::new(Book::open(&path, false).unwrap()), None);
        reader.navigate(1);
        assert!(reader.set_bookmark());
        assert!(reader.settings.bookmark_images.is_empty());
        reader.navigate(0);
        assert!(reader.resume_bookmark());
        assert_eq!(reader.current, 1);
        assert!(reader.clear_bookmark());
        assert!(reader.settings.bookmarks.is_empty());
    }

    #[test]
    fn slideshow_advances_by_mode_and_stops_at_last_page() {
        let (_directory, book) = book(4);
        let mut reader = Reader::default();
        reader.set_book(book, None);
        reader.set_mode(ReadingMode::Spread);
        assert!(reader.toggle_slideshow());
        assert!(reader.slideshow_tick());
        assert_eq!(reader.current, 2);
        assert!(reader.slideshow_tick());
        assert_eq!(reader.current, 3);
        assert!(!reader.slideshow);
        assert!(!reader.slideshow_tick());
        reader.close();
        assert!(!reader.slideshow);
    }

    #[test]
    fn transformed_page_size_reserves_scaled_and_rotated_space() {
        assert_eq!(
            transformed_page_size(800.0, 1200.0, 0, 1.5),
            PageSize {
                width: 1200.0,
                height: 1800.0
            }
        );
        assert_eq!(
            transformed_page_size(800.0, 1200.0, 90, 1.0),
            PageSize {
                width: 1200.0,
                height: 800.0
            }
        );
        assert_eq!(
            transformed_page_size(800.0, 1200.0, -270, 2.0),
            PageSize {
                width: 2400.0,
                height: 1600.0
            }
        );
    }

    #[test]
    fn fitted_page_layout_uses_rotated_dimensions() {
        assert_eq!(
            fitted_page_layout(800.0, 1200.0, 90, FitMode::Width, 1.0, 600.0, 900.0),
            PageLayout {
                width: 600.0,
                height: 400.0,
                scale: 0.5
            }
        );
        assert_eq!(
            fitted_page_layout(800.0, 1200.0, 90, FitMode::Height, 1.0, 600.0, 600.0),
            PageLayout {
                width: 900.0,
                height: 600.0,
                scale: 0.75
            }
        );
        assert_eq!(
            fitted_page_layout(800.0, 1200.0, 90, FitMode::Window, 1.0, 600.0, 600.0),
            PageLayout {
                width: 600.0,
                height: 400.0,
                scale: 0.5
            }
        );
    }

    #[test]
    fn fit_modes_handle_upscaling_original_custom_and_missing_dimensions() {
        let fit = |mode| fitted_page_layout(100.0, 200.0, 0, mode, 2.5, 400.0, 600.0);
        assert_eq!(fit(FitMode::Window).scale, 1.0);
        assert_eq!(fit(FitMode::Width).scale, 4.0);
        assert_eq!(fit(FitMode::Height).scale, 3.0);
        assert_eq!(fit(FitMode::Original).scale, 1.0);
        assert_eq!(
            fit(FitMode::Custom),
            PageLayout {
                width: 250.0,
                height: 500.0,
                scale: 2.5
            }
        );
        assert_eq!(
            fitted_page_layout(800.0, 1200.0, 0, FitMode::Window, 1.0, 0.0, 0.0).scale,
            1.0
        );
        let empty = fitted_page_layout(f32::NAN, 0.0, 90, FitMode::Width, 1.0, 500.0, 500.0);
        assert_eq!(
            empty,
            PageLayout {
                width: 0.0,
                height: 0.0,
                scale: 1.0
            }
        );
        assert!(uses_responsive_page_size(FitMode::Window));
        assert!(uses_responsive_page_size(FitMode::Width));
        assert!(uses_responsive_page_size(FitMode::Height));
        assert!(!uses_responsive_page_size(FitMode::Original));
        assert!(!uses_responsive_page_size(FitMode::Custom));
    }

    #[test]
    fn page_scroll_progress_restores_intra_page_resize_offsets() {
        let progress = page_scroll_progress(350.0, 100.0, 1000.0);
        assert_eq!(progress, 0.25);
        assert_eq!(page_scroll_top(200.0, 800.0, progress), 400.0);
        assert_eq!(page_scroll_progress(50.0, 100.0, 1000.0), 0.0);
        assert_eq!(page_scroll_top(200.0, 800.0, 2.0), 1000.0);
        assert_eq!(page_scroll_progress(300.0, 0.0, 0.0), 0.0);
        assert_eq!(page_scroll_progress(f32::NAN, 0.0, 100.0), 0.0);
        assert_eq!(page_scroll_top(200.0, 800.0, f32::NAN), 200.0);
        let anchor = ContinuousScrollAnchor::capture(4, 350.0, 100.0, 1000.0, 75.0);
        assert_eq!(anchor.index, 4);
        assert_eq!(anchor.restore(200.0, 800.0), (400.0, 75.0));
    }

    #[test]
    fn continuous_active_page_uses_viewport_center_and_frame_bottoms() {
        let frames = [(0.0, 100.0), (120.0, 100.0), (240.0, 100.0)];
        assert_eq!(continuous_page_index(0.0, 100.0, &frames), Some(0));
        assert_eq!(continuous_page_index(50.0, 100.0, &frames), Some(0));
        assert_eq!(continuous_page_index(51.0, 100.0, &frames), Some(1));
        assert_eq!(continuous_page_index(171.0, 100.0, &frames), Some(2));
        assert_eq!(continuous_page_index(10000.0, 100.0, &frames), Some(2));
        assert_eq!(continuous_page_index(0.0, 100.0, &[]), None);
    }

    #[test]
    fn continuous_retention_and_failure_state_match_original_behavior() {
        assert_eq!(
            distant_page_indices([0, 1, 8, 9, 10, 17, 18], 9, 8),
            [0, 18]
        );
        assert_eq!(distant_page_indices([0, usize::MAX], 0, 8), [usize::MAX]);
        assert_eq!(
            continuous_page_load_state(true, true),
            ContinuousPageLoadState::Error
        );
        assert_eq!(
            continuous_page_load_state(true, false),
            ContinuousPageLoadState::Error
        );
        assert_eq!(
            continuous_page_load_state(false, true),
            ContinuousPageLoadState::Ready
        );
        assert_eq!(
            continuous_page_load_state(false, false),
            ContinuousPageLoadState::Loading
        );
    }

    #[test]
    fn navigation_and_native_scroll_keys_are_separate() {
        for key in [
            "ArrowRight",
            "ArrowLeft",
            "PageDown",
            "PageUp",
            " ",
            "Home",
            "End",
        ] {
            assert!(is_page_navigation_key(key));
            assert!(!is_native_scroll_key(key));
        }
        assert!(is_native_scroll_key("ArrowUp"));
        assert!(is_native_scroll_key("ArrowDown"));
        assert!(!is_page_navigation_key("c"));
    }
}
