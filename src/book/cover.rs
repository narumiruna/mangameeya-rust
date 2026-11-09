//! Bounded automatic cover reads. Manual `Book::open` retains its existing limits.

use super::{
    DirectoryEntry, EntryKind, MAX_PAGE_BYTES, check_size, extension, is_image, read_bounded,
};
use std::{
    collections::HashSet,
    fmt,
    fs::{self, File},
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use zip::{ZipArchive, read::ArchiveOffset, read::Config};

// These limits apply before ZIP metadata parsing or folder collection. Unsupported
// layouts return a reason for the caller's format icon; they do not open a Book.
// This is a selected compressed-member limit, not a total archive-size limit.
const MAX_COVER_COMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const MAX_ZIP_FOOTER_BYTES: usize = 22 + u16::MAX as usize;
const MAX_COVER_ENTRIES: usize = 4096;
const MAX_COVER_METADATA_BYTES: usize = 2 * 1024 * 1024;
const ZIP_LAYOUT_ERROR: &str =
    "Automatic ZIP/CBZ cover is unavailable for this layout. Open the book manually.";
const ZIP64_ERROR: &str =
    "Automatic ZIP64 cover is unavailable: index allocation cannot be safely bounded.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CoverError {
    Unsupported(String),
    Failed(String),
}

impl fmt::Display for CoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(reason) | Self::Failed(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for CoverError {}

impl From<String> for CoverError {
    fn from(reason: String) -> Self {
        Self::Failed(reason)
    }
}

impl From<&str> for CoverError {
    fn from(reason: &str) -> Self {
        Self::Failed(reason.into())
    }
}

impl From<std::io::Error> for CoverError {
    fn from(error: std::io::Error) -> Self {
        Self::Failed(format!("Cannot read cover: {error}"))
    }
}

fn unsupported(reason: &str) -> CoverError {
    CoverError::Unsupported(reason.into())
}

#[derive(Debug)]
pub struct CoverSource {
    pub bytes: Vec<u8>,
    /// Images are 1; ZIP images use the complete bounded index. Folders are unknown.
    pub page_count: Option<usize>,
}

/// Reads encoded bytes only. The caller decodes the first image and must not
/// retry later pages when decoding fails. This function never writes to a source.
pub fn read_cover(entry: &DirectoryEntry) -> Result<CoverSource, String> {
    read_cover_classified(entry).map_err(|error| error.to_string())
}

pub(crate) fn read_cover_classified(entry: &DirectoryEntry) -> Result<CoverSource, CoverError> {
    match entry.kind {
        EntryKind::Image => Ok(CoverSource {
            bytes: read_image(&entry.path)?,
            page_count: Some(1),
        }),
        EntryKind::Folder => read_folder(&entry.path),
        EntryKind::Archive => match extension(&entry.path).as_str() {
            "zip" | "cbz" => read_zip(&entry.path),
            "rar" | "cbr" => Err(unsupported(
                "Automatic RAR/CBR cover is unavailable: index and solid-prefix work cannot be safely bounded. Open the book manually.",
            )),
            _ => Err(unsupported("Unsupported cover format.")),
        },
    }
}

fn open_regular(path: &Path) -> Result<(File, fs::Metadata), CoverError> {
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
            return Err(unsupported(
                "Automatic cover reads are unsupported on this Unix platform.",
            ));
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

fn read_image(path: &Path) -> Result<Vec<u8>, CoverError> {
    let (file, metadata) = open_regular(path)?;
    check_size(metadata.len()).map_err(CoverError::Unsupported)?;
    read_bounded(file, MAX_PAGE_BYTES as usize).map_err(CoverError::Failed)
}

fn read_folder(path: &Path) -> Result<CoverSource, CoverError> {
    // Keep only the minimum; never collect a directory's entire contents.
    let mut first: Option<(String, PathBuf)> = None;
    let mut metadata_bytes = 0usize;
    for (index, item) in fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .enumerate()
    {
        if index >= MAX_COVER_ENTRIES {
            return Err(unsupported(
                "Automatic folder cover exceeds the 4096-entry safety limit.",
            ));
        }
        let item = item.map_err(|error| error.to_string())?;
        let name = item.file_name();
        metadata_bytes = metadata_bytes.saturating_add(name.as_encoded_bytes().len());
        if metadata_bytes > MAX_COVER_METADATA_BYTES {
            return Err(unsupported(
                "Automatic folder cover exceeds the 2 MiB metadata safety limit.",
            ));
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
    let (_, first) =
        first.ok_or_else(|| unsupported("No supported images in the first folder level."))?;
    Ok(CoverSource {
        bytes: read_image(&first)?,
        page_count: None,
    })
}

fn read_zip(path: &Path) -> Result<CoverSource, CoverError> {
    let (mut file, before) = open_regular(path)?;
    let selected = snapshot_zip(&mut file, before.len())?;
    // This check uses the same handle. Path replacement cannot redirect reads.
    // Timestamp checks detect ordinary in-place edits, not an atomic filesystem
    // snapshot. Even undetected edits cannot change validated parser metadata or
    // the bounded immutable compressed input; its output must pass CRC and size.
    if !same_source(&before, &file.metadata()?) {
        return Err("ZIP/CBZ source changed while reading the cover.".into());
    }
    decode_zip(selected)
}

fn same_source(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.ctime() != after.ctime() || before.ctime_nsec() != after.ctime_nsec() {
            return false;
        }
    }
    true
}

fn u16_at(bytes: &[u8], at: usize) -> usize {
    u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize
}

fn u32_at(bytes: &[u8], at: usize) -> usize {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("checked ZIP header")) as usize
}

fn put_u16(bytes: &mut [u8], at: usize, value: usize) {
    bytes[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes());
}

fn put_u32(bytes: &mut [u8], at: usize, value: usize) {
    bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
}

fn check_extra(mut extra: &[u8]) -> Result<(), CoverError> {
    while !extra.is_empty() {
        if extra.len() < 4 {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        match u16_at(extra, 0) {
            1 => return Err(unsupported(ZIP64_ERROR)),
            // AES extra data can override the header's compression method.
            0x9901 => return Err(unsupported("Encrypted ZIP/CBZ covers are not supported.")),
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

fn check_flags_method(flags: usize, method: usize) -> Result<(), CoverError> {
    if flags & (1 | 64 | 8192) != 0 {
        return Err(unsupported("Encrypted ZIP/CBZ covers are not supported."));
    }
    // Only Deflate options, data descriptors, and UTF-8 names are supported.
    if flags & !(2 | 4 | 8 | 2048) != 0 {
        return Err(unsupported(ZIP_LAYOUT_ERROR));
    }
    // Both decoders have fixed, small working memory. Other methods can declare
    // a large dictionary before an actual-output byte limit can stop them.
    if !matches!(method, 0 | 8) {
        return Err(unsupported(
            "Automatic ZIP/CBZ cover supports only Stored and Deflated entries.",
        ));
    }
    Ok(())
}

fn metadata_limit(bytes: usize) -> Result<(), CoverError> {
    if bytes > MAX_COVER_METADATA_BYTES {
        return Err(unsupported(
            "Automatic ZIP/CBZ cover exceeds the 2 MiB metadata safety limit.",
        ));
    }
    Ok(())
}

fn read_at(
    reader: &mut (impl Read + Seek),
    file_len: u64,
    offset: u64,
    bytes: &mut [u8],
) -> Result<(), CoverError> {
    if offset
        .checked_add(bytes.len() as u64)
        .is_none_or(|end| end > file_len)
    {
        return Err(ZIP_LAYOUT_ERROR.into());
    }
    reader.seek(SeekFrom::Start(offset))?;
    reader.read_exact(bytes)?;
    Ok(())
}

fn read_range(
    reader: &mut (impl Read + Seek),
    file_len: u64,
    offset: u64,
    length: usize,
    limit: usize,
) -> Result<Vec<u8>, CoverError> {
    // Check the range and allocation budget before reserving any input bytes.
    if length > limit
        || offset
            .checked_add(length as u64)
            .is_none_or(|end| end > file_len)
    {
        return Err(ZIP_LAYOUT_ERROR.into());
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|error| error.to_string())?;
    bytes.resize(length, 0);
    read_at(reader, file_len, offset, &mut bytes)?;
    Ok(bytes)
}

struct ZipIndex {
    central: Vec<u8>,
    start: u64,
    count: usize,
    metadata_bytes: usize,
}

fn read_index(reader: &mut (impl Read + Seek), file_len: u64) -> Result<ZipIndex, CoverError> {
    // Classic ZIP comments are at most 65535 bytes. Never scan the whole file.
    let tail_len = file_len.min(MAX_ZIP_FOOTER_BYTES as u64) as usize;
    let tail_start = file_len - tail_len as u64;
    let tail = read_range(reader, file_len, tail_start, tail_len, MAX_ZIP_FOOTER_BYTES)?;
    let mut found = None;
    let mut saw_signature = false;
    for (at, signature) in tail.windows(4).enumerate() {
        if signature != b"PK\x05\x06" {
            continue;
        }
        saw_signature = true;
        // Only end-anchored footers are candidates. An invalid candidate is
        // never a reason to retry an earlier, less constrained archive layout.
        if tail.len() - at >= 22
            && at + 22 + u16_at(&tail[at..], 20) == tail.len()
            && found.replace(at).is_some()
        {
            return Err(unsupported(ZIP_LAYOUT_ERROR));
        }
    }
    let end = found.ok_or_else(|| {
        CoverError::Failed(if saw_signature {
            ZIP_LAYOUT_ERROR.into()
        } else {
            "Cannot open ZIP/CBZ cover: missing end record.".into()
        })
    })?;
    let mut locator_signature = [0; 4];
    if let Some(at) = end.checked_sub(20) {
        locator_signature.copy_from_slice(&tail[at..at + 4]);
    } else if let Some(at) = (tail_start + end as u64).checked_sub(20) {
        // A maximal comment can put the locator just outside the tail window.
        // Read only its fixed signature, never ZIP64 lengths or index records.
        read_at(reader, file_len, at, &mut locator_signature)?;
    }
    if &locator_signature == b"PK\x06\x07" {
        return Err(unsupported(ZIP64_ERROR));
    }
    let footer = &tail[end..];
    let count = u16_at(footer, 10);
    let size = u32_at(footer, 12);
    let start = u32_at(footer, 16) as u64;
    if count == u16::MAX as usize
        || u16_at(footer, 8) == u16::MAX as usize
        || size == u32::MAX as usize
        || start == u32::MAX as u64
    {
        return Err(unsupported(ZIP64_ERROR));
    }
    if count > MAX_COVER_ENTRIES {
        return Err(unsupported(
            "Automatic ZIP/CBZ cover exceeds the 4096-entry safety limit.",
        ));
    }
    // Include the real footer/comment and every local header in the same budget.
    let metadata_bytes = size + footer.len();
    metadata_limit(metadata_bytes)?;
    if u16_at(footer, 4) != 0 || u16_at(footer, 6) != 0 {
        return Err(unsupported(
            "Automatic multi-disk ZIP/CBZ covers are not supported.",
        ));
    }
    if u16_at(footer, 8) != count || start.checked_add(size as u64) != Some(tail_start + end as u64)
    {
        return Err(ZIP_LAYOUT_ERROR.into());
    }
    let central = read_range(reader, file_len, start, size, MAX_COVER_METADATA_BYTES)?;
    Ok(ZipIndex {
        central,
        start,
        count,
        metadata_bytes,
    })
}

#[derive(Debug)]
struct ZipMember {
    local_start: u64,
    data_start: u64,
    compressed: usize,
    size: usize,
    crc: usize,
    method: usize,
}

struct SelectedZip {
    archive: Vec<u8>,
    page_count: usize,
    size: usize,
}

fn snapshot_zip(reader: &mut (impl Read + Seek), file_len: u64) -> Result<SelectedZip, CoverError> {
    let mut index = read_index(reader, file_len)?;
    let mut members = Vec::with_capacity(index.count);
    let mut names = HashSet::with_capacity(index.count);
    let mut at = 0;
    for _ in 0..index.count {
        let central = index.central.get(at..).ok_or(ZIP_LAYOUT_ERROR)?;
        if central.len() < 46 || &central[..4] != b"PK\x01\x02" {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        let flags = u16_at(central, 8);
        let method = u16_at(central, 10);
        check_flags_method(flags, method)?;
        let compressed = u32_at(central, 20);
        let size = u32_at(central, 24);
        let offset = u32_at(central, 42) as u64;
        if compressed == u32::MAX as usize || size == u32::MAX as usize || offset == u32::MAX as u64
        {
            return Err(unsupported(ZIP64_ERROR));
        }
        if u16_at(central, 34) != 0 {
            return Err(unsupported(
                "Automatic multi-disk ZIP/CBZ covers are not supported.",
            ));
        }
        let name_end = 46 + u16_at(central, 28);
        let extra_end = name_end + u16_at(central, 30);
        let record_end = extra_end + u16_at(central, 32);
        if record_end > central.len() {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        check_extra(&central[name_end..extra_end])?;
        if !names.insert(&central[46..name_end]) {
            return Err(unsupported(
                "Automatic ZIP/CBZ cover does not support duplicate entry names.",
            ));
        }

        // Read each fixed header first. Its variable lengths must fit the combined
        // metadata budget and the pre-index range before allocation or further I/O.
        let mut local = [0; 30];
        read_at(reader, index.start, offset, &mut local)?;
        if &local[..4] != b"PK\x03\x04" {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        let local_name_len = u16_at(&local, 26);
        let local_len = local_name_len + u16_at(&local, 28);
        index.metadata_bytes += 30 + local_len;
        metadata_limit(index.metadata_bytes)?;
        if u32_at(&local, 18) == u32::MAX as usize || u32_at(&local, 22) == u32::MAX as usize {
            return Err(unsupported(ZIP64_ERROR));
        }
        let local_data = read_range(
            reader,
            index.start,
            offset + 30,
            local_len,
            MAX_COVER_METADATA_BYTES,
        )?;
        check_extra(&local_data[local_name_len..])?;
        let data_start = offset + 30 + local_len as u64;
        if data_start
            .checked_add(compressed as u64)
            .is_none_or(|end| end > index.start)
            || u16_at(&local, 6) != flags
            || u16_at(&local, 8) != method
            || local_data[..local_name_len] != central[46..name_end]
        {
            return Err(ZIP_LAYOUT_ERROR.into());
        }
        // A streaming writer may leave these fields zero and put them in a data
        // descriptor. The central CRC/size remain authoritative in that case.
        for (local_at, expected) in [(14, u32_at(central, 16)), (18, compressed), (22, size)] {
            let actual = u32_at(&local, local_at);
            if actual != expected && !(flags & 8 != 0 && actual == 0) {
                return Err(ZIP_LAYOUT_ERROR.into());
            }
        }
        members.push(ZipMember {
            local_start: offset,
            data_start,
            compressed,
            size,
            crc: u32_at(central, 16),
            method,
        });
        at += record_end;
    }
    if at != index.central.len() {
        return Err(ZIP_LAYOUT_ERROR.into());
    }
    drop(names);
    // An entry must not reuse another entry's header or compressed data range.
    // This list has at most the already checked 4096 records.
    let mut ranges: Vec<_> = members
        .iter()
        .map(|member| {
            (
                member.local_start,
                member.data_start + member.compressed as u64,
            )
        })
        .collect();
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(ZIP_LAYOUT_ERROR.into());
    }

    // zip 8.6 allocates its index from EOCD and retries earlier EOCD records on
    // error. Give it ONLY an immutable, bounded metadata archive, with a generated
    // footer and no alternate footer/ZIP64 signatures. No live handle is exposed.
    // This also preserves zip's CP437/UTF-8/Unicode-extra name semantics.
    if index
        .central
        .windows(4)
        .any(|part| part == b"PK\x05\x06" || part == b"PK\x06\x06" || part == b"PK\x06\x07")
    {
        return Err(unsupported(ZIP_LAYOUT_ERROR));
    }
    append_footer(&mut index.central, index.count, at, 0);
    let archive = ZipArchive::with_config(
        Config {
            archive_offset: ArchiveOffset::Known(0),
        },
        Cursor::new(index.central),
    )
    .map_err(|error| format!("Cannot inspect ZIP/CBZ cover index: {error}"))?;
    if archive.len() != members.len() {
        return Err(unsupported(
            "Automatic ZIP/CBZ cover does not support duplicate entry names.",
        ));
    }
    let mut first: Option<(String, usize)> = None;
    let mut page_count = 0;
    for (number, name) in archive.file_names().enumerate() {
        let name = name.replace('\\', "/");
        if name.ends_with('/') || !is_image(Path::new(&name)) {
            continue;
        }
        check_size(members[number].size as u64).map_err(CoverError::Unsupported)?;
        page_count += 1;
        if first.as_ref().is_none_or(|(other, _)| {
            natord::compare_ignore_case(&name, other)
                .then_with(|| name.cmp(other))
                .is_lt()
        }) {
            first = Some((name, number));
        }
    }
    let (_, number) = first.ok_or_else(|| unsupported("No supported images in this archive."))?;
    let member = &members[number];
    if member.compressed > MAX_COVER_COMPRESSED_BYTES {
        return Err(unsupported(
            "Automatic ZIP/CBZ cover exceeds the 64 MiB compressed-entry safety limit.",
        ));
    }
    // The decoder sees exactly one generated member, no source names, extra fields,
    // comments, flags, offsets or footer. Only its compressed bytes are copied.
    // The generated central header cannot fail and trigger an alternate-footer
    // retry into signatures that happen to occur inside the compressed payload.
    let mut snapshot = vec![0; 31];
    snapshot[..4].copy_from_slice(b"PK\x03\x04");
    put_u16(&mut snapshot, 4, 20);
    put_u16(&mut snapshot, 8, member.method);
    put_u32(&mut snapshot, 14, member.crc);
    put_u32(&mut snapshot, 18, member.compressed);
    put_u32(&mut snapshot, 22, member.size);
    put_u16(&mut snapshot, 26, 1);
    snapshot[30] = b'c';
    snapshot
        .try_reserve_exact(member.compressed + 47 + 22)
        .map_err(|error| error.to_string())?;
    snapshot.resize(31 + member.compressed, 0);
    read_at(reader, file_len, member.data_start, &mut snapshot[31..])?;
    let central_start = snapshot.len();
    snapshot.resize(central_start + 47, 0);
    let central = &mut snapshot[central_start..];
    central[..4].copy_from_slice(b"PK\x01\x02");
    put_u16(central, 4, 20);
    put_u16(central, 6, 20);
    put_u16(central, 10, member.method);
    put_u32(central, 16, member.crc);
    put_u32(central, 20, member.compressed);
    put_u32(central, 24, member.size);
    put_u16(central, 28, 1);
    central[46] = b'c';
    append_footer(&mut snapshot, 1, 47, central_start);
    Ok(SelectedZip {
        archive: snapshot,
        page_count,
        size: member.size,
    })
}

fn append_footer(bytes: &mut Vec<u8>, count: usize, size: usize, start: usize) {
    let at = bytes.len();
    bytes.resize(at + 22, 0);
    let footer = &mut bytes[at..];
    footer[..4].copy_from_slice(b"PK\x05\x06");
    put_u16(footer, 8, count);
    put_u16(footer, 10, count);
    put_u32(footer, 12, size);
    put_u32(footer, 16, start);
}

fn decode_zip(selected: SelectedZip) -> Result<CoverSource, CoverError> {
    let mut archive = ZipArchive::with_config(
        Config {
            archive_offset: ArchiveOffset::Known(0),
        },
        Cursor::new(selected.archive),
    )
    .map_err(|error| format!("Cannot open ZIP/CBZ cover: {error}"))?;
    let entry = archive
        .by_index(0)
        .map_err(|error| format!("Cannot read ZIP/CBZ cover: {error}"))?;
    let bytes = read_bounded(entry, MAX_PAGE_BYTES as usize)?;
    if bytes.len() != selected.size {
        return Err("ZIP/CBZ cover does not match its declared output size.".into());
    }
    Ok(CoverSource {
        bytes,
        page_count: Some(selected.page_count),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::{Book, browse_directory};
    use std::io::{self, Write};
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

    fn error<T: fmt::Debug, E: fmt::Debug + fmt::Display>(result: Result<T, E>, expected: &str) {
        let actual = result.unwrap_err();
        assert!(
            actual.to_string().contains(expected),
            "Expected {expected:?}, got {actual:?}"
        );
    }

    fn read_zip_bytes(bytes: Vec<u8>) -> Result<CoverSource, CoverError> {
        let length = bytes.len() as u64;
        decode_zip(snapshot_zip(&mut Cursor::new(bytes), length)?)
    }

    struct Counting<R> {
        inner: R,
        bytes: usize,
        reads: usize,
        max_request: usize,
    }

    impl<R> Counting<R> {
        fn new(inner: R) -> Self {
            Self {
                inner,
                bytes: 0,
                reads: 0,
                max_request: 0,
            }
        }
    }

    impl<R: Read> Read for Counting<R> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.max_request = self.max_request.max(bytes.len());
            self.reads += 1;
            let read = self.inner.read(bytes)?;
            self.bytes += read;
            Ok(read)
        }
    }

    impl<R: Seek> Seek for Counting<R> {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.inner.seek(from)
        }
    }

    struct Changing {
        inner: Cursor<Vec<u8>>,
        after_reads: usize,
        reads: usize,
        change: fn(&mut Vec<u8>),
    }

    impl Read for Changing {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let read = self.inner.read(bytes)?;
            self.reads += 1;
            if self.reads == self.after_reads {
                (self.change)(self.inner.get_mut());
            }
            Ok(read)
        }
    }

    impl Seek for Changing {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.inner.seek(from)
        }
    }

    #[test]
    fn actual_large_classic_zip_decodes_cover_with_bounded_io() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("large.cbz");
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            3,
            image::Rgba([24, 80, 190, 255]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file("notes.bin", stored).unwrap();
        let padding = MAX_COVER_COMPRESSED_BYTES as u64 + 1024;
        io::copy(&mut io::repeat(42).take(padding), &mut writer).unwrap();
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        writer.start_file("10.png", deflated).unwrap();
        writer
            .write_all(b"later image must not be decoded")
            .unwrap();
        writer.start_file("2.png", deflated).unwrap();
        writer.write_all(png.get_ref()).unwrap();
        writer.finish().unwrap();
        let (mut file, metadata) = open_regular(&path).unwrap();
        assert!(metadata.len() > 64 * 1024 * 1024);
        let mut counted = Counting::new(&mut file);
        let selected = snapshot_zip(&mut counted, metadata.len()).unwrap();
        assert!(counted.bytes < MAX_ZIP_FOOTER_BYTES + 1024);
        assert!(counted.reads <= 9);
        assert!(counted.max_request <= MAX_ZIP_FOOTER_BYTES);
        eprintln!(
            "large classic ZIP: {} source bytes, {} cover-read bytes, {} reads",
            metadata.len(),
            counted.bytes,
            counted.reads
        );
        let source = decode_zip(selected).unwrap();
        assert_eq!(source.page_count, Some(2));
        assert_eq!(source.bytes, png.into_inner());
        let public_source = read_cover(&listed(&path)).unwrap();
        assert_eq!(public_source.bytes, source.bytes);
        let decoded = crate::imaging::decode_thumbnail(&public_source.bytes, 140, 180).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 3));
        assert_eq!(decoded.pixels, [24, 80, 190, 255].repeat(6));
        // This is a real valid archive, not a sparse prefix or trailing-junk fixture.
        let mut archive = ZipArchive::new(File::open(&path).unwrap()).unwrap();
        assert_eq!(
            io::copy(&mut archive.by_name("notes.bin").unwrap(), &mut io::sink()).unwrap(),
            padding
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), metadata.len());
    }

    #[test]
    fn footer_limits_stop_before_index_io_and_allocation() {
        let original = zip_bytes(&[("1.png", b"cover")], CompressionMethod::Stored);
        for (field, value, message) in [
            (10, MAX_COVER_ENTRIES + 1, "4096-entry"),
            (12, MAX_COVER_METADATA_BYTES + 1, "2 MiB metadata"),
            (10, u16::MAX as usize, "ZIP64"),
        ] {
            let mut bytes = original.clone();
            let end = footer(&bytes);
            if field == 10 {
                put_u16(&mut bytes, end + field, value);
            } else {
                put_u32(&mut bytes, end + field, value);
            }
            let length = bytes.len();
            let mut counted = Counting::new(Cursor::new(bytes));
            error(
                snapshot_zip(&mut counted, length as u64).map(|_| ()),
                message,
            );
            assert_eq!(counted.reads, 1);
            assert_eq!(counted.bytes, length);
        }
    }

    #[test]
    fn huge_invalid_file_reads_only_finite_footer() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("sparse.cbz");
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(16 * 1024 * 1024 * 1024).unwrap();
        let mut counted = Counting::new(&mut file);
        error(
            snapshot_zip(&mut counted, 16 * 1024 * 1024 * 1024).map(|_| ()),
            "missing end record",
        );
        assert_eq!(counted.bytes, MAX_ZIP_FOOTER_BYTES);
        assert_eq!(counted.reads, 1);
    }

    #[test]
    fn combined_central_local_and_footer_metadata_is_capped() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let mut options =
            zip::write::FullFileOptions::default().compression_method(CompressionMethod::Stored);
        options
            .add_extra_data(0xF00D, vec![0; 60_000], false)
            .unwrap();
        for number in 0..19 {
            writer
                .start_file(format!("{number}.png"), options.clone())
                .unwrap();
            writer.write_all(b"x").unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();
        let length = bytes.len() as u64;
        assert!(u32_at(&bytes[footer(&bytes)..], 12) < MAX_COVER_METADATA_BYTES);
        let mut counted = Counting::new(Cursor::new(bytes));
        error(
            snapshot_zip(&mut counted, length).map(|_| ()),
            "2 MiB metadata",
        );
        assert!(counted.bytes <= MAX_ZIP_FOOTER_BYTES + MAX_COVER_METADATA_BYTES);
        assert!(counted.max_request <= MAX_COVER_METADATA_BYTES);
    }

    #[test]
    fn exactly_4096_indexed_entries_are_accepted() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for number in 0..MAX_COVER_ENTRIES {
            writer.start_file(format!("{number}.png"), options).unwrap();
            writer.write_all(b"x").unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();
        let length = bytes.len() as u64;
        let mut counted = Counting::new(Cursor::new(bytes));
        let source = decode_zip(snapshot_zip(&mut counted, length).unwrap()).unwrap();
        assert_eq!(source.page_count, Some(MAX_COVER_ENTRIES));
        assert_eq!(source.bytes, b"x");
        assert!(counted.bytes <= MAX_ZIP_FOOTER_BYTES + MAX_COVER_METADATA_BYTES + 1);
        assert!(counted.reads <= 3 + 2 * MAX_COVER_ENTRIES);
    }

    #[test]
    fn selected_compressed_member_limit_precedes_payload_allocation_or_io() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("compressed-limit.cbz");
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let compressed = MAX_COVER_COMPRESSED_BYTES + 1;
        let mut header = [0; 35];
        header[..4].copy_from_slice(b"PK\x03\x04");
        put_u16(&mut header, 4, 20);
        put_u16(&mut header, 8, 8);
        put_u32(&mut header, 18, compressed);
        put_u32(&mut header, 22, 1);
        put_u16(&mut header, 26, 5);
        header[30..].copy_from_slice(b"1.png");
        file.write_all(&header).unwrap();
        let start = 35 + compressed;
        file.seek(SeekFrom::Start(start as u64)).unwrap();
        let mut central = vec![0; 51];
        central[..4].copy_from_slice(b"PK\x01\x02");
        put_u16(&mut central, 6, 20);
        put_u16(&mut central, 10, 8);
        put_u32(&mut central, 20, compressed);
        put_u32(&mut central, 24, 1);
        put_u16(&mut central, 28, 5);
        central[46..].copy_from_slice(b"1.png");
        append_footer(&mut central, 1, 51, start);
        file.write_all(&central).unwrap();
        let length = file.metadata().unwrap().len();
        let mut counted = Counting::new(file);
        error(
            snapshot_zip(&mut counted, length).map(|_| ()),
            "compressed-entry",
        );
        assert!(counted.bytes < MAX_ZIP_FOOTER_BYTES + 128);
        assert_eq!(counted.reads, 4);
    }

    #[test]
    fn declared_output_size_does_not_replace_actual_decompression_limit() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "1.png",
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
        io::copy(&mut io::repeat(0).take(MAX_PAGE_BYTES + 1), &mut writer).unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        let at = central(&bytes);
        put_u32(&mut bytes, at + 24, 1);
        put_u32(&mut bytes, 22, 1);
        error(read_zip_bytes(bytes), "128 MiB safety limit");
    }

    #[test]
    fn actual_output_must_match_declared_size_even_with_valid_crc() {
        for method in [CompressionMethod::Stored, CompressionMethod::Deflated] {
            let mut bytes = zip_bytes(&[("1.png", b"actual")], method);
            let at = bytes
                .windows(4)
                .rposition(|part| part == b"PK\x01\x02")
                .unwrap();
            let local = u32_at(&bytes[at..], 42);
            put_u32(&mut bytes, at + 24, 1);
            put_u32(&mut bytes, local + 22, 1);
            error(read_zip_bytes(bytes), "declared output size");
        }
    }

    #[test]
    fn crc_is_checked_for_stored_and_deflated_without_later_page_retry() {
        for method in [CompressionMethod::Stored, CompressionMethod::Deflated] {
            let mut bytes = zip_bytes(&[("1.png", b"first"), ("2.png", b"later")], method);
            let at = central(&bytes);
            let at = at
                + 46
                + u16_at(&bytes[at..], 28)
                + u16_at(&bytes[at..], 30)
                + u16_at(&bytes[at..], 32);
            let local = u32_at(&bytes[at..], 42);
            let wrong_crc = u32_at(&bytes[at..], 16) ^ 1;
            put_u32(&mut bytes, at + 16, wrong_crc);
            put_u32(&mut bytes, local + 14, wrong_crc);
            let result = read_zip_bytes(bytes);
            assert!(matches!(result, Err(CoverError::Failed(_))), "{result:?}");
            error(result, "checksum");
        }
    }

    #[test]
    fn index_mutation_is_not_reparsed_and_local_mutation_is_rejected() {
        let bytes = zip_bytes(&[("1.png", b"original")], CompressionMethod::Stored);
        let length = bytes.len() as u64;
        let mut reader = Changing {
            inner: Cursor::new(bytes.clone()),
            reads: 0,
            after_reads: 2,
            change: |bytes| {
                let at = central(bytes);
                put_u32(bytes, at + 42, u32::MAX as usize);
                let end = footer(bytes);
                put_u16(bytes, end + 10, u16::MAX as usize);
            },
        };
        assert_eq!(
            decode_zip(snapshot_zip(&mut reader, length).unwrap())
                .unwrap()
                .bytes,
            b"original"
        );
        let mut reader = Changing {
            inner: Cursor::new(bytes),
            reads: 0,
            after_reads: 2,
            change: |bytes| {
                bytes[30] ^= 1;
            },
        };
        error(snapshot_zip(&mut reader, length).map(|_| ()), "layout");
    }

    #[test]
    fn payload_mutation_is_crc_checked_after_validated_metadata() {
        let bytes = zip_bytes(&[("1.png", b"original")], CompressionMethod::Stored);
        let length = bytes.len() as u64;
        let mut reader = Changing {
            inner: Cursor::new(bytes),
            reads: 0,
            after_reads: 6,
            change: |bytes| {
                let at = bytes
                    .windows(8)
                    .position(|part| part == b"original")
                    .unwrap();
                bytes[at] ^= 1;
            },
        };
        let selected = snapshot_zip(&mut reader, length).unwrap();
        error(decode_zip(selected), "checksum");
    }

    #[test]
    fn handle_replacement_cannot_redirect_zip_reads() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("source.cbz");
        fs::write(
            &path,
            zip_bytes(&[("1.png", b"original")], CompressionMethod::Stored),
        )
        .unwrap();
        let (mut handle, metadata) = open_regular(&path).unwrap();
        fs::rename(&path, temp.path().join("old.cbz")).unwrap();
        fs::write(
            &path,
            zip_bytes(&[("1.png", b"replacement")], CompressionMethod::Stored),
        )
        .unwrap();
        let source = decode_zip(snapshot_zip(&mut handle, metadata.len()).unwrap()).unwrap();
        assert_eq!(source.bytes, b"original");
        assert_eq!(read_cover(&listed(&path)).unwrap().bytes, b"replacement");
    }

    #[test]
    fn in_place_size_and_timestamp_changes_are_detected_on_same_handle() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("source.cbz");
        fs::write(
            &path,
            zip_bytes(&[("1.png", b"original")], CompressionMethod::Stored),
        )
        .unwrap();
        let (mut handle, before) = open_regular(&path).unwrap();
        let selected = snapshot_zip(&mut handle, before.len()).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(before.len() + 1)
            .unwrap();
        assert!(!same_source(&before, &handle.metadata().unwrap()));
        assert_eq!(decode_zip(selected).unwrap().bytes, b"original");
        let before = handle.metadata().unwrap();
        let replacement = fs::OpenOptions::new().write(true).open(&path).unwrap();
        replacement
            .set_modified(before.modified().unwrap() + std::time::Duration::from_secs(1))
            .unwrap();
        assert!(!same_source(&before, &handle.metadata().unwrap()));
    }

    #[test]
    fn alternate_footers_never_trigger_parser_retry_or_unbounded_index() {
        let original = zip_bytes(&[("1.png", b"original")], CompressionMethod::Stored);
        let mut bytes = original.clone();
        let original_end = footer(&bytes);
        // Two end-anchored EOCDs: the second is inside the first one's comment.
        put_u16(&mut bytes, original_end + 20, 22);
        let start = bytes.len();
        append_footer(&mut bytes, u16::MAX as usize, 0, start);
        error(read_zip_bytes(bytes), "layout");
        let mut bytes = original;
        // A malformed terminal footer must not fall back to the earlier valid one.
        append_footer(&mut bytes, MAX_COVER_ENTRIES + 1, 0, 0);
        let length = bytes.len();
        let mut counted = Counting::new(Cursor::new(bytes));
        error(
            snapshot_zip(&mut counted, length as u64).map(|_| ()),
            "4096-entry",
        );
        assert_eq!(counted.reads, 1);
    }

    #[test]
    fn zip64_locator_and_all_sentinel_fields_are_explicitly_unsupported() {
        let original = zip_bytes(&[("1.png", b"image")], CompressionMethod::Stored);
        for (field, width) in [(8, 2), (10, 2), (12, 4), (16, 4)] {
            let mut bytes = original.clone();
            let at = footer(&bytes);
            if width == 2 {
                put_u16(&mut bytes, at + field, u16::MAX as usize);
            } else {
                put_u32(&mut bytes, at + field, u32::MAX as usize);
            }
            assert!(
                matches!(read_zip_bytes(bytes), Err(CoverError::Unsupported(reason)) if reason.contains("ZIP64"))
            );
        }
        let mut bytes = original;
        let at = footer(&bytes);
        bytes.splice(at..at, [b'P', b'K', 6, 7].into_iter().chain([0; 16]));
        assert!(
            matches!(read_zip_bytes(bytes), Err(CoverError::Unsupported(reason)) if reason.contains("ZIP64"))
        );
    }

    #[test]
    fn encrypted_flags_and_aes_extras_are_explicitly_unsupported() {
        let original = zip_bytes(&[("1.png", b"image")], CompressionMethod::Stored);
        for flag in [1, 64, 8192] {
            let mut bytes = original.clone();
            let at = central(&bytes);
            put_u16(&mut bytes, at + 8, flag);
            assert!(
                matches!(read_zip_bytes(bytes), Err(CoverError::Unsupported(reason)) if reason.contains("Encrypted"))
            );
        }
        assert!(matches!(
            check_extra(&[1, 0x99, 0, 0]),
            Err(CoverError::Unsupported(_))
        ));
    }

    #[test]
    fn terminal_categories_distinguish_policy_from_corrupt_sources() {
        let temp = tempdir().unwrap();
        let folder = temp.path().join("empty");
        fs::create_dir(&folder).unwrap();
        assert!(matches!(
            read_cover_classified(&listed(&folder)),
            Err(CoverError::Unsupported(_))
        ));
        fs::create_dir_all(folder.join("nested")).unwrap();
        fs::write(folder.join("nested/1.png"), b"not visited").unwrap();
        assert!(matches!(
            read_cover_classified(&listed(&folder)),
            Err(CoverError::Unsupported(_))
        ));
        let path = temp.path().join("corrupt.cbz");
        fs::write(&path, b"corrupt").unwrap();
        assert!(matches!(
            read_cover_classified(&listed(&path)),
            Err(CoverError::Failed(_))
        ));
        fs::write(
            &path,
            zip_bytes(&[("notes.txt", b"text")], CompressionMethod::Stored),
        )
        .unwrap();
        assert!(matches!(
            read_cover_classified(&listed(&path)),
            Err(CoverError::Unsupported(_))
        ));
    }

    #[test]
    fn image_allocation_limits_still_apply_after_archive_read() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let mut png = png.into_inner();
        // A valid PNG IHDR CRC with excessive dimensions: the bounded image
        // decoder must reject it before pixel allocation.
        put_png_u32(&mut png, 16, crate::imaging::MAX_IMAGE_SIDE);
        put_png_u32(&mut png, 20, 2);
        let crc = test_crc(&png[12..29]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());
        let source =
            read_zip_bytes(zip_bytes(&[("1.png", &png)], CompressionMethod::Deflated)).unwrap();
        assert!(crate::imaging::decode_thumbnail(&source.bytes, 140, 180).is_err());
    }

    fn put_png_u32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn test_crc(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB88320 & (0u32.wrapping_sub(crc & 1)));
            }
        }
        !crc
    }

    #[test]
    fn unicode_extra_names_and_cp437_names_keep_zip_ordering_semantics() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let mut options =
            zip::write::FullFileOptions::default().compression_method(CompressionMethod::Stored);
        let mut unicode = vec![1];
        // The writer validates custom fields against an empty default filename.
        // Set the real filename checksum in the completed central record below.
        unicode.extend_from_slice(&0u32.to_le_bytes());
        unicode.extend_from_slice("2.png".as_bytes());
        options.add_extra_data(0x7075, unicode, true).unwrap();
        writer.start_file("original.dat", options).unwrap();
        writer.write_all(b"unicode page").unwrap();
        writer
            .start_file(
                "10.png",
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"later").unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        let at = central(&bytes);
        let extra = at + 46 + u16_at(&bytes[at..], 28);
        put_u32(&mut bytes, extra + 5, test_crc(b"original.dat") as usize);
        let source = read_zip_bytes(bytes).unwrap();
        assert_eq!(source.page_count, Some(2));
        assert_eq!(source.bytes, b"unicode page");
        let mut bytes = zip_bytes(&[("x.png", b"cp437")], CompressionMethod::Stored);
        for at in 0..bytes.len() - 5 {
            if &bytes[at..at + 5] == b"x.png" {
                bytes[at] = 0x82;
            }
        }
        assert_eq!(read_zip_bytes(bytes).unwrap().bytes, b"cp437");
    }

    #[test]
    fn maximal_comment_keeps_footer_and_zip64_detection_bounded() {
        let original = zip_bytes(&[("1.png", b"image")], CompressionMethod::Stored);
        let mut bytes = original.clone();
        let at = footer(&bytes);
        put_u16(&mut bytes, at + 20, u16::MAX as usize);
        bytes.extend_from_slice(&vec![0; u16::MAX as usize]);
        let length = bytes.len() as u64;
        let mut counted = Counting::new(Cursor::new(bytes));
        assert_eq!(
            decode_zip(snapshot_zip(&mut counted, length).unwrap())
                .unwrap()
                .bytes,
            b"image"
        );
        assert!(counted.bytes < MAX_ZIP_FOOTER_BYTES + 1024);
        let mut bytes = original;
        let at = footer(&bytes);
        bytes.splice(at..at, [b'P', b'K', 6, 7].into_iter().chain([0; 16]));
        put_u16(&mut bytes, at + 20 + 20, u16::MAX as usize);
        bytes.extend_from_slice(&vec![0; u16::MAX as usize]);
        assert!(
            matches!(read_zip_bytes(bytes), Err(CoverError::Unsupported(reason)) if reason.contains("ZIP64"))
        );
    }

    #[test]
    fn malformed_local_fields_and_ranges_cannot_redirect_or_expand_reads() {
        let original = zip_bytes(&[("1.png", b"image")], CompressionMethod::Stored);
        for field in [6, 8, 14, 18, 22, 26, 28] {
            let mut bytes = original.clone();
            bytes[field] ^= 1;
            error(read_zip_bytes(bytes), "layout");
        }
        for field in [28, 30, 32, 42] {
            let mut bytes = original.clone();
            let at = central(&bytes);
            if field == 42 {
                put_u32(&mut bytes, at + field, at);
            } else {
                put_u16(&mut bytes, at + field, u16::MAX as usize);
            }
            let length = bytes.len();
            let mut counted = Counting::new(Cursor::new(bytes));
            error(
                snapshot_zip(&mut counted, length as u64).map(|_| ()),
                "layout",
            );
            assert!(counted.bytes <= length * 2 + 30);
            assert!(counted.max_request <= length);
        }
    }

    #[test]
    fn truncated_source_fails_with_original_bounded_ranges() {
        let bytes = zip_bytes(&[("1.png", b"image")], CompressionMethod::Stored);
        let length = bytes.len() as u64;
        let mut reader = Changing {
            inner: Cursor::new(bytes),
            reads: 0,
            after_reads: 2,
            change: |bytes| {
                bytes.truncate(1);
            },
        };
        assert!(matches!(
            snapshot_zip(&mut reader, length),
            Err(CoverError::Failed(_))
        ));
    }

    #[test]
    fn streaming_classic_zip_data_descriptors_are_supported() {
        for method in [CompressionMethod::Stored, CompressionMethod::Deflated] {
            let mut writer = ZipWriter::new_stream(Vec::new());
            let options = SimpleFileOptions::default().compression_method(method);
            writer.start_file("10.png", options).unwrap();
            writer.write_all(b"later").unwrap();
            writer.start_file("2.png", options).unwrap();
            writer.write_all(b"first").unwrap();
            let bytes = writer.finish().unwrap().into_inner();
            assert_ne!(u16_at(&bytes, 6) & 8, 0);
            let source = read_zip_bytes(bytes).unwrap();
            assert_eq!(source.bytes, b"first");
            assert_eq!(source.page_count, Some(2));
        }
    }

    #[test]
    fn footer_signatures_in_index_are_rejected_but_payload_is_not_parsed() {
        let bytes = zip_bytes(&[("PK\u{5}\u{6}.png", b"cover")], CompressionMethod::Stored);
        assert!(matches!(
            read_zip_bytes(bytes),
            Err(CoverError::Unsupported(_))
        ));
        let mut fake = Vec::new();
        append_footer(
            &mut fake,
            u16::MAX as usize,
            u32::MAX as usize,
            u32::MAX as usize,
        );
        let bytes = zip_bytes(&[("1.png", &fake)], CompressionMethod::Stored);
        assert_eq!(read_zip_bytes(bytes).unwrap().bytes, fake);
    }

    #[test]
    fn duplicate_names_after_unicode_extra_resolution_are_rejected() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        let mut options =
            zip::write::FullFileOptions::default().compression_method(CompressionMethod::Stored);
        let mut unicode = vec![1, 0, 0, 0, 0];
        unicode.extend_from_slice(b"2.png");
        options.add_extra_data(0x7075, unicode, true).unwrap();
        writer.start_file("original.dat", options).unwrap();
        writer.write_all(b"first").unwrap();
        writer
            .start_file(
                "2.png",
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(b"second").unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        let at = central(&bytes);
        let extra = at + 46 + u16_at(&bytes[at..], 28);
        put_u32(&mut bytes, extra + 5, test_crc(b"original.dat") as usize);
        assert!(matches!(
            read_zip_bytes(bytes),
            Err(CoverError::Unsupported(reason)) if reason.contains("duplicate entry names")
        ));
    }

    #[test]
    fn local_entries_cannot_overlap_other_entries_payloads() {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let mut inner = ZipWriter::new(Cursor::new(Vec::new()));
        inner.start_file("2.png", options).unwrap();
        inner.write_all(b"inner").unwrap();
        let inner = inner.finish().unwrap().into_inner();
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("1.png", options).unwrap();
        writer.write_all(&inner[..central(&inner)]).unwrap();
        writer.start_file("2.png", options).unwrap();
        writer.write_all(b"inner").unwrap();
        let mut bytes = writer.finish().unwrap().into_inner();
        let at = central(&bytes);
        let second = at
            + 46
            + u16_at(&bytes[at..], 28)
            + u16_at(&bytes[at..], 30)
            + u16_at(&bytes[at..], 32);
        let nested = 30 + u16_at(&bytes, 26) + u16_at(&bytes, 28);
        put_u32(&mut bytes, second + 42, nested);
        error(read_zip_bytes(bytes), "layout");
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
    fn zip_cover_snapshot_survives_later_in_place_rewrite() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("source.cbz");
        let original = zip_bytes(&[("1.png", b"original")], CompressionMethod::Stored);
        fs::write(&path, &original).unwrap();
        let (mut file, metadata) = open_regular(&path).unwrap();
        let snapshot = snapshot_zip(&mut file, metadata.len()).unwrap();
        fs::write(
            &path,
            zip_bytes(&[("1.png", b"replacement")], CompressionMethod::Stored),
        )
        .unwrap();
        assert_eq!(decode_zip(snapshot).unwrap().bytes, b"original");
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
        let local = u32_at(&bytes[at..], 42);
        put_u32(&mut bytes, local + 22, (MAX_PAGE_BYTES + 1) as usize);
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
    fn zip64_and_alternate_footer_paths_have_explicit_fallbacks() {
        let original = zip_bytes(&[("1.png", b"image")], CompressionMethod::Stored);
        let mut bytes = original.clone();
        let at = footer(&bytes);
        bytes[at + 10..at + 12].copy_from_slice(&u16::MAX.to_le_bytes());
        error(read_zip_bytes(bytes), "ZIP64");
        // Payload signatures are not metadata and must never be parsed as such.
        for signature in [b"PK\x06\x06", b"PK\x06\x07", b"PK\x05\x06"] {
            let bytes = zip_bytes(&[("1.png", signature)], CompressionMethod::Stored);
            assert_eq!(read_zip_bytes(bytes).unwrap().bytes, signature);
        }
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
        assert!(matches!(
            read_zip_bytes(bytes),
            Err(CoverError::Unsupported(reason)) if reason.contains("duplicate entry names")
        ));
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
