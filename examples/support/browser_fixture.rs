//! Synthetic browser sources. All artwork and archive bytes are generated locally.

use anyhow::Result;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use std::{
    fs::{self, File},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

/// Expected automatic-cover outcomes after the classic-ZIP improvement.
/// The baseline rejects the large classic ZIP in `supported_paths` at 64 MiB.
#[derive(Debug)]
pub struct BrowserFixture {
    pub root: PathBuf,
    /// Launch this image, then press Backspace to browse `root`.
    pub source: PathBuf,
    pub supported_paths: Vec<PathBuf>,
    /// Formats or folder layouts outside the automatic-cover policy.
    pub unsupported_paths: Vec<PathBuf>,
    /// Malformed archives or undecodable first images. Never skip to a later image.
    pub failed_paths: Vec<PathBuf>,
}

const PALETTES: [[[u8; 3]; 3]; 5] = [
    [[190, 65, 55], [235, 175, 70], [90, 105, 155]],
    [[40, 145, 105], [145, 195, 135], [220, 170, 70]],
    [[50, 100, 190], [100, 175, 215], [165, 105, 180]],
    [[150, 65, 155], [220, 130, 150], [100, 120, 190]],
    [[200, 125, 40], [75, 155, 185], [90, 130, 90]],
];

impl BrowserFixture {
    /// Expected center pixel of the decoded cover, with small JPEG tolerance.
    /// The solid center rectangle is one quarter of each artwork dimension.
    pub fn expected_color(&self, path: &Path) -> Option<[u8; 4]> {
        let index = self
            .supported_paths
            .iter()
            .position(|candidate| candidate == path)?;
        let variant = [0, 3, 4, 1, 2, 0, 1, 1, 1, 2, 1][index];
        let [red, green, blue] = PALETTES[variant][1];
        Some([red, green, blue, 255])
    }

    /// Creates sources only in a new or empty directory. Does not read user settings.
    pub fn create(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        anyhow::ensure!(
            fs::read_dir(root)?.next().is_none(),
            "Fixture root must be empty"
        );
        let root = root.canonicalize()?;
        let mut fixture = Self {
            source: root.join("01 - Launch source.png"),
            root,
            supported_paths: Vec::new(),
            unsupported_paths: Vec::new(),
            failed_paths: Vec::new(),
        };
        let png = artwork(320, 448, 0, ImageFormat::Png)?;
        let green = artwork(320, 448, 1, ImageFormat::Png)?;
        let blue = artwork(320, 448, 2, ImageFormat::Png)?;
        let jpeg = artwork(320, 448, 3, ImageFormat::Jpeg)?;
        let landscape = artwork(512, 288, 4, ImageFormat::Png)?;
        fs::write(&fixture.source, &png)?;
        fixture.supported_paths.push(fixture.source.clone());
        for (name, bytes) in [
            ("02 - Encoded JPEG cover.jpg", &jpeg),
            ("03 - Landscape panels - complete image.png", &landscape),
            (
                "[Synthetic Press][Autumn Collection] 星海紀行 - 遙遠城市的記憶與約定 - 第01卷.png",
                &green,
            ),
            (
                "[Synthetic Press][Autumn Collection] 星海紀行 - 遙遠城市的記憶與約定 - 第02卷.png",
                &blue,
            ),
            (
                "UnbrokenSyntheticCollectionCommonPrefixABCDEFGHIJKLMNOPQRSTUVWXYZVolume03.png",
                &png,
            ),
            (
                "UnbrokenSyntheticCollectionCommonPrefixABCDEFGHIJKLMNOPQRSTUVWXYZVolume04.png",
                &green,
            ),
        ] {
            let path = fixture.root.join(name);
            fs::write(&path, bytes)?;
            fixture.supported_paths.push(path);
        }

        let folder = fixture.root.join("A - Folder with natural-order panels");
        fs::create_dir(&folder)?;
        fs::write(folder.join("10.png"), &blue)?;
        fs::write(folder.join("2.png"), &green)?;
        fixture.supported_paths.push(folder);
        for (name, method, first) in [
            (
                "04 - Stored natural order 2 before 10.cbz",
                CompressionMethod::Stored,
                &green,
            ),
            (
                "05 - Deflated natural order 2 before 10.cbz",
                CompressionMethod::Deflated,
                &blue,
            ),
        ] {
            let path = fixture.root.join(name);
            let mut archive = ZipWriter::new(File::create(&path)?);
            let options = SimpleFileOptions::default().compression_method(method);
            // Physical insertion order intentionally differs from natural order.
            archive.start_file("10.png", options)?;
            archive.write_all(&png)?;
            archive.start_file("2.png", options)?;
            archive.write_all(first)?;
            archive.finish()?;
            fixture.supported_paths.push(path);
        }

        let large = fixture.root.join("06 - Large classic ZIP over 64 MiB.cbz");
        let mut archive = ZipWriter::new(File::create(&large)?);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        archive.start_file("attachment/zeroes.dat", stored)?;
        // io::copy uses a bounded buffer. The attachment is real Stored data with
        // a valid CRC, not trailing padding, a forged size, or a giant Vec.
        io::copy(&mut io::repeat(0).take(65 * 1024 * 1024), &mut archive)?;
        archive.start_file("2.png", stored)?;
        archive.write_all(&green)?;
        archive.finish()?;
        fixture.supported_paths.push(large);

        let empty = fixture.root.join("B - Empty folder - no cover");
        fs::create_dir(&empty)?;
        fixture.unsupported_paths.push(empty);
        let nested = fixture
            .root
            .join("C - Nested-only folder - no shallow cover");
        fs::create_dir_all(nested.join("child"))?;
        fs::write(nested.join("child/1.png"), &blue)?;
        fixture.unsupported_paths.push(nested);
        let zip64 = fixture
            .root
            .join("07 - ZIP64 - automatic cover unsupported.cbz");
        let mut archive = ZipWriter::new(File::create(&zip64)?);
        archive.start_file("1.png", stored.large_file(true))?;
        archive.write_all(&png)?;
        // The local header contains a real ZIP64 extra field, even for this tiny page.
        archive.finish()?;
        fixture.unsupported_paths.push(zip64);
        let rar = fixture
            .root
            .join("08 - Invalid RAR - automatic cover unsupported.rar");
        fs::write(&rar, b"Rar!\x1a\x07\x00invalid synthetic archive")?;
        fixture.unsupported_paths.push(rar);

        let invalid = fixture.root.join("09 - Invalid CBZ - broken archive.cbz");
        fs::write(&invalid, b"PK\x03\x04invalid synthetic archive")?;
        fixture.failed_paths.push(invalid);
        let bad_folder = fixture.root.join("D - Bad first image - do not use second");
        fs::create_dir(&bad_folder)?;
        fs::write(bad_folder.join("1.png"), b"Not an encoded image")?;
        fs::write(bad_folder.join("2.png"), &green)?;
        fixture.failed_paths.push(bad_folder);
        Ok(fixture)
    }
}

fn artwork(width: u32, height: u32, variant: usize, format: ImageFormat) -> Result<Vec<u8>> {
    let mut page = RgbImage::from_pixel(width, height, Rgb([248, 244, 230]));
    for y in 16..height - 16 {
        for x in 16..width - 16 {
            let panel = if y < height / 3 {
                0
            } else if x < width * 3 / 5 {
                1
            } else {
                2
            };
            let border = x < 20
                || x >= width - 20
                || y < 20
                || y >= height - 20
                || y.abs_diff(height / 3) < 6
                || (y >= height / 3 && x.abs_diff(width * 3 / 5) < 6);
            let center = x.abs_diff(width / 2) < width / 8 && y.abs_diff(height / 2) < height / 8;
            let motif = !center && (x / 24 + y / 24 + variant as u32).is_multiple_of(9);
            page.put_pixel(
                x,
                y,
                Rgb(if center {
                    PALETTES[variant][1]
                } else if border {
                    [30, 35, 40]
                } else if motif {
                    [245, 235, 205]
                } else {
                    PALETTES[variant][panel]
                }),
            );
        }
    }
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgb8(page).write_to(&mut encoded, format)?;
    Ok(encoded.into_inner())
}
