#![cfg_attr(not(feature = "desktop"), allow(dead_code, unused_imports))]

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use rars::ArchiveReader;
use serde::Serialize;
#[cfg(feature = "desktop")]
use std::sync::Mutex;
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};
#[cfg(feature = "desktop")]
use tauri::{Emitter, Manager, State};
use zip::ZipArchive;

const MAX_PAGE_BYTES: u64 = 128 * 1024 * 1024;
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "jpe", "png", "gif", "bmp", "webp"];
const ARCHIVE_EXTENSIONS: &[&str] = &["zip", "cbz", "rar", "cbr"];

#[cfg(feature = "desktop")]
#[derive(Default)]
struct ReaderState(Mutex<Option<Book>>);

struct Book {
    title: String,
    path: PathBuf,
    pages: Vec<Page>,
    initial_page: usize,
}

struct Page {
    name: String,
    source: PageSource,
}

enum PageSource {
    File(PathBuf),
    Zip {
        archive: PathBuf,
        entry_index: usize,
    },
    Rar {
        archive: PathBuf,
        entry: Vec<u8>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BookInfo {
    title: String,
    path: String,
    page_names: Vec<String>,
    initial_page: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PageData {
    index: usize,
    name: String,
    mime: String,
    data_url: String,
    byte_size: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryListing {
    path: String,
    parent: Option<String>,
    entries: Vec<DirectoryEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryEntry {
    name: String,
    path: String,
    is_directory: bool,
    kind: &'static str,
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

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn natural_sort_paths(paths: &mut [PathBuf]) {
    paths.sort_by(|left, right| {
        natord::compare_ignore_case(&left.to_string_lossy(), &right.to_string_lossy())
    });
}

fn natural_sort_pages(pages: &mut [Page]) {
    pages.sort_by(|left, right| natord::compare_ignore_case(&left.name, &right.name));
}

fn collect_images(folder: &Path, recursive: bool) -> io::Result<Vec<PathBuf>> {
    let mut images = Vec::new();
    for item in fs::read_dir(folder)? {
        let path = item?.path();
        if path.is_file() && is_image(&path) {
            images.push(path);
        } else if recursive && path.is_dir() {
            images.extend(collect_images(&path, true)?);
        }
    }
    natural_sort_paths(&mut images);
    Ok(images)
}

fn load_folder(path: &Path, selected: Option<&Path>) -> Result<Book, String> {
    let files = collect_images(path, true).map_err(|error| error.to_string())?;
    if files.is_empty() {
        return Err("此資料夾中找不到支援的圖片。".into());
    }
    let initial_page = selected
        .and_then(|selected| files.iter().position(|file| file == selected))
        .unwrap_or(0);
    let pages = files
        .into_iter()
        .map(|file| Page {
            name: file
                .strip_prefix(path)
                .unwrap_or(&file)
                .to_string_lossy()
                .into_owned(),
            source: PageSource::File(file),
        })
        .collect();
    Ok(Book {
        title: display_name(path),
        path: path.to_path_buf(),
        pages,
        initial_page,
    })
}

fn load_zip(path: &Path) -> Result<Book, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("無法開啟 ZIP/CBZ：{error}"))?;
    let mut pages = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("無法讀取封存項目：{error}"))?;
        let name = entry.name().replace('\\', "/");
        if !entry.is_dir() && is_image(Path::new(&name)) {
            if entry.size() > MAX_PAGE_BYTES {
                return Err(format!("圖片「{name}」超過 128 MiB 安全限制。"));
            }
            pages.push(Page {
                name: name.clone(),
                source: PageSource::Zip {
                    archive: path.to_path_buf(),
                    entry_index: index,
                },
            });
        }
    }
    if pages.is_empty() {
        return Err("封存檔中找不到支援的圖片。".into());
    }
    natural_sort_pages(&mut pages);
    Ok(Book {
        title: display_name(path),
        path: path.to_path_buf(),
        pages,
        initial_page: 0,
    })
}

fn load_rar(path: &Path) -> Result<Book, String> {
    let archive =
        ArchiveReader::read_path(path).map_err(|error| format!("無法開啟 RAR/CBR：{error}"))?;
    let mut pages = Vec::new();
    for member in archive.members() {
        let name = member.meta.name_lossy().replace('\\', "/");
        if !member.meta.is_directory && is_image(Path::new(&name)) {
            if member.meta.is_encrypted {
                return Err(format!("RAR 圖片「{name}」需要密碼，目前無法開啟。"));
            }
            if member.meta.unpacked_size > MAX_PAGE_BYTES {
                return Err(format!("圖片「{name}」超過 128 MiB 安全限制。"));
            }
            pages.push(Page {
                name,
                source: PageSource::Rar {
                    archive: path.to_path_buf(),
                    entry: member.meta.name,
                },
            });
        }
    }
    if pages.is_empty() {
        return Err("封存檔中找不到支援的圖片。".into());
    }
    natural_sort_pages(&mut pages);
    Ok(Book {
        title: display_name(path),
        path: path.to_path_buf(),
        pages,
        initial_page: 0,
    })
}

fn load_book_from_path(path: &Path) -> Result<Book, String> {
    if !path.exists() {
        return Err(format!("找不到路徑：{}", path.display()));
    }
    if path.is_dir() {
        return load_folder(path, None);
    }
    if is_image(path) {
        let folder = path.parent().ok_or("圖片沒有父資料夾。")?;
        return load_folder(folder, Some(path));
    }
    match extension(path).as_str() {
        "zip" | "cbz" => load_zip(path),
        "rar" | "cbr" => load_rar(path),
        _ => Err("不支援此檔案格式。支援 JPG、PNG、GIF、BMP、WebP、ZIP/CBZ、RAR/CBR。".into()),
    }
}

fn book_info(book: &Book) -> BookInfo {
    BookInfo {
        title: book.title.clone(),
        path: book.path.to_string_lossy().into_owned(),
        page_names: book.pages.iter().map(|page| page.name.clone()).collect(),
        initial_page: book.initial_page,
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn open_book(path: String, state: State<'_, ReaderState>) -> Result<BookInfo, String> {
    let book = load_book_from_path(Path::new(&path))?;
    let info = book_info(&book);
    *state.0.lock().map_err(|_| "閱讀器狀態鎖定失敗。")? = Some(book);
    Ok(info)
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn close_book(state: State<'_, ReaderState>) -> Result<(), String> {
    *state.0.lock().map_err(|_| "閱讀器狀態鎖定失敗。")? = None;
    Ok(())
}

fn read_page(source: &PageSource) -> Result<Vec<u8>, String> {
    let bytes = match source {
        PageSource::File(path) => fs::read(path).map_err(|error| error.to_string())?,
        PageSource::Zip {
            archive,
            entry_index,
        } => {
            let file = File::open(archive).map_err(|error| error.to_string())?;
            let mut zip = ZipArchive::new(file).map_err(|error| error.to_string())?;
            let mut page = zip
                .by_index(*entry_index)
                .map_err(|error| format!("ZIP 圖片已不存在：{error}"))?;
            let mut bytes = Vec::with_capacity(page.size().min(MAX_PAGE_BYTES) as usize);
            page.read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            bytes
        }
        PageSource::Rar { archive, entry } => ArchiveReader::read_path(archive)
            .map_err(|error| error.to_string())?
            .read_member(entry, None)
            .map_err(|error| error.to_string())?
            .ok_or("RAR 圖片已不存在。")?,
    };
    if bytes.len() as u64 > MAX_PAGE_BYTES {
        return Err("圖片超過 128 MiB 安全限制。".into());
    }
    Ok(bytes)
}

fn mime_for_name(name: &str) -> &'static str {
    match extension(Path::new(name)).as_str() {
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn get_page(index: usize, state: State<'_, ReaderState>) -> Result<PageData, String> {
    let guard = state.0.lock().map_err(|_| "閱讀器狀態鎖定失敗。")?;
    let book = guard.as_ref().ok_or("尚未開啟漫畫。")?;
    let page = book.pages.get(index).ok_or("頁碼超出範圍。")?;
    let bytes = read_page(&page.source)?;
    let mime = mime_for_name(&page.name);
    Ok(PageData {
        index,
        name: page.name.clone(),
        mime: mime.into(),
        data_url: format!("data:{mime};base64,{}", BASE64.encode(&bytes)),
        byte_size: bytes.len(),
    })
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn save_page(
    index: usize,
    destination: String,
    state: State<'_, ReaderState>,
) -> Result<(), String> {
    let guard = state.0.lock().map_err(|_| "閱讀器狀態鎖定失敗。")?;
    let book = guard.as_ref().ok_or("尚未開啟漫畫。")?;
    let page = book.pages.get(index).ok_or("頁碼超出範圍。")?;
    fs::write(&destination, read_page(&page.source)?).map_err(|error| error.to_string())
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn browse_directory(path: Option<String>) -> Result<DirectoryListing, String> {
    let requested = path.map(PathBuf::from).unwrap_or_else(|| {
        #[cfg(windows)]
        {
            PathBuf::from(std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".into()))
        }
        #[cfg(not(windows))]
        {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
        }
    });
    if !requested.is_dir() {
        return Err("指定路徑不是資料夾。".into());
    }
    let mut entries = Vec::new();
    for item in fs::read_dir(&requested).map_err(|error| error.to_string())? {
        let path = item.map_err(|error| error.to_string())?.path();
        let is_directory = path.is_dir();
        if is_directory || is_image(&path) || is_archive(&path) {
            let kind = if is_directory {
                "folder"
            } else if is_archive(&path) {
                "archive"
            } else {
                "image"
            };
            entries.push(DirectoryEntry {
                name: display_name(&path),
                path: path.to_string_lossy().into_owned(),
                is_directory,
                kind,
            });
        }
    }
    entries.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| natord::compare_ignore_case(&left.name, &right.name))
    });
    Ok(DirectoryListing {
        path: requested.to_string_lossy().into_owned(),
        parent: requested
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned()),
        entries,
    })
}

#[cfg(feature = "desktop")]
#[tauri::command]
fn launch_path() -> Option<String> {
    std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .find(|path| path.exists() && (path.is_dir() || is_image(path) || is_archive(path)))
        .map(|path| path.to_string_lossy().into_owned())
}

#[cfg(feature = "desktop")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
                let _ = window.emit("open-args", args);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(ReaderState::default())
        .invoke_handler(tauri::generate_handler![
            open_book,
            close_book,
            get_page,
            save_page,
            browse_directory,
            launch_path
        ])
        .run(tauri::generate_context!())
        .expect("error while running MangaMeeya Rust");
}

#[cfg(not(feature = "desktop"))]
pub fn run() {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::{ZipWriter, write::SimpleFileOptions};

    #[test]
    fn image_extensions_are_case_insensitive() {
        assert!(is_image(Path::new("PAGE.JPEG")));
        assert!(is_archive(Path::new("book.CBZ")));
        assert!(!is_image(Path::new("notes.txt")));
    }

    #[test]
    fn folders_use_natural_page_order_and_selected_page() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("10.jpg"), b"ten").unwrap();
        fs::write(temp.path().join("2.jpg"), b"two").unwrap();
        fs::write(temp.path().join("1.jpg"), b"one").unwrap();
        let selected = temp.path().join("2.jpg");
        let book = load_folder(temp.path(), Some(&selected)).unwrap();
        let names: Vec<_> = book.pages.iter().map(|page| page.name.as_str()).collect();
        assert_eq!(names, ["1.jpg", "2.jpg", "10.jpg"]);
        assert_eq!(book.initial_page, 1);
    }

    #[test]
    fn zip_books_list_only_images_in_natural_order() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("sample.cbz");
        let file = File::create(&path).unwrap();
        let mut zip = ZipWriter::new(file);
        for (name, bytes) in [
            ("10.png", b"ten".as_slice()),
            ("readme.txt", b"no"),
            ("2.png", b"two"),
        ] {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        let book = load_zip(&path).unwrap();
        let names: Vec<_> = book.pages.iter().map(|page| page.name.as_str()).collect();
        assert_eq!(names, ["2.png", "10.png"]);
        assert_eq!(read_page(&book.pages[0].source).unwrap(), b"two");
    }
}
