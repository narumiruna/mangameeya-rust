//! Own-process HWND capture. Does not capture the desktop or alter display scaling.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{ffi::c_void, path::Path, ptr};

type Handle = *mut c_void;

#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    colors_used: u32,
    colors_important: u32,
}

#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 1],
}

#[link(name = "user32")]
unsafe extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> Handle;
    fn GetWindowThreadProcessId(window: Handle, process: *mut u32) -> u32;
    fn GetClientRect(window: Handle, rect: *mut Rect) -> i32;
    fn GetDpiForWindow(window: Handle) -> u32;
    fn GetDC(window: Handle) -> Handle;
    fn ReleaseDC(window: Handle, dc: Handle) -> i32;
    fn PrintWindow(window: Handle, dc: Handle, flags: u32) -> i32;
    fn ShowWindow(window: Handle, command: i32) -> i32;
    fn IsWindowVisible(window: Handle) -> i32;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(dc: Handle) -> Handle;
    fn CreateCompatibleBitmap(dc: Handle, width: i32, height: i32) -> Handle;
    fn SelectObject(dc: Handle, object: Handle) -> Handle;
    fn DeleteObject(object: Handle) -> i32;
    fn DeleteDC(dc: Handle) -> i32;
    fn GetDIBits(
        dc: Handle,
        bitmap: Handle,
        start: u32,
        lines: u32,
        bits: *mut c_void,
        info: *mut BitmapInfo,
        usage: u32,
    ) -> i32;
}

fn own_window(title: &str) -> Result<Handle> {
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    // SAFETY: NUL-terminated UTF-16 title, output pointer valid for one u32.
    unsafe {
        let window = FindWindowW(ptr::null(), title.as_ptr());
        anyhow::ensure!(!window.is_null(), "Synthetic capture HWND was not found");
        let mut process = 0;
        GetWindowThreadProcessId(window, &mut process);
        anyhow::ensure!(
            process == std::process::id(),
            "Refusing capture of another process's HWND"
        );
        Ok(window)
    }
}

pub fn show_own_window(title: &str) -> Result<()> {
    let window = own_window(title)?;
    // SAFETY: own_window validates that this HWND belongs to this process.
    unsafe {
        ShowWindow(window, 5);
    } // SW_SHOW; no display settings change.
    Ok(())
}

struct CaptureDc {
    window: Handle,
    source: Handle,
    memory: Handle,
    bitmap: Handle,
    previous: Handle,
}

impl Drop for CaptureDc {
    fn drop(&mut self) {
        // SAFETY: Handles were created by GDI; restore selection before deletion.
        unsafe {
            if !self.previous.is_null() && self.previous as isize != -1 {
                SelectObject(self.memory, self.previous);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.memory.is_null() {
                DeleteDC(self.memory);
            }
            if !self.source.is_null() {
                ReleaseDC(self.window, self.source);
            }
        }
    }
}

/// Call off the UI thread: PrintWindow can send WM_PRINT to the owning thread.
/// The caller's process watchdog also bounds blocking driver/PrintWindow calls.
pub fn capture_own_window(
    title: &str,
    path: &Path,
    expected_source_color: [u8; 4],
) -> Result<Value> {
    let window = own_window(title)?;
    let mut rect = Rect::default();
    // SAFETY: Validated own-process HWND and initialized FFI output structures.
    unsafe {
        anyhow::ensure!(IsWindowVisible(window) != 0, "Capture HWND must be visible");
        anyhow::ensure!(
            GetClientRect(window, &mut rect) != 0,
            "GetClientRect failed"
        );
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        anyhow::ensure!(
            width > 0 && height > 0 && width <= 8192 && height <= 8192,
            "Unexpected physical client dimensions: {width} x {height}"
        );
        let dpi = GetDpiForWindow(window);
        anyhow::ensure!(dpi > 0, "GetDpiForWindow failed");
        let mut dc = CaptureDc {
            window,
            source: GetDC(window),
            memory: ptr::null_mut(),
            bitmap: ptr::null_mut(),
            previous: ptr::null_mut(),
        };
        anyhow::ensure!(!dc.source.is_null(), "GetDC failed");
        dc.memory = CreateCompatibleDC(dc.source);
        anyhow::ensure!(!dc.memory.is_null(), "CreateCompatibleDC failed");
        dc.bitmap = CreateCompatibleBitmap(dc.source, width, height);
        anyhow::ensure!(!dc.bitmap.is_null(), "CreateCompatibleBitmap failed");
        dc.previous = SelectObject(dc.memory, dc.bitmap);
        anyhow::ensure!(
            !dc.previous.is_null() && dc.previous as isize != -1,
            "SelectObject failed"
        );
        // PW_CLIENTONLY | PW_RENDERFULLCONTENT. No screen or desktop blit fallback.
        anyhow::ensure!(PrintWindow(window, dc.memory, 3) != 0, "PrintWindow failed");
        SelectObject(dc.memory, dc.previous);
        dc.previous = ptr::null_mut();
        let mut info = BitmapInfo {
            header: BitmapInfoHeader {
                size: std::mem::size_of::<BitmapInfoHeader>() as u32,
                width,
                height: -height,
                planes: 1,
                bit_count: 32,
                compression: 0,
                size_image: 0,
                x_pels_per_meter: 0,
                y_pels_per_meter: 0,
                colors_used: 0,
                colors_important: 0,
            },
            colors: [0],
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        anyhow::ensure!(
            GetDIBits(
                dc.memory,
                dc.bitmap,
                0,
                height as u32,
                pixels.as_mut_ptr().cast(),
                &mut info,
                0
            ) == height,
            "GetDIBits failed"
        );
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2); // BGRA to RGBA.
            pixel[3] = 255;
        }
        let image = image::RgbaImage::from_raw(width as u32, height as u32, pixels)
            .context("Invalid native bitmap dimensions")?;
        let first = image.get_pixel(0, 0);
        anyhow::ensure!(
            image.pixels().any(|pixel| pixel != first),
            "PrintWindow returned a uniform image; native visual evidence is unavailable"
        );
        // GPU-backed windows can return chrome but omit the rendered contents.
        // Require pixels from the synthetic source, not just a nonuniform frame.
        let artwork_pixels = image
            .pixels()
            .filter(|pixel| {
                pixel
                    .0
                    .iter()
                    .zip(expected_source_color)
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 3)
            })
            .count();
        anyhow::ensure!(
            artwork_pixels >= 100,
            "PrintWindow omitted the synthetic artwork; native visual evidence is unavailable"
        );
        image.save(path)?;
        Ok(json!({
            "file": path.file_name().unwrap().to_string_lossy(),
            "method": "PrintWindow(PW_CLIENTONLY | PW_RENDERFULLCONTENT)",
            "own_hwnd": format!("0x{:x}", window as usize),
            "process_id": std::process::id(),
            "physical_client_size": [width, height],
            "physical_image_size": [image.width(), image.height()],
            "window_dpi": dpi,
            "windows_scale_factor": dpi as f64 / 96.,
            "display_scaling_changed": false,
            "verified_artwork_pixels": artwork_pixels
        }))
    }
}
