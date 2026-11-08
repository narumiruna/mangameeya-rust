//! Toolkit-independent browser order, selection, and navigation snapshots.
//!
//! Rebuild uses only listing metadata and in-memory settings. Updating known page
//! counts does not change the visible order until the caller requests a rebuild.

use crate::{
    book::{DirectoryEntry, DirectoryListing, EntryKind},
    settings::{AppSettings, MAX_RECENT_BOOKS},
};
use std::{
    cmp::Ordering,
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SortKey {
    #[default]
    Natural,
    Name,
    Modified,
    Created,
    Size,
    Type,
    Pages,
    Progress,
    Recent,
    Random,
}

impl SortKey {
    pub const ALL: [Self; 10] = [
        Self::Natural,
        Self::Name,
        Self::Modified,
        Self::Created,
        Self::Size,
        Self::Type,
        Self::Pages,
        Self::Progress,
        Self::Recent,
        Self::Random,
    ];
}

/// Per-directory state. Sort preferences remain global in `BrowserModel`.
/// Paths identify native children, not their canonical targets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BrowserSnapshot {
    pub query: String,
    pub selected: Option<PathBuf>,
    pub anchor: Option<PathBuf>,
    /// Nonnegative, finite displacement within the anchor row, in logical pixels.
    pub offset: f32,
}

#[derive(Clone, Debug)]
pub struct BrowserModel {
    pub sort: SortKey,
    pub descending: bool,
    pub folders_first: bool,
    pub query: String,
    /// Native child path (`DirectoryEntry::entry_path`), not the canonical target.
    pub selected: Option<PathBuf>,
    /// Visible entries as indexes into the current `DirectoryListing::entries`.
    pub order: Vec<usize>,
    /// Canonical book path and folder recursion policy. Archives use `false`.
    pub known_pages: HashMap<(PathBuf, bool), usize>,
    seed: u64,
    listing_fingerprint: Option<u64>,
    random_keys: Vec<u64>,
}

impl Default for BrowserModel {
    fn default() -> Self {
        static FALLBACK_SEED: AtomicU64 = AtomicU64::new(0x517c_c1b7_2722_0a95);
        let mut bytes = [0; 8];
        let seed = if getrandom::fill(&mut bytes).is_ok() {
            u64::from_ne_bytes(bytes)
        } else {
            FALLBACK_SEED.fetch_add(0x9e37_79b9_7f4a_7c15, AtomicOrdering::Relaxed)
        };
        Self {
            sort: SortKey::Natural,
            descending: false,
            folders_first: true,
            query: String::new(),
            selected: None,
            order: Vec::new(),
            known_pages: HashMap::new(),
            seed,
            listing_fingerprint: None,
            random_keys: Vec::new(),
        }
    }
}

impl BrowserModel {
    /// Filter by a case-insensitive name substring and sort existing metadata.
    /// Unknown values and fixed name/native-path ties do not reverse direction.
    pub fn rebuild(&mut self, listing: &DirectoryListing, settings: &AppSettings) {
        let fingerprint = listing_fingerprint(listing);
        if self.listing_fingerprint != Some(fingerprint) {
            self.shuffle();
            self.listing_fingerprint = Some(fingerprint);
        }
        if self.random_keys.len() != listing.entries.len() {
            self.random_keys = listing
                .entries
                .iter()
                .map(|entry| {
                    let mut hasher = DefaultHasher::new();
                    self.seed.hash(&mut hasher);
                    entry.entry_path.hash(&mut hasher);
                    hasher.finish()
                })
                .collect();
        }

        let mut recent = HashMap::new();
        for (rank, path) in settings.recent.iter().take(MAX_RECENT_BOOKS).enumerate() {
            recent.entry(path).or_insert(rank);
        }
        // Prepare keys once. The comparator does not read files or change state.
        let metadata: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| SortMetadata {
                pages: self.page_count(entry, settings),
                progress: self.progress_fraction(entry, settings),
                recent: recent.get(&entry.path).copied(),
                extension: entry
                    .entry_path
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase(),
            })
            .collect();
        let query = self.query.trim().to_lowercase();
        let mut order: Vec<_> = listing
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect();
        order.sort_unstable_by(|&left, &right| {
            let a = &listing.entries[left];
            let b = &listing.entries[right];
            let a_meta = &metadata[left];
            let b_meta = &metadata[right];
            let folders = if self.folders_first {
                b.is_directory.cmp(&a.is_directory)
            } else {
                Ordering::Equal
            };
            folders
                .then_with(|| match self.sort {
                    SortKey::Natural => directed(
                        natord::compare_ignore_case(&a.name, &b.name),
                        self.descending,
                    ),
                    SortKey::Name => directed(a.name.cmp(&b.name), self.descending),
                    SortKey::Modified => optional(a.modified, b.modified, self.descending),
                    SortKey::Created => optional(a.created, b.created, self.descending),
                    SortKey::Size => optional(
                        (!a.is_directory).then_some(a.size).flatten(),
                        (!b.is_directory).then_some(b.size).flatten(),
                        self.descending,
                    ),
                    SortKey::Type => directed(
                        category(a)
                            .cmp(&category(b))
                            .then_with(|| a_meta.extension.cmp(&b_meta.extension)),
                        self.descending,
                    ),
                    SortKey::Pages => optional(a_meta.pages, b_meta.pages, self.descending),
                    SortKey::Progress => {
                        optional(a_meta.progress, b_meta.progress, self.descending)
                    }
                    SortKey::Recent => optional(a_meta.recent, b_meta.recent, self.descending),
                    SortKey::Random => directed(
                        self.random_keys[left].cmp(&self.random_keys[right]),
                        self.descending,
                    ),
                })
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.entry_path.cmp(&b.entry_path))
        });
        if !order
            .iter()
            .any(|&index| Some(&listing.entries[index].entry_path) == self.selected.as_ref())
        {
            self.selected = order
                .first()
                .map(|&index| listing.entries[index].entry_path.clone());
        }
        self.order = order;
    }

    /// Change random keys on the next explicit rebuild. Does not change order now.
    pub fn shuffle(&mut self) {
        self.seed = self.seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        self.random_keys.clear();
    }

    /// Return a positive known count without opening the entry or changing order.
    pub fn page_count(&self, entry: &DirectoryEntry, settings: &AppSettings) -> Option<usize> {
        let recursive = match entry.kind {
            EntryKind::Image => return Some(1),
            EntryKind::Folder => settings.preferences.recursive_folder,
            EntryKind::Archive => false,
        };
        self.known_pages
            .get(&(entry.path.clone(), recursive))
            .copied()
            .filter(|&count| count > 0)
    }

    /// Return the saved `(page + 1) / count` ratio, without display rounding.
    /// Images have no independent book position. Disabling position storage does
    /// not erase existing history; this accessor never writes to settings.
    pub fn progress(&self, entry: &DirectoryEntry, settings: &AppSettings) -> Option<f64> {
        self.progress_fraction(entry, settings)
            .map(|value| value.numerator as f64 / value.denominator as f64)
    }

    fn progress_fraction(
        &self,
        entry: &DirectoryEntry,
        settings: &AppSettings,
    ) -> Option<Fraction> {
        if entry.kind == EntryKind::Image {
            return None;
        }
        let count = self.page_count(entry, settings)?;
        let page = settings.positions.get(&entry.path).copied()?.min(count - 1);
        Some(Fraction {
            numerator: page + 1,
            denominator: count,
        })
    }

    /// Capture lightweight navigation state. Invalid offsets become zero.
    pub fn snapshot(&self, anchor: Option<PathBuf>, offset: f32) -> BrowserSnapshot {
        BrowserSnapshot {
            query: self.query.clone(),
            selected: self.selected.clone(),
            anchor,
            offset: if offset.is_finite() {
                offset.max(0.0)
            } else {
                0.0
            },
        }
    }

    /// Restore query and selection before rebuilding the destination listing.
    /// The UI restores the snapshot's native anchor and row offset separately.
    pub fn restore(&mut self, snapshot: &BrowserSnapshot) {
        self.query.clone_from(&snapshot.query);
        self.selected.clone_from(&snapshot.selected);
    }
}

struct SortMetadata {
    pages: Option<usize>,
    progress: Option<Fraction>,
    recent: Option<usize>,
    extension: String,
}

/// Exact comparison avoids rounding two distinct large page ratios to one value.
#[derive(Clone, Copy)]
struct Fraction {
    numerator: usize,
    denominator: usize,
}

impl Eq for Fraction {}

impl PartialEq for Fraction {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Ord for Fraction {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.numerator as u128 * other.denominator as u128)
            .cmp(&(other.numerator as u128 * self.denominator as u128))
    }
}

impl PartialOrd for Fraction {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn directed(order: Ordering, descending: bool) -> Ordering {
    if descending { order.reverse() } else { order }
}

fn optional<T: Ord>(left: Option<T>, right: Option<T>, descending: bool) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => directed(left.cmp(&right), descending),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn category(entry: &DirectoryEntry) -> u8 {
    match entry.kind {
        EntryKind::Folder => 0,
        EntryKind::Archive => 1,
        EntryKind::Image => 2,
    }
}

fn listing_fingerprint(listing: &DirectoryListing) -> u64 {
    let mut hasher = DefaultHasher::new();
    listing.path.hash(&mut hasher);
    listing.entries.len().hash(&mut hasher);
    for entry in &listing.entries {
        entry.name.hash(&mut hasher);
        entry.path.hash(&mut hasher);
        entry.entry_path.hash(&mut hasher);
        entry.is_directory.hash(&mut hasher);
        category(entry).hash(&mut hasher);
        entry.size.hash(&mut hasher);
        entry.modified.hash(&mut hasher);
        entry.created.hash(&mut hasher);
    }
    hasher.finish()
}

/// Tile width includes any horizontal gap. Invalid geometry uses one column.
pub fn columns(width: f32, tile_width: f32) -> usize {
    if !width.is_finite() || !tile_width.is_finite() || width <= 0.0 || tile_width <= 0.0 {
        return 1;
    }
    ((width / tile_width).floor() as usize).max(1)
}

/// Zero columns are treated as one. The calculation does not overflow.
pub fn row_count(count: usize, columns: usize) -> usize {
    count.div_ceil(columns.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::Path,
        time::{Duration, UNIX_EPOCH},
    };

    fn entry(name: &str, kind: EntryKind) -> DirectoryEntry {
        DirectoryEntry {
            name: name.into(),
            path: Path::new("canonical").join(name),
            entry_path: Path::new("native").join(name),
            is_directory: kind == EntryKind::Folder,
            kind,
            size: None,
            modified: None,
            created: None,
        }
    }

    fn listing(entries: Vec<DirectoryEntry>) -> DirectoryListing {
        DirectoryListing {
            path: PathBuf::from("native"),
            parent: None,
            entries,
        }
    }

    fn fixture() -> (DirectoryListing, BrowserModel, AppSettings) {
        let mut entries = vec![
            entry("Folder", EntryKind::Folder),
            entry("book10.cbz", EntryKind::Archive),
            entry("book2.cbz", EntryKind::Archive),
            entry("Alpha.PNG", EntryKind::Image),
            entry("z-unknown.cbr", EntryKind::Archive),
        ];
        for (index, size, modified, created) in [(1, 30, 20, 10), (2, 10, 10, 30), (3, 20, 30, 20)]
        {
            entries[index].size = Some(size);
            entries[index].modified = Some(UNIX_EPOCH + Duration::from_secs(modified));
            entries[index].created = Some(UNIX_EPOCH + Duration::from_secs(created));
        }
        let mut model = BrowserModel {
            folders_first: false,
            ..BrowserModel::default()
        };
        let mut settings = AppSettings::default();
        for (index, count, position) in [(0, 6, 2), (1, 3, 0), (2, 9, 8)] {
            let path = entries[index].path.clone();
            model.known_pages.insert((path.clone(), index == 0), count);
            settings.positions.insert(path, position);
        }
        settings.recent = [1, 2, 0].map(|index| entries[index].path.clone()).into();
        (listing(entries), model, settings)
    }

    #[test]
    fn all_ten_sorts_in_both_directions() {
        let (listing, mut model, settings) = fixture();
        assert_eq!(SortKey::ALL.len(), 10);
        for key in SortKey::ALL {
            for descending in [false, true] {
                model.sort = key;
                model.descending = descending;
                model.rebuild(&listing, &settings);
                let expected = match (key, descending) {
                    (SortKey::Natural, false) => vec![3, 2, 1, 0, 4],
                    (SortKey::Natural, true) => vec![4, 0, 1, 2, 3],
                    (SortKey::Name, false) => vec![3, 0, 1, 2, 4],
                    (SortKey::Name, true) => vec![4, 2, 1, 0, 3],
                    (SortKey::Modified, false) => vec![2, 1, 3, 0, 4],
                    (SortKey::Modified, true) => vec![3, 1, 2, 0, 4],
                    (SortKey::Created, false) => vec![1, 3, 2, 0, 4],
                    (SortKey::Created, true) => vec![2, 3, 1, 0, 4],
                    (SortKey::Size, false) => vec![2, 3, 1, 0, 4],
                    (SortKey::Size, true) => vec![1, 3, 2, 0, 4],
                    (SortKey::Type, false) => vec![0, 4, 1, 2, 3],
                    (SortKey::Type, true) => vec![3, 1, 2, 4, 0],
                    (SortKey::Pages, false) => vec![3, 1, 0, 2, 4],
                    (SortKey::Pages, true) => vec![2, 0, 1, 3, 4],
                    (SortKey::Progress, false) => vec![1, 0, 2, 3, 4],
                    (SortKey::Progress, true) => vec![2, 0, 1, 3, 4],
                    (SortKey::Recent, false) => vec![1, 2, 0, 3, 4],
                    (SortKey::Recent, true) => vec![0, 2, 1, 3, 4],
                    (SortKey::Random, _) => {
                        let mut indexes: Vec<_> = (0..listing.entries.len()).collect();
                        indexes.sort_by_key(|&index| model.random_keys[index]);
                        if descending {
                            indexes.reverse();
                        }
                        indexes
                    }
                };
                assert_eq!(model.order, expected, "{key:?}, descending={descending}");
            }
        }
    }

    #[test]
    fn folders_first_is_independent_of_direction_for_every_key() {
        let (listing, mut model, settings) = fixture();
        model.folders_first = true;
        for key in SortKey::ALL {
            for descending in [false, true] {
                model.sort = key;
                model.descending = descending;
                model.rebuild(&listing, &settings);
                assert_eq!(model.order[0], 0, "{key:?}, descending={descending}");
            }
        }
    }

    #[test]
    fn missing_metadata_is_last_in_each_folder_group_in_both_directions() {
        let (mut listing, mut model, mut settings) = fixture();
        let mut unknown_folder = entry("A-unknown-folder", EntryKind::Folder);
        // A directory length is not a file size, even if a caller supplies one.
        unknown_folder.size = Some(1);
        listing.entries.push(unknown_folder);
        listing.entries[0].modified = Some(UNIX_EPOCH);
        listing.entries[0].created = Some(UNIX_EPOCH);
        settings
            .positions
            .insert(listing.entries[4].path.clone(), 1);
        model.folders_first = true;
        for key in [
            SortKey::Modified,
            SortKey::Created,
            SortKey::Pages,
            SortKey::Progress,
            SortKey::Recent,
        ] {
            for descending in [false, true] {
                model.sort = key;
                model.descending = descending;
                model.rebuild(&listing, &settings);
                assert_eq!(&model.order[..2], &[0, 5], "{key:?}, {descending}");
                assert_eq!(model.order.last(), Some(&4), "{key:?}, {descending}");
            }
        }
        for descending in [false, true] {
            model.sort = SortKey::Size;
            model.descending = descending;
            model.rebuild(&listing, &settings);
            assert_eq!(&model.order[..2], &[5, 0]);
            assert_eq!(model.order.last(), Some(&4));
        }
    }

    #[test]
    fn aliases_and_duplicate_display_names_keep_native_selection_and_ties() {
        let mut a = entry("same.cbz", EntryKind::Archive);
        let mut b = a.clone();
        a.entry_path = PathBuf::from("native/a.cbz");
        b.entry_path = PathBuf::from("native/b.cbz");
        a.size = Some(7);
        b.size = Some(7);
        let listing = listing(vec![b.clone(), a.clone()]);
        let mut model = BrowserModel {
            selected: Some(b.entry_path.clone()),
            ..BrowserModel::default()
        };
        for key in SortKey::ALL
            .into_iter()
            .filter(|key| *key != SortKey::Random)
        {
            for descending in [false, true] {
                model.sort = key;
                model.descending = descending;
                model.rebuild(&listing, &AppSettings::default());
                assert_eq!(model.order, [1, 0], "{key:?}, {descending}");
                assert_eq!(model.selected.as_ref(), Some(&b.entry_path));
            }
        }
        assert_eq!(a.path, b.path);
        assert_ne!(a.entry_path, b.entry_path);
    }

    #[test]
    fn natural_ignores_case_but_literal_name_sort_does_not() {
        let listing = listing(
            ["page10", "page2", "Page2", "page01", "page1"]
                .map(|name| entry(name, EntryKind::Image))
                .into(),
        );
        let mut model = BrowserModel::default();
        model.rebuild(&listing, &AppSettings::default());
        let expected = {
            let mut indexes: Vec<_> = (0..5).collect();
            indexes.sort_by(|&a, &b| {
                natord::compare_ignore_case(&listing.entries[a].name, &listing.entries[b].name)
                    .then_with(|| listing.entries[a].name.cmp(&listing.entries[b].name))
            });
            indexes
        };
        assert_eq!(model.order, expected);
        model.sort = SortKey::Name;
        model.rebuild(&listing, &AppSettings::default());
        assert_eq!(model.order, [2, 3, 4, 0, 1]);
    }

    #[test]
    fn type_uses_category_then_lowercase_native_child_extension() {
        let mut upper = entry("a.ZIP", EntryKind::Archive);
        upper.path = PathBuf::from("canonical/target.cbr");
        let listing = listing(vec![
            upper,
            entry("b.zip", EntryKind::Archive),
            entry("z.CBR", EntryKind::Archive),
        ]);
        let mut model = BrowserModel {
            sort: SortKey::Type,
            ..BrowserModel::default()
        };
        model.rebuild(&listing, &AppSettings::default());
        assert_eq!(model.order, [2, 0, 1]);
        model.descending = true;
        model.rebuild(&listing, &AppSettings::default());
        assert_eq!(model.order, [0, 1, 2]);
    }

    #[test]
    fn known_counts_match_book_identity_and_recursive_policy() {
        let (listing, mut model, mut settings) = fixture();
        let folder = &listing.entries[0];
        let archive = &listing.entries[1];
        let image = &listing.entries[3];
        assert_eq!(model.page_count(folder, &settings), Some(6));
        settings.preferences.recursive_folder = false;
        assert_eq!(model.page_count(folder, &settings), None);
        model.known_pages.insert((folder.path.clone(), false), 2);
        assert_eq!(model.page_count(folder, &settings), Some(2));
        assert_eq!(model.page_count(archive, &settings), Some(3));
        model.known_pages.insert((archive.path.clone(), false), 0);
        model.known_pages.insert((archive.path.clone(), true), 100);
        assert_eq!(model.page_count(archive, &settings), None);
        model.known_pages.insert((image.path.clone(), false), 200);
        assert_eq!(model.page_count(image, &settings), Some(1));
        assert_eq!(model.progress(image, &settings), None);
    }

    #[test]
    fn progress_requires_saved_position_and_count_clamps_and_never_writes() {
        let (listing, mut model, mut settings) = fixture();
        let archive = &listing.entries[1];
        assert_eq!(model.progress(archive, &settings), Some(1.0 / 3.0));
        settings.positions.insert(archive.path.clone(), usize::MAX);
        assert_eq!(model.progress(archive, &settings), Some(1.0));
        model
            .known_pages
            .insert((archive.path.clone(), false), usize::MAX);
        assert_eq!(model.progress(archive, &settings), Some(1.0));
        settings.positions.remove(&archive.path);
        assert_eq!(model.progress(archive, &settings), None);
        settings.positions.insert(archive.path.clone(), 0);
        settings.preferences.remember_position = false;
        let before = settings.clone();
        for key in SortKey::ALL {
            model.sort = key;
            model.rebuild(&listing, &settings);
            assert!(model.progress(archive, &settings).is_some());
        }
        assert_eq!(settings, before);
        assert_eq!(
            model.progress(archive, &settings),
            Some(1.0 / usize::MAX as f64)
        );
    }

    #[test]
    fn progress_sort_compares_exact_ratios_and_keeps_equal_ratios_stable() {
        let listing = listing(
            ["a.cbz", "b.cbz", "c.cbz"]
                .map(|name| entry(name, EntryKind::Archive))
                .into(),
        );
        let mut model = BrowserModel {
            sort: SortKey::Progress,
            ..BrowserModel::default()
        };
        let mut settings = AppSettings::default();
        for (index, count, page) in [(0, 6, 1), (1, 3, 0), (2, 10, 2)] {
            let path = listing.entries[index].path.clone();
            model.known_pages.insert((path.clone(), false), count);
            settings.positions.insert(path, page);
        }
        model.rebuild(&listing, &settings);
        assert_eq!(model.order, [2, 0, 1]);
        model.descending = true;
        model.rebuild(&listing, &settings);
        assert_eq!(model.order, [0, 1, 2]);
        let path = listing.entries[0].path.clone();
        model.known_pages.insert((path.clone(), false), usize::MAX);
        settings.positions.insert(path, usize::MAX - 2);
        let path = listing.entries[1].path.clone();
        model.known_pages.insert((path.clone(), false), usize::MAX);
        settings.positions.insert(path, usize::MAX - 1);
        model.rebuild(&listing, &settings);
        assert_eq!(model.order, [1, 0, 2]);
    }

    #[test]
    fn recent_is_bounded_rank_and_first_duplicate_wins() {
        let listing = listing(
            ["a.cbz", "b.cbz", "c.cbz"]
                .map(|name| entry(name, EntryKind::Archive))
                .into(),
        );
        let mut settings = AppSettings {
            recent: vec![listing.entries[1].path.clone(); MAX_RECENT_BOOKS],
            ..AppSettings::default()
        };
        settings.recent[1] = listing.entries[0].path.clone();
        settings.recent.push(listing.entries[2].path.clone());
        let mut model = BrowserModel {
            sort: SortKey::Recent,
            ..BrowserModel::default()
        };
        model.rebuild(&listing, &settings);
        assert_eq!(model.order, [1, 0, 2]);
        model.descending = true;
        model.rebuild(&listing, &settings);
        assert_eq!(model.order, [0, 1, 2]);
    }

    #[test]
    fn random_is_stable_across_redraw_filter_selection_and_direction() {
        let listing = listing(
            (0..100)
                .map(|index| entry(&format!("book{index}.cbz"), EntryKind::Archive))
                .collect(),
        );
        let mut model = BrowserModel {
            sort: SortKey::Random,
            ..BrowserModel::default()
        };
        let settings = AppSettings::default();
        model.rebuild(&listing, &settings);
        let original = model.order.clone();
        let keys = model.random_keys.clone();
        for _ in 0..3 {
            model.selected = Some(listing.entries[42].entry_path.clone());
            model.rebuild(&listing, &settings);
            assert_eq!(model.order, original);
            assert_eq!(model.random_keys, keys);
        }
        model.query = "book1".into();
        model.rebuild(&listing, &settings);
        let filtered: Vec<_> = original
            .iter()
            .copied()
            .filter(|&index| listing.entries[index].name.contains("book1"))
            .collect();
        assert_eq!(model.order, filtered);
        model.query.clear();
        model.descending = true;
        model.rebuild(&listing, &settings);
        assert_eq!(
            model.order,
            original.iter().copied().rev().collect::<Vec<_>>()
        );
        model.shuffle();
        assert_eq!(
            model.order,
            original.iter().copied().rev().collect::<Vec<_>>()
        );
        model.descending = false;
        model.rebuild(&listing, &settings);
        assert_ne!(model.random_keys, keys);
        assert_ne!(model.order, original);
    }

    #[test]
    fn listing_changes_regenerate_random_keys_but_known_counts_do_not_reorder() {
        let (mut listing, mut model, settings) = fixture();
        model.sort = SortKey::Pages;
        model.rebuild(&listing, &settings);
        let order = model.order.clone();
        let keys = model.random_keys.clone();
        model
            .known_pages
            .insert((listing.entries[4].path.clone(), false), 2);
        assert_eq!(model.order, order);
        model.rebuild(&listing, &settings);
        assert_eq!(model.random_keys, keys);
        assert_ne!(model.order, order);
        listing.entries[0].entry_path = PathBuf::from("native/renamed");
        model.rebuild(&listing, &settings);
        assert_ne!(model.random_keys, keys);
        let keys = model.random_keys.clone();
        listing.path = PathBuf::from("other");
        model.rebuild(&listing, &settings);
        assert_ne!(model.random_keys, keys);
    }

    #[test]
    fn search_and_snapshot_restore_selection_with_empty_and_deleted_boundaries() {
        let listing = listing(
            ["BÖÖK 2.cbz", "böök 10.cbz", "other.cbz"]
                .map(|name| entry(name, EntryKind::Archive))
                .into(),
        );
        let settings = AppSettings::default();
        let mut model = BrowserModel {
            query: "  BÖÖK  ".into(),
            selected: Some(listing.entries[1].entry_path.clone()),
            ..BrowserModel::default()
        };
        model.rebuild(&listing, &settings);
        assert_eq!(model.order, [0, 1]);
        assert_eq!(
            model.selected.as_ref(),
            Some(&listing.entries[1].entry_path)
        );
        let snapshot = model.snapshot(Some(listing.entries[0].entry_path.clone()), 12.5);
        model.query = "missing".into();
        model.rebuild(&listing, &settings);
        assert!(model.order.is_empty());
        assert!(model.selected.is_none());
        model.sort = SortKey::Size;
        model.restore(&snapshot);
        model.rebuild(&listing, &settings);
        assert_eq!(model.selected, snapshot.selected);
        assert_eq!(model.query, snapshot.query);
        assert_eq!(model.sort, SortKey::Size);
        assert_eq!(snapshot.offset, 12.5);
        assert_eq!(
            snapshot.anchor.as_ref(),
            Some(&listing.entries[0].entry_path)
        );
        let mut deleted = listing.clone();
        deleted.entries.remove(1);
        model.rebuild(&deleted, &settings);
        assert_eq!(
            model.selected.as_ref(),
            Some(&deleted.entries[0].entry_path)
        );
        let empty = DirectoryListing {
            path: listing.path.clone(),
            parent: None,
            entries: Vec::new(),
        };
        model.rebuild(&empty, &settings);
        assert!(model.order.is_empty());
        assert!(model.selected.is_none());
    }

    #[test]
    fn snapshot_offsets_and_grid_geometry_clamp_invalid_values() {
        let model = BrowserModel::default();
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
            assert_eq!(model.snapshot(None, invalid).offset, 0.0);
            assert_eq!(columns(invalid, 100.0), 1);
            assert_eq!(columns(100.0, invalid), 1);
        }
        assert_eq!(BrowserSnapshot::default().offset, 0.0);
        assert_eq!(columns(0.0, 100.0), 1);
        assert_eq!(columns(100.0, 0.0), 1);
        assert_eq!(columns(99.0, 100.0), 1);
        assert_eq!(columns(300.0, 100.0), 3);
        assert_eq!(columns(299.0, 100.0), 2);
        assert_eq!(columns(f32::MAX, f32::MIN_POSITIVE), usize::MAX);
        assert_eq!(row_count(0, 0), 0);
        assert_eq!(row_count(10, 0), 10);
        assert_eq!(row_count(10, 3), 4);
        assert_eq!(row_count(9, 3), 3);
        assert_eq!(row_count(usize::MAX, 2), usize::MAX / 2 + 1);
        assert_eq!(row_count(usize::MAX, usize::MAX), 1);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn native_paths_with_the_same_lossy_display_remain_distinct() {
        use std::ffi::OsString;
        #[cfg(unix)]
        let paths = {
            use std::os::unix::ffi::OsStringExt;
            [
                OsString::from_vec(vec![b'x', 0xfe]),
                OsString::from_vec(vec![b'x', 0xff]),
            ]
        };
        #[cfg(windows)]
        let paths = {
            use std::os::windows::ffi::OsStringExt;
            [
                OsString::from_wide(&[120, 0xd800]),
                OsString::from_wide(&[120, 0xd801]),
            ]
        };
        assert_eq!(paths[0].to_string_lossy(), paths[1].to_string_lossy());
        let mut entries = [
            entry("same", EntryKind::Archive),
            entry("same", EntryKind::Archive),
        ];
        for (entry, path) in entries.iter_mut().zip(paths) {
            entry.entry_path = PathBuf::from(path);
        }
        let listing = listing(entries.into());
        let mut model = BrowserModel {
            selected: Some(listing.entries[1].entry_path.clone()),
            ..BrowserModel::default()
        };
        for descending in [false, true] {
            model.descending = descending;
            model.rebuild(&listing, &AppSettings::default());
            assert_eq!(model.order, [0, 1]);
            assert_eq!(
                model.selected.as_ref(),
                Some(&listing.entries[1].entry_path)
            );
        }
    }

    #[test]
    fn ten_thousand_synthetic_entries_keep_valid_indexes_and_native_selection() {
        let listing = listing(
            (0..10_000)
                .rev()
                .map(|index| entry(&format!("book{index}.cbz"), EntryKind::Archive))
                .collect(),
        );
        let mut model = BrowserModel {
            selected: Some(listing.entries[1234].entry_path.clone()),
            ..BrowserModel::default()
        };
        let settings = AppSettings::default();
        for key in SortKey::ALL {
            for descending in [false, true] {
                model.sort = key;
                model.descending = descending;
                model.rebuild(&listing, &settings);
                assert_eq!(model.order.len(), 10_000);
                let mut indexes = model.order.clone();
                indexes.sort_unstable();
                assert_eq!(indexes, (0..10_000).collect::<Vec<_>>());
                assert_eq!(
                    model.selected.as_ref(),
                    Some(&listing.entries[1234].entry_path)
                );
            }
        }
        model.query = "book9999".into();
        model.rebuild(&listing, &settings);
        assert_eq!(model.order, [0]);
        assert_eq!(
            model.selected.as_ref(),
            Some(&listing.entries[0].entry_path)
        );
    }
}
