//! Bounded image decoding and native pixel transformations. Source bytes stay unchanged.

use image::{
    AnimationDecoder, Delay, DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits,
    RgbaImage,
    codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder},
    imageops,
    metadata::Orientation,
};
use std::io::Cursor;

pub const MAX_ENCODED_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
/// Maximum source side. The total pixel limit also applies to long pages.
pub const MAX_IMAGE_SIDE: u32 = MAX_IMAGE_PIXELS as u32;
const MAX_DECODE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ANIMATION_FRAMES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filters {
    /// Percent, limited to 50 through 150.
    pub brightness: f32,
    /// Percent, limited to 50 through 150.
    pub contrast: f32,
    /// Percent, limited to 0 through 100.
    pub grayscale: f32,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            brightness: 100.0,
            contrast: 100.0,
            grayscale: 0.0,
        }
    }
}

impl Filters {
    pub fn normalized(self) -> Self {
        fn percent(value: f32, minimum: f32, maximum: f32, default: f32) -> f32 {
            if value.is_finite() {
                value.clamp(minimum, maximum)
            } else {
                default
            }
        }
        Self {
            brightness: percent(self.brightness, 50.0, 150.0, 100.0),
            contrast: percent(self.contrast, 50.0, 150.0, 100.0),
            grayscale: percent(self.grayscale, 0.0, 100.0, 0.0),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedPage {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8 pixels, with straight (not premultiplied) alpha.
    pub pixels: Vec<u8>,
    /// Size of the decoded pixel buffer, not the source file.
    pub byte_size: usize,
}

impl DecodedPage {
    /// Consume RGBA pixels and return BGRA8 for a GPUI image upload.
    /// Alpha is unchanged. This does not allocate another pixel buffer.
    pub fn into_bgra(mut self) -> Vec<u8> {
        for pixel in self.pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        self.pixels
    }

    fn from_image(image: RgbaImage) -> Result<Self, String> {
        let (width, height) = image.dimensions();
        let count = rgba_byte_count(width, height)?;
        let pixels = image.into_raw();
        if pixels.len() != count {
            return Err("Invalid decoded RGBA buffer length".to_owned());
        }
        Ok(Self {
            width,
            height,
            byte_size: pixels.len(),
            pixels,
        })
    }
}

/// Decode the first frame, apply EXIF orientation, rotate clockwise, then filter.
/// Reader rotation must be a multiple of 90 degrees.
pub fn decode_page(bytes: &[u8], rotation: i32, filters: Filters) -> Result<DecodedPage, String> {
    Ok(decode_pages(bytes, rotation, filters, true)?.remove(0).0)
}

/// Decode full-canvas animation frames with their source delays.
/// Static images return one frame with a zero delay. EXIF orientation precedes
/// reader rotation and filters. Animations are limited to 256 frames and 128 MiB
/// of decoded RGBA canvases, including when making thumbnails.
pub fn decode_frames(
    bytes: &[u8],
    rotation: i32,
    filters: Filters,
) -> Result<Vec<(DecodedPage, Delay)>, String> {
    decode_pages(bytes, rotation, filters, false)
}

fn decode_pages(
    bytes: &[u8],
    rotation: i32,
    filters: Filters,
    first_only: bool,
) -> Result<Vec<(DecodedPage, Delay)>, String> {
    if rotation % 90 != 0 {
        return Err("Image rotation must be a multiple of 90 degrees".to_owned());
    }
    let filters = filters.normalized();
    decode_images(bytes, first_only, |image| {
        let mut image = rotate(image, rotation.rem_euclid(360))?;
        apply_filters(&mut image, filters);
        Ok(image)
    })
}

/// Make an auto-oriented first-frame thumbnail without enlarging it.
/// Reader rotation and filters do not change sidebar thumbnails.
pub fn decode_thumbnail(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
) -> Result<DecodedPage, String> {
    Ok(decode_thumbnails(bytes, max_width, max_height, true)?
        .remove(0)
        .0)
}

/// Make auto-oriented animation thumbnails with unchanged source delays.
/// Each full-canvas frame fits both bounds without enlargement or reader filters.
pub fn decode_thumbnail_frames(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
) -> Result<Vec<(DecodedPage, Delay)>, String> {
    decode_thumbnails(bytes, max_width, max_height, false)
}

fn decode_thumbnails(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
    first_only: bool,
) -> Result<Vec<(DecodedPage, Delay)>, String> {
    if max_width == 0 || max_height == 0 {
        return Err("Thumbnail dimensions must be greater than zero".to_owned());
    }
    decode_images(bytes, first_only, |image| {
        Ok(
            if image.width() > max_width || image.height() > max_height {
                DynamicImage::ImageRgba8(image)
                    .thumbnail(max_width, max_height)
                    .into_rgba8()
            } else {
                image
            },
        )
    })
}

fn decode_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    limits
}

fn header_error(error: image::ImageError) -> String {
    match error {
        image::ImageError::Limits(ref limit)
            if limit.kind() == image::error::LimitErrorKind::DimensionError =>
        {
            format!(
                "Image exceeds the source dimension safety limit ({MAX_IMAGE_SIDE} pixels per side)"
            )
        }
        _ => format!("Cannot decode image header: {error}"),
    }
}

fn prepare_decoder(decoder: &mut impl ImageDecoder, animated: bool) -> Result<Orientation, String> {
    set_decoder_limits(decoder, animated)?;
    decoder
        .orientation()
        .map_err(|error| format!("Cannot read image orientation: {error}"))
}

fn set_decoder_limits(decoder: &mut impl ImageDecoder, animated: bool) -> Result<(), String> {
    let (width, height) = decoder.dimensions();
    let rgba_bytes = rgba_byte_count(width, height)?;
    let mut limits = decode_limits();
    // Account for the output as well as the decoder's working buffers. Animation
    // iterators retain their own compositing canvases. Conversion and rotation
    // each use at most one additional, bounded image buffer.
    let output_bytes = if animated {
        rgba_bytes as u64
    } else {
        decoder.total_bytes()
    };
    limits
        .reserve(output_bytes)
        .and_then(|()| decoder.set_limits(limits))
        .map_err(|error| format!("Image exceeds the decoding memory limit: {error}"))
}

fn decode_images(
    bytes: &[u8],
    first_only: bool,
    transform: impl Fn(RgbaImage) -> Result<RgbaImage, String>,
) -> Result<Vec<(DecodedPage, Delay)>, String> {
    check_encoded_size(bytes.len())?;
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("Cannot identify image: {error}"))?;
    reader.limits(decode_limits());
    match reader.format() {
        Some(ImageFormat::Gif) => {
            let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(header_error)?;
            let orientation = prepare_decoder(&mut decoder, true)?;
            let dimensions = decoder.dimensions();
            collect_animation(
                decoder.into_frames(),
                dimensions,
                orientation,
                first_only,
                transform,
            )
        }
        Some(ImageFormat::Png) => {
            let mut decoder = PngDecoder::with_limits(Cursor::new(bytes), decode_limits())
                .map_err(header_error)?;
            if decoder.is_apng().map_err(header_error)? {
                let dimensions = decoder.dimensions();
                validate_apng_budget(bytes, dimensions, first_only)?;
                let orientation = prepare_decoder(&mut decoder, true)?;
                let frames = decoder.apng().map_err(header_error)?.into_frames();
                collect_animation(frames, dimensions, orientation, first_only, transform)
            } else {
                decode_static(decoder, transform)
            }
        }
        Some(ImageFormat::WebP) => {
            let orientation = validate_webp(bytes, first_only)?;
            let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(header_error)?;
            let animated = decoder.has_animation();
            set_decoder_limits(&mut decoder, animated)?;
            if animated {
                let dimensions = decoder.dimensions();
                collect_animation(
                    decoder.into_frames(),
                    dimensions,
                    orientation,
                    first_only,
                    transform,
                )
            } else {
                decode_oriented_static(decoder, orientation, transform)
            }
        }
        _ => decode_static(reader.into_decoder().map_err(header_error)?, transform),
    }
}

fn check_encoded_size(size: usize) -> Result<(), String> {
    if size > MAX_ENCODED_BYTES {
        return Err("Encoded image exceeds the 128 MiB limit".to_owned());
    }
    Ok(())
}

fn oriented_rgba(mut image: DynamicImage, orientation: Orientation) -> RgbaImage {
    image.apply_orientation(orientation);
    image.into_rgba8()
}

fn decode_static(
    mut decoder: impl ImageDecoder,
    transform: impl Fn(RgbaImage) -> Result<RgbaImage, String>,
) -> Result<Vec<(DecodedPage, Delay)>, String> {
    let orientation = prepare_decoder(&mut decoder, false)?;
    decode_oriented_static(decoder, orientation, transform)
}

fn decode_oriented_static(
    decoder: impl ImageDecoder,
    orientation: Orientation,
    transform: impl Fn(RgbaImage) -> Result<RgbaImage, String>,
) -> Result<Vec<(DecodedPage, Delay)>, String> {
    let image = DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("Cannot decode image pixels: {error}"))?;
    let image = transform(oriented_rgba(image, orientation))?;
    Ok(vec![(
        DecodedPage::from_image(image)?,
        Delay::from_numer_denom_ms(0, 1),
    )])
}

fn check_frame_budget(count: usize, decoded_bytes: u64) -> Result<(), String> {
    if count > MAX_ANIMATION_FRAMES {
        return Err(format!(
            "Animation exceeds the {MAX_ANIMATION_FRAMES}-frame limit"
        ));
    }
    if decoded_bytes > MAX_DECODE_BYTES {
        return Err("Animation exceeds the 128 MiB decoded frame limit".to_owned());
    }
    Ok(())
}

fn collect_animation(
    frames: image::Frames<'_>,
    dimensions: (u32, u32),
    orientation: Orientation,
    first_only: bool,
    transform: impl Fn(RgbaImage) -> Result<RgbaImage, String>,
) -> Result<Vec<(DecodedPage, Delay)>, String> {
    let canvas_bytes = rgba_byte_count(dimensions.0, dimensions.1)? as u64;
    let mut decoded = Vec::new();
    let mut decoded_bytes = 0;
    for frame in frames {
        let frame = frame.map_err(|error| format!("Cannot decode animation frame: {error}"))?;
        if frame.left() != 0 || frame.top() != 0 || frame.buffer().dimensions() != dimensions {
            return Err("Animation decoder did not return a full-canvas frame".to_owned());
        }
        // Check source canvases even for small thumbnails, so downsampling cannot
        // bypass the animation decoding budget. Do not accumulate an extra frame.
        decoded_bytes += canvas_bytes;
        check_frame_budget(decoded.len() + 1, decoded_bytes)?;
        let delay = frame.delay();
        let image = transform(oriented_rgba(
            DynamicImage::ImageRgba8(frame.into_buffer()),
            orientation,
        ))?;
        let output_bytes = rgba_byte_count(image.width(), image.height())? as u64;
        if output_bytes > canvas_bytes {
            return Err("Transformed animation frame exceeds its source canvas".to_owned());
        }
        decoded.push((DecodedPage::from_image(image)?, delay));
        if first_only {
            break;
        }
    }
    if decoded.is_empty() {
        return Err("Animation contains no frames".to_owned());
    }
    Ok(decoded)
}

fn validate_apng_budget(
    bytes: &[u8],
    dimensions: (u32, u32),
    first_only: bool,
) -> Result<(), String> {
    // acTL is before IDAT. Read its count without decoding or allocating frames.
    // The PNG decoder checks the chunk order, lengths, and checksums separately.
    let invalid = || "Invalid APNG animation control".to_owned();
    let mut chunks = bytes.get(8..).ok_or_else(invalid)?;
    while let Some(header) = chunks.get(..8) {
        let size = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let end = size.checked_add(12).ok_or_else(invalid)?;
        let chunk = chunks.get(..end).ok_or_else(invalid)?;
        if &header[4..8] == b"acTL" {
            let count =
                u32::from_be_bytes(chunk.get(8..12).ok_or_else(invalid)?.try_into().unwrap());
            let count = if first_only { 1 } else { count as usize };
            let canvas_bytes = rgba_byte_count(dimensions.0, dimensions.1)? as u64;
            return check_frame_budget(count, canvas_bytes * count as u64);
        }
        if &header[4..8] == b"IDAT" {
            break;
        }
        chunks = &chunks[end..];
    }
    Err(invalid())
}

/// The image WebP adapter does not forward allocation limits to image-webp.
/// Check chunk extents and embedded dimensions before pixel decoding. Read EXIF
/// from a validated slice, without the adapter's unchecked metadata allocation.
fn validate_webp(bytes: &[u8], first_only: bool) -> Result<Orientation, String> {
    let invalid = || "Invalid or truncated WebP chunk".to_owned();
    let header = bytes.get(..12).ok_or_else(invalid)?;
    let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let end = size
        .checked_add(8)
        .filter(|&end| end <= bytes.len())
        .ok_or_else(invalid)?;
    if end < 12 {
        return Err(invalid());
    }
    // Validate trailing chunks too: image-webp can read beyond the RIFF length.
    Ok(validate_webp_chunks(&bytes[12..], None, first_only)?.unwrap_or(Orientation::NoTransforms))
}

fn validate_webp_chunks(
    mut bytes: &[u8],
    canvas: Option<(u32, u32)>,
    first_only: bool,
) -> Result<Option<Orientation>, String> {
    let invalid = || "Invalid or truncated WebP chunk".to_owned();
    let mut dimensions = canvas;
    let mut frame_count = 0;
    let mut orientation = None;
    while !bytes.is_empty() {
        let header = bytes.get(..8).ok_or_else(invalid)?;
        let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        let padded = size.checked_add(size & 1).ok_or_else(invalid)?;
        let end = 8usize.checked_add(padded).ok_or_else(invalid)?;
        let data = bytes
            .get(8..end)
            .ok_or_else(invalid)?
            .get(..size)
            .ok_or_else(invalid)?;
        match &header[..4] {
            b"VP8X" => {
                if dimensions.is_some() {
                    return Err("Duplicate or nested WebP canvas header".to_owned());
                }
                let data = data.get(..10).ok_or_else(invalid)?;
                let width = little_u24(&data[4..7]) + 1;
                let height = little_u24(&data[7..10]) + 1;
                rgba_byte_count(width, height)?;
                dimensions = Some((width, height));
            }
            b"VP8 " | b"VP8L" => {
                let (width, height) = if &header[..4] == b"VP8L" {
                    let data = data.get(..5).ok_or_else(invalid)?;
                    let bits = u32::from_le_bytes(data[1..5].try_into().unwrap());
                    ((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
                } else {
                    let data = data.get(..10).ok_or_else(invalid)?;
                    (
                        u32::from(u16::from_le_bytes([data[6], data[7]]) & 0x3fff),
                        u32::from(u16::from_le_bytes([data[8], data[9]]) & 0x3fff),
                    )
                };
                rgba_byte_count(width, height)?;
                if dimensions.is_some_and(|canvas| canvas != (width, height)) {
                    return Err("WebP bitstream dimensions do not match the canvas".to_owned());
                }
            }
            b"ANMF" => {
                if canvas.is_some() {
                    return Err("Nested WebP animation frame".to_owned());
                }
                let frame = data.get(..16).ok_or_else(invalid)?;
                let x = little_u24(&frame[..3]) * 2;
                let y = little_u24(&frame[3..6]) * 2;
                let width = little_u24(&frame[6..9]) + 1;
                let height = little_u24(&frame[9..12]) + 1;
                rgba_byte_count(width, height)?;
                if !dimensions.is_some_and(|(w, h)| x + width <= w && y + height <= h) {
                    return Err("WebP animation frame exceeds the canvas".to_owned());
                }
                frame_count += 1;
                if !first_only {
                    let (w, h) = dimensions.unwrap();
                    check_frame_budget(
                        frame_count,
                        rgba_byte_count(w, h)? as u64 * frame_count as u64,
                    )?;
                }
                validate_webp_chunks(&data[16..], Some((width, height)), first_only)?;
            }
            b"EXIF" if canvas.is_none() && dimensions.is_some() && orientation.is_none() => {
                orientation =
                    Some(Orientation::from_exif_chunk(data).unwrap_or(Orientation::NoTransforms));
            }
            _ => {}
        }
        bytes = &bytes[end..];
    }
    Ok(orientation)
}

fn little_u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

fn rgba_byte_count(width: u32, height: u32) -> Result<usize, String> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 {
        return Err("Image dimensions must be greater than zero".to_owned());
    }
    if width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err(format!(
            "Image exceeds the source dimension safety limit ({MAX_IMAGE_SIDE} pixels per side)"
        ));
    }
    if pixels > MAX_IMAGE_PIXELS {
        return Err("Image exceeds the 32-megapixel limit".to_owned());
    }
    usize::try_from(pixels * 4).map_err(|_| "Image allocation is too large".to_owned())
}

fn rotate(mut image: RgbaImage, rotation: i32) -> Result<RgbaImage, String> {
    match rotation {
        0 => Ok(image),
        180 => {
            imageops::rotate180_in_place(&mut image);
            Ok(image)
        }
        90 | 270 => {
            let (width, height) = (image.height(), image.width());
            let count = rgba_byte_count(width, height)?;
            let mut pixels = Vec::new();
            pixels
                .try_reserve_exact(count)
                .map_err(|error| format!("Cannot allocate rotated image: {error}"))?;
            pixels.resize(count, 0);
            let mut result = RgbaImage::from_raw(width, height, pixels)
                .ok_or_else(|| "Invalid rotated image buffer".to_owned())?;
            if rotation == 90 {
                imageops::rotate90_in(&image, &mut result)
            } else {
                imageops::rotate270_in(&image, &mut result)
            }
            .map_err(|error| format!("Cannot rotate image: {error}"))?;
            Ok(result)
        }
        _ => Err("Image rotation must be a multiple of 90 degrees".to_owned()),
    }
}

fn apply_filters(image: &mut RgbaImage, filters: Filters) {
    if filters == Filters::default() {
        return;
    }
    let brightness = filters.brightness / 100.0;
    let contrast = filters.contrast / 100.0;
    let grayscale = filters.grayscale / 100.0;
    for pixel in image.pixels_mut() {
        let mut channels = [0.0; 3];
        for (channel, original) in channels.iter_mut().zip(pixel.0[..3].iter()) {
            // Match the original CSS filter order: brightness, contrast, grayscale.
            let brightened = (f32::from(*original) * brightness).clamp(0.0, 255.0);
            *channel = ((brightened - 127.5) * contrast + 127.5).clamp(0.0, 255.0);
        }
        let gray = channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
        for (output, channel) in pixel.0[..3].iter_mut().zip(channels) {
            *output = (channel * (1.0 - grayscale) + gray * grayscale)
                .round()
                .clamp(0.0, 255.0) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{
        Frame, ImageEncoder, Rgba,
        codecs::{gif::GifEncoder, jpeg::JpegEncoder, png::PngEncoder, webp::WebPEncoder},
    };

    // A real 2 x 3 RGBA PNG: red/green, blue/yellow, black/white.
    // Alpha values also test that transformations preserve transparency.
    const PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 2, 0, 0, 0, 3, 8, 6,
        0, 0, 0, 185, 234, 222, 129, 0, 0, 0, 26, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 8, 27, 24, 128, 52, 16, 0, 153, 64, 224, 0, 98, 1, 0, 153, 76, 11, 182, 125, 151, 72,
        156, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 128];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const YELLOW: [u8; 4] = [255, 255, 0, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 64];
    const WHITE: [u8; 4] = [255, 255, 255, 0];

    // Two 2 x 1 frames: red/red, then red/blue, with delays of 30 and 70 ms.
    // APNG: FFmpeg's APNG encoder, with fcTL delays changed and CRCs updated.
    // Its second frame is a 1 x 1 subrectangle at x = 1.
    const APNG_HEX: &str = concat!(
        "89504e470d0a1a0a0000000d4948445200000002000000010806000000f4227f8a",
        "000000097048597300000000000000010084791773000000086163544c00000002",
        "00000000f38d93700000001a6663544c00000000000000020000000100000000",
        "00000000000300640000c1a003330000000f49444154789c63fccfc000440c0c00",
        "0d0502005b7dbc9e0000001a6663544c00000001000000010000000100000001",
        "00000000000700640000cd8b164e000000116664415400000002789c636460f8ff",
        "1f0003070200fc747b080000000049454e44ae426082",
    );
    // WebP: cwebp -lossless on each PNG, then webpmux -frame frame0.webp
    // +30+0+0+0-b -frame frame1.webp +70+0+0+0-b.
    const WEBP_HEX: &str = concat!(
        "524946468600000057454250565038580a00000002000000010000000000414e",
        "494d06000000ffffffff0000414e4d4628000000000000000000010000000000",
        "1e0000025650384c0f0000002f010000000710fd8ffe0722a2ff0100414e4d46",
        "2a000000000000000000010000000000460000025650384c110000002f010000",
        "000f10f3bffff31f0e32a2ff0100",
    );

    // Encoded with GifEncoder, then set the first disposal to Keep and the
    // second frame's left offset to 1. This tests native GIF compositing.
    const GIF_HEX: &str = concat!(
        "4749463839610200010080000000000000000021f90404030000002c00000000",
        "0200010080ff00000000000202040a0021f90408070000002c01000000010001",
        "00800000ff00000002024401003b",
    );

    fn fixture(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect()
    }

    fn animated_gif(count: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            for index in 0..count {
                let pixels = if index % 2 == 0 {
                    [RED, RED]
                } else {
                    [RED, BLUE]
                };
                let delay = if index % 2 == 0 { 30 } else { 70 };
                encoder
                    .encode_frame(Frame::from_parts(
                        RgbaImage::from_raw(2, 1, pixels.concat()).unwrap(),
                        0,
                        0,
                        Delay::from_numer_denom_ms(delay, 1),
                    ))
                    .unwrap();
            }
        }
        bytes
    }

    fn animations() -> [Vec<u8>; 3] {
        [fixture(GIF_HEX), fixture(APNG_HEX), fixture(WEBP_HEX)]
    }

    fn colors(page: &DecodedPage) -> Vec<[u8; 4]> {
        page.pixels.as_chunks::<4>().0.to_vec()
    }

    fn exif(orientation: u8) -> Vec<u8> {
        // Little-endian TIFF, one SHORT Orientation entry in IFD0.
        vec![
            b'I',
            b'I',
            42,
            0,
            8,
            0,
            0,
            0,
            1,
            0,
            0x12,
            1,
            3,
            0,
            1,
            0,
            0,
            0,
            orientation,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ]
    }

    fn oriented_png(orientation: u8) -> Vec<u8> {
        let image = image::load_from_memory(PNG).unwrap().into_rgba8();
        let mut bytes = Vec::new();
        let mut encoder = PngEncoder::new(&mut bytes);
        encoder.set_exif_metadata(exif(orientation)).unwrap();
        encoder
            .write_image(&image, 2, 3, image::ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    fn orientation_cases() -> [(u8, u32, u32, [usize; 6]); 8] {
        [
            (1, 2, 3, [0, 1, 2, 3, 4, 5]),
            (2, 2, 3, [1, 0, 3, 2, 5, 4]),
            (3, 2, 3, [5, 4, 3, 2, 1, 0]),
            (4, 2, 3, [4, 5, 2, 3, 0, 1]),
            (5, 3, 2, [0, 2, 4, 1, 3, 5]),
            (6, 3, 2, [4, 2, 0, 5, 3, 1]),
            (7, 3, 2, [5, 3, 1, 4, 2, 0]),
            (8, 3, 2, [1, 3, 5, 0, 2, 4]),
        ]
    }

    #[test]
    fn all_exif_orientations_transform_geometry_and_rgba_before_reader_rotation() {
        let source = [RED, GREEN, BLUE, YELLOW, BLACK, WHITE];
        for (orientation, width, height, indices) in orientation_cases() {
            let bytes = oriented_png(orientation);
            let expected = indices.map(|index| source[index]);
            let page = decode_page(&bytes, 0, Filters::default()).unwrap();
            assert_eq!(
                (page.width, page.height),
                (width, height),
                "EXIF {orientation}"
            );
            assert_eq!(colors(&page), expected, "EXIF {orientation}");
            assert_eq!(decode_thumbnail(&bytes, 20, 20).unwrap(), page);
            assert_eq!(
                decode_frames(&bytes, 0, Filters::default()).unwrap()[0].0,
                page
            );
            assert_eq!(decode_thumbnail_frames(&bytes, 20, 20).unwrap()[0].0, page);

            let mut rotated = Vec::new();
            for x in 0..width {
                for y in (0..height).rev() {
                    rotated.push(expected[(y * width + x) as usize]);
                }
            }
            let page = decode_page(&bytes, 90, Filters::default()).unwrap();
            assert_eq!((page.width, page.height), (height, width));
            assert_eq!(
                colors(&page),
                rotated,
                "EXIF {orientation} then reader rotation"
            );
        }
        // Apply orientation before checking whether a thumbnail needs resizing.
        let thumbnail = decode_thumbnail(&oriented_png(6), 2, 3).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (2, 1));
    }

    #[test]
    fn jpeg_metadata_applies_all_eight_orientations() {
        let source = image::load_from_memory(PNG).unwrap().into_rgb8();
        for (orientation, width, height, indices) in orientation_cases() {
            let mut bytes = Vec::new();
            let mut encoder = JpegEncoder::new_with_quality(&mut bytes, 100);
            encoder.set_exif_metadata(exif(orientation)).unwrap();
            encoder
                .encode(&source, 2, 3, image::ExtendedColorType::Rgb8)
                .unwrap();
            // The raw decoder deliberately does not apply orientation. Compare
            // JPEG colors exactly, without assumptions about lossy compression.
            let raw = image::load_from_memory(&bytes).unwrap().into_rgba8();
            let expected = indices.map(|index| raw.as_raw().as_chunks::<4>().0[index]);
            let page = decode_page(&bytes, 0, Filters::default()).unwrap();
            assert_eq!(
                (page.width, page.height),
                (width, height),
                "JPEG EXIF {orientation}"
            );
            assert_eq!(colors(&page), expected, "JPEG EXIF {orientation}");
            assert_eq!(decode_thumbnail(&bytes, 20, 20).unwrap(), page);
        }
    }

    #[test]
    fn webp_metadata_applies_all_eight_orientations() {
        let source = image::load_from_memory(PNG).unwrap().into_rgba8();
        for (orientation, width, height, indices) in orientation_cases() {
            let mut bytes = Vec::new();
            let mut encoder = WebPEncoder::new_lossless(&mut bytes);
            encoder.set_exif_metadata(exif(orientation)).unwrap();
            encoder
                .write_image(&source, 2, 3, image::ExtendedColorType::Rgba8)
                .unwrap();
            let expected = indices.map(|index| source.as_raw().as_chunks::<4>().0[index]);
            let page = decode_page(&bytes, 0, Filters::default()).unwrap();
            assert_eq!((page.width, page.height), (width, height));
            assert_eq!(colors(&page), expected, "WebP EXIF {orientation}");
            assert_eq!(decode_thumbnail(&bytes, 20, 20).unwrap(), page);
        }
    }

    #[test]
    fn static_frame_apis_return_the_first_rgba_image_with_zero_delay() {
        let frames = decode_frames(PNG, 90, Filters::default()).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].0,
            decode_page(PNG, 90, Filters::default()).unwrap()
        );
        assert_eq!(frames[0].1.numer_denom_ms(), (0, 1));
        let thumbnails = decode_thumbnail_frames(PNG, 1, 2).unwrap();
        assert_eq!(thumbnails.len(), 1);
        assert_eq!(thumbnails[0].0, decode_thumbnail(PNG, 1, 2).unwrap());
        assert_eq!(thumbnails[0].1.numer_denom_ms(), (0, 1));
    }

    #[test]
    fn gif_apng_and_webp_return_full_canvases_with_source_timing() {
        for bytes in animations() {
            let format = image::guess_format(&bytes).unwrap();
            let original = bytes.clone();
            let frames = decode_frames(&bytes, 0, Filters::default()).unwrap();
            assert_eq!(frames.len(), 2, "{format:?}");
            assert_eq!(colors(&frames[0].0), [RED, RED], "{format:?}");
            assert_eq!(colors(&frames[1].0), [RED, BLUE], "{format:?}");
            for (index, (page, delay)) in frames.iter().enumerate() {
                assert_eq!((page.width, page.height), (2, 1));
                assert_eq!(page.byte_size, 8);
                assert_eq!(page.pixels.len(), page.byte_size);
                assert_eq!(delay.numer_denom_ms(), ([30, 70][index], 1));
            }
            assert_eq!(
                decode_page(&bytes, 0, Filters::default()).unwrap(),
                frames[0].0
            );
            assert_eq!(bytes, original);
        }
    }

    #[test]
    fn apng_preserves_fractional_delays_and_the_default_denominator() {
        let mut first = true;
        let bytes = rewrite_png(&fixture(APNG_HEX), |kind, data| {
            if kind == b"fcTL" {
                let (numerator, denominator) = if first { (1u16, 3u16) } else { (7, 0) };
                data[20..22].copy_from_slice(&numerator.to_be_bytes());
                data[22..24].copy_from_slice(&denominator.to_be_bytes());
                first = false;
            }
            true
        });
        for frames in [
            decode_frames(&bytes, 0, Filters::default()).unwrap(),
            decode_thumbnail_frames(&bytes, 1, 1).unwrap(),
        ] {
            assert_eq!(frames[0].1.numer_denom_ms(), (1000, 3));
            assert_eq!(frames[1].1.numer_denom_ms(), (70, 1));
        }
    }

    #[test]
    fn reader_transforms_apply_to_every_animation_frame_but_not_thumbnails() {
        for bytes in animations() {
            let frames = decode_frames(
                &bytes,
                -270,
                Filters {
                    brightness: 50.0,
                    grayscale: 100.0,
                    ..Filters::default()
                },
            )
            .unwrap();
            assert_eq!(frames.len(), 2);
            assert_eq!((frames[0].0.width, frames[0].0.height), (1, 2));
            assert_eq!(colors(&frames[0].0), [[27, 27, 27, 255]; 2]);
            assert_eq!(colors(&frames[1].0), [[27, 27, 27, 255], [9, 9, 9, 255]]);
            let thumbnails = decode_thumbnail_frames(&bytes, 1, 1).unwrap();
            assert_eq!(thumbnails.len(), 2);
            assert_eq!(colors(&thumbnails[0].0), [RED]);
            assert_eq!(colors(&thumbnails[1].0), [[128, 0, 128, 255]]);
            assert_eq!(decode_thumbnail(&bytes, 1, 1).unwrap(), thumbnails[0].0);
            assert_eq!(
                decode_thumbnail_frames(&bytes, 20, 20).unwrap(),
                decode_frames(&bytes, 0, Filters::default()).unwrap()
            );
            for (index, (thumbnail, delay)) in thumbnails.iter().enumerate() {
                assert_eq!((thumbnail.width, thumbnail.height), (1, 1));
                assert_eq!(delay, &frames[index].1);
            }
        }
    }

    fn png_chunks(bytes: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
        let mut bytes = &bytes[8..];
        std::iter::from_fn(move || {
            if bytes.is_empty() {
                return None;
            }
            let size = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
            let chunk = &bytes[..size + 12];
            bytes = &bytes[size + 12..];
            Some((&chunk[4..8], &chunk[8..8 + size]))
        })
    }

    fn append_png_chunk(bytes: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = bytes.len();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(data);
        bytes.extend_from_slice(&crc32(&bytes[start..]).to_be_bytes());
    }

    fn rewrite_png(bytes: &[u8], mut change: impl FnMut(&[u8], &mut Vec<u8>) -> bool) -> Vec<u8> {
        let mut result = bytes[..8].to_vec();
        for (kind, data) in png_chunks(bytes) {
            let mut data = data.to_vec();
            if change(kind, &mut data) {
                append_png_chunk(&mut result, kind, &data);
            }
        }
        result
    }

    #[test]
    fn animated_exif_orientation_precedes_reader_rotation_for_every_frame() {
        let source = fixture(APNG_HEX);
        let mut apng = source[..33].to_vec();
        append_png_chunk(&mut apng, b"eXIf", &exif(2));
        apng.extend_from_slice(&source[33..]);
        let mut webp = fixture(WEBP_HEX);
        webp[20] |= 0x08; // VP8X EXIF flag.
        webp.extend_from_slice(b"EXIF");
        let metadata = exif(2);
        webp.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
        webp.extend_from_slice(&metadata);
        let size = (webp.len() - 8) as u32;
        webp[4..8].copy_from_slice(&size.to_le_bytes());
        for bytes in [apng, webp] {
            let frames = decode_frames(&bytes, 90, Filters::default()).unwrap();
            assert_eq!(frames.len(), 2);
            assert_eq!((frames[1].0.width, frames[1].0.height), (1, 2));
            assert_eq!(colors(&frames[1].0), [BLUE, RED]);
            assert_eq!(frames[1].1.numer_denom_ms(), (70, 1));
            let thumbnails = decode_thumbnail_frames(&bytes, 20, 20).unwrap();
            assert_eq!((thumbnails[1].0.width, thumbnails[1].0.height), (2, 1));
            assert_eq!(colors(&thumbnails[1].0), [BLUE, RED]);
        }
    }

    #[test]
    fn gif_disposal_restores_background_and_previous_canvas() {
        for (disposal, expected) in [(8, [0, 0, 0, 0]), (12, RED)] {
            let mut bytes = fixture(GIF_HEX);
            let gce = bytes
                .windows(3)
                .rposition(|part| part == [0x21, 0xf9, 4])
                .unwrap();
            bytes[gce + 3] = disposal;
            bytes.pop(); // Replace the trailer with a third, green subframe.
            bytes.extend_from_slice(&fixture(
                "21f90404090000002c00000000010001008000ff0000000002024401003b",
            ));
            let frames = decode_frames(&bytes, 0, Filters::default()).unwrap();
            assert_eq!(frames.len(), 3);
            assert_eq!(colors(&frames[1].0), [RED, BLUE]);
            assert_eq!(colors(&frames[2].0), [[0, 255, 0, 255], expected]);
            assert_eq!(frames[2].1.numer_denom_ms(), (90, 1));
        }
    }

    #[test]
    fn apng_skips_the_default_image_when_it_is_not_an_animation_frame() {
        let mut first_control = true;
        let bytes = rewrite_png(&fixture(APNG_HEX), |kind, data| {
            match kind {
                b"acTL" => data[..4].copy_from_slice(&1u32.to_be_bytes()),
                b"fcTL" if first_control => {
                    first_control = false;
                    return false;
                }
                b"fcTL" => data[..4].copy_from_slice(&0u32.to_be_bytes()),
                b"fdAT" => data[..4].copy_from_slice(&1u32.to_be_bytes()),
                _ => {}
            }
            true
        });
        let frames = decode_frames(&bytes, 0, Filters::default()).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(colors(&frames[0].0), [[0, 0, 0, 0], BLUE]);
        assert_eq!(frames[0].1.numer_denom_ms(), (70, 1));
        assert_eq!(
            decode_page(&bytes, 0, Filters::default()).unwrap(),
            frames[0].0
        );
        assert_eq!(decode_thumbnail(&bytes, 20, 20).unwrap(), frames[0].0);
    }

    #[test]
    fn animation_count_limits_fail_without_affecting_first_frame_apis() {
        let maximum = animated_gif(MAX_ANIMATION_FRAMES);
        assert_eq!(
            decode_frames(&maximum, 0, Filters::default())
                .unwrap()
                .len(),
            MAX_ANIMATION_FRAMES
        );
        let excessive = animated_gif(MAX_ANIMATION_FRAMES + 1);
        assert!(
            decode_frames(&excessive, 0, Filters::default())
                .unwrap_err()
                .contains("256-frame")
        );
        assert!(
            decode_thumbnail_frames(&excessive, 1, 1)
                .unwrap_err()
                .contains("256-frame")
        );
        assert_eq!(
            colors(&decode_page(&excessive, 0, Filters::default()).unwrap()),
            [RED, RED]
        );
        assert_eq!(colors(&decode_thumbnail(&excessive, 1, 1).unwrap()), [RED]);
        let apng = rewrite_png(&fixture(APNG_HEX), |kind, data| {
            if kind == b"acTL" {
                data[..4].copy_from_slice(&257u32.to_be_bytes());
            }
            true
        });
        assert!(
            decode_frames(&apng, 0, Filters::default())
                .unwrap_err()
                .contains("256-frame")
        );
        assert!(decode_page(&apng, 0, Filters::default()).is_ok());
        let source = fixture(WEBP_HEX);
        let mut webp = source[..44].to_vec();
        for _ in 0..=MAX_ANIMATION_FRAMES {
            webp.extend_from_slice(&source[44..92]);
        }
        let size = (webp.len() - 8) as u32;
        webp[4..8].copy_from_slice(&size.to_le_bytes());
        assert!(
            decode_frames(&webp, 0, Filters::default())
                .unwrap_err()
                .contains("256-frame")
        );
        assert!(
            decode_thumbnail_frames(&webp, 1, 1)
                .unwrap_err()
                .contains("256-frame")
        );
        assert!(decode_page(&webp, 0, Filters::default()).is_ok());
    }

    #[test]
    fn animation_total_memory_is_checked_before_large_pixel_allocations() {
        // Change headers only. Compressed pixels deliberately remain tiny and
        // invalid for these canvases. The budget must reject them before decode.
        let (width, height) = (8192u32, 4096u32);
        let mut first_control = true;
        let apng = rewrite_png(&fixture(APNG_HEX), |kind, data| {
            if kind == b"IHDR" {
                data[..4].copy_from_slice(&width.to_be_bytes());
                data[4..8].copy_from_slice(&height.to_be_bytes());
            } else if kind == b"fcTL" && first_control {
                data[4..8].copy_from_slice(&width.to_be_bytes());
                data[8..12].copy_from_slice(&height.to_be_bytes());
                first_control = false;
            }
            true
        });
        let mut webp = fixture(WEBP_HEX);
        webp[24..27].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
        webp[27..30].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
        for bytes in [apng, webp] {
            assert!(
                decode_frames(&bytes, 0, Filters::default())
                    .unwrap_err()
                    .contains("128 MiB decoded frame")
            );
            assert!(
                decode_thumbnail_frames(&bytes, 1, 1)
                    .unwrap_err()
                    .contains("128 MiB decoded frame")
            );
        }
        assert!(check_frame_budget(1, MAX_DECODE_BYTES).is_ok());
        assert!(check_frame_budget(2, MAX_DECODE_BYTES + 4).is_err());
        assert!(check_frame_budget(MAX_ANIMATION_FRAMES, 4 * MAX_ANIMATION_FRAMES as u64).is_ok());
        assert!(check_encoded_size(MAX_ENCODED_BYTES).is_ok());
        assert!(check_encoded_size(MAX_ENCODED_BYTES + 1).is_err());
    }

    #[test]
    fn malformed_animations_and_invalid_frame_parameters_fail() {
        for bytes in animations() {
            let truncated = &bytes[..bytes.len() / 2];
            assert!(decode_frames(truncated, 0, Filters::default()).is_err());
            assert!(decode_thumbnail_frames(truncated, 1, 1).is_err());
            assert!(decode_frames(&bytes, 45, Filters::default()).is_err());
            assert!(decode_thumbnail_frames(&bytes, 0, 1).is_err());
            assert!(decode_thumbnail_frames(&bytes, 1, 0).is_err());
        }
        let empty = image::Frames::new(Box::new(std::iter::empty()));
        assert!(
            collect_animation(empty, (2, 1), Orientation::NoTransforms, false, Ok)
                .unwrap_err()
                .contains("no frames")
        );
        let frame = Frame::from_parts(
            RgbaImage::from_pixel(1, 1, Rgba(RED)),
            1,
            0,
            Delay::from_numer_denom_ms(10, 1),
        );
        let partial = image::Frames::new(Box::new(std::iter::once(Ok(frame))));
        assert!(
            collect_animation(partial, (2, 1), Orientation::NoTransforms, false, Ok)
                .unwrap_err()
                .contains("full-canvas")
        );
    }

    #[test]
    fn webp_chunk_extents_and_embedded_dimensions_are_checked_before_decode() {
        let mut bytes = fixture(WEBP_HEX);
        let chunk = bytes.windows(4).position(|part| part == b"VP8L").unwrap();
        bytes[chunk + 4..chunk + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(
            decode_frames(&bytes, 0, Filters::default())
                .unwrap_err()
                .contains("WebP chunk")
        );
        let mut bytes = fixture(WEBP_HEX);
        bytes[chunk + 9..chunk + 13].copy_from_slice(&0x0fff_ffffu32.to_le_bytes());
        assert!(
            decode_frames(&bytes, 0, Filters::default())
                .unwrap_err()
                .contains("32-megapixel")
        );
        let mut bytes = fixture(WEBP_HEX);
        bytes[chunk + 9..chunk + 13].copy_from_slice(&2u32.to_le_bytes()); // 3 x 1, not 2 x 1.
        assert!(
            decode_frames(&bytes, 0, Filters::default())
                .unwrap_err()
                .contains("dimensions")
        );
        let mut bytes = fixture(WEBP_HEX);
        bytes[24..27].copy_from_slice(&8191u32.to_le_bytes()[..3]);
        bytes[27..30].copy_from_slice(&8191u32.to_le_bytes()[..3]);
        assert!(
            decode_thumbnail_frames(&bytes, 1, 1)
                .unwrap_err()
                .contains("32-megapixel")
        );
    }

    fn append_webp_chunk(bytes: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(data);
        if !data.len().is_multiple_of(2) {
            bytes.push(0);
        }
    }

    #[test]
    fn webp_forged_frame_metadata_cannot_trigger_a_metadata_allocation() {
        // image-webp 0.2.4 reads a second ANMF subchunk header from the first
        // subchunk's payload. This can create a false EXIF range of 4 GiB.
        // Orientation must come from validated, top-level EXIF slices instead.
        let mut bytes = fixture(WEBP_HEX)[..44].to_vec();
        let mut frame = vec![0; 16];
        frame[6] = 1; // 2 x 1.
        let mut alpha = b"EXIF".to_vec();
        alpha.extend_from_slice(&u32::MAX.to_le_bytes());
        append_webp_chunk(&mut frame, b"ALPH", &alpha);
        append_webp_chunk(&mut frame, b"VP8 ", &[0, 0, 0, 0x9d, 1, 0x2a, 2, 0, 1, 0]);
        append_webp_chunk(&mut bytes, b"ANMF", &frame);
        let size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        assert!(
            decode_frames(&bytes, 0, Filters::default())
                .unwrap_err()
                .contains("animation frame")
        );
        assert!(decode_thumbnail_frames(&bytes, 1, 1).is_err());
    }

    #[test]
    fn output_buffers_must_be_tightly_packed_rgba() {
        let extra = RgbaImage::from_raw(1, 1, vec![0; 8]).unwrap();
        assert!(
            DecodedPage::from_image(extra)
                .unwrap_err()
                .contains("RGBA buffer length")
        );
    }

    #[test]
    fn png_decodes_to_unchanged_native_rgba() {
        let bytes = PNG.to_vec();
        let decoded = decode_page(&bytes, 0, Filters::default()).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 3));
        assert_eq!(decoded.byte_size, 24);
        assert_eq!(decoded.pixels.len(), decoded.byte_size);
        assert_eq!(colors(&decoded), [RED, GREEN, BLUE, YELLOW, BLACK, WHITE]);
        assert_eq!(bytes, PNG);
    }

    #[test]
    fn quarter_turns_rotate_clockwise_and_preserve_alpha() {
        let cases = [
            (90, 3, 2, vec![BLACK, BLUE, RED, WHITE, YELLOW, GREEN]),
            (180, 2, 3, vec![WHITE, BLACK, YELLOW, BLUE, GREEN, RED]),
            (270, 3, 2, vec![GREEN, YELLOW, WHITE, RED, BLUE, BLACK]),
            (-90, 3, 2, vec![GREEN, YELLOW, WHITE, RED, BLUE, BLACK]),
            (-270, 3, 2, vec![BLACK, BLUE, RED, WHITE, YELLOW, GREEN]),
            (450, 3, 2, vec![BLACK, BLUE, RED, WHITE, YELLOW, GREEN]),
            (720, 2, 3, vec![RED, GREEN, BLUE, YELLOW, BLACK, WHITE]),
        ];
        for (rotation, width, height, expected) in cases {
            let page = decode_page(PNG, rotation, Filters::default()).unwrap();
            assert_eq!((page.width, page.height), (width, height));
            assert_eq!(colors(&page), expected, "rotation {rotation}");
        }
        assert!(decode_page(PNG, 45, Filters::default()).is_err());
        assert!(decode_page(PNG, i32::MIN, Filters::default()).is_err());
    }

    #[test]
    fn brightness_contrast_and_grayscale_match_known_values() {
        let cases = [
            (
                Filters {
                    brightness: 50.0,
                    ..Filters::default()
                },
                [128, 0, 0, 255],
            ),
            (
                Filters {
                    contrast: 50.0,
                    ..Filters::default()
                },
                [191, 64, 64, 255],
            ),
            (
                Filters {
                    grayscale: 100.0,
                    ..Filters::default()
                },
                [54, 54, 54, 255],
            ),
            (
                Filters {
                    grayscale: 50.0,
                    ..Filters::default()
                },
                [155, 27, 27, 255],
            ),
            (
                Filters {
                    brightness: 50.0,
                    contrast: 50.0,
                    grayscale: 0.0,
                },
                [128, 64, 64, 255],
            ),
        ];
        for (filters, expected) in cases {
            let page = decode_page(PNG, 0, filters).unwrap();
            assert_eq!(colors(&page)[0], expected, "{filters:?}");
            assert_eq!(colors(&page)[1][3], 128);
            assert_eq!(colors(&page)[4][3], 64);
            assert_eq!(colors(&page)[5][3], 0);
        }
        let page = decode_page(
            PNG,
            0,
            Filters {
                grayscale: 100.0,
                ..Filters::default()
            },
        )
        .unwrap();
        assert_eq!(colors(&page)[1], [182, 182, 182, 128]);
        assert_eq!(colors(&page)[2], [18, 18, 18, 255]);
    }

    #[test]
    fn filters_clamp_and_replace_nonfinite_values() {
        let clamped = Filters {
            brightness: -100.0,
            contrast: 500.0,
            grayscale: 200.0,
        }
        .normalized();
        assert_eq!(
            clamped,
            Filters {
                brightness: 50.0,
                contrast: 150.0,
                grayscale: 100.0
            }
        );
        let invalid = Filters {
            brightness: f32::NAN,
            contrast: f32::INFINITY,
            grayscale: f32::NEG_INFINITY,
        };
        assert_eq!(invalid.normalized(), Filters::default());
        assert_eq!(
            decode_page(PNG, 0, invalid).unwrap(),
            decode_page(PNG, 0, Filters::default()).unwrap()
        );
    }

    #[test]
    fn bgra_conversion_swaps_only_red_and_blue_without_allocating() {
        let page = decode_page(PNG, 0, Filters::default()).unwrap();
        let pointer = page.pixels.as_ptr();
        let pixels = page.into_bgra();
        assert_eq!(pointer, pixels.as_ptr());
        assert_eq!(&pixels[..4], &[0, 0, 255, 255]);
        assert_eq!(&pixels[4..8], &[0, 255, 0, 128]);
        assert_eq!(&pixels[8..12], &[255, 0, 0, 255]);
    }

    #[test]
    fn thumbnails_fit_both_bounds_and_do_not_enlarge() {
        let small = decode_thumbnail(PNG, 1, 2).unwrap();
        assert_eq!((small.width, small.height), (1, 2));
        assert_eq!(small.byte_size, 8);
        assert_eq!(
            decode_thumbnail(PNG, 200, 200).unwrap(),
            decode_page(PNG, 0, Filters::default()).unwrap()
        );
        assert_eq!(decode_thumbnail(PNG, u32::MAX, 1).unwrap().height, 1);
        assert!(decode_thumbnail(PNG, 0, 100).is_err());
        assert!(decode_thumbnail(PNG, 100, 0).is_err());
    }

    #[test]
    fn malformed_truncated_and_unknown_images_fail() {
        for bytes in [
            b"".as_slice(),
            b"not an image".as_slice(),
            &PNG[..8],
            &PNG[..40],
            &PNG[..60],
        ] {
            assert!(decode_page(bytes, 0, Filters::default()).is_err());
            assert!(decode_thumbnail(bytes, 100, 100).is_err());
        }
        let mut corrupt = PNG.to_vec();
        corrupt[50] ^= 0xff;
        assert!(decode_page(&corrupt, 0, Filters::default()).is_err());
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        !crc
    }

    #[test]
    fn extreme_png_dimensions_fail_before_pixel_decoding() {
        for (width, height) in [(u32::MAX, u32::MAX), (MAX_IMAGE_SIDE + 1, 1), (8192, 8192)] {
            let mut bytes = PNG.to_vec();
            bytes[16..20].copy_from_slice(&width.to_be_bytes());
            bytes[20..24].copy_from_slice(&height.to_be_bytes());
            let checksum = crc32(&bytes[12..29]);
            bytes[29..33].copy_from_slice(&checksum.to_be_bytes());
            assert!(decode_page(&bytes, 0, Filters::default()).is_err());
        }
        assert!(rgba_byte_count(0, 1).is_err());
        assert!(rgba_byte_count(1, 0).is_err());
        assert!(rgba_byte_count(u32::MAX, u32::MAX).is_err());
        assert_eq!(
            rgba_byte_count(8192, 4096).unwrap(),
            MAX_DECODE_BYTES as usize
        );
    }

    #[test]
    fn decoder_native_allocations_are_checked_before_rgba_conversion() {
        let mut bytes = PNG.to_vec();
        bytes[16..20].copy_from_slice(&8192u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&4096u32.to_be_bytes());
        bytes[24] = 16; // RGBA16: 256 MiB native, although RGBA8 would fit 128 MiB.
        let checksum = crc32(&bytes[12..29]);
        bytes[29..33].copy_from_slice(&checksum.to_be_bytes());
        assert!(
            decode_page(&bytes, 0, Filters::default())
                .unwrap_err()
                .contains("decoding memory limit")
        );
        assert!(
            decode_thumbnail(&bytes, 1, 1)
                .unwrap_err()
                .contains("decoding memory limit")
        );
    }

    #[test]
    fn long_pages_decode_and_make_thumbnails_within_the_total_pixel_limit() {
        let mut bytes = Vec::new();
        {
            let source = RgbaImage::from_pixel(1000, 20_000, Rgba([40, 80, 120, 255]));
            PngEncoder::new_with_quality(
                &mut bytes,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::NoFilter,
            )
            .write_image(&source, 1000, 20_000, image::ExtendedColorType::Rgba8)
            .unwrap();
        }
        let page = decode_page(&bytes, 0, Filters::default()).unwrap();
        assert_eq!((page.width, page.height), (1000, 20_000));
        assert_eq!(page.byte_size, 80_000_000);
        assert_eq!(&page.pixels[..4], &[40, 80, 120, 255]);
        assert_eq!(&page.pixels[page.byte_size - 4..], &[40, 80, 120, 255]);
        drop(page);
        let thumbnail = decode_thumbnail(&bytes, 144, 144).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (7, 144));
        assert_eq!(&thumbnail.pixels[..4], &[40, 80, 120, 255]);
        assert!(rgba_byte_count(1, MAX_IMAGE_SIDE).is_ok());
        assert!(rgba_byte_count(MAX_IMAGE_SIDE, 1).is_ok());
        assert!(rgba_byte_count(2, MAX_IMAGE_SIDE).is_err());
    }

    #[test]
    fn supported_formats_decode_native_pixels() {
        let source =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(3, 2, Rgba([40, 80, 120, 255])));
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::Gif,
            image::ImageFormat::Bmp,
            image::ImageFormat::WebP,
        ] {
            let mut bytes = Cursor::new(Vec::new());
            source.write_to(&mut bytes, format).unwrap();
            let page = decode_page(bytes.get_ref(), 0, Filters::default()).unwrap();
            assert_eq!((page.width, page.height), (3, 2), "{format:?}");
            assert_eq!(page.byte_size, 24);
        }
    }

    #[test]
    fn sixteen_bit_png_converts_to_eight_bit_rgba() {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(
                &[255, 255, 0, 0, 0, 0, 255, 255],
                1,
                1,
                image::ExtendedColorType::Rgba16,
            )
            .unwrap();
        let page = decode_page(&bytes, 0, Filters::default()).unwrap();
        assert_eq!(colors(&page), [RED]);
    }
}
