//! Image folders and comic archives. Page bytes are loaded only when requested.

use rars::{Archive, ArchiveReadOptions, ArchiveReader};
use same_file::Handle;
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use zip::ZipArchive;

pub const MAX_PAGE_BYTES: u64 = 128 * 1024 * 1024;
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "jpe", "png", "gif", "bmp", "webp"];
pub const ARCHIVE_EXTENSIONS: &[&str] = &["zip", "cbz", "rar", "cbr"];

const SIZE_ERROR: &str = "Page exceeds the 128 MiB safety limit.";
const RAR_PASSWORD_ERROR: &str =
    "Encrypted RAR/CBR archives are not supported. No password is requested or stored.";

#[derive(Debug, Clone)]
pub struct Book {
    pub title: String,
    /// Canonical folder or archive path, also used as the history key.
    pub path: PathBuf,
    pub pages: Vec<Page>,
    pub initial_page: usize,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub name: String,
    source: PageSource,
}

#[derive(Debug, Clone)]
enum PageSource {
    File(PathBuf),
    Zip {
        archive: PathBuf,
        entry_index: usize,
        entry_name: Vec<u8>,
    },
    Rar {
        archive: PathBuf,
        entry_index: usize,
        entry_name: Vec<u8>,
    },
}

impl PageSource {
    fn path(&self) -> &Path {
        match self {
            Self::File(path) => path,
            Self::Zip { archive, .. } | Self::Rar { archive, .. } => archive,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DirectoryListing {
    pub path: PathBuf,
    pub parent: Option<PathBuf>,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Clone)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_directory: bool,
    pub kind: EntryKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    Archive,
    Image,
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_image(path: &Path) -> bool {
    IMAGE_EXTENSIONS.contains(&extension(path).as_str())
}

fn is_archive(path: &Path) -> bool {
    ARCHIVE_EXTENSIONS.contains(&extension(path).as_str())
}

/// Returns true for an existing folder or supported regular file.
pub fn is_supported(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|meta| meta.is_dir() || (meta.is_file() && (is_image(path) || is_archive(path))))
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn canonicalize(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|error| format!("Cannot resolve {}: {error}", path.display()))
}

fn check_size(size: u64) -> Result<(), String> {
    if size > MAX_PAGE_BYTES {
        Err(SIZE_ERROR.into())
    } else {
        Ok(())
    }
}

fn sort_pages(pages: &mut [Page]) {
    pages.sort_by(|left, right| {
        natord::compare_ignore_case(&left.name, &right.name)
            .then_with(|| left.name.cmp(&right.name))
    });
}

impl Book {
    /// Opens a folder, an image's containing folder, or a ZIP/CBZ or RAR/CBR.
    /// `recursive` controls folder traversal; archive entries always include subfolders.
    pub fn open(path: &Path, recursive: bool) -> Result<Self, String> {
        let path = canonicalize(path)?;
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if metadata.is_dir() {
            return Self::open_folder(&path, recursive, None);
        }
        if !metadata.is_file() {
            return Err("The path is not a regular file or folder.".into());
        }
        if is_image(&path) {
            let parent = path.parent().ok_or("The image has no parent folder.")?;
            return Self::open_folder(parent, recursive, Some(&path));
        }
        match extension(&path).as_str() {
            "zip" | "cbz" => Self::open_zip(&path),
            "rar" | "cbr" => Self::open_rar(&path),
            _ => Err("Unsupported file format. Use an image, ZIP/CBZ, or RAR/CBR.".into()),
        }
    }

    fn open_folder(path: &Path, recursive: bool, selected: Option<&Path>) -> Result<Self, String> {
        let mut pages = Vec::new();
        let mut pending = vec![path.to_path_buf()];
        let mut visited = HashSet::new();
        // Use an explicit stack and canonical directory identities to prevent link loops.
        while let Some(folder) = pending.pop() {
            if !visited.insert(canonicalize(&folder)?) {
                continue;
            }
            let mut entries = fs::read_dir(&folder)
                .map_err(|error| format!("Cannot read {}: {error}", folder.display()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            // A fixed traversal order also fixes which name represents a linked folder.
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries.into_iter().rev() {
                let file = entry.path();
                let metadata = match fs::metadata(&file) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => {
                        return Err(format!("Cannot inspect {}: {error}", file.display()));
                    }
                };
                if metadata.is_dir() {
                    if recursive {
                        pending.push(file);
                    }
                } else if metadata.is_file() && is_image(&file) {
                    check_size(metadata.len())
                        .map_err(|error| format!("{}: {error}", file.display()))?;
                    pages.push(Page {
                        name: file
                            .strip_prefix(path)
                            .unwrap_or(&file)
                            .to_string_lossy()
                            .into_owned(),
                        source: PageSource::File(canonicalize(&file)?),
                    });
                }
            }
        }
        if pages.is_empty() {
            return Err("No supported images in this folder.".into());
        }
        sort_pages(&mut pages);
        let initial_page = selected
            .and_then(|selected| pages.iter().position(|page| page.source.path() == selected))
            .unwrap_or(0);
        Ok(Self {
            title: display_name(path),
            path: path.to_path_buf(),
            pages,
            initial_page,
        })
    }

    fn open_zip(path: &Path) -> Result<Self, String> {
        let mut archive = open_zip(path)?;
        let mut pages = Vec::new();
        for entry_index in 0..archive.len() {
            let entry = archive
                .by_index(entry_index)
                .map_err(|error| format!("Cannot read ZIP/CBZ entry: {error}"))?;
            let name = entry.name().replace('\\', "/");
            if !entry.is_dir() && is_image(Path::new(&name)) {
                check_size(entry.size()).map_err(|error| format!("{name}: {error}"))?;
                pages.push(Page {
                    name,
                    source: PageSource::Zip {
                        archive: path.to_path_buf(),
                        entry_index,
                        entry_name: entry.name_raw().to_vec(),
                    },
                });
            }
        }
        Self::archive_book(path, pages)
    }

    fn open_rar(path: &Path) -> Result<Self, String> {
        let archive = open_rar(path)?;
        let mut pages = Vec::new();
        for (entry_index, member) in rar_members(&archive).enumerate() {
            if member.meta.is_encrypted {
                return Err(RAR_PASSWORD_ERROR.into());
            }
            let name = member.meta.name_lossy().replace('\\', "/");
            if !member.meta.is_directory && is_image(Path::new(&name)) {
                check_size(member.meta.unpacked_size)
                    .map_err(|error| format!("{name}: {error}"))?;
                pages.push(Page {
                    name,
                    source: PageSource::Rar {
                        archive: path.to_path_buf(),
                        entry_index,
                        entry_name: member.meta.name,
                    },
                });
            }
        }
        Self::archive_book(path, pages)
    }

    fn archive_book(path: &Path, mut pages: Vec<Page>) -> Result<Self, String> {
        if pages.is_empty() {
            return Err("No supported images in this archive.".into());
        }
        sort_pages(&mut pages);
        Ok(Self {
            title: display_name(path),
            path: path.to_path_buf(),
            pages,
            initial_page: 0,
        })
    }

    pub fn read_page(&self, index: usize) -> Result<Vec<u8>, String> {
        let page = self.pages.get(index).ok_or("Page index is out of range.")?;
        match &page.source {
            PageSource::File(path) => {
                let file = File::open(path).map_err(|error| error.to_string())?;
                let metadata = file.metadata().map_err(|error| error.to_string())?;
                if !metadata.is_file() {
                    return Err("The page is no longer a regular file.".into());
                }
                check_size(metadata.len())?;
                read_bounded(file, MAX_PAGE_BYTES as usize)
            }
            PageSource::Zip {
                archive,
                entry_index,
                entry_name,
            } => {
                let mut archive = open_zip(archive)?;
                let entry = archive
                    .by_index(*entry_index)
                    .map_err(|error| format!("ZIP/CBZ page is no longer available: {error}"))?;
                if entry.is_dir() || entry.name_raw() != entry_name {
                    return Err("ZIP/CBZ page has changed. Open the book again.".into());
                }
                check_size(entry.size())?;
                read_bounded(entry, MAX_PAGE_BYTES as usize)
            }
            PageSource::Rar {
                archive,
                entry_index,
                entry_name,
            } => read_rar_page(archive, *entry_index, entry_name),
        }
    }

    /// Saves the original bytes. Never truncates an original page or its archive.
    pub fn save_page(&self, index: usize, destination: &Path) -> Result<(), String> {
        let bytes = self.read_page(index)?;
        // Do not truncate on open. Check the actual output handle before any write,
        // so a symlink or hard link to a source cannot bypass the path check.
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(destination)
            .map_err(|error| format!("Cannot save {}: {error}", destination.display()))?;
        if !file
            .metadata()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("The export destination is not a regular file.".into());
        }
        let mut output = Handle::from_file(file).map_err(|error| error.to_string())?;
        let sources: HashSet<_> = self.pages.iter().map(|page| page.source.path()).collect();
        for source in sources {
            let source = Handle::from_path(source)
                .map_err(|error| format!("Cannot verify the original file: {error}"))?;
            if source == output {
                return Err("Cannot overwrite an original comic archive or image.".into());
            }
        }
        let output = output.as_file_mut();
        output.set_len(0).map_err(|error| error.to_string())?;
        output.write_all(&bytes).map_err(|error| error.to_string())
    }
}

fn open_zip(path: &Path) -> Result<ZipArchive<File>, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    ZipArchive::new(file).map_err(|error| format!("Cannot open ZIP/CBZ: {error}"))
}

fn rar_options() -> ArchiveReadOptions<'static> {
    ArchiveReadOptions::new().with_rar50_buffered_decode_limit(MAX_PAGE_BYTES)
}

fn open_rar(path: &Path) -> Result<Archive, String> {
    ArchiveReader::read_path_with_options(path, rar_options()).map_err(rar_error)
}

fn rar_error(error: rars::Error) -> String {
    fn needs_password(error: &rars::Error) -> bool {
        match error {
            rars::Error::NeedPassword
            | rars::Error::WrongPasswordOrCorruptData
            | rars::Error::UnsupportedEncryption { .. } => true,
            rars::Error::AtArchiveOffset { source, .. }
            | rars::Error::AtEntry { source, .. }
            | rars::Error::InVolume { source, .. } => needs_password(source),
            _ => false,
        }
    }
    if needs_password(&error) {
        RAR_PASSWORD_ERROR.into()
    } else {
        format!("Cannot read RAR/CBR: {error}")
    }
}

// RAR5 redirections do not supply page bytes and are not emitted by extract_to.
// Filter them before assigning indices, including when member names are duplicates.
fn rar_members(archive: &Archive) -> impl Iterator<Item = rars::ArchiveMember> + '_ {
    let redirections: HashSet<_> = match archive {
        Archive::Rar50Plus(archive) => archive
            .files()
            .enumerate()
            .filter_map(|(index, file)| file.is_redirection().then_some(index))
            .collect(),
        _ => HashSet::new(),
    };
    archive
        .members()
        .enumerate()
        .filter_map(move |(index, member)| (!redirections.contains(&index)).then_some(member))
}

fn read_rar_page(path: &Path, entry_index: usize, entry_name: &[u8]) -> Result<Vec<u8>, String> {
    let archive = open_rar(path)?;
    let bytes = Arc::new(Mutex::new(PageBuffer::new(MAX_PAGE_BYTES as usize)));
    let mut members = rar_members(&archive);
    let mut current = 0;
    let mut found = false;
    let mut stopped = false;
    // Solid RAR members require the preceding entries. Stream them to a sink,
    // and stop after the selected member has passed its checksum check.
    let result = archive.extract_to_with_options(rar_options(), |meta| {
        if found {
            stopped = true;
            return Err(rars::Error::Cancelled);
        }
        let member = members
            .next()
            .ok_or(rars::Error::InvalidHeader("RAR member order changed"))?;
        if member.meta.is_encrypted {
            return Err(rars::Error::NeedPassword);
        }
        check_size(member.meta.unpacked_size)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let index = current;
        current += 1;
        if index == entry_index {
            if meta.is_directory || meta.name != entry_name {
                return Err(rars::Error::InvalidHeader(
                    "RAR page changed; open the book again",
                ));
            }
            found = true;
            Ok(Box::new(SharedPageBuffer(Arc::clone(&bytes))) as Box<dyn Write>)
        } else {
            Ok(Box::new(io::sink()) as Box<dyn Write>)
        }
    });
    match result {
        Err(rars::Error::Cancelled) if stopped => {}
        Err(error) => return Err(rar_error(error)),
        Ok(()) => {}
    }
    if !found {
        return Err("RAR/CBR page is no longer available.".into());
    }
    let mut buffer = bytes.lock().map_err(|_| "The page buffer lock failed.")?;
    Ok(std::mem::take(&mut buffer.bytes))
}

/// A fallible output buffer with a fixed byte limit. It does not trust a file's
/// metadata or an archive's declared unpacked size.
struct PageBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl PageBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }
}

impl Write for PageBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, SIZE_ERROR));
        }
        let required = self.bytes.len() + bytes.len();
        if required > self.bytes.capacity() {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(required)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct SharedPageBuffer(Arc<Mutex<PageBuffer>>);

impl Write for SharedPageBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("The page buffer lock failed."))?
            .write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut output = PageBuffer::new(limit);
    // The extra byte detects a page that grew after the metadata check.
    io::copy(&mut reader.take(limit as u64 + 1), &mut output)
        .map_err(|error| format!("Cannot read page: {error}"))?;
    Ok(output.bytes)
}

/// Lists folders first, then supported files, with natural case-insensitive order.
/// Returned paths are canonical, including paths reached through symbolic links.
pub fn browse_directory(path: &Path) -> Result<DirectoryListing, String> {
    let path = canonicalize(path)?;
    if !path.is_dir() {
        return Err("The path is not a folder.".into());
    }
    let mut entries = Vec::new();
    for item in fs::read_dir(&path).map_err(|error| error.to_string())? {
        let file = item.map_err(|error| error.to_string())?.path();
        let metadata = match fs::metadata(&file) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        let kind = if metadata.is_dir() {
            EntryKind::Folder
        } else if metadata.is_file() && is_archive(&file) {
            EntryKind::Archive
        } else if metadata.is_file() && is_image(&file) {
            EntryKind::Image
        } else {
            continue;
        };
        entries.push(DirectoryEntry {
            name: display_name(&file),
            path: canonicalize(&file)?,
            is_directory: kind == EntryKind::Folder,
            kind,
        });
    }
    entries.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| natord::compare_ignore_case(&left.name, &right.name))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(DirectoryListing {
        parent: path.parent().map(Path::to_path_buf),
        path,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rars::{ArchiveVersion, Builder};
    use std::io::Cursor;
    use tempfile::tempdir;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    fn names(book: &Book) -> Vec<&str> {
        book.pages.iter().map(|page| page.name.as_str()).collect()
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut archive = ZipWriter::new(File::create(path).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        archive.add_directory("directory.jpg/", options).unwrap();
        for (name, bytes) in entries {
            archive.start_file(*name, options).unwrap();
            archive.write_all(bytes).unwrap();
        }
        archive.finish().unwrap();
    }

    fn rar_builder(version: ArchiveVersion, entries: &[(&str, &[u8])]) -> Builder {
        let mut builder = Builder::new(version).store(true);
        for (name, bytes) in entries {
            builder
                .add_bytes(name.as_bytes().to_vec(), bytes.to_vec(), None, None)
                .unwrap();
        }
        builder
    }

    fn write_rar(path: &Path, entries: &[(&str, &[u8])]) {
        rar_builder(ArchiveVersion::Rar29, entries)
            .write_to_path(path, None)
            .unwrap();
    }

    fn assert_error<T: std::fmt::Debug>(result: Result<T, String>, expected: &str) {
        let error = result.unwrap_err();
        assert!(
            error.contains(expected),
            "Expected {expected:?}, got {error:?}"
        );
    }

    #[test]
    fn supported_paths_require_a_folder_or_supported_regular_file() {
        let temp = tempdir().unwrap();
        assert!(is_supported(temp.path()));
        for extension in IMAGE_EXTENSIONS.iter().chain(ARCHIVE_EXTENSIONS) {
            let path = temp
                .path()
                .join(format!("page.{}", extension.to_uppercase()));
            fs::write(&path, b"test").unwrap();
            assert!(is_supported(&path), "{}", path.display());
        }
        let path = temp.path().join("notes.txt");
        fs::write(&path, b"text").unwrap();
        assert!(!is_supported(&path));
        assert!(!is_supported(&temp.path().join("missing.cbz")));
        assert!(!is_supported(&temp.path().join("no-extension")));
    }

    #[test]
    fn folders_have_natural_order_and_images_select_the_initial_page() {
        let temp = tempdir().unwrap();
        for (name, bytes) in [("10.jpg", "ten"), ("2.JPG", "two"), ("1.png", "one")] {
            fs::write(temp.path().join(name), bytes).unwrap();
        }
        fs::write(temp.path().join("readme.txt"), b"ignored").unwrap();
        let selected = temp.path().join("2.JPG");
        let book = Book::open(&selected, true).unwrap();
        assert_eq!(names(&book), ["1.png", "2.JPG", "10.jpg"]);
        assert_eq!(book.initial_page, 1);
        assert_eq!(book.path, fs::canonicalize(temp.path()).unwrap());
        assert_eq!(book.title, display_name(temp.path()));
        assert_eq!(book.read_page(1).unwrap(), b"two");
        assert_eq!(book.clone().read_page(2).unwrap(), b"ten");
        assert_eq!(Book::open(temp.path(), false).unwrap().initial_page, 0);
    }

    #[test]
    fn recursive_folders_include_nested_images_but_not_archive_contents() {
        let temp = tempdir().unwrap();
        for directory in ["chapter10", "chapter2", "chapter2/deep"] {
            fs::create_dir_all(temp.path().join(directory)).unwrap();
        }
        for name in [
            "root.jpg",
            "chapter10/1.jpg",
            "chapter2/10.jpg",
            "chapter2/2.jpg",
            "chapter2/deep/1.jpg",
        ] {
            fs::write(temp.path().join(name), name).unwrap();
        }
        write_zip(&temp.path().join("ignored.cbz"), &[("1.png", b"archive")]);
        let shallow = Book::open(temp.path(), false).unwrap();
        assert_eq!(names(&shallow), ["root.jpg"]);
        let recursive = Book::open(temp.path(), true).unwrap();
        let expected: Vec<_> = [
            "chapter2/2.jpg",
            "chapter2/10.jpg",
            "chapter2/deep/1.jpg",
            "chapter10/1.jpg",
            "root.jpg",
        ]
        .iter()
        .map(|name| name.replace('/', std::path::MAIN_SEPARATOR_STR))
        .collect();
        assert_eq!(names(&recursive), expected);
        let nested_image = Book::open(&temp.path().join("chapter2/10.jpg"), true).unwrap();
        assert_eq!(nested_image.initial_page, 1);
        assert_eq!(
            nested_image.path,
            fs::canonicalize(temp.path().join("chapter2")).unwrap()
        );
    }

    #[test]
    fn empty_unsupported_and_missing_inputs_report_errors() {
        let temp = tempdir().unwrap();
        assert_error(Book::open(temp.path(), true), "No supported images");
        fs::create_dir(temp.path().join("nested")).unwrap();
        fs::write(temp.path().join("nested/1.jpg"), b"nested").unwrap();
        assert_error(Book::open(temp.path(), false), "No supported images");
        assert_eq!(Book::open(temp.path(), true).unwrap().pages.len(), 1);
        let unsupported = temp.path().join("notes.txt");
        fs::write(&unsupported, b"text").unwrap();
        assert_error(Book::open(&unsupported, false), "Unsupported file format");
        assert_error(
            Book::open(&temp.path().join("missing.cbz"), false),
            "Cannot resolve",
        );
    }

    #[test]
    fn canonical_paths_remove_relative_components() {
        let temp = tempdir().unwrap();
        fs::create_dir(temp.path().join("child")).unwrap();
        fs::write(temp.path().join("1.jpg"), b"original").unwrap();
        let input = temp.path().join("child/..");
        let expected = fs::canonicalize(temp.path()).unwrap();
        assert_eq!(Book::open(&input, true).unwrap().path, expected);
        assert_eq!(browse_directory(&input).unwrap().path, expected);
        let archive = temp.path().join("book.cbz");
        write_zip(&archive, &[("1.jpg", b"original")]);
        assert_eq!(
            Book::open(&temp.path().join("child/../book.cbz"), false)
                .unwrap()
                .path,
            fs::canonicalize(archive).unwrap()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_directory_loops_and_dangling_links_do_not_repeat_pages() {
        use std::os::unix::fs::symlink;
        let temp = tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/1.png"), b"one").unwrap();
        symlink(&root, root.join("nested/back")).unwrap();
        symlink(root.join("missing"), root.join("dangling.png")).unwrap();
        let alias = temp.path().join("alias");
        symlink(&root, &alias).unwrap();
        let book = Book::open(&alias, true).unwrap();
        assert_eq!(names(&book), ["nested/1.png"]);
        assert_eq!(book.path, fs::canonicalize(&root).unwrap());
        assert_eq!(book.read_page(0).unwrap(), b"one");
        let listing = browse_directory(&alias).unwrap();
        assert_eq!(listing.path, book.path);
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(
            listing.entries[0].path,
            fs::canonicalize(root.join("nested")).unwrap()
        );
        assert_eq!(
            browse_directory(&root.join("nested")).unwrap().entries[0].path,
            book.path
        );
    }

    #[test]
    fn page_indices_and_removed_files_report_errors() {
        let temp = tempdir().unwrap();
        let image = temp.path().join("1.jpg");
        fs::write(&image, b"one").unwrap();
        let book = Book::open(temp.path(), false).unwrap();
        assert_error(book.read_page(1), "index");
        assert_error(book.read_page(usize::MAX), "index");
        let destination = temp.path().join("not-created.jpg");
        assert_error(book.save_page(1, &destination), "index");
        assert!(!destination.exists());
        fs::remove_file(&image).unwrap();
        assert!(book.read_page(0).is_err());
        fs::create_dir(&image).unwrap();
        assert_error(book.read_page(0), "regular file");
    }

    #[test]
    fn oversized_files_are_rejected_at_open_and_after_they_grow() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("1.jpg");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_PAGE_BYTES + 1).unwrap();
        assert_error(Book::open(temp.path(), false), "128 MiB");
        file.set_len(1).unwrap();
        let book = Book::open(&path, false).unwrap();
        file.set_len(MAX_PAGE_BYTES + 1).unwrap();
        assert_error(book.read_page(0), "128 MiB");
        assert_error(
            book.save_page(0, &temp.path().join("output.jpg")),
            "128 MiB",
        );
        assert!(!temp.path().join("output.jpg").exists());
    }

    #[test]
    fn bounded_streams_accept_the_limit_and_reject_a_single_extra_byte() {
        assert_eq!(read_bounded(Cursor::new(b"1234"), 4).unwrap(), b"1234");
        assert_eq!(read_bounded(Cursor::new(b""), 0).unwrap(), b"");
        assert_error(read_bounded(Cursor::new(b"12345"), 4), "safety limit");
        // Even an endless source must stop after the probe byte, not at EOF.
        assert_error(read_bounded(io::repeat(0), 4), "safety limit");
        struct BrokenReader;
        impl Read for BrokenReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("injected read error"))
            }
        }
        assert_error(read_bounded(BrokenReader, 4), "injected read error");
        let mut buffer = PageBuffer::new(4);
        buffer.write_all(b"12").unwrap();
        buffer.write_all(b"34").unwrap();
        assert!(buffer.write_all(b"5").is_err());
        assert_eq!(buffer.bytes, b"1234");
        assert!(buffer.bytes.capacity() <= 4);
    }

    #[test]
    fn zip_and_cbz_include_nested_images_in_natural_order() {
        let temp = tempdir().unwrap();
        for extension in ["zip", "CBZ"] {
            let path = temp.path().join(format!("sample.{extension}"));
            write_zip(
                &path,
                &[
                    ("nested/10.png", b"ten"),
                    ("readme.txt", b"no"),
                    ("nested/2.PNG", b"two"),
                ],
            );
            let book = Book::open(&path, false).unwrap();
            assert_eq!(names(&book), ["nested/2.PNG", "nested/10.png"]);
            assert_eq!(book.title, format!("sample.{extension}"));
            assert_eq!(book.path, fs::canonicalize(&path).unwrap());
            assert_eq!(book.initial_page, 0);
            assert_eq!(book.read_page(0).unwrap(), b"two");
            assert_eq!(book.read_page(1).unwrap(), b"ten");
            let destination = temp.path().join("export.png");
            book.save_page(1, &destination).unwrap();
            assert_eq!(fs::read(destination).unwrap(), b"ten");
            assert_error(book.save_page(0, &path), "original");
            assert_eq!(
                Book::open(&path, true).unwrap().read_page(0).unwrap(),
                b"two"
            );
        }
    }

    #[test]
    fn zip_compression_methods_preserve_original_bytes() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("compressed.cbz");
        let content = b"original page bytes, not a decoded image".repeat(100);
        for method in [
            CompressionMethod::Stored,
            CompressionMethod::Deflated,
            CompressionMethod::Bzip2,
            CompressionMethod::Xz,
        ] {
            let mut writer = ZipWriter::new(File::create(&path).unwrap());
            writer
                .start_file(
                    "1.jpg",
                    SimpleFileOptions::default().compression_method(method),
                )
                .unwrap();
            writer.write_all(&content).unwrap();
            writer.finish().unwrap();
            let book = Book::open(&path, true).unwrap();
            assert_eq!(book.read_page(0).unwrap(), content, "{method:?}");
        }
    }

    #[test]
    fn zip_lzma_and_deflate64_pages_preserve_original_bytes() {
        // The ZIP writer cannot encode these methods. These small textual fixtures
        // contain one page, generated with Python zipfile and fixed timestamps.
        let temp = tempdir().unwrap();
        let path = temp.path().join("fixture.cbz");
        for fixture in [
            include_str!("../tests/fixtures/lzma.zip.hex"),
            include_str!("../tests/fixtures/deflate64.zip.hex"),
        ] {
            let bytes: Vec<_> = fixture
                .trim()
                .as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect();
            fs::write(&path, bytes).unwrap();
            let book = Book::open(&path, false).unwrap();
            assert_eq!(names(&book), ["nested/1.png"]);
            assert_eq!(book.read_page(0).unwrap(), b"original page bytes");
        }
    }

    #[test]
    fn archive_names_are_display_only_and_never_extraction_destinations() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("paths.cbz");
        write_zip(
            &path,
            &[
                ("../outside.png", b"parent"),
                ("nested\\2.png", b"backslash"),
                ("/absolute.png", b"absolute"),
                ("C:\\10.jpg", b"drive"),
            ],
        );
        let book = Book::open(&path, true).unwrap();
        assert_eq!(book.pages.len(), 4);
        let index = book
            .pages
            .iter()
            .position(|page| page.name == "nested/2.png")
            .unwrap();
        assert_eq!(book.read_page(index).unwrap(), b"backslash");
        let destination = temp.path().join("explicit-destination.png");
        book.save_page(index, &destination).unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"backslash");
        assert!(!temp.path().join("nested").exists());
        assert!(!temp.path().join("outside.png").exists());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    }

    #[test]
    fn corrupt_and_empty_archives_report_errors() {
        let temp = tempdir().unwrap();
        for extension in ["zip", "cbz", "rar", "cbr"] {
            let path = temp.path().join(format!("corrupt.{extension}"));
            fs::write(&path, b"not an archive").unwrap();
            assert!(Book::open(&path, true).is_err());
        }
        let zip = temp.path().join("empty.cbz");
        write_zip(&zip, &[]);
        assert_error(Book::open(&zip, false), "No supported images");
        write_zip(&zip, &[("notes.txt", b"not a page")]);
        assert_error(Book::open(&zip, false), "No supported images");
        let rar = temp.path().join("empty.cbr");
        fs::write(
            &rar,
            rars::rar15_40::write_stored_archive(&[], Default::default()).unwrap(),
        )
        .unwrap();
        assert_error(Book::open(&rar, false), "No supported images");
        write_rar(&rar, &[("notes.txt", b"not a page")]);
        assert_error(Book::open(&rar, false), "No supported images");
    }

    #[test]
    fn zip_checksum_errors_and_changed_or_removed_entries_are_reported() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("sample.cbz");
        write_zip(
            &path,
            &[("1.png", b"unique page payload"), ("2.png", b"two")],
        );
        let book = Book::open(&path, false).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let offset = bytes
            .windows(b"unique page payload".len())
            .position(|part| part == b"unique page payload")
            .unwrap();
        bytes[offset] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert!(book.read_page(0).is_err());
        write_zip(&path, &[("replacement.png", b"changed")]);
        assert_error(book.read_page(0), "changed");
        assert_error(book.read_page(1), "no longer available");
        fs::write(&path, b"broken").unwrap();
        assert_error(book.read_page(0), "ZIP/CBZ");
        fs::remove_file(&path).unwrap();
        assert!(book.read_page(0).is_err());
    }

    fn inflate_zip_declared_size(path: &Path) {
        let mut bytes = fs::read(path).unwrap();
        let central = bytes
            .windows(4)
            .rposition(|part| part == b"PK\x01\x02")
            .unwrap();
        bytes[central + 24..central + 28]
            .copy_from_slice(&((MAX_PAGE_BYTES + 1) as u32).to_le_bytes());
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn zip_page_limits_are_checked_at_open_and_at_read() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("oversized.cbz");
        write_zip(&path, &[("1.png", b"small")]);
        let book = Book::open(&path, false).unwrap();
        inflate_zip_declared_size(&path);
        assert_error(Book::open(&path, false), "128 MiB");
        assert_error(book.read_page(0), "128 MiB");
    }

    #[test]
    fn rar_and_cbr_support_each_archive_family_and_nested_names() {
        let temp = tempdir().unwrap();
        for version in [
            ArchiveVersion::Rar13,
            ArchiveVersion::Rar15,
            ArchiveVersion::Rar20,
            ArchiveVersion::Rar29,
            ArchiveVersion::Rar50,
            ArchiveVersion::Rar70,
        ] {
            for extension in ["rar", "CBR"] {
                let path = temp.path().join(format!("sample.{extension}"));
                rar_builder(
                    version,
                    &[
                        ("nested/10.png", b"ten"),
                        ("notes.txt", b"ignored"),
                        ("nested/2.PNG", b"two"),
                    ],
                )
                .write_to_path(&path, None)
                .unwrap();
                let book = Book::open(&path, false).unwrap();
                assert_eq!(
                    names(&book),
                    ["nested/2.PNG", "nested/10.png"],
                    "{version:?}"
                );
                assert_eq!(book.read_page(0).unwrap(), b"two", "{version:?}");
                assert_eq!(book.read_page(1).unwrap(), b"ten", "{version:?}");
                let destination = temp.path().join("export.png");
                book.save_page(0, &destination).unwrap();
                assert_eq!(fs::read(destination).unwrap(), b"two");
                assert_error(book.save_page(1, &path), "original");
            }
        }
    }

    #[test]
    fn compressed_solid_rar_pages_decode_their_preceding_members() {
        let temp = tempdir().unwrap();
        for version in [ArchiveVersion::Rar29, ArchiveVersion::Rar50] {
            let path = temp.path().join("solid.cbr");
            let first = b"repeated original image bytes".repeat(50);
            let second = b"repeated original image bytes".repeat(70);
            let builder = rar_builder(version, &[("10.jpg", &first), ("2.jpg", &second)])
                .store(false)
                .compression_level(Some(1))
                .solid(true);
            builder.write_to_path(&path, None).unwrap();
            let book = Book::open(&path, true).unwrap();
            assert_eq!(names(&book), ["2.jpg", "10.jpg"]);
            assert_eq!(book.read_page(0).unwrap(), second, "{version:?}");
            assert_eq!(book.read_page(1).unwrap(), first, "{version:?}");
        }
    }

    #[test]
    fn duplicate_rar_names_keep_their_archive_indices() {
        use rars::rar15_40::{StoredEntry, WriterOptions, write_stored_archive};
        let temp = tempdir().unwrap();
        let path = temp.path().join("duplicates.cbr");
        let entries: Vec<_> = [b"first".as_slice(), b"second".as_slice()]
            .iter()
            .map(|data| StoredEntry {
                name: b"same.jpg",
                data,
                file_time: 0,
                file_attr: 0x20,
                host_os: 3,
                password: None,
                file_comment: None,
            })
            .collect();
        fs::write(
            &path,
            write_stored_archive(&entries, WriterOptions::default()).unwrap(),
        )
        .unwrap();
        let book = Book::open(&path, true).unwrap();
        assert_eq!(names(&book), ["same.jpg", "same.jpg"]);
        assert_eq!(book.read_page(0).unwrap(), b"first");
        assert_eq!(book.read_page(1).unwrap(), b"second");
    }

    #[test]
    fn rar_encrypted_payloads_and_headers_have_a_clear_error() {
        let temp = tempdir().unwrap();
        for version in [ArchiveVersion::Rar30, ArchiveVersion::Rar50] {
            for headers in [false, true] {
                let path = temp.path().join("encrypted.cbr");
                rar_builder(version, &[("1.jpg", b"private")])
                    .password(Some(b"test-only".to_vec()))
                    .header_encryption(headers)
                    .write_to_path(&path, None)
                    .unwrap();
                assert_error(Book::open(&path, true), RAR_PASSWORD_ERROR);
            }
        }
    }

    fn inflate_rar_declared_size(path: &Path) {
        let mut bytes = fs::read(path).unwrap();
        assert_eq!(&bytes[..7], b"Rar!\x1a\x07\x00");
        let mut offset = 7;
        while bytes[offset + 2] != 0x74 {
            let size =
                u16::from_le_bytes(bytes[offset + 5..offset + 7].try_into().unwrap()) as usize;
            offset += size;
        }
        let size = u16::from_le_bytes(bytes[offset + 5..offset + 7].try_into().unwrap()) as usize;
        bytes[offset + 11..offset + 15]
            .copy_from_slice(&((MAX_PAGE_BYTES + 1) as u32).to_le_bytes());
        let checksum = rars::crc32::crc32(&bytes[offset + 2..offset + size]) as u16;
        bytes[offset..offset + 2].copy_from_slice(&checksum.to_le_bytes());
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn rar_page_limits_are_checked_at_open_and_at_read() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("oversized.cbr");
        write_rar(&path, &[("1.jpg", b"small")]);
        let book = Book::open(&path, false).unwrap();
        inflate_rar_declared_size(&path);
        assert_error(Book::open(&path, false), "128 MiB");
        assert_error(book.read_page(0), "128 MiB");
    }

    #[test]
    fn rar_checksum_errors_and_changed_or_removed_entries_are_reported() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("sample.cbr");
        write_rar(
            &path,
            &[("1.jpg", b"unique original bytes"), ("2.jpg", b"two")],
        );
        let book = Book::open(&path, false).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        let offset = bytes
            .windows(b"unique original bytes".len())
            .position(|part| part == b"unique original bytes")
            .unwrap();
        bytes[offset] ^= 1;
        fs::write(&path, bytes).unwrap();
        assert_error(book.read_page(0), "checksum");
        write_rar(&path, &[("replacement.jpg", b"changed")]);
        assert_error(book.read_page(0), "changed");
        assert_error(book.read_page(1), "no longer available");
        fs::write(&path, b"broken").unwrap();
        assert_error(book.read_page(0), "RAR/CBR");
        fs::remove_file(&path).unwrap();
        assert!(book.read_page(0).is_err());
    }

    #[test]
    fn export_writes_original_bytes_and_can_replace_an_unrelated_file() {
        let temp = tempdir().unwrap();
        let image = temp.path().join("1.png");
        let original = b"original encoded image, no display transformations";
        fs::write(&image, original).unwrap();
        let book = Book::open(&image, false).unwrap();
        let destination = temp.path().join("output.png");
        book.save_page(0, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), original);
        fs::write(&destination, vec![0; 1000]).unwrap();
        book.save_page(0, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), original);
        assert_eq!(fs::read(&image).unwrap(), original);
        assert!(book.save_page(0, temp.path()).is_err());
        assert!(
            book.save_page(0, &temp.path().join("missing/output.png"))
                .is_err()
        );
    }

    #[test]
    fn export_rejects_every_original_image_and_hard_link_alias() {
        let temp = tempdir().unwrap();
        let first = temp.path().join("1.jpg");
        let second = temp.path().join("2.jpg");
        fs::write(&first, b"one").unwrap();
        fs::write(&second, b"two").unwrap();
        let book = Book::open(&first, true).unwrap();
        for destination in [&first, &second, &temp.path().join("./1.jpg")] {
            assert_error(book.save_page(0, destination), "original");
        }
        let alias = temp.path().join("alias.jpg");
        fs::hard_link(&second, &alias).unwrap();
        assert_error(book.save_page(0, &alias), "original");
        assert_eq!(fs::read(first).unwrap(), b"one");
        assert_eq!(fs::read(second).unwrap(), b"two");
    }

    #[cfg(unix)]
    #[test]
    fn export_rejects_symbolic_links_to_an_image_or_archive() {
        use std::os::unix::fs::symlink;
        let temp = tempdir().unwrap();
        let image = temp.path().join("1.jpg");
        fs::write(&image, b"one").unwrap();
        let book = Book::open(&image, false).unwrap();
        let alias = temp.path().join("alias.jpg");
        symlink(&image, &alias).unwrap();
        assert_error(book.save_page(0, &alias), "original");
        for extension in ["cbz", "cbr"] {
            let path = temp.path().join(format!("original.{extension}"));
            if extension == "cbz" {
                write_zip(&path, &[("1.jpg", b"one")]);
            } else {
                write_rar(&path, &[("1.jpg", b"one")]);
            }
            let original = fs::read(&path).unwrap();
            let book = Book::open(&path, true).unwrap();
            let alias = temp.path().join(format!("alias.{extension}"));
            symlink(&path, &alias).unwrap();
            assert_eq!(Book::open(&alias, true).unwrap().path, book.path);
            assert_error(book.save_page(0, &alias), "original");
            let hard_link = temp.path().join(format!("hard-link.{extension}"));
            fs::hard_link(&path, &hard_link).unwrap();
            assert_error(book.save_page(0, &hard_link), "original");
            assert_eq!(fs::read(path).unwrap(), original);
        }
    }

    #[test]
    fn directory_browser_filters_entries_and_sorts_folders_before_files() {
        let temp = tempdir().unwrap();
        for name in ["folder10", "folder2"] {
            fs::create_dir(temp.path().join(name)).unwrap();
        }
        for name in [
            "10.JPG",
            "2.png",
            "archive.CBR",
            "book.CBZ",
            "notes.txt",
            "no-extension",
        ] {
            fs::write(temp.path().join(name), b"contents").unwrap();
        }
        let listing = browse_directory(temp.path()).unwrap();
        let names: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "folder2",
                "folder10",
                "2.png",
                "10.JPG",
                "archive.CBR",
                "book.CBZ"
            ]
        );
        let kinds: Vec<_> = listing.entries.iter().map(|entry| entry.kind).collect();
        assert_eq!(
            kinds,
            [
                EntryKind::Folder,
                EntryKind::Folder,
                EntryKind::Image,
                EntryKind::Image,
                EntryKind::Archive,
                EntryKind::Archive
            ]
        );
        assert_eq!(listing.parent.as_deref(), listing.path.parent());
        assert!(listing.entries.iter().all(|entry| entry.path.is_absolute()
            && entry.path == fs::canonicalize(&entry.path).unwrap()
            && entry.is_directory == (entry.kind == EntryKind::Folder)));
        assert_error(browse_directory(&temp.path().join("2.png")), "not a folder");
        assert_error(
            browse_directory(&temp.path().join("missing")),
            "Cannot resolve",
        );
    }
}
