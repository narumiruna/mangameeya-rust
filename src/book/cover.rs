//! Bounded automatic cover reads. Manual `Book::open` retains its existing limits.

use super::{
    DirectoryEntry, EntryKind, MAX_PAGE_BYTES, check_size, extension, is_image, read_bounded,
};
use std::{
    fs::{self, File},
    io::Cursor,
    path::{Path, PathBuf},
};
use zip::{ZipArchive, read::ArchiveOffset, read::Config};

// These limits apply before ZIP metadata parsing or folder collection. Unsupported
// layouts return a reason for the caller's format icon; they do not open a Book.
const MAX_COVER_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_COVER_ENTRIES: usize = 4096;
const MAX_COVER_METADATA_BYTES: usize = 2 * 1024 * 1024;
const ZIP_LAYOUT_ERROR: &str =
    "Automatic ZIP/CBZ cover is unavailable for this layout. Open the book manually.";
const ZIP64_ERROR: &str =
    "Automatic ZIP64 cover is unavailable: index allocation cannot be safely bounded.";

#[derive(Debug)]
pub struct CoverSource {
    pub bytes: Vec<u8>,
    /// Images are 1; ZIP images use the complete bounded index. Folders are unknown.
    pub page_count: Option<usize>,
}

/// Reads encoded bytes only. The caller decodes the first image and must not
/// retry later pages when decoding fails. This function never writes to a source.
pub fn read_cover(entry: &DirectoryEntry) -> Result<CoverSource, String> {
    match entry.kind {
        EntryKind::Image => Ok(CoverSource {
            bytes: read_image(&entry.path)?,
            page_count: Some(1),
        }),
        EntryKind::Folder => read_folder(&entry.path),
        EntryKind::Archive => match extension(&entry.path).as_str() {
            "zip" | "cbz" => read_zip(&entry.path),
            "rar" | "cbr" => Err(
                "Automatic RAR/CBR cover is unavailable: index and solid-prefix work cannot be safely bounded. Open the book manually."
                    .into(),
            ),
            _ => Err("Unsupported cover format.".into()),
        },
    }
}

fn open_regular(path: &Path) -> Result<(File, fs::Metadata), String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NONBLOCK prevents a replaced FIFO from waiting for a writer before
        // handle validation. These are the supported Unix platform ABI values.
        let nonblock = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            0x0800
        } else if cfg!(target_os = "macos") {
            0x0004
        } else {
            return Err("Automatic cover reads are unsupported on this Unix platform.".into());
        };
        options.custom_flags(nonblock);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    // Use the opened handle, not earlier browser metadata or a second path lookup.
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("The cover source is no longer a regular file.".into());
    }
    Ok((file, metadata))
}

fn read_image(path: &Path) -> Result<Vec<u8>, String> {
    let (file, metadata) = open_regular(path)?;
    check_size(metadata.len())?;
    read_bounded(file, MAX_PAGE_BYTES as usize)
}

fn read_folder(path: &Path) -> Result<CoverSource, String> {
    // Keep only the minimum; never collect a directory's entire contents.
    let mut first: Option<(String, PathBuf)> = None;
    let mut metadata_bytes = 0usize;
    for (index, item) in fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .enumerate()
    {
        if index >= MAX_COVER_ENTRIES {
            return Err("Automatic folder cover exceeds the 4096-entry safety limit.".into());
        }
        let item = item.map_err(|error| error.to_string())?;
        let name = item.file_name();
        metadata_bytes = metadata_bytes.saturating_add(name.as_encoded_bytes().len());
        if metadata_bytes > MAX_COVER_METADATA_BYTES {
            return Err("Automatic folder cover exceeds the 2 MiB metadata safety limit.".into());
        }
        let child = item.path();
        if !is_image(&child) {
            continue;
        }
        let metadata = fs::metadata(&child).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            continue;
        }
        let name = name.to_string_lossy().into_owned();
        let earlier = first.as_ref().is_none_or(|(other_name, other_path)| {
            natord::compare_ignore_case(&name, other_name)
                .then_with(|| name.cmp(other_name))
                .then_with(|| child.cmp(other_path))
                .is_lt()
        });
        if earlier {
            first = Some((name, child));
        }
    }
    let (_, first) = first.ok_or("No supported images in the first folder level.")?;
    Ok(CoverSource {
        bytes: read_image(&first)?,
        page_count: None,
    })
}

fn read_zip(path: &Path) -> Result<CoverSource, String> {
    let (file, metadata) = open_regular(path)?;
    if metadata.len() > MAX_COVER_SOURCE_BYTES as u64 {
        return Err("Automatic ZIP/CBZ cover exceeds the 64 MiB source safety limit.".into());
    }
    // Snapshot the same handle with a bounded read. A replacement of the path
    // cannot change parser input after preflight. In-place changes can produce
    // invalid bytes, but cannot bypass preflight or increase its allocation caps.
    let bytes = read_bounded(file, MAX_COVER_SOURCE_BYTES)?;
    if bytes.len() as u64 != metadata.len() {
        return Err("ZIP/CBZ source changed while reading the cover.".into());
    }
    read_zip_bytes(bytes)
}

fn read_zip_bytes(bytes: Vec<u8>) -> Result<CoverSource, String> {
    let entry_count = preflight_zip(&bytes)?;
    // zip 8.6 allocates its index from EOCD before parsing entries, and can
    // retry earlier EOCD records after a parse error. Preflight permits exactly
    // one EOCD, no ZIP64 records, bounded central data, and no offset guessing.
    let mut archive = ZipArchive::with_config(
        Config {
            archive_offset: ArchiveOffset::Known(0),
        },
        Cursor::new(bytes),
    )
    .map_err(|error| format!("Cannot open ZIP/CBZ cover: {error}"))?;
    if archive.len() != entry_count {
        return Err("Automatic ZIP/CBZ cover does not support duplicate entry names.".into());
    }
    let mut first: Option<(String, usize, Vec<u8>)> = None;
    let mut page_count = 0;
    for index in 0..archive.len() {
        // Raw access does not construct a decoder for every indexed page.
        let entry = archive
            .by_index_raw(index)
            .map_err(|error| format!("Cannot inspect ZIP/CBZ cover entry: {error}"))?;
        let name = entry.name().replace('\\', "/");
        if entry.is_dir() || !is_image(Path::new(&name)) {
            continue;
        }
        check_size(entry.size())?;
        page_count += 1;
        if first.as_ref().is_none_or(|(other, _, _)| {
            natord::compare_ignore_case(&name, other)
                .then_with(|| name.cmp(other))
                .is_lt()
        }) {
            first = Some((name, index, entry.name_raw().to_vec()));
        }
    }
    let (_, index, expected_name) = first.ok_or("No supported images in this archive.")?;
    let entry = archive
        .by_index(index)
        .map_err(|error| format!("Cannot read ZIP/CBZ cover: {error}"))?;
    if entry.is_dir() || entry.name_raw() != expected_name {
        return Err("ZIP/CBZ cover page has changed. Open the book again.".into());
    }
    check_size(entry.size())?;
    Ok(CoverSource {
        // Uses zip's CRC reader and the existing actual-output byte limit.
        bytes: read_bounded(entry, MAX_PAGE_BYTES as usize)?,
        page_count: Some(page_count),
    })
}

fn u16_at(bytes: &[u8], at: usize) -> usize {
    u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize
}

fn u32_at(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("checked ZIP header")) as usize
}

fn check_extra(mut extra: &[u8]) -> Result<(), String> {
    while !extra.is_empty() {
        if extra.len() < 4 {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        match u16_at(extra, 0) {
            1 => return Err(ZIP64_ERROR.into()),
            // AES extra data can override the header's compression method.
            0x9901 => return Err("Encrypted ZIP/CBZ covers are not supported.".into()),
            _ => {}
        }
        let end = 4 + u16_at(extra, 2);
        if end > extra.len() {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        extra = &extra[end..];
    }
    Ok(())
}

fn preflight_zip(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() > MAX_COVER_SOURCE_BYTES {
        return Err("Automatic ZIP/CBZ cover exceeds the 64 MiB source safety limit.".into());
    }
    let mut eocd = None;
    // Conservative: signatures embedded in payload/comments also cause fallback.
    // This prevents zip's alternate-footer and ZIP64 allocation paths entirely.
    for (at, signature) in bytes.windows(4).enumerate() {
        if signature == b"PK\x06\x06" || signature == b"PK\x06\x07" {
            return Err(ZIP64_ERROR.into());
        }
        if signature == b"PK\x05\x06" && eocd.replace(at).is_some() {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
    }
    let end = eocd.ok_or("Cannot open ZIP/CBZ cover: missing end record.")?;
    let footer = bytes.get(end..).ok_or(ZIP_LAYOUT_ERROR)?;
    if footer.len() < 22 || footer.len() != 22 + u16_at(footer, 20) {
        return Err(ZIP_LAYOUT_ERROR.into());
    }
    let count = u16_at(footer, 10);
    let size = u32_at(footer, 12);
    let start = u32_at(footer, 16);
    if count == u16::MAX as usize || size == u32::MAX as usize || start == u32::MAX as usize {
        return Err(ZIP64_ERROR.into());
    }
    if count > MAX_COVER_ENTRIES {
        return Err("Automatic ZIP/CBZ cover exceeds the 4096-entry safety limit.".into());
    }
    if size > MAX_COVER_METADATA_BYTES {
        return Err("Automatic ZIP/CBZ cover exceeds the 2 MiB metadata safety limit.".into());
    }
    if u16_at(footer, 4) != 0
        || u16_at(footer, 6) != 0
        || u16_at(footer, 8) != count
        || start.checked_add(size) != Some(end)
    {
        return Err(ZIP_LAYOUT_ERROR.into());
    }
    let mut central = bytes.get(start..end).ok_or(ZIP_LAYOUT_ERROR)?;
    let mut metadata_bytes = size;
    for _ in 0..count {
        if central.len() < 46 || &central[..4] != b"PK\x01\x02" {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        let flags = u16_at(central, 8);
        if flags & (1 | 64 | 8192) != 0 {
            return Err("Encrypted ZIP/CBZ covers are not supported.".into());
        }
        // Stored and Deflate have fixed, small decoder working memory. Other
        // methods can declare large dictionaries before the output limit fires.
        if !matches!(u16_at(central, 10), 0 | 8) {
            return Err(
                "Automatic ZIP/CBZ cover supports only Stored and Deflated entries.".into(),
            );
        }
        if u32_at(central, 20) == u32::MAX as usize
            || u32_at(central, 24) == u32::MAX as usize
            || u32_at(central, 42) == u32::MAX as usize
        {
            return Err(ZIP64_ERROR.into());
        }
        if u16_at(central, 34) != 0 {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        let name_end = 46 + u16_at(central, 28);
        let extra_end = name_end + u16_at(central, 30);
        let record_end = extra_end + u16_at(central, 32);
        if record_end > central.len() {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        check_extra(&central[name_end..extra_end])?;
        // Validate the local range too. No allocation or decoder is needed.
        let offset = u32_at(central, 42);
        let local = bytes.get(offset..start).ok_or(ZIP_LAYOUT_ERROR)?;
        if local.len() < 30 || &local[..4] != b"PK\x03\x04" {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        let local_name_end = 30 + u16_at(local, 26);
        let data_start = local_name_end + u16_at(local, 28);
        if data_start > local.len()
            || u32_at(central, 20) > local.len() - data_start
            || u16_at(local, 6) != flags
            || u16_at(local, 8) != u16_at(central, 10)
            || local.get(30..local_name_end) != Some(&central[46..name_end])
        {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        metadata_bytes = metadata_bytes.saturating_add(data_start);
        if metadata_bytes > MAX_COVER_METADATA_BYTES {
            return Err("Automatic ZIP/CBZ cover exceeds the 2 MiB metadata safety limit.".into());
        }
        check_extra(&local[local_name_end..data_start])?;
        central = &central[record_end..];
    }
    if !central.is_empty() {
        return Err(ZIP_LAYOUT_ERROR.into());
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::{Book, browse_directory};
    use std::io::{Seek, SeekFrom, Write};
    use tempfile::tempdir;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    fn listed(path: &Path) -> DirectoryEntry {
        browse_directory(path.parent().unwrap())
            .unwrap()
            .entries
            .into_iter()
            .find(|entry| entry.entry_path.file_name() == path.file_name())
            .unwrap()
    }

    fn zip_bytes(entries: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(method);
        zip.add_directory("directory.jpg/", options).unwrap();
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn central(bytes: &[u8]) -> usize {
        bytes
            .windows(4)
            .position(|part| part == b"PK\x01\x02")
            .unwrap()
    }

    fn footer(bytes: &[u8]) -> usize {
        bytes
            .windows(4)
            .rposition(|part| part == b"PK\x05\x06")
            .unwrap()
    }

    fn error<T: std::fmt::Debug>(result: Result<T, String>, expected: &str) {
        let actual = result.unwrap_err();
        assert!(
            actual.contains(expected),
            "Expected {expected:?}, got {actual:?}"
        );
    }

    #[test]
    fn image_cover_reads_exact_file_not_sibling_page_zero() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("1.png"), b"first sibling").unwrap();
        let selected = temp.path().join("10.png");
        fs::write(&selected, b"selected bytes").unwrap();
        let source = read_cover(&listed(&selected)).unwrap();
        assert_eq!(source.bytes, b"selected bytes");
        assert_eq!(source.page_count, Some(1));
        assert_eq!(Book::open(&selected, false).unwrap().pages.len(), 2);
        assert_eq!(fs::read(selected).unwrap(), b"selected bytes");
    }

    #[test]
    fn image_cover_checks_current_handle_not_stale_listing_size() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("1.png");
        fs::write(&path, b"small").unwrap();
        let entry = listed(&path);
        let file = File::create(&path).unwrap();
        file.set_len(MAX_PAGE_BYTES + 1).unwrap();
        error(read_cover(&entry), "128 MiB");
        drop(file);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(read_cover(&entry).is_err());
    }

    #[test]
    fn folder_cover_is_shallow_natural_and_has_unknown_count() {
        let temp = tempdir().unwrap();
        let folder = temp.path().join("folder");
        fs::create_dir_all(folder.join("nested")).unwrap();
        fs::write(folder.join("nested/1.png"), b"nested").unwrap();
        fs::write(folder.join("0.cbz"), b"not opened").unwrap();
        fs::write(folder.join("10.png"), b"ten").unwrap();
        fs::write(folder.join("2.PNG"), b"two").unwrap();
        let entry = listed(&folder);
        let source = read_cover(&entry).unwrap();
        assert_eq!(source.bytes, b"two");
        assert_eq!(source.page_count, None);
        fs::remove_file(folder.join("2.PNG")).unwrap();
        fs::remove_file(folder.join("10.png")).unwrap();
        error(read_cover(&entry), "first folder level");
    }

    #[test]
    fn folder_cover_does_not_skip_bad_first_image() {
        let temp = tempdir().unwrap();
        let folder = temp.path().join("folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("2.png"), b"readable later image").unwrap();
        let first = File::create(folder.join("1.png")).unwrap();
        first.set_len(MAX_PAGE_BYTES + 1).unwrap();
        error(read_cover(&listed(&folder)), "128 MiB");
        first.set_len(0).unwrap();
        // Invalid encoded bytes go to the decoder unchanged; no later image retry.
        assert!(read_cover(&listed(&folder)).unwrap().bytes.is_empty());
    }

    #[test]
    fn folder_cover_caps_all_children_including_unsupported_files() {
        let temp = tempdir().unwrap();
        let folder = temp.path().join("folder");
        fs::create_dir(&folder).unwrap();
        for index in 0..MAX_COVER_ENTRIES {
            File::create(folder.join(format!("{index}.txt"))).unwrap();
        }
        fs::write(folder.join("1.png"), b"image").unwrap();
        error(read_cover(&listed(&folder)), "4096-entry");
    }

    #[test]
    fn zip_cover_uses_natural_page_zero_and_complete_image_count() {
        for method in [CompressionMethod::Stored, CompressionMethod::Deflated] {
            let bytes = zip_bytes(
                &[
                    ("nested/10.png", b"ten"),
                    ("notes.txt", b"ignored"),
                    ("nested/2.PNG", b"two"),
                    ("20.gif", b"twenty"),
                ],
                method,
            );
            let source = read_zip_bytes(bytes).unwrap();
            assert_eq!(source.bytes, b"twenty");
            assert_eq!(source.page_count, Some(3));
            let source = read_zip_bytes(zip_bytes(
                &[("nested/10.png", b"ten"), ("nested/2.PNG", b"two")],
                method,
            ))
            .unwrap();
            assert_eq!(source.bytes, b"two");
            assert_eq!(source.page_count, Some(2));
        }
    }

    #[test]
    fn zip_cover_reads_real_source_without_writes_or_extraction() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("source.cbz");
        let bytes = zip_bytes(&[("../1.png", b"original")], CompressionMethod::Stored);
        fs::write(&path, &bytes).unwrap();
        assert_eq!(read_cover(&listed(&path)).unwrap().bytes, b"original");
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        assert!(!temp.path().join("1.png").exists());
    }

    #[test]
    fn zip_cover_snapshot_remains_bounded_after_path_replacement() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("source.cbz");
        let original = zip_bytes(&[("1.png", b"original")], CompressionMethod::Stored);
        fs::write(&path, &original).unwrap();
        let (file, _) = open_regular(&path).unwrap();
        let snapshot = read_bounded(file, MAX_COVER_SOURCE_BYTES).unwrap();
        fs::write(
            &path,
            zip_bytes(&[("1.png", b"replacement")], CompressionMethod::Stored),
        )
        .unwrap();
        assert_eq!(read_zip_bytes(snapshot).unwrap().bytes, b"original");
        assert_eq!(read_cover(&listed(&path)).unwrap().bytes, b"replacement");
    }

    #[test]
    fn zip_cover_empty_corrupt_and_crc_errors_do_not_retry() {
        let empty = ZipWriter::new(Cursor::new(Vec::new()))
            .finish()
            .unwrap()
            .into_inner();
        error(read_zip_bytes(empty), "No supported images");
        error(read_zip_bytes(Vec::new()), "missing end record");
        error(read_zip_bytes(b"broken".to_vec()), "missing end record");
        error(
            read_zip_bytes(zip_bytes(&[], CompressionMethod::Stored)),
            "No supported images",
        );
        let mut bytes = zip_bytes(
            &[("1.png", b"unique payload"), ("2.png", b"later")],
            CompressionMethod::Stored,
        );
        let offset = bytes
            .windows(14)
            .position(|part| part == b"unique payload")
            .unwrap();
        bytes[offset] ^= 1;
        assert!(read_zip_bytes(bytes).is_err());
    }

    #[test]
    fn zip_cover_rejects_encrypted_and_unsupported_decoder_entries() {
        let mut bytes = zip_bytes(&[("1.png", b"image")], CompressionMethod::Stored);
        let at = central(&bytes);
        bytes[at + 8] |= 1;
        error(read_zip_bytes(bytes), "Encrypted");
        error(
            read_zip_bytes(zip_bytes(&[("1.png", b"image")], CompressionMethod::Bzip2)),
            "Stored and Deflated",
        );
    }

    #[test]
    fn zip_cover_rejects_oversize_images_anywhere_in_index() {
        let mut bytes = zip_bytes(
            &[("1.png", b"first"), ("2.png", b"last")],
            CompressionMethod::Stored,
        );
        let at = bytes
            .windows(4)
            .rposition(|part| part == b"PK\x01\x02")
            .unwrap();
        bytes[at + 24..at + 28].copy_from_slice(&((MAX_PAGE_BYTES + 1) as u32).to_le_bytes());
        error(read_zip_bytes(bytes), "128 MiB");
    }

    #[test]
    fn zip_cover_preflight_rejects_entry_and_metadata_limits_before_parser() {
        let original = zip_bytes(&[("1.png", b"image")], CompressionMethod::Stored);
        let mut bytes = original.clone();
        let at = footer(&bytes);
        bytes[at + 10..at + 12].copy_from_slice(&((MAX_COVER_ENTRIES + 1) as u16).to_le_bytes());
        error(read_zip_bytes(bytes), "4096-entry");
        let mut bytes = original;
        bytes[at + 12..at + 16]
            .copy_from_slice(&((MAX_COVER_METADATA_BYTES + 1) as u32).to_le_bytes());
        error(read_zip_bytes(bytes), "2 MiB metadata");
    }

    #[test]
    fn zip_cover_rejects_oversize_source_before_snapshot_allocation() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("large.cbz");
        let mut file = File::create(&path).unwrap();
        file.seek(SeekFrom::Start(MAX_COVER_SOURCE_BYTES as u64))
            .unwrap();
        file.write_all(b"x").unwrap();
        error(read_cover(&listed(&path)), "64 MiB source");
    }

    #[test]
    fn zip64_and_alternate_footer_paths_have_explicit_fallbacks() {
        let original = zip_bytes(&[("1.png", b"image")], CompressionMethod::Stored);
        let mut bytes = original.clone();
        let at = footer(&bytes);
        bytes[at + 10..at + 12].copy_from_slice(&u16::MAX.to_le_bytes());
        error(read_zip_bytes(bytes), "ZIP64");
        for signature in [b"PK\x06\x06", b"PK\x06\x07"] {
            let bytes = zip_bytes(&[("1.png", signature)], CompressionMethod::Stored);
            error(read_zip_bytes(bytes), "ZIP64");
        }
        error(
            read_zip_bytes(zip_bytes(
                &[("1.png", b"PK\x05\x06")],
                CompressionMethod::Stored,
            )),
            "layout",
        );
        error(check_extra(&[1, 0, 0, 0]), "ZIP64");
        error(check_extra(&[1, 0x99, 0, 0]), "Encrypted");
        error(check_extra(&[2, 0, 8, 0]), "layout");
        let mut bytes = original;
        bytes.push(0);
        error(read_zip_bytes(bytes), "layout");
    }

    #[test]
    fn zip64_local_extra_falls_back_without_changing_manual_open() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("zip64.cbz");
        let mut zip = ZipWriter::new(File::create(&path).unwrap());
        zip.start_file(
            "1.png",
            SimpleFileOptions::default()
                .compression_method(CompressionMethod::Stored)
                .large_file(true),
        )
        .unwrap();
        zip.write_all(b"image").unwrap();
        zip.finish().unwrap();
        error(read_cover(&listed(&path)), "ZIP64");
        assert_eq!(
            Book::open(&path, false).unwrap().read_page(0).unwrap(),
            b"image"
        );
    }

    #[test]
    fn zip_cover_rejects_duplicate_names_instead_of_reporting_a_partial_count() {
        let mut bytes = zip_bytes(
            &[("1.png", b"one"), ("2.png", b"two")],
            CompressionMethod::Stored,
        );
        for at in 0..bytes.len().saturating_sub(4) {
            if &bytes[at..at + 5] == b"2.png" {
                bytes[at] = b'1';
            }
        }
        error(read_zip_bytes(bytes), "duplicate entry names");
    }

    #[test]
    fn zip_cover_rejects_local_name_changes_and_truncated_ranges() {
        let original = zip_bytes(&[("1.png", b"image")], CompressionMethod::Stored);
        let mut bytes = original.clone();
        // The first local entry is the writer's directory entry.
        bytes[30] ^= 1;
        error(read_zip_bytes(bytes), "layout");
        let mut bytes = original;
        let at = central(&bytes);
        bytes[at + 42..at + 46].copy_from_slice(&0x7fff_ffffu32.to_le_bytes());
        error(read_zip_bytes(bytes), "layout");
    }

    #[test]
    fn rar_cover_returns_reason_without_parsing_even_a_corrupt_archive() {
        let temp = tempdir().unwrap();
        for name in ["book.rar", "book.cbr"] {
            let path = temp.path().join(name);
            fs::write(&path, b"not parsed").unwrap();
            error(read_cover(&listed(&path)), "index and solid-prefix");
            assert_eq!(fs::read(path).unwrap(), b"not parsed");
        }
    }

    #[cfg(any(all(target_os = "linux", target_arch = "x86_64"), target_os = "macos"))]
    #[test]
    fn image_and_archive_covers_reject_fifo_replacements_without_a_writer() {
        use std::{process::Command, sync::mpsc, thread, time::Duration};
        let temp = tempdir().unwrap();
        for name in ["replaced.png", "replaced.cbz"] {
            let path = temp.path().join(name);
            fs::write(&path, b"previous regular file").unwrap();
            let entry = listed(&path);
            fs::remove_file(&path).unwrap();
            assert!(
                Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                _ = sender.send(read_cover(&entry));
            });
            let result = receiver
                .recv_timeout(Duration::from_secs(3))
                .expect("Cover open must not wait for a FIFO writer");
            error(result, "regular file");
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn image_cover_preserves_unicode_native_paths() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("comic-\u{1f4da}.png");
        fs::write(&path, b"unicode native image").unwrap();
        let entry = listed(&path);
        assert_eq!(entry.entry_path.file_name(), path.file_name());
        assert_eq!(read_cover(&entry).unwrap().bytes, b"unicode native image");
    }

    #[cfg(unix)]
    #[test]
    fn image_cover_preserves_non_utf8_native_paths() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        let temp = tempdir().unwrap();
        let path = temp.path().join(OsStr::from_bytes(b"native-\xff.png"));
        let lossy = PathBuf::from(path.to_string_lossy().into_owned());
        fs::write(&lossy, b"lossy neighbor").unwrap();
        assert_eq!(
            read_cover(&listed(&lossy)).unwrap().bytes,
            b"lossy neighbor"
        );
        match fs::write(&path, b"native image") {
            Ok(()) => {
                let entry = listed(&path);
                assert_eq!(entry.entry_path.file_name(), path.file_name());
                assert_eq!(read_cover(&entry).unwrap().bytes, b"native image");
            }
            Err(error) => {
                // APFS rejects this name with EILSEQ (92). The cover reader must
                // return that native-path error, not open the valid lossy neighbor.
                assert!(
                    cfg!(target_os = "macos") && error.raw_os_error() == Some(92),
                    "{error}"
                );
                let mut entry = listed(&lossy);
                entry.path = path.clone();
                entry.entry_path = path;
                assert!(read_cover(&entry).is_err());
            }
        }
    }
}
