//! Session-only Files tree. Directory reads run outside this model.

use crate::book::{self, DirectoryEntry, EntryKind};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Component, Path, PathBuf},
};

// History snapshots retain loaded metadata; cap them independently of virtual rows.
const MAX_HISTORY: usize = 32;

#[derive(Clone, Debug)]
pub(crate) struct TreeEntry {
    pub entry: DirectoryEntry,
    pub hidden: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct LoadedDirectory {
    /// Original request spelling, used to validate the result.
    pub requested_path: PathBuf,
    /// Accepted absolute logical path, including valid aliases.
    pub path: PathBuf,
    pub target: PathBuf,
    pub entries: Vec<TreeEntry>,
}

/// Call on a worker. Never recursively enumerates children.
pub(crate) fn load_directory(path: &Path) -> Result<LoadedDirectory, String> {
    let listing = book::browse_directory(path)?;
    let accepted_path = normalize_entry_path(path, &listing.path)?;
    let mut entries = Vec::with_capacity(listing.entries.len());
    for mut entry in listing.entries {
        let name = entry
            .entry_path
            .file_name()
            .ok_or("Entry has no native name.")?;
        entry.entry_path = accepted_path.join(name);
        let metadata =
            fs::symlink_metadata(&entry.entry_path).map_err(|error| error.to_string())?;
        let hidden = hidden_entry(&entry.entry_path, &metadata);
        entries.push(TreeEntry { entry, hidden });
    }
    Ok(LoadedDirectory {
        requested_path: path.to_path_buf(),
        path: accepted_path,
        target: listing.path,
        entries,
    })
}

/// Normalize a successful native entry on a worker without merging valid aliases.
/// `target` must be the actual canonical target of the original request.
pub(crate) fn normalize_entry_path(path: &Path, target: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(path).map_err(|error| error.to_string())?;
    // Windows absolute paths can already have their parent components folded.
    let had_parent = path
        .components()
        .chain(absolute.components())
        .any(|component| component == Component::ParentDir);
    let mut logical = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                logical.pop();
            }
            _ => logical.push(component.as_os_str()),
        }
    }
    // POSIX resolves symlinks before `..`. Keep the lexical alias only when
    // folding its parent components still identifies the directory actually read.
    if had_parent && !fs::canonicalize(&logical).is_ok_and(|candidate| candidate == target) {
        return Ok(target.to_path_buf());
    }
    Ok(logical)
}

fn hidden_entry(path: &Path, metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    let attributes = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes()
    };
    #[cfg(not(windows))]
    let attributes = {
        let _ = metadata;
        0
    };
    hidden_name_and_attributes(path, attributes)
}

fn hidden_name_and_attributes(path: &Path, attributes: u32) -> bool {
    path.file_name()
        .is_some_and(|name| name.as_encoded_bytes().starts_with(b"."))
        || attributes & (0x2 | 0x4) != 0
}

/// Display only: do not use this string as a filesystem identity.
pub(crate) fn display_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(drive) = value.strip_prefix(r"\\?\") {
        if drive.as_bytes().get(1) == Some(&b':') {
            drive.to_owned()
        } else {
            value.into_owned()
        }
    } else {
        value.into_owned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RequestToken {
    generation: u64,
    serial: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct DirectoryRequest {
    pub token: RequestToken,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    Entry,
    Loading,
    Error,
}

#[derive(Clone, Debug)]
pub(crate) struct VisibleRow {
    /// Native entry identity, or owner identity for a status row.
    pub path: PathBuf,
    pub depth: usize,
    pub kind: RowKind,
    pub entry: Option<DirectoryEntry>,
    pub expanded: bool,
    pub cycle: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub entry: DirectoryEntry,
    pub hidden: bool,
    pub expanded: bool,
    pub loaded: bool,
    pub cycle: bool,
    pub error: Option<String>,
    children: Vec<PathBuf>,
    pending: Option<RequestToken>,
}

impl Node {
    pub(crate) fn loading(&self) -> bool {
        self.pending.is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ScrollAnchor {
    pub path: Option<PathBuf>,
    pub offset: f32,
}

#[derive(Clone, Debug, Default)]
struct Snapshot {
    root: Option<PathBuf>,
    location: Option<PathBuf>,
    nodes: HashMap<PathBuf, Node>,
    selected: Option<PathBuf>,
    focused: Option<PathBuf>,
    query: String,
    show_hidden: bool,
    scroll: ScrollAnchor,
}

#[derive(Clone, Debug)]
enum Purpose {
    Navigation,
    Expansion,
    Locate,
}

struct Locate {
    source: PathBuf,
    base: PathBuf,
    state: Snapshot,
    staged: Vec<LoadedDirectory>,
    refreshed: HashSet<PathBuf>,
    request: Option<DirectoryRequest>,
}

#[derive(Clone, Debug)]
struct Work {
    request: DirectoryRequest,
    purpose: Purpose,
    expected_target: Option<PathBuf>,
}

#[derive(Default)]
pub(crate) struct FileTree {
    state: Snapshot,
    history: VecDeque<Snapshot>,
    rows: Vec<VisibleRow>,
    queue: VecDeque<Work>,
    running: HashMap<RequestToken, Work>,
    generation: u64,
    serial: u64,
    navigation: Option<DirectoryRequest>,
    locate: Option<Locate>,
    error: Option<String>,
}

impl FileTree {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn root(&self) -> Option<&Path> {
        self.state.root.as_deref()
    }

    pub(crate) fn location(&self) -> Option<&Path> {
        self.state.location.as_deref()
    }

    pub(crate) fn query(&self) -> &str {
        &self.state.query
    }

    pub(crate) fn set_query(&mut self, query: String) {
        self.state.query = query;
        self.cancel_locate();
        self.rebuild();
    }

    pub(crate) fn show_hidden(&self) -> bool {
        self.state.show_hidden
    }

    pub(crate) fn set_show_hidden(&mut self, show: bool) {
        self.state.show_hidden = show;
        self.cancel_locate();
        if show {
            self.resume_visible_expansions();
        }
        self.rebuild();
    }

    pub(crate) fn selected(&self) -> Option<&Path> {
        self.state.selected.as_deref()
    }

    pub(crate) fn focused(&self) -> Option<&Path> {
        self.state.focused.as_deref()
    }

    pub(crate) fn selected_entry(&self) -> Option<&DirectoryEntry> {
        self.selected()
            .and_then(|path| self.node(path))
            .map(|node| &node.entry)
    }

    pub(crate) fn selected_visible_entry(&self) -> Option<&DirectoryEntry> {
        let path = self.selected()?;
        self.rows
            .iter()
            .find(|row| row.kind == RowKind::Entry && row.path == path)?
            .entry
            .as_ref()
    }

    pub(crate) fn select(&mut self, path: &Path) -> bool {
        if !self.state.nodes.contains_key(path) {
            return false;
        }
        self.cancel_locate();
        self.state.selected = Some(path.to_path_buf());
        self.focus(path);
        true
    }

    pub(crate) fn focus(&mut self, path: &Path) -> bool {
        if !self
            .rows
            .iter()
            .any(|row| row.kind == RowKind::Entry && row.path == path)
        {
            return false;
        }
        self.cancel_locate();
        self.state.focused = Some(path.to_path_buf());
        true
    }

    pub(crate) fn visible_rows(&self) -> &[VisibleRow] {
        &self.rows
    }

    pub(crate) fn node(&self, path: &Path) -> Option<&Node> {
        self.state.nodes.get(path)
    }

    pub(crate) fn children(&self, path: &Path) -> impl Iterator<Item = &Node> {
        self.node(path)
            .into_iter()
            .flat_map(|node| &node.children)
            .filter_map(|path| self.node(path))
    }

    pub(crate) fn is_loading(&self, path: &Path) -> bool {
        self.node(path).is_some_and(Node::loading)
            || self
                .navigation
                .as_ref()
                .is_some_and(|request| request.path == path)
            || self
                .locate
                .as_ref()
                .and_then(|locate| locate.request.as_ref())
                .is_some_and(|request| request.path == path)
    }

    pub(crate) fn node_error(&self, path: &Path) -> Option<&str> {
        self.node(path).and_then(|node| node.error.as_deref())
    }

    pub(crate) fn scroll_anchor(&self) -> &ScrollAnchor {
        &self.state.scroll
    }

    pub(crate) fn set_scroll_anchor(&mut self, anchor: ScrollAnchor) {
        self.state.scroll = anchor;
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn clear_error(&mut self) {
        self.error = None;
    }

    pub(crate) fn cancel_navigation(&mut self) {
        self.navigation = None;
    }

    pub(crate) fn cancel_locate(&mut self) {
        if self.locate.take().is_some() {
            self.resume_visible_expansions();
            self.rebuild();
        }
    }

    fn resume_visible_expansions(&mut self) {
        if let Some(root) = self.state.root.clone() {
            self.resume_expansions(&root);
        }
    }

    pub(crate) fn is_navigation_request(&self, token: RequestToken) -> bool {
        self.navigation
            .as_ref()
            .is_some_and(|request| request.token == token)
    }

    pub(crate) fn pending_navigation(&self) -> Option<&Path> {
        self.navigation
            .as_ref()
            .map(|request| request.path.as_path())
    }

    pub(crate) fn can_back(&self) -> bool {
        !self.history.is_empty()
    }

    pub(crate) fn can_parent(&self) -> bool {
        self.location()
            .and_then(Path::parent)
            .is_some_and(|path| !path.as_os_str().is_empty())
    }

    fn token(&mut self) -> RequestToken {
        self.serial += 1;
        RequestToken {
            generation: self.generation,
            serial: self.serial,
        }
    }

    /// The old location remains usable until a successful result is accepted.
    pub(crate) fn request_navigation(&mut self, path: PathBuf) {
        self.cancel_locate();
        if self
            .navigation
            .as_ref()
            .is_some_and(|request| request.path == path)
        {
            return;
        }
        let request = DirectoryRequest {
            token: self.token(),
            path,
        };
        self.navigation = Some(request.clone());
        self.error = None;
        self.queue.push_back(Work {
            request,
            purpose: Purpose::Navigation,
            expected_target: None,
        });
    }

    pub(crate) fn request_parent(&mut self) -> bool {
        let Some(parent) = self
            .location()
            .and_then(Path::parent)
            .filter(|path| !path.as_os_str().is_empty())
        else {
            return false;
        };
        self.request_navigation(parent.to_path_buf());
        true
    }

    pub(crate) fn back(&mut self) -> bool {
        let Some(snapshot) = self.history.pop_back() else {
            return false;
        };
        self.invalidate();
        self.state = snapshot;
        self.error = None;
        self.resume_visible_expansions();
        self.rebuild();
        true
    }

    /// Cancels source-specific work without clearing the accepted sidebar state.
    /// Running synchronous reads retain their slots even after close.
    /// Canceled unloaded branches collapse so their disclosures can start a new read.
    pub(crate) fn close(&mut self) {
        self.invalidate();
        for node in self.state.nodes.values_mut() {
            if !node.loaded {
                node.expanded = false;
            }
        }
        self.error = None;
        self.rebuild();
    }

    fn invalidate(&mut self) {
        self.generation += 1;
        self.navigation = None;
        self.locate = None;
        self.queue.clear();
        for node in self.state.nodes.values_mut() {
            node.pending = None;
        }
    }

    pub(crate) fn expand(&mut self, path: &Path) -> bool {
        self.cancel_locate();
        if !self.query().trim().is_empty() {
            return false;
        }
        let Some(node) = self.state.nodes.get_mut(path) else {
            return false;
        };
        if !node.entry.is_directory || node.cycle {
            return false;
        }
        node.expanded = true;
        node.error = None;
        self.resume_expansions(path);
        self.rebuild();
        true
    }

    // Resume explicit expansion only, not temporary search expansion. Do not read
    // hidden descendants or branches below a collapsed ancestor.
    fn resume_expansions(&mut self, path: &Path) {
        let mut stack = vec![path.to_path_buf()];
        while let Some(path) = stack.pop() {
            let Some(node) = self.node(&path) else {
                continue;
            };
            if (!node.expanded && self.root() != Some(path.as_path())) || node.cycle {
                continue;
            }
            let needs_load = !node.loaded && node.pending.is_none() && node.error.is_none();
            stack.extend(node.children.iter().filter_map(|child| {
                self.node(child)
                    .filter(|node| self.show_hidden() || !node.hidden)
                    .map(|_| child.clone())
            }));
            if needs_load {
                self.enqueue_expansion(&path);
            }
        }
    }

    fn enqueue_expansion(&mut self, path: &Path) {
        let token = self.token();
        let node = self.state.nodes.get_mut(path).expect("known folder");
        node.pending = Some(token);
        node.error = None;
        self.queue.push_back(Work {
            request: DirectoryRequest {
                token,
                path: path.to_path_buf(),
            },
            purpose: Purpose::Expansion,
            expected_target: Some(node.entry.path.clone()),
        });
    }

    pub(crate) fn collapse(&mut self, path: &Path) -> bool {
        self.cancel_locate();
        if !self.query().trim().is_empty() || self.root() == Some(path) {
            return false;
        }
        let Some(node) = self.state.nodes.get_mut(path) else {
            return false;
        };
        if !node.entry.is_directory || node.cycle {
            return false;
        }
        node.expanded = false;
        for (child_path, node) in &mut self.state.nodes {
            if child_path.starts_with(path) {
                node.pending = None;
            }
        }
        self.rebuild();
        true
    }

    pub(crate) fn toggle_expanded(&mut self, path: &Path) -> bool {
        if self.node(path).is_some_and(|node| node.expanded) {
            self.collapse(path)
        } else {
            self.expand(path)
        }
    }

    pub(crate) fn retry(&mut self, path: &Path) -> bool {
        self.cancel_locate();
        let Some(node) = self.node(path) else {
            return false;
        };
        if !node.entry.is_directory || node.cycle || node.loading() {
            return false;
        }
        self.enqueue_expansion(path);
        self.rebuild();
        true
    }

    /// Reveals a known entry without crawling. Returns false if the entry is not loaded.
    /// Hidden visibility is enabled only when an ancestor or the entry requires it.
    pub(crate) fn reveal_loaded(&mut self, path: &Path) -> bool {
        if !self.state.nodes.contains_key(path) {
            return false;
        }
        self.cancel_locate();
        self.state.query.clear();
        for ancestor in path.ancestors() {
            if let Some(node) = self.state.nodes.get_mut(ancestor) {
                self.state.show_hidden |= node.hidden;
                if ancestor != path && !node.cycle {
                    node.expanded = true;
                }
            }
        }
        self.state.selected = Some(path.to_path_buf());
        self.state.focused = Some(path.to_path_buf());
        self.rebuild();
        true
    }

    /// Finds the original source entry. Reads only its ancestor chain, one folder at a time.
    /// Results are staged until the entry is found, so failures retain the accepted tree.
    pub(crate) fn locate(&mut self, source: PathBuf) {
        self.navigation = None;
        self.locate = None;
        self.error = None;
        let parent = source.parent().filter(|path| !path.as_os_str().is_empty());
        let keep_root = self
            .root()
            .is_some_and(|root| source.starts_with(root) && (source != root || parent.is_none()));
        let base = if keep_root {
            self.state.root.clone().expect("existing root")
        } else {
            parent.unwrap_or(&source).to_path_buf()
        };
        // Reads already running for this chain must not overwrite staged Locate results.
        // Unrelated branch reads remain valid and retain their worker slots.
        for (path, node) in &mut self.state.nodes {
            if source.starts_with(path) {
                node.pending = None;
            }
        }
        let state = if keep_root {
            self.snapshot()
        } else {
            Snapshot {
                show_hidden: self.show_hidden(),
                ..Snapshot::default()
            }
        };
        self.advance_locate(Locate {
            source,
            base,
            state,
            staged: Vec::new(),
            refreshed: HashSet::new(),
            request: None,
        });
        self.rebuild();
    }

    fn locate_next(locate: &Locate) -> Result<Option<PathBuf>, String> {
        let Some(root) = locate.state.root.as_deref() else {
            return Ok(Some(locate.base.clone()));
        };
        if locate.state.nodes.contains_key(&locate.source) {
            return Ok(None);
        }
        let mut chain: Vec<_> = locate
            .source
            .ancestors()
            .filter(|path| path.starts_with(root))
            .collect();
        chain.reverse();
        for pair in chain.windows(2) {
            let folder = pair[0];
            let child = pair[1];
            let Some(node) = locate.state.nodes.get(folder) else {
                return Err("Source ancestor is not available.".into());
            };
            if !node.entry.is_directory || node.cycle {
                return Err("Source path crosses a file or a folder cycle.".into());
            }
            if !node.loaded
                || (!locate.state.nodes.contains_key(child) && !locate.refreshed.contains(folder))
            {
                return Ok(Some(folder.to_path_buf()));
            }
            if !locate.state.nodes.contains_key(child) {
                return Err(format!(
                    "Source entry is no longer available: {}",
                    display_path(&locate.source)
                ));
            }
        }
        Err("Source entry is not inside the requested tree.".into())
    }

    fn advance_locate(&mut self, mut locate: Locate) {
        match Self::locate_next(&locate) {
            Err(error) => {
                self.error = Some(error);
                self.resume_visible_expansions();
            }
            Ok(Some(path)) => {
                let request = DirectoryRequest {
                    token: self.token(),
                    path,
                };
                self.queue.push_back(Work {
                    request: request.clone(),
                    purpose: Purpose::Locate,
                    expected_target: locate
                        .state
                        .nodes
                        .get(&request.path)
                        .map(|node| node.entry.path.clone()),
                });
                locate.request = Some(request);
                self.locate = Some(locate);
            }
            Ok(None) => {
                let same_root = self.state.root == locate.state.root;
                let mut candidate = Self {
                    state: if same_root {
                        self.state.clone()
                    } else {
                        locate.state
                    },
                    ..Self::default()
                };
                if same_root {
                    // Check the accepted state before replay: a staged parent must not
                    // restore an old child identity over a concurrent parent refresh.
                    if locate.staged.iter().any(|loaded| {
                        self.node(&loaded.path)
                            .is_some_and(|node| node.entry.path != loaded.target)
                    }) {
                        self.error =
                            Some("Source ancestor target changed. Locate the source again.".into());
                        self.resume_visible_expansions();
                        return;
                    }
                    for loaded in locate.staged {
                        if !candidate
                            .node(&loaded.path)
                            .is_some_and(|node| node.entry.path == loaded.target)
                        {
                            self.error = Some(
                                "Source ancestor is no longer available or its target changed."
                                    .into(),
                            );
                            self.resume_visible_expansions();
                            return;
                        }
                        candidate.install(loaded);
                    }
                }
                if !candidate.reveal_loaded(&locate.source) {
                    self.error = Some("Source entry is no longer available.".into());
                    self.resume_visible_expansions();
                    return;
                }
                candidate.state.location = Some(
                    locate
                        .source
                        .parent()
                        .filter(|path| !path.as_os_str().is_empty())
                        .unwrap_or(&locate.source)
                        .to_path_buf(),
                );
                candidate.state.scroll = ScrollAnchor {
                    path: Some(locate.source),
                    offset: 0.0,
                };
                if self.root().is_some()
                    && (self.state.location != candidate.state.location || !same_root)
                {
                    self.remember_history();
                }
                if !same_root {
                    self.invalidate();
                }
                self.state = candidate.state;
                self.error = None;
                self.rebuild();
            }
        }
    }

    fn valid(&self, work: &Work) -> bool {
        if work.request.token.generation != self.generation {
            return false;
        }
        match work.purpose {
            Purpose::Navigation => self
                .navigation
                .as_ref()
                .is_some_and(|request| request.token == work.request.token),
            Purpose::Expansion => self
                .node(&work.request.path)
                .is_some_and(|node| node.pending == Some(work.request.token)),
            Purpose::Locate => self
                .locate
                .as_ref()
                .and_then(|locate| locate.request.as_ref())
                .is_some_and(|request| request.token == work.request.token),
        }
    }

    /// Marks returned requests as running. Call again after each completion.
    pub(crate) fn take_requests(&mut self) -> Vec<DirectoryRequest> {
        let mut requests = Vec::new();
        while self.running.len() < 2 {
            let Some(work) = self.queue.pop_front() else {
                break;
            };
            if self.valid(&work) {
                requests.push(work.request.clone());
                self.running.insert(work.request.token, work);
            }
        }
        requests
    }

    /// Returns true only for a current result. Duplicate and obsolete completions do nothing.
    pub(crate) fn complete(
        &mut self,
        token: RequestToken,
        result: Result<LoadedDirectory, String>,
    ) -> bool {
        let Some(work) = self.running.remove(&token) else {
            return false;
        };
        if !self.valid(&work) {
            return false;
        }
        if let Ok(loaded) = &result
            && loaded.requested_path == work.request.path
            && loaded.path == work.request.path
            && let Some(expected) = &work.expected_target
            && loaded.target != *expected
        {
            let error = "Folder target changed. Retry if restored, or refresh its parent.";
            if self
                .node(&work.request.path)
                .is_some_and(|node| node.entry.path == *expected)
            {
                self.clear_owner(&work.request.path, error);
            }
            if matches!(work.purpose, Purpose::Locate) {
                self.locate = None;
                self.error = Some(error.into());
                self.resume_visible_expansions();
            }
            self.rebuild();
            return true;
        }
        let result = result.and_then(|loaded| {
            if loaded.requested_path != work.request.path {
                Err("Directory result does not match its requested native path.".into())
            } else if !matches!(work.purpose, Purpose::Navigation)
                && loaded.path != work.request.path
            {
                Err("Directory result does not match its current owner path.".into())
            } else {
                Ok(loaded)
            }
        });
        match work.purpose {
            Purpose::Navigation => {
                self.navigation = None;
                match result {
                    Ok(loaded) => self.accept_navigation(loaded),
                    Err(error) => self.error = Some(error),
                }
            }
            Purpose::Locate => {
                let mut locate = self.locate.take().expect("current locate");
                locate.request = None;
                match result {
                    Ok(loaded) => {
                        let mut candidate = Self {
                            state: locate.state,
                            ..Self::default()
                        };
                        if candidate.root().is_none() {
                            candidate.accept_navigation(loaded.clone());
                        } else {
                            candidate.install(loaded.clone());
                        }
                        locate.refreshed.insert(loaded.path.clone());
                        locate.staged.push(loaded);
                        locate.state = candidate.state;
                        self.advance_locate(locate);
                    }
                    Err(error) => {
                        self.error = Some(error);
                        self.resume_visible_expansions();
                    }
                }
            }
            Purpose::Expansion => {
                let node = self
                    .state
                    .nodes
                    .get_mut(&work.request.path)
                    .expect("current node");
                node.pending = None;
                match result {
                    Ok(loaded) => self.install(loaded),
                    Err(error) => node.error = Some(error),
                }
            }
        }
        self.rebuild();
        true
    }

    fn remember_history(&mut self) {
        if self.history.len() == MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(self.snapshot());
    }

    fn snapshot(&self) -> Snapshot {
        let mut snapshot = self.state.clone();
        for node in snapshot.nodes.values_mut() {
            node.pending = None;
        }
        snapshot
    }

    fn accept_navigation(&mut self, loaded: LoadedDirectory) {
        if self.state.root.is_some() && self.location() != Some(loaded.path.as_path()) {
            self.remember_history();
        }
        let inside = self
            .root()
            .is_some_and(|root| loaded.path.starts_with(root))
            && self.node(&loaded.path).is_some_and(|node| {
                node.entry.is_directory && !node.cycle && node.entry.path == loaded.target
            })
            // A filtered ancestor must not hide an explicitly accepted destination.
            && (self.show_hidden()
                || !loaded
                    .path
                    .ancestors()
                    .any(|path| self.node(path).is_some_and(|node| node.hidden)));
        if !inside {
            self.invalidate();
            self.state.nodes.clear();
            self.state.root = Some(loaded.path.clone());
            self.state.nodes.insert(
                loaded.path.clone(),
                Node {
                    entry: DirectoryEntry {
                        name: loaded
                            .path
                            .file_name()
                            .unwrap_or(loaded.path.as_os_str())
                            .to_string_lossy()
                            .into_owned(),
                        entry_path: loaded.path.clone(),
                        path: loaded.target.clone(),
                        is_directory: true,
                        kind: EntryKind::Folder,
                        size: None,
                        modified: None,
                        created: None,
                    },
                    hidden: false,
                    expanded: true,
                    loaded: false,
                    cycle: false,
                    error: None,
                    children: Vec::new(),
                    pending: None,
                },
            );
        }
        self.state.location = Some(loaded.path.clone());
        self.state.selected = Some(loaded.path.clone());
        self.state.focused = Some(loaded.path.clone());
        self.state.scroll = ScrollAnchor::default();
        for ancestor in loaded.path.ancestors() {
            if let Some(node) = self.state.nodes.get_mut(ancestor) {
                node.expanded = true;
            }
        }
        self.install(loaded);
        self.resume_visible_expansions();
        self.error = None;
    }

    fn install(&mut self, mut loaded: LoadedDirectory) {
        let ancestors: HashSet<_> = loaded
            .path
            .ancestors()
            .filter(|path| *path != loaded.path)
            .filter_map(|path| self.node(path).map(|node| node.entry.path.clone()))
            .chain(std::iter::once(loaded.target.clone()))
            .collect();
        loaded.entries.sort_by(|left, right| {
            right
                .entry
                .is_directory
                .cmp(&left.entry.is_directory)
                .then_with(|| natord::compare_ignore_case(&left.entry.name, &right.entry.name))
                .then_with(|| left.entry.name.cmp(&right.entry.name))
                .then_with(|| left.entry.entry_path.cmp(&right.entry.entry_path))
        });
        let mut children = Vec::with_capacity(loaded.entries.len());
        let mut seen = HashSet::new();
        for item in loaded.entries {
            let entry = item.entry;
            let path = entry.entry_path.clone();
            // A malformed result cannot inject a parent, grandchild, or duplicate identity.
            if path.parent() != Some(loaded.path.as_path()) || !seen.insert(path.clone()) {
                continue;
            }
            let cycle = entry.is_directory && ancestors.contains(&entry.path);
            let preserve = self.node(&path).is_some_and(|old| {
                old.entry.path == entry.path && old.entry.kind == entry.kind && old.cycle == cycle
            });
            if preserve {
                let node = self.state.nodes.get_mut(&path).expect("existing child");
                node.entry = entry;
                node.hidden = item.hidden;
            } else {
                self.state.nodes.insert(
                    path.clone(),
                    Node {
                        entry,
                        hidden: item.hidden,
                        expanded: false,
                        loaded: false,
                        cycle,
                        error: None,
                        children: Vec::new(),
                        pending: None,
                    },
                );
            }
            children.push(path);
        }
        let node = self
            .state
            .nodes
            .get_mut(&loaded.path)
            .expect("loaded folder exists");
        node.entry.path = loaded.target;
        node.children = children;
        node.loaded = true;
        node.error = None;
        node.pending = None;
        self.prune_detached();
    }

    fn clear_owner(&mut self, path: &Path, error: &str) {
        let node = self.state.nodes.get_mut(path).expect("known owner");
        node.children.clear();
        node.loaded = false;
        node.pending = None;
        node.error = Some(error.into());
        self.prune_detached();
    }

    fn prune_detached(&mut self) {
        // Remove detached descendants so deleted selections cannot be read.
        let mut reachable = HashSet::new();
        let mut stack: Vec<_> = self.state.root.iter().cloned().collect();
        while let Some(path) = stack.pop() {
            if reachable.insert(path.clone())
                && let Some(node) = self.node(&path)
            {
                stack.extend(node.children.iter().cloned());
            }
        }
        self.state.nodes.retain(|path, _| reachable.contains(path));
        let queue = std::mem::take(&mut self.queue);
        self.queue = queue.into_iter().filter(|work| self.valid(work)).collect();
        if self
            .state
            .selected
            .as_ref()
            .is_some_and(|path| !reachable.contains(path))
        {
            self.state.selected = None;
        }
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        let Some(root) = self.state.root.clone() else {
            return;
        };
        let query = self.query().trim().to_lowercase();
        let searching = !query.is_empty();
        let mut matches = HashSet::new();
        if searching {
            let mut order = Vec::new();
            let mut stack = vec![root.clone()];
            while let Some(path) = stack.pop() {
                let Some(node) = self.node(&path) else {
                    continue;
                };
                if path != root && node.hidden && !self.show_hidden() {
                    continue;
                }
                order.push(path.clone());
                if node.entry.name.to_lowercase().contains(&query) {
                    matches.insert(path.clone());
                }
                stack.extend(node.children.iter().cloned());
            }
            for path in order.into_iter().rev() {
                if matches.contains(&path)
                    && let Some(parent) = path.parent()
                {
                    matches.insert(parent.to_path_buf());
                }
            }
        }
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((path, depth)) = stack.pop() {
            let Some(node) = self.node(&path) else {
                continue;
            };
            let is_root = path == root;
            if !is_root
                && ((!self.show_hidden() && node.hidden) || (searching && !matches.contains(&path)))
            {
                continue;
            }
            let expanded =
                node.entry.is_directory && !node.cycle && (is_root || node.expanded || searching);
            let children = if expanded {
                node.children.clone()
            } else {
                Vec::new()
            };
            let status = if expanded && !searching {
                if node.loading() {
                    Some(RowKind::Loading)
                } else if node.error.is_some() {
                    Some(RowKind::Error)
                } else {
                    None
                }
            } else {
                None
            };
            let row = VisibleRow {
                path: path.clone(),
                depth,
                kind: RowKind::Entry,
                entry: Some(node.entry.clone()),
                expanded,
                cycle: node.cycle,
                error: node.error.clone(),
            };
            let status_error = node.error.clone();
            if !is_root {
                self.rows.push(row);
            }
            let child_depth = if is_root { 0 } else { depth + 1 };
            if let Some(kind) = status {
                self.rows.push(VisibleRow {
                    path: path.clone(),
                    depth: child_depth,
                    kind,
                    entry: None,
                    expanded: false,
                    cycle: false,
                    error: status_error,
                });
            }
            stack.extend(children.into_iter().rev().map(|path| (path, child_depth)));
        }
        let visible: HashSet<_> = self
            .rows
            .iter()
            .filter(|row| row.kind == RowKind::Entry)
            .map(|row| row.path.as_path())
            .collect();
        let root_location_focus =
            self.state.focused.as_deref() == Some(root.as_path()) && root.parent().is_none();
        if !root_location_focus
            && !self
                .state
                .focused
                .as_deref()
                .is_some_and(|path| visible.contains(path))
        {
            self.state.focused = self
                .state
                .focused
                .as_deref()
                .and_then(|path| path.ancestors().find(|path| visible.contains(path)))
                .map(Path::to_path_buf)
                .or_else(|| {
                    self.rows
                        .iter()
                        .find(|row| row.kind == RowKind::Entry)
                        .map(|row| row.path.clone())
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn item(parent: &Path, name: &str, folder: bool) -> TreeEntry {
        let path = parent.join(name);
        TreeEntry {
            entry: DirectoryEntry {
                name: name.into(),
                entry_path: path.clone(),
                path,
                is_directory: folder,
                kind: if folder {
                    EntryKind::Folder
                } else {
                    EntryKind::Image
                },
                size: None,
                modified: None,
                created: None,
            },
            hidden: name.starts_with('.'),
        }
    }

    fn listing(path: &Path, entries: Vec<TreeEntry>) -> LoadedDirectory {
        LoadedDirectory {
            requested_path: path.into(),
            path: path.into(),
            target: path.into(),
            entries,
        }
    }

    fn initialize(entries: &[(&str, bool)]) -> FileTree {
        let mut tree = FileTree::new();
        let root = Path::new("root");
        tree.request_navigation(root.into());
        let request = tree.take_requests().pop().unwrap();
        assert!(
            tree.complete(
                request.token,
                Ok(listing(
                    root,
                    entries
                        .iter()
                        .map(|(name, folder)| item(root, name, *folder))
                        .collect()
                ))
            )
        );
        tree
    }

    fn load_branch(tree: &mut FileTree, folder: &str, entries: &[(&str, bool)]) {
        let path = Path::new(folder);
        assert!(tree.expand(path));
        let request = tree.take_requests().pop().unwrap();
        assert_eq!(request.path, path);
        assert!(
            tree.complete(
                request.token,
                Ok(listing(
                    path,
                    entries
                        .iter()
                        .map(|(name, folder)| item(path, name, *folder))
                        .collect()
                ))
            )
        );
    }

    fn names(tree: &FileTree) -> Vec<&str> {
        tree.visible_rows()
            .iter()
            .filter_map(|row| row.entry.as_ref().map(|entry| entry.name.as_str()))
            .collect()
    }

    #[test]
    fn history_snapshot_count_is_bounded_without_losing_recent_locations() {
        let mut tree = initialize(&[("page.png", false)]);
        for index in 0..MAX_HISTORY + 3 {
            let path = PathBuf::from(format!("folder-{index}"));
            tree.request_navigation(path.clone());
            let request = tree.take_requests().pop().unwrap();
            assert!(tree.complete(request.token, Ok(listing(&path, vec![]))));
            assert!(tree.history.len() <= MAX_HISTORY);
        }
        let mut restored = 0;
        while tree.back() {
            restored += 1;
        }
        assert_eq!(restored, MAX_HISTORY);
        assert_eq!(tree.location(), Some(Path::new("folder-2")));
    }

    #[test]
    fn ten_thousand_siblings_have_natural_order_and_valid_virtual_indices() {
        let root = Path::new("root");
        let mut tree = FileTree::new();
        tree.request_navigation(root.into());
        let request = tree.take_requests().pop().unwrap();
        let entries = (0..10_000)
            .rev()
            .map(|index| item(root, &format!("page{index}.png"), false))
            .collect();
        tree.complete(request.token, Ok(listing(root, entries)));
        assert_eq!(tree.visible_rows().len(), 10_000);
        for (index, row) in tree.visible_rows().iter().enumerate() {
            assert_eq!(row.entry.as_ref().unwrap().name, format!("page{index}.png"));
            assert_eq!(row.depth, 0);
            assert_eq!(row.kind, RowKind::Entry);
            assert!(tree.node(&row.path).is_some());
        }
        assert!(tree.take_requests().is_empty());
    }

    #[test]
    fn folders_first_natural_case_order_and_native_ties() {
        let tree = initialize(&[
            ("10.png", false),
            ("folder10", true),
            ("2.png", false),
            ("folder2", true),
            ("a.png", false),
            ("A.png", false),
        ]);
        assert_eq!(
            names(&tree),
            ["folder2", "folder10", "2.png", "10.png", "A.png", "a.png"]
        );
    }

    #[test]
    fn loaded_search_temporarily_expands_and_restores_filters_and_focus() {
        let mut tree = initialize(&[("a", true), ("unloaded", true), (".hidden.png", false)]);
        load_branch(&mut tree, "root/a", &[("b", true)]);
        load_branch(
            &mut tree,
            "root/a/b",
            &[("needle.png", false), ("other.png", false)],
        );
        tree.select(Path::new("root/a/b/needle.png"));
        tree.collapse(Path::new("root/a"));
        assert_eq!(tree.focused(), Some(Path::new("root/a")));
        assert!(tree.selected_visible_entry().is_none());
        tree.set_query("NEEDLE".into());
        assert_eq!(names(&tree), ["a", "b", "needle.png"]);
        assert!(tree.selected_visible_entry().is_some());
        assert!(!tree.collapse(Path::new("root/a")));
        assert!(!tree.expand(Path::new("root/unloaded")));
        assert!(tree.take_requests().is_empty());
        tree.set_query(String::new());
        assert_eq!(names(&tree), ["a", "unloaded"]);
        assert!(!tree.node(Path::new("root/a")).unwrap().expanded);
        tree.set_show_hidden(true);
        assert!(names(&tree).contains(&".hidden.png"));
        tree.set_query("no match".into());
        assert!(tree.visible_rows().is_empty());
        assert!(tree.focused().is_none());
        tree.set_query(String::new());
        assert!(tree.focused().is_some());
    }

    #[test]
    fn hidden_ancestors_do_not_leak_search_results_and_reveal_is_explicit() {
        let mut tree = initialize(&[(".secret", true)]);
        load_branch(&mut tree, "root/.secret", &[("needle.png", false)]);
        tree.set_query("needle".into());
        assert!(tree.visible_rows().is_empty());
        assert!(!tree.reveal_loaded(Path::new("root/missing.png")));
        assert!(tree.reveal_loaded(Path::new("root/.secret/needle.png")));
        assert!(tree.show_hidden());
        assert_eq!(tree.query(), "");
        assert_eq!(tree.focused(), Some(Path::new("root/.secret/needle.png")));
        assert!(tree.selected_visible_entry().is_some());
    }

    #[test]
    fn navigation_parent_and_back_restore_native_state_query_and_scroll() {
        let mut tree = initialize(&[("a", true), ("b", true)]);
        load_branch(&mut tree, "root/b", &[("page.png", false)]);
        tree.select(Path::new("root/b/page.png"));
        tree.set_query("page".into());
        tree.set_show_hidden(true);
        let anchor = ScrollAnchor {
            path: Some("root/b/page.png".into()),
            offset: 12.5,
        };
        tree.set_scroll_anchor(anchor.clone());
        tree.request_navigation("root/a".into());
        assert_eq!(tree.location(), Some(Path::new("root")));
        let request = tree.take_requests().pop().unwrap();
        tree.complete(request.token, Ok(listing(Path::new("root/a"), vec![])));
        assert_eq!(tree.root(), Some(Path::new("root")));
        assert_eq!(tree.location(), Some(Path::new("root/a")));
        assert!(tree.node(Path::new("root/b")).unwrap().expanded);
        assert!(tree.back());
        assert_eq!(tree.location(), Some(Path::new("root")));
        assert_eq!(tree.query(), "page");
        assert!(tree.show_hidden());
        assert_eq!(tree.selected(), Some(Path::new("root/b/page.png")));
        assert_eq!(tree.scroll_anchor(), &anchor);
        assert!(!tree.can_back());
        tree.set_query(String::new());
        assert!(tree.request_parent() == tree.can_parent());
        let mut empty = FileTree::new();
        assert!(!empty.request_parent());
        assert!(!empty.back());
    }

    #[test]
    fn invalid_navigation_keeps_last_good_tree_history_and_scroll() {
        let mut tree = initialize(&[("page.png", false)]);
        tree.select(Path::new("root/page.png"));
        tree.set_scroll_anchor(ScrollAnchor {
            path: tree.selected().map(Path::to_path_buf),
            offset: 3.0,
        });
        tree.request_navigation("deleted".into());
        let request = tree.take_requests().pop().unwrap();
        assert!(tree.complete(request.token, Err("deleted or unreadable".into())));
        assert_eq!(tree.error(), Some("deleted or unreadable"));
        assert_eq!(tree.root(), Some(Path::new("root")));
        assert_eq!(
            tree.selected_entry().unwrap().entry_path,
            Path::new("root/page.png")
        );
        assert_eq!(tree.scroll_anchor().offset, 3.0);
        assert!(!tree.can_back());
        assert_eq!(names(&tree), ["page.png"]);
    }

    #[test]
    fn obsolete_workers_keep_both_slots_until_each_finishes() {
        let mut tree = initialize(&[("a", true), ("b", true), ("c", true)]);
        tree.expand(Path::new("root/a"));
        tree.expand(Path::new("root/b"));
        tree.expand(Path::new("root/c"));
        let requests = tree.take_requests();
        assert_eq!(requests.len(), 2);
        assert!(tree.take_requests().is_empty());
        tree.collapse(Path::new("root/a"));
        tree.collapse(Path::new("root/b"));
        assert!(tree.take_requests().is_empty());
        assert!(!tree.complete(requests[0].token, Ok(listing(&requests[0].path, vec![]))));
        let third = tree.take_requests();
        assert_eq!(third.len(), 1);
        assert_eq!(third[0].path, Path::new("root/c"));
        assert!(tree.take_requests().is_empty());
        assert!(!tree.complete(requests[1].token, Err("obsolete".into())));
        assert!(tree.complete(third[0].token, Ok(listing(&third[0].path, vec![]))));
        assert!(tree.node(Path::new("root/c")).unwrap().loaded);
    }

    #[test]
    fn collapse_reexpand_uses_new_node_token_and_ignores_old_result() {
        let mut tree = initialize(&[("a", true)]);
        tree.expand(Path::new("root/a"));
        let old = tree.take_requests().pop().unwrap();
        tree.collapse(Path::new("root/a"));
        tree.expand(Path::new("root/a"));
        let new = tree.take_requests().pop().unwrap();
        assert_ne!(old.token, new.token);
        assert!(tree.complete(
            new.token,
            Ok(listing(&new.path, vec![item(&new.path, "new.png", false)]))
        ));
        assert!(!tree.complete(
            old.token,
            Ok(listing(&old.path, vec![item(&old.path, "old.png", false)]))
        ));
        assert_eq!(names(&tree), ["a", "new.png"]);
        assert!(!tree.complete(new.token, Err("duplicate".into())));
    }

    #[test]
    fn collapse_reexpand_resumes_pending_expanded_descendants() {
        let mut tree = initialize(&[("a", true)]);
        load_branch(&mut tree, "root/a", &[("b", true)]);
        tree.expand(Path::new("root/a/b"));
        let old = tree.take_requests().pop().unwrap();
        tree.collapse(Path::new("root/a"));
        tree.expand(Path::new("root/a"));
        let new = tree.take_requests().pop().unwrap();
        assert_eq!(old.path, new.path);
        assert_ne!(old.token, new.token);
        assert!(!tree.complete(old.token, Err("obsolete".into())));
        assert!(tree.complete(new.token, Ok(listing(&new.path, vec![]))));
    }

    #[test]
    fn review_navigation_resumes_canceled_descendants_with_worker_bound() {
        let mut tree = initialize(&[("a", true), ("unrelated", true)]);
        load_branch(&mut tree, "root/a", &[("b", true), ("collapsed", true)]);
        let ancestor = Path::new("root/a");
        let child = Path::new("root/a/b");
        tree.expand(child);
        let old = tree.take_requests().pop().unwrap();
        tree.collapse(ancestor);
        assert!(tree.node(child).unwrap().expanded);
        assert!(!tree.is_loading(child));
        tree.request_navigation(ancestor.into());
        let navigation = tree.take_requests().pop().unwrap();
        assert!(tree.complete(
            navigation.token,
            Ok(listing(
                ancestor,
                vec![item(ancestor, "b", true), item(ancestor, "collapsed", true)]
            ))
        ));
        assert!(
            tree.is_loading(child),
            "navigation must resume the visible child"
        );
        let resumed = tree.take_requests().pop().unwrap();
        assert_eq!(resumed.path, child);
        assert_ne!(resumed.token, old.token);
        tree.expand(Path::new("root/unrelated"));
        assert!(
            tree.take_requests().is_empty(),
            "obsolete work keeps its slot"
        );
        assert!(!tree.complete(
            old.token,
            Ok(listing(child, vec![item(child, "obsolete.png", false)]))
        ));
        let unrelated = tree.take_requests().pop().unwrap();
        assert_eq!(unrelated.path, Path::new("root/unrelated"));
        assert!(tree.complete(
            resumed.token,
            Ok(listing(child, vec![item(child, "fresh.png", false)]))
        ));
        assert!(tree.complete(unrelated.token, Ok(listing(&unrelated.path, vec![]))));
        assert!(tree.take_requests().is_empty());
        assert!(names(&tree).contains(&"fresh.png"));
        assert!(!names(&tree).contains(&"obsolete.png"));
        assert!(!tree.is_loading(Path::new("root/a/collapsed")));
    }

    #[test]
    fn review_navigation_reroots_hidden_destinations_and_restores_history() {
        for hidden_name in [".hidden", "protected"] {
            for nested in [false, true] {
                let mut tree = initialize(&[(hidden_name, true), ("other.png", false)]);
                let hidden = Path::new("root").join(hidden_name);
                tree.state.nodes.get_mut(&hidden).unwrap().hidden = true;
                load_branch(&mut tree, hidden.to_str().unwrap(), &[("visible", true)]);
                let destination = if nested {
                    hidden.join("visible")
                } else {
                    hidden.clone()
                };
                tree.select(Path::new("root/other.png"));
                let anchor = ScrollAnchor {
                    path: tree.selected().map(Path::to_path_buf),
                    offset: 4.0,
                };
                tree.set_scroll_anchor(anchor.clone());
                tree.request_navigation(destination.clone());
                let failed = tree.take_requests().pop().unwrap();
                assert!(tree.complete(failed.token, Err("unreadable".into())));
                assert_eq!(tree.root(), Some(Path::new("root")));
                assert!(!tree.can_back());
                assert_eq!(tree.selected(), Some(Path::new("root/other.png")));
                tree.request_navigation(destination.clone());
                let request = tree.take_requests().pop().unwrap();
                assert!(tree.complete(
                    request.token,
                    Ok(listing(
                        &destination,
                        vec![
                            item(&destination, "page.png", false),
                            item(&destination, ".private.png", false),
                        ]
                    ))
                ));
                assert_eq!(tree.root(), Some(destination.as_path()));
                assert_eq!(tree.location(), Some(destination.as_path()));
                assert!(
                    !tree.show_hidden(),
                    "explicit paths must not change the hidden toggle"
                );
                assert_eq!(names(&tree), ["page.png"]);
                assert!(tree.back());
                assert_eq!(tree.root(), Some(Path::new("root")));
                assert_eq!(tree.location(), Some(Path::new("root")));
                assert_eq!(tree.selected(), Some(Path::new("root/other.png")));
                assert_eq!(tree.scroll_anchor(), &anchor);
                assert!(!tree.show_hidden());
                assert_eq!(names(&tree), ["other.png"]);
            }
        }
    }

    #[test]
    fn review_navigation_with_hidden_items_visible_keeps_the_tree_root() {
        let mut tree = initialize(&[(".hidden", true), ("other", true)]);
        tree.set_show_hidden(true);
        load_branch(&mut tree, "root/other", &[("retained.png", false)]);
        let hidden = Path::new("root/.hidden");
        tree.request_navigation(hidden.into());
        let request = tree.take_requests().pop().unwrap();
        assert!(tree.complete(
            request.token,
            Ok(listing(hidden, vec![item(hidden, "page.png", false)]))
        ));
        assert_eq!(tree.root(), Some(Path::new("root")));
        assert_eq!(tree.location(), Some(hidden));
        assert!(tree.show_hidden());
        assert!(names(&tree).contains(&"page.png"));
        assert!(names(&tree).contains(&"retained.png"));
        assert!(tree.node(Path::new("root/other")).unwrap().expanded);
    }

    #[test]
    fn duplicate_expansion_navigation_and_completion_are_idempotent() {
        let mut tree = initialize(&[("a", true)]);
        for _ in 0..3 {
            tree.expand(Path::new("root/a"));
        }
        let requests = tree.take_requests();
        assert_eq!(requests.len(), 1);
        assert!(tree.complete(requests[0].token, Ok(listing(&requests[0].path, vec![]))));
        tree.expand(Path::new("root/a"));
        assert!(tree.take_requests().is_empty());
        tree.request_navigation("another".into());
        tree.request_navigation("another".into());
        assert_eq!(tree.take_requests().len(), 1);
    }

    #[test]
    fn latest_navigation_wins_out_of_order_and_root_generation_rejects_branches() {
        let mut tree = initialize(&[("a", true)]);
        tree.expand(Path::new("root/a"));
        tree.request_navigation("new-root".into());
        let requests = tree.take_requests();
        let branch = &requests[0];
        let navigation = &requests[1];
        assert!(tree.complete(navigation.token, Ok(listing(&navigation.path, vec![]))));
        assert!(!tree.complete(branch.token, Ok(listing(&branch.path, vec![]))));
        assert_eq!(tree.root(), Some(Path::new("new-root")));
        tree.request_navigation("older".into());
        let older = tree.take_requests().pop().unwrap();
        tree.request_navigation("newer".into());
        let newer = tree.take_requests().pop().unwrap();
        assert!(tree.complete(newer.token, Err("newer failed".into())));
        assert!(!tree.complete(older.token, Ok(listing(&older.path, vec![]))));
        assert_eq!(tree.root(), Some(Path::new("new-root")));
        assert_eq!(tree.error(), Some("newer failed"));
    }

    #[test]
    fn close_invalidates_work_without_releasing_running_slots() {
        let mut tree = initialize(&[("a", true), ("b", true)]);
        tree.expand(Path::new("root/a"));
        tree.expand(Path::new("root/b"));
        let requests = tree.take_requests();
        tree.close();
        assert_eq!(tree.root(), Some(Path::new("root")));
        assert_eq!(names(&tree), ["a", "b"]);
        tree.request_navigation("next".into());
        assert!(tree.take_requests().is_empty());
        assert!(!tree.complete(requests[0].token, Err("closed".into())));
        let next = tree.take_requests().pop().unwrap();
        assert_eq!(next.path, Path::new("next"));
        assert!(tree.complete(next.token, Ok(listing(&next.path, vec![]))));
        assert!(!tree.complete(requests[1].token, Err("closed".into())));
        assert_eq!(tree.root(), Some(Path::new("next")));
        assert!(tree.can_back());
        assert!(tree.back());
        assert_eq!(tree.root(), Some(Path::new("root")));
    }

    #[test]
    fn back_resumes_inflight_expansions_without_releasing_old_worker_slots() {
        let mut tree = initialize(&[("a", true), ("b", true), ("c", true)]);
        tree.expand(Path::new("root/a"));
        let old = tree.take_requests().pop().unwrap();
        tree.request_navigation("other".into());
        let navigation = tree.take_requests().pop().unwrap();
        tree.complete(navigation.token, Ok(listing(&navigation.path, vec![])));
        assert!(tree.back());
        assert!(tree.is_loading(Path::new("root/a")));
        let resumed = tree.take_requests().pop().unwrap();
        assert_eq!(resumed.path, old.path);
        assert_ne!(resumed.token, old.token);
        tree.expand(Path::new("root/b"));
        tree.expand(Path::new("root/c"));
        assert!(tree.take_requests().is_empty());
        assert!(!tree.complete(old.token, Err("obsolete".into())));
        let b = tree.take_requests().pop().unwrap();
        assert_eq!(b.path, Path::new("root/b"));
        assert!(tree.complete(resumed.token, Ok(listing(&resumed.path, vec![]))));
        let c = tree.take_requests().pop().unwrap();
        assert_eq!(c.path, Path::new("root/c"));
        assert!(tree.complete(b.token, Ok(listing(&b.path, vec![]))));
        assert!(tree.complete(c.token, Ok(listing(&c.path, vec![]))));
        assert!(tree.take_requests().is_empty());
    }

    #[test]
    fn back_defers_hidden_and_collapsed_descendants_until_explicitly_revealed() {
        let mut tree = initialize(&[("a", true), (".hidden", true)]);
        load_branch(&mut tree, "root/a", &[("deep", true)]);
        tree.expand(Path::new("root/a/deep"));
        tree.collapse(Path::new("root/a"));
        tree.set_show_hidden(true);
        tree.expand(Path::new("root/.hidden"));
        tree.set_show_hidden(false);
        tree.request_navigation("other".into());
        // The hidden read and navigation run; the collapsed read was discarded.
        let requests = tree.take_requests();
        let hidden = requests
            .iter()
            .find(|r| r.path == Path::new("root/.hidden"))
            .unwrap();
        let navigation = requests
            .iter()
            .find(|r| r.path == Path::new("other"))
            .unwrap();
        tree.complete(navigation.token, Ok(listing(&navigation.path, vec![])));
        assert!(tree.back());
        assert!(tree.take_requests().is_empty());
        assert!(!tree.is_loading(Path::new("root/a/deep")));
        assert!(!tree.is_loading(Path::new("root/.hidden")));
        tree.set_show_hidden(true);
        let revealed = tree.take_requests().pop().unwrap();
        assert_eq!(revealed.path, hidden.path);
        tree.expand(Path::new("root/a"));
        assert!(tree.take_requests().is_empty());
        assert!(!tree.complete(hidden.token, Err("obsolete".into())));
        let deep = tree.take_requests().pop().unwrap();
        assert_eq!(deep.path, Path::new("root/a/deep"));
        assert!(tree.complete(deep.token, Ok(listing(&deep.path, vec![]))));
        assert!(tree.complete(revealed.token, Ok(listing(&revealed.path, vec![]))));
    }

    #[test]
    fn close_keeps_loaded_state_and_history_but_collapses_canceled_unloaded_nodes() {
        let mut tree = initialize(&[("a", true)]);
        load_branch(&mut tree, "root/a", &[("deep", true), ("page.png", false)]);
        tree.select(Path::new("root/a/page.png"));
        tree.request_navigation("root/a".into());
        let navigation = tree.take_requests().pop().unwrap();
        tree.complete(
            navigation.token,
            Ok(listing(
                &navigation.path,
                vec![
                    item(&navigation.path, "deep", true),
                    item(&navigation.path, "page.png", false),
                ],
            )),
        );
        tree.select(Path::new("root/a/page.png"));
        tree.expand(Path::new("root/a/deep"));
        let old = tree.take_requests().pop().unwrap();
        tree.close();
        assert!(tree.can_back());
        assert!(tree.node(Path::new("root/a")).unwrap().expanded);
        assert!(tree.node(Path::new("root/a")).unwrap().loaded);
        assert!(!tree.node(&old.path).unwrap().expanded);
        assert!(!tree.is_loading(&old.path));
        assert_eq!(tree.selected(), Some(Path::new("root/a/page.png")));
        assert!(tree.take_requests().is_empty());
        assert!(tree.expand(&old.path));
        let fresh = tree.take_requests().pop().unwrap();
        assert!(!tree.complete(old.token, Ok(listing(&old.path, vec![]))));
        assert!(tree.complete(fresh.token, Ok(listing(&fresh.path, vec![]))));
    }

    #[test]
    fn cancel_navigation_and_locate_retain_slots_and_do_not_install_late_results() {
        let mut tree = initialize(&[("a", true), ("b", true)]);
        tree.expand(Path::new("root/a"));
        let expansion = tree.take_requests().pop().unwrap();
        tree.locate("root/a/book.png".into());
        let locate = tree.take_requests().pop().unwrap();
        tree.cancel_locate();
        assert!(tree.node(Path::new("root/a")).unwrap().expanded);
        assert!(tree.is_loading(Path::new("root/a")));
        tree.request_navigation("canceled".into());
        tree.cancel_navigation();
        tree.expand(Path::new("root/b"));
        assert!(tree.take_requests().is_empty());
        assert!(!tree.complete(expansion.token, Err("obsolete".into())));
        let resumed = tree.take_requests().pop().unwrap();
        assert_eq!(resumed.path, Path::new("root/a"));
        assert!(!tree.complete(
            locate.token,
            Ok(listing(
                &locate.path,
                vec![item(&locate.path, "book.png", false)]
            ))
        ));
        let b = tree.take_requests().pop().unwrap();
        assert_eq!(b.path, Path::new("root/b"));
        assert!(tree.complete(resumed.token, Ok(listing(&resumed.path, vec![]))));
        assert!(tree.complete(b.token, Ok(listing(&b.path, vec![]))));
        assert_eq!(tree.location(), Some(Path::new("root")));
        assert!(tree.node(Path::new("root/a/book.png")).is_none());
        assert!(!tree.can_back());
        assert!(tree.take_requests().is_empty());
    }

    #[test]
    fn failed_locate_resumes_canceled_expansion_without_reading_hidden_or_collapsed_nodes() {
        for staged_failure in [false, true] {
            let mut tree = initialize(&[("a", true), (".hidden", true), ("collapsed", true)]);
            load_branch(&mut tree, "root/collapsed", &[("deep", true)]);
            tree.expand(Path::new("root/collapsed/deep"));
            tree.collapse(Path::new("root/collapsed"));
            tree.expand(Path::new("root/a"));
            let old = tree.take_requests().pop().unwrap();
            tree.locate("root/a/deep/book.png".into());
            let a = tree.take_requests().pop().unwrap();
            if staged_failure {
                tree.complete(
                    a.token,
                    Ok(listing(&a.path, vec![item(&a.path, "deep", true)])),
                );
                let deep = tree.take_requests().pop().unwrap();
                // A staged-only owner changes to an ancestor while the accepted
                // a expansion still needs to be resumed after Locate fails.
                let mut drift = listing(&deep.path, vec![item(&deep.path, "book.png", false)]);
                drift.target = "root".into();
                tree.complete(deep.token, Ok(drift));
            } else {
                tree.complete(a.token, Err("permission denied".into()));
            }
            assert!(tree.error().is_some());
            assert!(tree.is_loading(Path::new("root/a")));
            let resumed = tree.take_requests().pop().unwrap();
            assert_eq!(resumed.path, old.path);
            assert_ne!(resumed.token, old.token);
            assert!(!tree.is_loading(Path::new("root/.hidden")));
            assert!(!tree.is_loading(Path::new("root/collapsed/deep")));
            assert!(tree.take_requests().is_empty());
            assert!(!tree.complete(old.token, Err("obsolete".into())));
            assert!(tree.complete(
                resumed.token,
                Ok(listing(
                    &resumed.path,
                    vec![item(&resumed.path, "accepted.png", false)]
                ))
            ));
            assert!(tree.node(Path::new("root/a/accepted.png")).is_some());
            assert!(tree.node(Path::new("root/a/deep")).is_none());
        }
    }

    #[test]
    fn owner_target_drift_clears_descendants_and_pending_work_but_not_other_branches() {
        for target in [Path::new("root"), Path::new("new-target")] {
            let mut tree = initialize(&[("a", true), ("b", true)]);
            load_branch(
                &mut tree,
                "root/a",
                &[("deep", true), ("queued", true), ("old.png", false)],
            );
            load_branch(&mut tree, "root/b", &[("keep.png", false)]);
            tree.select(Path::new("root/a/old.png"));
            tree.retry(Path::new("root/a"));
            tree.expand(Path::new("root/a/deep"));
            tree.expand(Path::new("root/a/queued"));
            let requests = tree.take_requests();
            let owner = &requests[0];
            let child = &requests[1];
            let mut drift = listing(&owner.path, vec![item(&owner.path, "injected.png", false)]);
            drift.target = target.into();
            assert!(tree.complete(owner.token, Ok(drift)));
            assert_eq!(
                tree.node(&owner.path).unwrap().entry.path,
                Path::new("root/a")
            );
            assert!(!tree.node(&owner.path).unwrap().loaded);
            assert!(
                tree.node_error(&owner.path)
                    .unwrap()
                    .contains("target changed")
            );
            assert!(
                tree.visible_rows()
                    .iter()
                    .any(|row| row.path == owner.path && row.kind == RowKind::Error)
            );
            for name in ["deep", "queued", "old.png", "injected.png"] {
                assert!(tree.node(&owner.path.join(name)).is_none());
            }
            assert!(tree.selected_entry().is_none());
            assert!(tree.node(Path::new("root/b/keep.png")).is_some());
            assert!(tree.node(Path::new("root/b")).unwrap().expanded);
            assert!(tree.take_requests().is_empty());
            // A retry can recover if the original target is restored.
            assert!(tree.retry(&owner.path));
            let retry = tree.take_requests().pop().unwrap();
            assert!(tree.complete(retry.token, Ok(listing(&retry.path, vec![]))));
            assert!(tree.node_error(&retry.path).is_none());
            assert!(!tree.complete(child.token, Ok(listing(&child.path, vec![]))));
            // Refreshing the parent accepts a new identity and recomputes cycles.
            assert!(tree.retry(Path::new("root")));
            let parent = tree.take_requests().pop().unwrap();
            let mut a = item(&parent.path, "a", true);
            a.entry.path = target.into();
            tree.complete(
                parent.token,
                Ok(listing(
                    &parent.path,
                    vec![a, item(&parent.path, "b", true)],
                )),
            );
            assert_eq!(
                tree.node(&owner.path).unwrap().cycle,
                target == Path::new("root")
            );
            if target == Path::new("root") {
                assert!(!tree.expand(&owner.path));
            } else {
                assert!(tree.expand(&owner.path));
                let request = tree.take_requests().pop().unwrap();
                let mut loaded =
                    listing(&request.path, vec![item(&request.path, "new.png", false)]);
                loaded.target = target.into();
                assert!(tree.complete(request.token, Ok(loaded)));
                assert!(tree.node(&owner.path.join("new.png")).is_some());
            }
            assert!(tree.node(Path::new("root/b/keep.png")).is_some());
        }
    }

    #[test]
    fn unloaded_expansion_rejects_retarget_and_navigation_can_establish_new_root() {
        let mut tree = initialize(&[("a", true)]);
        tree.expand(Path::new("root/a"));
        let request = tree.take_requests().pop().unwrap();
        let mut drift = listing(
            &request.path,
            vec![item(&request.path, "injected.png", false)],
        );
        drift.target = "root".into();
        assert!(tree.complete(request.token, Ok(drift.clone())));
        assert!(tree.node(&request.path.join("injected.png")).is_none());
        assert!(tree.node_error(&request.path).is_some());
        tree.request_navigation(request.path.clone());
        let navigation = tree.take_requests().pop().unwrap();
        assert!(tree.complete(navigation.token, Ok(drift)));
        assert_eq!(tree.root(), Some(request.path.as_path()));
        assert_eq!(
            tree.node(&request.path).unwrap().entry.path,
            Path::new("root")
        );
        assert!(tree.node(&request.path.join("injected.png")).is_some());
    }

    #[test]
    fn locate_rejects_known_owner_target_drift_without_installing_contents() {
        for target in [Path::new("root"), Path::new("elsewhere")] {
            let mut tree = initialize(&[("a", true), ("b", true)]);
            load_branch(&mut tree, "root/a", &[("old.png", false)]);
            load_branch(&mut tree, "root/b", &[("keep.png", false)]);
            tree.locate("root/a/book.png".into());
            let request = tree.take_requests().pop().unwrap();
            let mut loaded = listing(&request.path, vec![item(&request.path, "book.png", false)]);
            loaded.target = target.into();
            assert!(tree.complete(request.token, Ok(loaded)));
            assert!(tree.error().unwrap().contains("target changed"));
            assert!(tree.node_error(&request.path).is_some());
            assert!(tree.node(&request.path.join("old.png")).is_none());
            assert!(tree.node(&request.path.join("book.png")).is_none());
            assert!(tree.node(Path::new("root/b/keep.png")).is_some());
            assert!(!tree.can_back());
        }
    }

    #[test]
    fn locate_chain_captures_owner_identity_from_staged_state() {
        let mut tree = initialize(&[("a", true)]);
        tree.locate("root/a/deep/book.png".into());
        let a = tree.take_requests().pop().unwrap();
        let mut deep = item(&a.path, "deep", true);
        deep.entry.path = "expected-target".into();
        tree.complete(a.token, Ok(listing(&a.path, vec![deep])));
        let deep = tree.take_requests().pop().unwrap();
        let mut loaded = listing(&deep.path, vec![item(&deep.path, "book.png", false)]);
        loaded.target = "root".into();
        assert!(tree.complete(deep.token, Ok(loaded)));
        assert!(tree.error().unwrap().contains("target changed"));
        assert!(tree.node(Path::new("root/a/deep")).is_none());
        assert!(tree.take_requests().is_empty());
        assert!(!tree.can_back());
    }

    #[test]
    fn staged_locate_does_not_overwrite_owner_replaced_by_running_parent_read() {
        for stage_root in [false, true] {
            let mut tree = initialize(&[("a", true), ("b", true)]);
            load_branch(&mut tree, "root/b", &[("keep.png", false)]);
            let owner_name = if stage_root { "missing" } else { "a" };
            let owner_path = Path::new("root").join(owner_name);
            tree.locate(owner_path.join("deep/book.png"));
            // Include a staged root to check replay cannot hide identity drift.
            if stage_root {
                let root = tree.take_requests().pop().unwrap();
                tree.complete(
                    root.token,
                    Ok(listing(
                        &root.path,
                        vec![
                            item(&root.path, "a", true),
                            item(&root.path, "b", true),
                            item(&root.path, owner_name, true),
                        ],
                    )),
                );
            }
            let a = tree.take_requests().pop().unwrap();
            tree.complete(
                a.token,
                Ok(listing(&a.path, vec![item(&a.path, "deep", true)])),
            );
            let deep = tree.take_requests().pop().unwrap();
            // Schedule a concurrent model refresh without canceling Locate intent.
            tree.enqueue_expansion(Path::new("root"));
            let parent = tree.take_requests().pop().unwrap();
            let mut replacement = item(&parent.path, owner_name, true);
            replacement.entry.path = "new-owner-target".into();
            assert!(tree.complete(
                parent.token,
                Ok(listing(
                    &parent.path,
                    vec![replacement, item(&parent.path, "b", true)]
                ))
            ));
            assert!(tree.complete(
                deep.token,
                Ok(listing(
                    &deep.path,
                    vec![item(&deep.path, "book.png", false)]
                ))
            ));
            assert!(tree.error().unwrap().contains("target changed"));
            assert_eq!(
                tree.node(&owner_path).unwrap().entry.path,
                Path::new("new-owner-target")
            );
            assert!(tree.node(&owner_path.join("deep")).is_none());
            assert!(tree.node(Path::new("root/b/keep.png")).is_some());
            assert!(!tree.can_back());
            assert!(tree.take_requests().is_empty());
            tree.locate(owner_path.join("book.png"));
            let retry = tree.take_requests().pop().unwrap();
            let mut loaded = listing(&retry.path, vec![item(&retry.path, "book.png", false)]);
            loaded.target = "new-owner-target".into();
            assert!(tree.complete(retry.token, Ok(loaded)));
            assert_eq!(tree.selected(), Some(owner_path.join("book.png").as_path()));
        }
    }

    #[test]
    fn failed_refresh_keeps_good_children_and_retry_accepts_deletion() {
        let mut tree = initialize(&[("a", true)]);
        load_branch(&mut tree, "root/a", &[("page.png", false)]);
        tree.select(Path::new("root/a/page.png"));
        assert!(tree.retry(Path::new("root/a")));
        let request = tree.take_requests().pop().unwrap();
        assert!(tree.complete(request.token, Err("permission denied".into())));
        assert_eq!(names(&tree), ["a", "page.png"]);
        assert_eq!(
            tree.node(Path::new("root/a")).unwrap().error.as_deref(),
            Some("permission denied")
        );
        assert!(
            tree.visible_rows()
                .iter()
                .any(|row| row.kind == RowKind::Error)
        );
        assert!(tree.retry(Path::new("root/a")));
        let request = tree.take_requests().pop().unwrap();
        assert!(tree.complete(request.token, Ok(listing(&request.path, vec![]))));
        assert!(tree.selected_entry().is_none());
        assert_eq!(names(&tree), ["a"]);
        assert_eq!(tree.focused(), Some(Path::new("root/a")));
    }

    #[test]
    fn aliases_remain_distinct_and_cycles_are_ancestor_local() {
        let root = Path::new("root");
        let mut tree = initialize(&[]);
        let mut a = item(root, "alias1", true);
        let mut b = item(root, "alias2", true);
        a.entry.path = "shared-target".into();
        b.entry.path = a.entry.path.clone();
        tree.retry(root);
        let request = tree.take_requests().pop().unwrap();
        tree.complete(request.token, Ok(listing(root, vec![a, b])));
        for alias in ["root/alias1", "root/alias2"] {
            tree.expand(Path::new(alias));
            let request = tree.take_requests().pop().unwrap();
            let mut back = item(&request.path, "back", true);
            back.entry.path = root.into();
            let mut nested = item(&request.path, "self", true);
            nested.entry.path = "shared-target".into();
            tree.complete(
                request.token,
                Ok(LoadedDirectory {
                    requested_path: request.path.clone(),
                    path: request.path.clone(),
                    target: "shared-target".into(),
                    entries: vec![back, nested, item(&request.path, "page.png", false)],
                }),
            );
            assert!(!tree.node(Path::new(alias)).unwrap().cycle);
            assert!(tree.node(&Path::new(alias).join("back")).unwrap().cycle);
            assert!(!tree.expand(&Path::new(alias).join("back")));
            assert!(tree.node(&Path::new(alias).join("self")).unwrap().cycle);
        }
        assert_eq!(
            tree.visible_rows()
                .iter()
                .filter(|row| row
                    .entry
                    .as_ref()
                    .is_some_and(|entry| entry.name == "page.png"))
                .count(),
            2
        );
        assert!(tree.node(Path::new("root/alias1/page.png")).is_some());
        assert!(tree.node(Path::new("root/alias2/page.png")).is_some());
    }

    #[test]
    fn duplicate_native_entries_are_removed_but_same_display_names_are_not() {
        let root = Path::new("root");
        let mut tree = initialize(&[]);
        let first = item(root, "first.png", false);
        let mut second = item(root, "second.png", false);
        second.entry.name = first.entry.name.clone();
        tree.retry(root);
        let request = tree.take_requests().pop().unwrap();
        tree.complete(
            request.token,
            Ok(listing(root, vec![first.clone(), first, second])),
        );
        assert_eq!(names(&tree), ["first.png", "first.png"]);
        assert_ne!(tree.visible_rows()[0].path, tree.visible_rows()[1].path);
    }

    #[test]
    fn mismatched_result_path_is_a_failure_not_a_root_change() {
        let mut tree = initialize(&[("page.png", false)]);
        tree.request_navigation("requested".into());
        let request = tree.take_requests().pop().unwrap();
        assert!(tree.complete(request.token, Ok(listing(Path::new("wrong"), vec![]))));
        assert_eq!(tree.root(), Some(Path::new("root")));
        assert!(tree.error().unwrap().contains("native path"));
    }

    #[test]
    fn locate_stages_only_the_ancestor_chain_and_reveals_hidden_source_on_success() {
        let mut tree = initialize(&[("a", true), ("b", true)]);
        tree.expand(Path::new("root/b"));
        let unrelated = tree.take_requests().pop().unwrap();
        tree.set_query("previous filter".into());
        let old_scroll = ScrollAnchor {
            path: Some("root/b".into()),
            offset: 8.0,
        };
        tree.set_scroll_anchor(old_scroll.clone());
        tree.locate("root/a/deep/.book.png".into());
        assert_eq!(tree.location(), Some(Path::new("root")));
        assert_eq!(tree.query(), "previous filter");
        assert!(tree.is_loading(Path::new("root/a")));
        let a = tree.take_requests().pop().unwrap();
        assert_eq!(a.path, Path::new("root/a"));
        assert!(tree.complete(
            a.token,
            Ok(listing(&a.path, vec![item(&a.path, "deep", true)]))
        ));
        assert!(tree.node(Path::new("root/a/deep")).is_none());
        let deep = tree.take_requests().pop().unwrap();
        assert_eq!(deep.path, Path::new("root/a/deep"));
        assert!(tree.complete(
            unrelated.token,
            Ok(listing(
                &unrelated.path,
                vec![item(&unrelated.path, "unrelated.png", false)]
            ))
        ));
        assert!(tree.complete(
            deep.token,
            Ok(listing(
                &deep.path,
                vec![item(&deep.path, ".book.png", false)]
            ))
        ));
        assert_eq!(tree.root(), Some(Path::new("root")));
        assert_eq!(tree.location(), Some(Path::new("root/a/deep")));
        assert_eq!(tree.selected(), Some(Path::new("root/a/deep/.book.png")));
        assert_eq!(tree.focused(), tree.selected());
        assert!(tree.selected_visible_entry().is_some());
        assert!(tree.show_hidden());
        assert!(tree.query().is_empty());
        assert!(tree.node(Path::new("root/b")).unwrap().expanded);
        assert!(tree.node(Path::new("root/b/unrelated.png")).is_some());
        assert!(tree.take_requests().is_empty());
        assert!(tree.back());
        assert_eq!(tree.location(), Some(Path::new("root")));
        assert_eq!(tree.query(), "previous filter");
        assert_eq!(tree.scroll_anchor(), &old_scroll);
    }

    #[test]
    fn locate_failure_discards_staged_results_and_keeps_the_last_good_snapshot() {
        let mut tree = initialize(&[("a", true), ("page.png", false)]);
        tree.select(Path::new("root/page.png"));
        tree.set_query("page".into());
        tree.locate("root/a/deleted/book.png".into());
        let a = tree.take_requests().pop().unwrap();
        tree.complete(
            a.token,
            Ok(listing(&a.path, vec![item(&a.path, "deleted", true)])),
        );
        let deleted = tree.take_requests().pop().unwrap();
        assert!(tree.complete(deleted.token, Err("folder was deleted".into())));
        assert_eq!(tree.error(), Some("folder was deleted"));
        assert_eq!(tree.location(), Some(Path::new("root")));
        assert_eq!(tree.query(), "page");
        assert_eq!(tree.selected(), Some(Path::new("root/page.png")));
        assert!(!tree.node(Path::new("root/a")).unwrap().loaded);
        assert!(tree.node(Path::new("root/a/deleted")).is_none());
        assert!(!tree.can_back());
    }

    #[test]
    fn locate_loaded_source_is_immediate_and_refreshes_only_missing_parent_entries() {
        let mut tree = initialize(&[("a", true), ("unloaded", true)]);
        load_branch(&mut tree, "root/a", &[("old.png", false)]);
        tree.collapse(Path::new("root/a"));
        tree.locate("root/a/old.png".into());
        assert!(tree.take_requests().is_empty());
        assert_eq!(tree.focused(), Some(Path::new("root/a/old.png")));
        assert!(!tree.show_hidden());
        tree.locate("root/a/new.png".into());
        let request = tree.take_requests().pop().unwrap();
        assert_eq!(request.path, Path::new("root/a"));
        tree.complete(
            request.token,
            Ok(listing(
                &request.path,
                vec![item(&request.path, "new.png", false)],
            )),
        );
        assert_eq!(tree.selected(), Some(Path::new("root/a/new.png")));
        assert!(tree.take_requests().is_empty());
        tree.locate("root/a/missing.png".into());
        let missing = tree.take_requests().pop().unwrap();
        tree.complete(missing.token, Ok(listing(&missing.path, vec![])));
        assert!(tree.error().unwrap().contains("no longer available"));
        assert!(tree.take_requests().is_empty());
        assert_eq!(tree.selected(), Some(Path::new("root/a/new.png")));
    }

    #[test]
    fn locate_outside_root_and_root_folder_books_focus_the_original_entry() {
        let mut tree = initialize(&[("previous.png", false)]);
        tree.locate("outside/folder".into());
        let request = tree.take_requests().pop().unwrap();
        assert_eq!(request.path, Path::new("outside"));
        tree.complete(
            request.token,
            Ok(listing(
                &request.path,
                vec![item(&request.path, "folder", true)],
            )),
        );
        assert_eq!(tree.root(), Some(Path::new("outside")));
        assert_eq!(
            tree.selected_visible_entry().unwrap().kind,
            EntryKind::Folder
        );
        assert_eq!(tree.focused(), Some(Path::new("outside/folder")));
        tree.request_navigation("outside/folder".into());
        let request = tree.take_requests().pop().unwrap();
        tree.complete(request.token, Ok(listing(&request.path, vec![])));
        // Establish the folder itself as the root, then locate its original entry.
        tree.request_navigation("elsewhere/current".into());
        let request = tree.take_requests().pop().unwrap();
        tree.complete(request.token, Ok(listing(&request.path, vec![])));
        tree.locate("elsewhere/current".into());
        let request = tree.take_requests().pop().unwrap();
        assert_eq!(request.path, Path::new("elsewhere"));
        tree.complete(
            request.token,
            Ok(listing(
                &request.path,
                vec![item(&request.path, "current", true)],
            )),
        );
        assert_eq!(tree.root(), Some(Path::new("elsewhere")));
        assert_eq!(tree.focused(), Some(Path::new("elsewhere/current")));
        assert!(tree.selected_visible_entry().is_some());
    }

    #[test]
    fn locate_filesystem_root_uses_stable_location_focus() {
        #[cfg(windows)]
        let root = Path::new(r"C:\");
        #[cfg(not(windows))]
        let root = Path::new("/");
        let mut tree = FileTree::new();
        tree.locate(root.into());
        let request = tree.take_requests().pop().unwrap();
        tree.complete(request.token, Ok(listing(root, vec![])));
        assert_eq!(tree.root(), Some(root));
        assert_eq!(tree.location(), Some(root));
        assert_eq!(tree.selected(), Some(root));
        assert_eq!(tree.focused(), Some(root));
        assert!(tree.take_requests().is_empty());
    }

    #[test]
    fn latest_locate_wins_and_obsolete_locate_reads_retain_slots() {
        let mut tree = initialize(&[("a", true), ("b", true)]);
        tree.locate("root/a/old.png".into());
        let old = tree.take_requests().pop().unwrap();
        tree.locate("root/b/new.png".into());
        let new = tree.take_requests().pop().unwrap();
        assert!(tree.take_requests().is_empty());
        assert!(tree.complete(
            new.token,
            Ok(listing(&new.path, vec![item(&new.path, "new.png", false)]))
        ));
        assert!(!tree.complete(
            old.token,
            Ok(listing(&old.path, vec![item(&old.path, "old.png", false)]))
        ));
        assert_eq!(tree.selected(), Some(Path::new("root/b/new.png")));
        assert!(!tree.node(Path::new("root/a")).unwrap().loaded);
        assert_eq!(tree.location(), Some(Path::new("root/b")));
    }

    #[test]
    fn collapse_close_and_navigation_cancel_pending_locate_without_late_reopening() {
        let mut tree = initialize(&[("a", true), ("page.png", false)]);
        tree.select(Path::new("root/page.png"));
        tree.locate("root/a/book.png".into());
        let collapsed = tree.take_requests().pop().unwrap();
        tree.collapse(Path::new("root/a"));
        assert!(!tree.complete(
            collapsed.token,
            Ok(listing(
                &collapsed.path,
                vec![item(&collapsed.path, "book.png", false)]
            ))
        ));
        assert!(!tree.node(Path::new("root/a")).unwrap().expanded);
        tree.locate("root/a/book.png".into());
        let closed = tree.take_requests().pop().unwrap();
        tree.close();
        assert_eq!(tree.selected(), Some(Path::new("root/page.png")));
        assert!(!tree.complete(closed.token, Ok(listing(&closed.path, vec![]))));
        tree.locate("root/a/book.png".into());
        let superseded = tree.take_requests().pop().unwrap();
        tree.request_navigation("new-root".into());
        let navigation = tree.take_requests().pop().unwrap();
        tree.complete(navigation.token, Ok(listing(&navigation.path, vec![])));
        assert!(!tree.complete(superseded.token, Ok(listing(&superseded.path, vec![]))));
        assert_eq!(tree.root(), Some(Path::new("new-root")));
    }

    #[test]
    fn locate_rejects_an_ancestor_cycle_without_loading_the_cycle() {
        let mut tree = initialize(&[("a", true)]);
        tree.locate("root/a/back/book.png".into());
        let request = tree.take_requests().pop().unwrap();
        let mut back = item(&request.path, "back", true);
        back.entry.path = "root".into();
        tree.complete(request.token, Ok(listing(&request.path, vec![back])));
        assert!(tree.error().unwrap().contains("cycle"));
        assert!(tree.take_requests().is_empty());
        assert_eq!(tree.location(), Some(Path::new("root")));
        assert!(tree.node(Path::new("root/a/back")).is_none());
    }

    #[test]
    fn children_and_node_status_are_independent_of_visible_filters() {
        let mut tree = initialize(&[("a", true), (".hidden.png", false)]);
        tree.set_query("no result".into());
        let children: Vec<_> = tree.children(Path::new("root")).collect();
        assert_eq!(children.len(), 2);
        assert!(children.iter().any(|node| node.hidden));
        assert!(tree.visible_rows().is_empty());
        tree.set_query(String::new());
        tree.expand(Path::new("root/a"));
        assert!(tree.is_loading(Path::new("root/a")));
        let request = tree.take_requests().pop().unwrap();
        tree.complete(request.token, Err("unreadable".into()));
        assert!(!tree.is_loading(Path::new("root/a")));
        assert_eq!(tree.node_error(Path::new("root/a")), Some("unreadable"));
    }

    #[test]
    fn relative_dot_accepts_absolute_location_and_requests_its_real_parent() {
        let cwd = std::env::current_dir().unwrap();
        let mut tree = FileTree::new();
        tree.request_navigation(".".into());
        let request = tree.take_requests().pop().unwrap();
        // Do not enumerate the shared process CWD while parallel relative fixtures
        // create and remove children there. The next test covers the real loader.
        let target = fs::canonicalize(&request.path).unwrap();
        let loaded = LoadedDirectory {
            requested_path: request.path.clone(),
            path: normalize_entry_path(&request.path, &target).unwrap(),
            target,
            entries: Vec::new(),
        };
        assert_eq!(loaded.requested_path, Path::new("."));
        assert_eq!(loaded.path, cwd);
        assert!(tree.is_navigation_request(request.token));
        assert!(tree.complete(request.token, Ok(loaded)));
        assert!(!tree.is_navigation_request(request.token));
        assert_eq!(tree.location(), Some(cwd.as_path()));
        assert!(tree.can_parent());
        assert!(tree.request_parent());
        assert_eq!(tree.pending_navigation(), cwd.parent());
        assert_eq!(std::env::current_dir().unwrap(), cwd);
    }

    #[test]
    fn relative_temporary_directory_rebases_children_under_absolute_location() {
        let temp = tempfile::tempdir_in(".").unwrap();
        fs::write(temp.path().join("page.png"), b"image").unwrap();
        let name = temp.path().file_name().unwrap();
        let requested = Path::new(".").join(name).join(".");
        assert!(requested.is_relative());
        let expected = std::env::current_dir().unwrap().join(name);
        let loaded = load_directory(&requested).unwrap();
        assert_eq!(loaded.requested_path, requested);
        assert_eq!(loaded.path, expected);
        assert_eq!(
            loaded.entries[0].entry.entry_path,
            expected.join("page.png")
        );
        assert_eq!(loaded.target, fs::canonicalize(temp.path()).unwrap());
        let mut tree = FileTree::new();
        tree.request_navigation(requested);
        let request = tree.take_requests().pop().unwrap();
        assert!(tree.complete(request.token, Ok(loaded)));
        assert!(tree.request_parent());
        assert_eq!(tree.pending_navigation(), expected.parent());
    }

    #[test]
    fn accepted_series_parent_requests_the_next_parent_not_series() {
        let temp = tempdir().unwrap();
        let library = temp.path().join("library");
        fs::create_dir_all(library.join("series")).unwrap();
        let requested = library.join("series/..");
        let mut tree = FileTree::new();
        tree.request_navigation(requested.clone());
        let request = tree.take_requests().pop().unwrap();
        let loaded = load_directory(&request.path).unwrap();
        assert_eq!(loaded.requested_path, requested);
        assert_eq!(loaded.path, library);
        assert_eq!(loaded.entries[0].entry.entry_path, library.join("series"));
        assert!(tree.complete(request.token, Ok(loaded)));
        assert_eq!(tree.location(), Some(library.as_path()));
        assert!(tree.request_parent());
        assert_eq!(tree.pending_navigation(), Some(temp.path()));
        let parent = tree.take_requests().pop().unwrap();
        assert!(tree.complete(parent.token, load_directory(&parent.path)));
        assert_eq!(tree.location(), Some(temp.path()));
        assert!(tree.back());
        assert_eq!(tree.location(), Some(library.as_path()));
    }

    #[test]
    fn original_request_proof_is_required_even_when_accepted_path_matches() {
        let temp = tempdir().unwrap();
        let mut tree = initialize(&[("page.png", false)]);
        tree.request_navigation(temp.path().into());
        let request = tree.take_requests().pop().unwrap();
        let mut loaded = load_directory(&request.path).unwrap();
        loaded.requested_path = temp.path().join("wrong");
        assert_eq!(loaded.path, request.path);
        assert!(tree.complete(request.token, Ok(loaded)));
        assert_eq!(tree.location(), Some(Path::new("root")));
        assert!(tree.error().unwrap().contains("native path"));
        assert!(!tree.can_back());
    }

    #[test]
    fn expansion_and_locate_require_the_accepted_path_to_match_the_owner() {
        for locate in [false, true] {
            let mut tree = initialize(&[("a", true)]);
            if locate {
                tree.locate("root/a/book.png".into());
            } else {
                tree.expand(Path::new("root/a"));
            }
            let request = tree.take_requests().pop().unwrap();
            assert!(!tree.is_navigation_request(request.token));
            let mut loaded = listing(&request.path, vec![]);
            loaded.path = "wrong-owner".into();
            assert!(tree.complete(request.token, Ok(loaded)));
            let error = if locate {
                tree.error()
            } else {
                tree.node_error(&request.path)
            };
            assert!(error.unwrap().contains("owner path"));
            assert_eq!(tree.location(), Some(Path::new("root")));
            assert!(!tree.node(&request.path).unwrap().loaded);
        }
    }

    #[test]
    fn root_parent_navigation_and_back_preserve_the_previous_tree() {
        let temp = tempdir().unwrap();
        let child = temp.path().join("child");
        fs::create_dir(&child).unwrap();
        let mut tree = FileTree::new();
        tree.request_navigation(child.clone());
        let request = tree.take_requests().pop().unwrap();
        tree.complete(request.token, load_directory(&request.path));
        assert!(tree.can_parent());
        assert!(tree.request_parent());
        assert_eq!(tree.pending_navigation(), Some(temp.path()));
        let parent = tree.take_requests().pop().unwrap();
        tree.complete(parent.token, load_directory(&parent.path));
        assert_eq!(tree.root(), Some(temp.path()));
        assert!(tree.can_back());
        assert!(tree.back());
        assert_eq!(tree.root(), Some(child.as_path()));
        #[cfg(windows)]
        let filesystem_root = Path::new(r"C:\");
        #[cfg(not(windows))]
        let filesystem_root = Path::new("/");
        tree.request_navigation(filesystem_root.into());
        let request = tree.take_requests().pop().unwrap();
        tree.complete(request.token, Ok(listing(filesystem_root, vec![])));
        assert!(!tree.can_parent());
        assert!(!tree.request_parent());
    }

    #[test]
    fn back_to_same_native_root_does_not_accept_an_older_root_generation() {
        let mut tree = initialize(&[("a", true)]);
        tree.expand(Path::new("root/a"));
        let old = tree.take_requests().pop().unwrap();
        tree.request_navigation("other".into());
        let other = tree.take_requests().pop().unwrap();
        tree.complete(other.token, Ok(listing(&other.path, vec![])));
        assert!(tree.back());
        tree.expand(Path::new("root/a"));
        let current = tree.take_requests().pop().unwrap();
        assert!(!tree.complete(
            old.token,
            Ok(listing(
                &old.path,
                vec![item(&old.path, "obsolete.png", false)]
            ))
        ));
        assert!(tree.complete(current.token, Ok(listing(&current.path, vec![]))));
        assert_eq!(names(&tree), ["a"]);
    }

    #[test]
    fn book_rows_never_expand_and_disclosure_does_not_navigate() {
        let mut tree = initialize(&[("a", true), ("page.png", false)]);
        assert!(tree.toggle_expanded(Path::new("root/a")));
        assert_eq!(tree.location(), Some(Path::new("root")));
        assert!(tree.toggle_expanded(Path::new("root/a")));
        assert!(tree.take_requests().is_empty());
        assert!(!tree.toggle_expanded(Path::new("root/page.png")));
        assert!(!tree.collapse(Path::new("root/page.png")));
        tree.set_query("page".into());
        let row = &tree.visible_rows()[0];
        assert!(!row.expanded);
        assert!(!row.cycle);
        assert!(row.error.is_none());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn distinct_non_unicode_native_names_are_not_lossy_string_keys() {
        #[cfg(unix)]
        fn native(value: u8) -> std::ffi::OsString {
            use std::os::unix::ffi::OsStringExt;
            std::ffi::OsString::from_vec(vec![value, b'.', b'p', b'n', b'g'])
        }
        #[cfg(windows)]
        fn native(value: u16) -> std::ffi::OsString {
            use std::os::windows::ffi::OsStringExt;
            std::ffi::OsString::from_wide(&[value, 46, 112, 110, 103])
        }
        #[cfg(unix)]
        let values = [0xfe, 0xff];
        #[cfg(windows)]
        let values = [0xd800, 0xd801];
        let root = Path::new("root");
        let entries: Vec<_> = values
            .into_iter()
            .map(|value| {
                let mut entry = item(root, "placeholder", false);
                let path = root.join(native(value));
                entry.entry.name = path.file_name().unwrap().to_string_lossy().into_owned();
                entry.entry.entry_path = path.clone();
                entry.entry.path = path;
                entry
            })
            .collect();
        assert_eq!(entries[0].entry.name, entries[1].entry.name);
        let mut tree = initialize(&[]);
        tree.retry(root);
        let request = tree.take_requests().pop().unwrap();
        tree.complete(request.token, Ok(listing(root, entries)));
        assert_eq!(tree.visible_rows().len(), 2);
        assert_ne!(tree.visible_rows()[0].path, tree.visible_rows()[1].path);
    }

    #[test]
    fn display_paths_strip_only_verbatim_drive_and_unc_prefixes() {
        assert_eq!(
            display_path(Path::new(r"\\?\C:\漫画\page")),
            r"C:\漫画\page"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share\漫画")),
            r"\\server\share\漫画"
        );
        assert_eq!(
            display_path(Path::new(r"\\server\share\page")),
            r"\\server\share\page"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\Volume{abc}\page")),
            r"\\?\Volume{abc}\page"
        );
        assert_eq!(display_path(Path::new("/tmp/漫画")), "/tmp/漫画");
    }

    #[test]
    fn hidden_classification_uses_dot_names_hidden_and_system_bits() {
        for (name, attrs, expected) in [
            (".dot.png", 0, true),
            ("normal.png", 2, true),
            ("system.png", 4, true),
            ("both.png", 6, true),
            ("visible.png", 0x20, false),
            ("file.name.png", 0, false),
        ] {
            assert_eq!(hidden_name_and_attributes(Path::new(name), attrs), expected);
        }
    }

    #[test]
    fn loader_is_shallow_and_keeps_hidden_entries_out_of_central_filtering() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join("folder2")).unwrap();
        fs::create_dir(temp.path().join("folder10")).unwrap();
        fs::write(temp.path().join("folder2/nested.png"), b"image").unwrap();
        fs::write(temp.path().join(".hidden.png"), b"image").unwrap();
        fs::write(temp.path().join("2.png"), b"image").unwrap();
        fs::write(temp.path().join("10.png"), b"image").unwrap();
        fs::write(temp.path().join("ignored.txt"), b"ignored").unwrap();
        let loaded = load_directory(temp.path()).unwrap();
        assert_eq!(loaded.path, temp.path());
        assert_eq!(loaded.entries.len(), 5);
        assert_eq!(
            loaded
                .entries
                .iter()
                .map(|item| item.entry.name.as_str())
                .collect::<Vec<_>>(),
            ["folder2", "folder10", ".hidden.png", "2.png", "10.png"]
        );
        assert!(loaded.entries[2].hidden);
        assert_eq!(
            book::browse_directory(temp.path()).unwrap().entries.len(),
            5
        );
        assert!(load_directory(&temp.path().join("2.png")).is_err());
        assert!(load_directory(&temp.path().join("missing")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn actual_windows_hidden_and_system_attributes_are_classified() {
        let temp = tempdir().unwrap();
        for (name, attribute) in [("hidden.png", "+H"), ("system.png", "+S")] {
            let path = temp.path().join(name);
            fs::write(&path, b"image").unwrap();
            let result = std::process::Command::new("attrib.exe")
                .arg(attribute)
                .arg(&path)
                .output()
                .unwrap();
            assert!(result.status.success(), "{result:?}");
        }
        fs::write(temp.path().join("visible.png"), b"image").unwrap();
        let loaded = load_directory(temp.path()).unwrap();
        assert_eq!(loaded.entries.iter().filter(|item| item.hidden).count(), 2);
        assert!(
            loaded
                .entries
                .iter()
                .find(|item| item.entry.name == "visible.png")
                .is_some_and(|item| !item.hidden)
        );
        for item in &loaded.entries {
            let _ = std::process::Command::new("attrib.exe")
                .args(["-H", "-S"])
                .arg(&item.entry.entry_path)
                .output()
                .unwrap();
        }
    }

    #[cfg(any(windows, unix))]
    fn directory_alias(target: &Path, alias: &Path) {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, alias).unwrap();
        #[cfg(windows)]
        {
            let result = std::process::Command::new("cmd.exe")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(alias)
                .arg(target)
                .output()
                .unwrap();
            assert!(result.status.success(), "{result:?}");
        }
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn alias_sub_parent_keeps_alias_when_lexical_and_actual_targets_match() {
        let temp = tempdir().unwrap();
        let target = temp.path().join("target");
        let alias = temp.path().join("alias");
        fs::create_dir_all(target.join("sub")).unwrap();
        fs::write(target.join("page.png"), b"image").unwrap();
        directory_alias(&target, &alias);
        let requested = alias.join("sub/..");
        let loaded = load_directory(&requested).unwrap();
        assert_eq!(loaded.requested_path, requested);
        assert_eq!(loaded.path, alias);
        assert_eq!(loaded.target, fs::canonicalize(&target).unwrap());
        assert!(loaded.entries.iter().any(|item| {
            item.entry.entry_path == alias.join("page.png")
                && item.entry.path == fs::canonicalize(target.join("page.png")).unwrap()
        }));
        let source = alias.join("sub/../page.png");
        let source_target = fs::canonicalize(&source).unwrap();
        assert_eq!(
            normalize_entry_path(&source, &source_target).unwrap(),
            alias.join("page.png")
        );
        let mut tree = FileTree::new();
        tree.request_navigation(requested);
        let request = tree.take_requests().pop().unwrap();
        assert!(tree.complete(request.token, Ok(loaded)));
        assert_eq!(tree.root(), Some(alias.as_path()));
        assert!(tree.request_parent());
        assert_eq!(tree.pending_navigation(), Some(temp.path()));
    }

    #[cfg(unix)]
    #[test]
    fn posix_alias_parent_uses_physical_parent_when_lexical_parent_differs() {
        let temp = tempdir().unwrap();
        let library = temp.path().join("library");
        let physical = temp.path().join("physical");
        let target = physical.join("series");
        let alias = library.join("alias");
        fs::create_dir(&library).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(physical.join("page.png"), b"image").unwrap();
        directory_alias(&target, &alias);
        let requested = alias.join("..");
        let loaded = load_directory(&requested).unwrap();
        let expected = fs::canonicalize(&physical).unwrap();
        assert_eq!(loaded.requested_path, requested);
        assert_eq!(loaded.path, expected);
        assert_ne!(loaded.path, library);
        assert_eq!(loaded.target, expected);
        assert!(
            loaded
                .entries
                .iter()
                .any(|item| { item.entry.entry_path == expected.join("page.png") })
        );
        let source = alias.join("../page.png");
        assert_eq!(
            normalize_entry_path(&source, &fs::canonicalize(&source).unwrap()).unwrap(),
            expected.join("page.png")
        );
        let mut tree = FileTree::new();
        tree.request_navigation(requested);
        let request = tree.take_requests().pop().unwrap();
        assert!(tree.complete(request.token, Ok(loaded)));
        assert!(tree.request_parent());
        assert_eq!(tree.pending_navigation(), expected.parent());
    }

    #[cfg(any(windows, unix))]
    #[test]
    fn actual_directory_alias_rebases_native_children_and_preserves_targets() {
        let temp = tempdir().unwrap();
        let target = temp.path().join("target");
        let alias = temp.path().join("alias");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("page.png"), b"image").unwrap();
        directory_alias(&target, &alias);
        let second_alias = temp.path().join("second-alias");
        directory_alias(&target, &second_alias);
        let second = load_directory(&second_alias).unwrap();
        let loaded = load_directory(&alias).unwrap();
        assert_eq!(loaded.requested_path, alias);
        assert_eq!(second.path, second_alias);
        assert_ne!(loaded.path, second.path);
        assert_eq!(loaded.target, second.target);
        assert_eq!(loaded.path, alias);
        assert_eq!(loaded.target, fs::canonicalize(&target).unwrap());
        assert_eq!(loaded.entries[0].entry.entry_path, alias.join("page.png"));
        assert_eq!(
            loaded.entries[0].entry.path,
            fs::canonicalize(target.join("page.png")).unwrap()
        );
        let mut tree = FileTree::new();
        tree.request_navigation(alias.clone());
        let request = tree.take_requests().pop().unwrap();
        tree.complete(request.token, Ok(loaded));
        assert_eq!(tree.root(), Some(alias.as_path()));
        assert!(tree.select(&alias.join("page.png")));
        assert_eq!(
            tree.selected_entry().unwrap().entry_path,
            alias.join("page.png")
        );
    }
}
