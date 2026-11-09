//! Bounded CBZ archive inspection and image decoding without filesystem extraction.
//!
//! Safe non-image entries are ignored; PNG/JPEG entries must match their declared format.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

use enzui_iced_core::decode_limits::{DecodeLimits, DecodeVerdict};
use image::{ImageFormat, ImageReader};
use zip::ZipArchive;

use crate::message::{PageBitmap, PageThumbnail};
use crate::raster::PageExtent;

pub const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ARCHIVE_ENTRIES: usize = 2_048;
pub const MAX_PAGES: usize = 1_000;
pub const MAX_TOTAL_PIXELS: u64 = 512 * 1024 * 1024;
pub const MAX_TOTAL_RGBA_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const INITIAL_PAGES: usize = 1;
const IMAGE_LIMITS: DecodeLimits = DecodeLimits::DEFAULT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CbzPage {
    pub archive_name: String,
    pub extent: PageExtent,
    pub encoded_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct CbzLoadResult {
    pub entries: Vec<CbzPage>,
    pub pages: Vec<PageBitmap>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CbzError {
    OpenFailed,
    InvalidArchive,
    ArchiveTooLarge,
    TooManyEntries,
    TooManyPages,
    EmptyArchive,
    UnsafeEntry { index: usize },
    EncryptedEntry { index: usize },
    UnsupportedEntry { index: usize },
    DuplicateEntry { index: usize },
    EntryTooLarge { index: usize },
    TotalCompressedBytesExceeded,
    TotalUncompressedBytesExceeded,
    TotalPixelsExceeded,
    TotalDecodedBytesExceeded,
    CorruptEntry { index: usize },
    ImageLimit { index: usize, reason: &'static str },
    ArchiveChanged,
}

impl std::fmt::Display for CbzError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenFailed => f.write_str("Could not open the comic book archive."),
            Self::InvalidArchive => f.write_str("The comic book archive is corrupt."),
            Self::ArchiveTooLarge => f.write_str("The comic book archive is too large."),
            Self::TooManyEntries => f.write_str("The comic book archive has too many entries."),
            Self::TooManyPages => f.write_str("The comic book archive has too many pages."),
            Self::EmptyArchive => f.write_str("The comic book archive contains no pages."),
            Self::UnsafeEntry { index } => {
                write!(f, "Archive entry {} has an unsafe path.", index + 1)
            }
            Self::EncryptedEntry { index } => {
                write!(f, "Archive entry {} is encrypted.", index + 1)
            }
            Self::UnsupportedEntry { index } => write!(
                f,
                "Archive entry {} is not a supported PNG or JPEG image.",
                index + 1
            ),
            Self::DuplicateEntry { index } => {
                write!(f, "Archive entry {} duplicates another path.", index + 1)
            }
            Self::EntryTooLarge { index } => {
                write!(f, "Archive entry {} is too large.", index + 1)
            }
            Self::TotalCompressedBytesExceeded => {
                f.write_str("The comic book archive exceeds the compressed-byte limit.")
            }
            Self::TotalUncompressedBytesExceeded => {
                f.write_str("The comic book archive expands beyond the uncompressed-byte limit.")
            }
            Self::TotalPixelsExceeded => {
                f.write_str("The comic book archive exceeds the total pixel limit.")
            }
            Self::TotalDecodedBytesExceeded => {
                f.write_str("The comic book archive exceeds the decoded-byte limit.")
            }
            Self::CorruptEntry { index } => {
                write!(f, "Archive entry {} is corrupt.", index + 1)
            }
            Self::ImageLimit { index, reason } => {
                write!(f, "Archive entry {} exceeds the {reason} limit.", index + 1)
            }
            Self::ArchiveChanged => {
                f.write_str("The comic book archive changed while it was open.")
            }
        }
    }
}

impl std::error::Error for CbzError {}

pub fn load_cbz(path: &Path) -> Result<CbzLoadResult, CbzError> {
    let mut archive = open_archive(path)?;
    let mut entries = enumerate_entries(&mut archive)?;
    entries.sort_by(|a, b| natural_cmp(&a.archive_name, &b.archive_name));

    let mut total_pixels = 0u64;
    let mut total_rgba_bytes = 0u64;
    let mut pages = Vec::new();
    for (index, entry) in entries.iter_mut().enumerate() {
        let bytes = read_entry(&mut archive, entry, index)?;
        let format =
            format_for_name(&entry.archive_name).ok_or(CbzError::UnsupportedEntry { index })?;
        verify_format(&bytes, format, index)?;
        let (width, height) = image_dimensions(&bytes, format, index)?;
        enforce_image_limits(bytes.len() as u64, width, height, index)?;
        add_decoded_totals(&mut total_pixels, &mut total_rgba_bytes, width, height)?;

        let rgba = decode_rgba(bytes, format, index)?;
        entry.extent = PageExtent { width, height };
        if index < INITIAL_PAGES {
            pages.push(PageBitmap {
                index,
                rgba,
                width,
                height,
            });
        }
    }

    Ok(CbzLoadResult { entries, pages })
}

pub fn render_pages(
    path: &Path,
    entries: &[CbzPage],
    indices: &[usize],
) -> Result<Vec<PageBitmap>, CbzError> {
    if indices.len() > MAX_PAGES {
        return Err(CbzError::TooManyPages);
    }
    let mut archive = open_archive(path)?;
    let mut pages = Vec::with_capacity(indices.len());
    for &index in indices {
        let Some(entry) = entries.get(index) else {
            continue;
        };
        let bytes = read_entry(&mut archive, entry, index)?;
        let format =
            format_for_name(&entry.archive_name).ok_or(CbzError::UnsupportedEntry { index })?;
        verify_format(&bytes, format, index)?;
        let (width, height) = image_dimensions(&bytes, format, index)?;
        enforce_image_limits(bytes.len() as u64, width, height, index)?;
        if (PageExtent { width, height }) != entry.extent {
            return Err(CbzError::ArchiveChanged);
        }
        pages.push(PageBitmap {
            index,
            rgba: decode_rgba(bytes, format, index)?,
            width,
            height,
        });
    }
    Ok(pages)
}

pub fn render_first_page_thumbnail(path: &str, target_width: u32) -> Option<PageThumbnail> {
    let loaded = load_cbz(Path::new(path)).ok()?;
    let first = loaded.pages.into_iter().next()?;
    let image = image::RgbaImage::from_raw(first.width, first.height, first.rgba)?;
    let target_width = target_width.max(1);
    let resized = image::DynamicImage::ImageRgba8(image)
        .thumbnail(target_width, target_width.saturating_mul(2).max(1))
        .into_rgba8();
    Some(PageThumbnail {
        width: resized.width(),
        height: resized.height(),
        rgba: resized.into_raw(),
    })
}

fn open_archive(path: &Path) -> Result<ZipArchive<BufReader<File>>, CbzError> {
    let file = File::open(path).map_err(|_| CbzError::OpenFailed)?;
    let size = file.metadata().map_err(|_| CbzError::OpenFailed)?.len();
    if size > MAX_ARCHIVE_BYTES {
        return Err(CbzError::ArchiveTooLarge);
    }
    let archive = ZipArchive::new(BufReader::new(file)).map_err(|_| CbzError::InvalidArchive)?;
    let central_directory_start = archive.central_directory_start();
    let unique_entries = archive.len();
    let mut reader = archive.into_inner();
    validate_central_directory(&mut reader, central_directory_start, unique_entries)?;
    let archive = ZipArchive::new(reader).map_err(|_| CbzError::InvalidArchive)?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(CbzError::TooManyEntries);
    }
    Ok(archive)
}

fn validate_central_directory<R: Read + Seek>(
    reader: &mut R,
    start: u64,
    unique_entries: usize,
) -> Result<(), CbzError> {
    reader
        .seek(SeekFrom::Start(start))
        .map_err(|_| CbzError::InvalidArchive)?;
    let mut names = HashSet::new();
    let mut count = 0usize;
    loop {
        let mut signature = [0u8; 4];
        reader
            .read_exact(&mut signature)
            .map_err(|_| CbzError::InvalidArchive)?;
        if signature != *b"PK\x01\x02" {
            break;
        }
        let mut fixed = [0u8; 42];
        reader
            .read_exact(&mut fixed)
            .map_err(|_| CbzError::InvalidArchive)?;
        let name_len = usize::from(u16::from_le_bytes([fixed[24], fixed[25]]));
        let extra_len = u64::from(u16::from_le_bytes([fixed[26], fixed[27]]));
        let comment_len = u64::from(u16::from_le_bytes([fixed[28], fixed[29]]));
        let mut name = vec![0u8; name_len];
        reader
            .read_exact(&mut name)
            .map_err(|_| CbzError::InvalidArchive)?;
        if !names.insert(name) {
            return Err(CbzError::DuplicateEntry { index: count });
        }
        let variable_tail = extra_len
            .checked_add(comment_len)
            .ok_or(CbzError::InvalidArchive)?;
        let variable_tail = i64::try_from(variable_tail).map_err(|_| CbzError::InvalidArchive)?;
        reader
            .seek(SeekFrom::Current(variable_tail))
            .map_err(|_| CbzError::InvalidArchive)?;
        count = count.checked_add(1).ok_or(CbzError::TooManyEntries)?;
        if count > MAX_ARCHIVE_ENTRIES {
            return Err(CbzError::TooManyEntries);
        }
    }
    if count != unique_entries {
        return Err(CbzError::DuplicateEntry {
            index: unique_entries,
        });
    }
    Ok(())
}

fn enumerate_entries<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<Vec<CbzPage>, CbzError> {
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(CbzError::TooManyEntries);
    }
    let mut total_compressed_bytes = 0u64;
    let mut total_uncompressed_bytes = 0u64;
    let mut names = HashSet::new();
    let mut pages = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|_| CbzError::InvalidArchive)?;
        let name = entry.name();
        validate_entry_path(name, entry.enclosed_name().is_some(), index)?;
        if entry.encrypted() {
            return Err(CbzError::EncryptedEntry { index });
        }
        if !names.insert(name.to_string()) {
            return Err(CbzError::DuplicateEntry { index });
        }
        add_archive_entry_bytes(
            &mut total_compressed_bytes,
            &mut total_uncompressed_bytes,
            entry.compressed_size(),
            entry.size(),
            index,
        )?;
        if entry.is_dir() {
            continue;
        }
        if !entry.is_file() {
            return Err(CbzError::UnsafeEntry { index });
        }
        if is_macos_resource_metadata(name) || format_for_name(name).is_none() {
            continue;
        }
        pages.push(CbzPage {
            archive_name: name.to_string(),
            extent: PageExtent {
                width: 1,
                height: 1,
            },
            encoded_bytes: entry.size(),
        });
        if pages.len() > MAX_PAGES {
            return Err(CbzError::TooManyPages);
        }
    }
    if pages.is_empty() {
        return Err(CbzError::EmptyArchive);
    }
    Ok(pages)
}

fn add_archive_entry_bytes(
    total_compressed_bytes: &mut u64,
    total_uncompressed_bytes: &mut u64,
    compressed_bytes: u64,
    uncompressed_bytes: u64,
    index: usize,
) -> Result<(), CbzError> {
    if IMAGE_LIMITS.check_encoded(uncompressed_bytes).is_refused() {
        return Err(CbzError::EntryTooLarge { index });
    }
    *total_compressed_bytes = total_compressed_bytes
        .checked_add(compressed_bytes)
        .ok_or(CbzError::TotalCompressedBytesExceeded)?;
    if *total_compressed_bytes > MAX_ARCHIVE_BYTES {
        return Err(CbzError::TotalCompressedBytesExceeded);
    }
    *total_uncompressed_bytes = total_uncompressed_bytes
        .checked_add(uncompressed_bytes)
        .ok_or(CbzError::TotalUncompressedBytesExceeded)?;
    if *total_uncompressed_bytes > MAX_ARCHIVE_BYTES {
        return Err(CbzError::TotalUncompressedBytesExceeded);
    }
    Ok(())
}

fn is_macos_resource_metadata(name: &str) -> bool {
    let mut components = name.split(['/', '\\']);
    components.next() == Some("__MACOSX")
        && components
            .next_back()
            .is_some_and(|file_name| file_name.starts_with("._"))
}

fn validate_entry_path(name: &str, enclosed: bool, index: usize) -> Result<(), CbzError> {
    let has_traversal = name
        .split(['/', '\\'])
        .any(|component| component == "." || component == "..");
    let has_windows_prefix = name.as_bytes().get(1).is_some_and(|byte| *byte == b':');
    if name.is_empty()
        || name.contains('\0')
        || name.starts_with('/')
        || name.starts_with('\\')
        || has_windows_prefix
        || has_traversal
        || !enclosed
    {
        return Err(CbzError::UnsafeEntry { index });
    }
    Ok(())
}

fn read_entry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    expected: &CbzPage,
    index: usize,
) -> Result<Vec<u8>, CbzError> {
    let mut entry = archive
        .by_name(&expected.archive_name)
        .map_err(|_| CbzError::ArchiveChanged)?;
    if entry.encrypted() || !entry.is_file() || entry.enclosed_name().is_none() {
        return Err(CbzError::ArchiveChanged);
    }
    if entry.size() != expected.encoded_bytes
        || IMAGE_LIMITS.check_encoded(entry.size()).is_refused()
    {
        return Err(CbzError::ArchiveChanged);
    }
    let capacity = usize::try_from(entry.size()).map_err(|_| CbzError::EntryTooLarge { index })?;
    let mut bytes = Vec::with_capacity(capacity);
    (&mut entry)
        .take(IMAGE_LIMITS.max_encoded_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| CbzError::CorruptEntry { index })?;
    if bytes.len() as u64 != expected.encoded_bytes {
        return Err(CbzError::CorruptEntry { index });
    }
    Ok(bytes)
}

fn format_for_name(name: &str) -> Option<ImageFormat> {
    match Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        _ => None,
    }
}

fn verify_format(bytes: &[u8], expected: ImageFormat, index: usize) -> Result<(), CbzError> {
    match image::guess_format(bytes) {
        Ok(actual) if actual == expected => Ok(()),
        Ok(_) => Err(CbzError::UnsupportedEntry { index }),
        Err(_) => Err(CbzError::CorruptEntry { index }),
    }
}

fn image_dimensions(
    bytes: &[u8],
    format: ImageFormat,
    index: usize,
) -> Result<(u32, u32), CbzError> {
    ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .map_err(|_| CbzError::CorruptEntry { index })
}

fn add_decoded_totals(
    total_pixels: &mut u64,
    total_rgba_bytes: &mut u64,
    width: u32,
    height: u32,
) -> Result<(), CbzError> {
    let pixels = DecodeLimits::pixels(width, height);
    *total_pixels = total_pixels
        .checked_add(pixels)
        .ok_or(CbzError::TotalPixelsExceeded)?;
    if *total_pixels > MAX_TOTAL_PIXELS {
        return Err(CbzError::TotalPixelsExceeded);
    }
    let decoded_bytes =
        DecodeLimits::rgba_bytes(pixels).ok_or(CbzError::TotalDecodedBytesExceeded)?;
    *total_rgba_bytes = total_rgba_bytes
        .checked_add(decoded_bytes)
        .ok_or(CbzError::TotalDecodedBytesExceeded)?;
    if *total_rgba_bytes > MAX_TOTAL_RGBA_BYTES {
        return Err(CbzError::TotalDecodedBytesExceeded);
    }
    Ok(())
}

fn enforce_image_limits(
    encoded_bytes: u64,
    width: u32,
    height: u32,
    index: usize,
) -> Result<(), CbzError> {
    match IMAGE_LIMITS.check(encoded_bytes, width, height) {
        DecodeVerdict::Allow => Ok(()),
        refused => Err(CbzError::ImageLimit {
            index,
            reason: refused.reason(),
        }),
    }
}

fn decode_rgba(bytes: Vec<u8>, format: ImageFormat, index: usize) -> Result<Vec<u8>, CbzError> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(IMAGE_LIMITS.max_width);
    limits.max_image_height = Some(IMAGE_LIMITS.max_height);
    limits.max_alloc = Some(IMAGE_LIMITS.max_rgba_bytes);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    reader
        .decode()
        .map(|image| image.into_rgba8().into_raw())
        .map_err(|_| CbzError::CorruptEntry { index })
}

fn natural_cmp(a: &str, b: &str) -> Ordering {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let (mut ai, mut bi) = (0usize, 0usize);
    while ai < a.len() && bi < b.len() {
        if a[ai].is_ascii_digit() && b[bi].is_ascii_digit() {
            let a_start = ai;
            let b_start = bi;
            while ai < a.len() && a[ai].is_ascii_digit() {
                ai += 1;
            }
            while bi < b.len() && b[bi].is_ascii_digit() {
                bi += 1;
            }
            let a_digits = &a[a_start..ai];
            let b_digits = &b[b_start..bi];
            let a_number = trim_zeroes(a_digits);
            let b_number = trim_zeroes(b_digits);
            let number_order = a_number
                .len()
                .cmp(&b_number.len())
                .then_with(|| a_number.cmp(b_number));
            if number_order != Ordering::Equal {
                return number_order;
            }
            continue;
        }
        let order = a[ai].to_ascii_lowercase().cmp(&b[bi].to_ascii_lowercase());
        if order != Ordering::Equal {
            return order;
        }
        ai += 1;
        bi += 1;
    }
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn trim_zeroes(digits: &[u8]) -> &[u8] {
    let first = digits
        .iter()
        .position(|byte| *byte != b'0')
        .unwrap_or(digits.len().saturating_sub(1));
    &digits[first..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn png(width: u32, height: u32, value: u8) -> Vec<u8> {
        let pixels = image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([value, value.wrapping_add(1), value.wrapping_add(2), 255]),
        );
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn jpeg(width: u32, height: u32, value: u8) -> Vec<u8> {
        let pixels = image::RgbImage::from_pixel(
            width,
            height,
            image::Rgb([value, value.wrapping_add(1), value.wrapping_add(2)]),
        );
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(pixels)
            .write_to(&mut bytes, ImageFormat::Jpeg)
            .unwrap();
        bytes.into_inner()
    }

    fn archive(entries: &[(&str, Vec<u8>)]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.cbz");
        let file = File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        (dir, path)
    }

    fn replace_entry_name(path: &Path, from: &[u8], to: &[u8]) {
        assert_eq!(from.len(), to.len());
        let mut bytes = std::fs::read(path).unwrap();
        let mut replacements = 0;
        let mut offset = 0;
        while offset + from.len() <= bytes.len() {
            if bytes[offset..].starts_with(from) {
                bytes[offset..offset + to.len()].copy_from_slice(to);
                replacements += 1;
                offset += from.len();
            } else {
                offset += 1;
            }
        }
        assert_eq!(replacements, 2);
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn comic_admission_rejects_unsafe_corrupt_and_oversized_archives() {
        {
            let (_dir, traversal) = archive(&[
                ("../ComicInfo.xml", b"<comic/>".to_vec()),
                ("page1.png", png(1, 1, 1)),
            ]);
            assert!(matches!(
                load_cbz(&traversal),
                Err(CbzError::UnsafeEntry { .. })
            ));

            let (_dir, mismatch) = archive(&[
                ("__MACOSX/page1.png", jpeg(1, 1, 1)),
                ("page2.png", png(1, 1, 2)),
            ]);
            assert!(matches!(
                load_cbz(&mismatch),
                Err(CbzError::UnsupportedEntry { .. })
            ));

            let (_dir, duplicate) = archive(&[
                ("page1.png", png(1, 1, 1)),
                ("ComicInfo-a.xml", b"first".to_vec()),
                ("ComicInfo-b.xml", b"second".to_vec()),
            ]);
            replace_entry_name(&duplicate, b"ComicInfo-b.xml", b"ComicInfo-a.xml");
            assert!(matches!(
                load_cbz(&duplicate),
                Err(CbzError::DuplicateEntry { .. })
            ));

            let dir = tempfile::tempdir().unwrap();
            let symlink = dir.path().join("symlink.cbz");
            let file = File::create(&symlink).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            zip.add_symlink("ComicInfo.xml", "target", options).unwrap();
            zip.start_file("page1.png", options).unwrap();
            zip.write_all(&png(1, 1, 1)).unwrap();
            zip.finish().unwrap();
            assert!(matches!(
                load_cbz(&symlink),
                Err(CbzError::UnsafeEntry { .. })
            ));
        }
        {
            let invalid_dir = tempfile::tempdir().unwrap();
            let invalid = invalid_dir.path().join("invalid.cbz");
            std::fs::write(&invalid, b"not a zip").unwrap();
            assert_eq!(load_cbz(&invalid).unwrap_err(), CbzError::InvalidArchive);

            let (_dir, corrupt) = archive(&[("page1.png", b"not an image".to_vec())]);
            assert!(matches!(
                load_cbz(&corrupt),
                Err(CbzError::CorruptEntry { .. })
            ));

            let oversized_len = usize::try_from(IMAGE_LIMITS.max_encoded_bytes + 1).unwrap();
            let (_dir, oversized_metadata) = archive(&[
                ("notes.dat", vec![0; oversized_len]),
                ("page1.png", png(1, 1, 1)),
            ]);
            assert_eq!(
                load_cbz(&oversized_metadata).unwrap_err(),
                CbzError::EntryTooLarge { index: 0 }
            );

            assert!(matches!(
                enforce_image_limits(1, IMAGE_LIMITS.max_width + 1, 1, 0),
                Err(CbzError::ImageLimit {
                    reason: "width-too-large",
                    ..
                })
            ));
            assert!(matches!(
                enforce_image_limits(IMAGE_LIMITS.max_encoded_bytes + 1, 1, 1, 0),
                Err(CbzError::ImageLimit {
                    reason: "encoded-too-large",
                    ..
                })
            ));

            let mut compressed = MAX_ARCHIVE_BYTES;
            let mut uncompressed = 0;
            assert_eq!(
                add_archive_entry_bytes(&mut compressed, &mut uncompressed, 1, 0, 0),
                Err(CbzError::TotalCompressedBytesExceeded)
            );
            compressed = 0;
            uncompressed = MAX_ARCHIVE_BYTES;
            assert_eq!(
                add_archive_entry_bytes(&mut compressed, &mut uncompressed, 0, 1, 0),
                Err(CbzError::TotalUncompressedBytesExceeded)
            );

            let mut pixels = MAX_TOTAL_PIXELS;
            let mut rgba = MAX_TOTAL_RGBA_BYTES;
            assert_eq!(
                add_decoded_totals(&mut pixels, &mut rgba, 1, 1),
                Err(CbzError::TotalPixelsExceeded)
            );
            pixels = 0;
            rgba = MAX_TOTAL_RGBA_BYTES;
            assert_eq!(
                add_decoded_totals(&mut pixels, &mut rgba, 1, 1),
                Err(CbzError::TotalDecodedBytesExceeded)
            );
        }
        {
            let page_dir = tempfile::tempdir().unwrap();
            let page_path = page_dir.path().join("too-many-pages.cbz");
            let file = File::create(&page_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default();
            for index in 0..=MAX_PAGES {
                zip.start_file(format!("{index}.png"), options).unwrap();
                zip.write_all(b"x").unwrap();
            }
            zip.finish().unwrap();
            assert!(matches!(load_cbz(&page_path), Err(CbzError::TooManyPages)));

            let entry_dir = tempfile::tempdir().unwrap();
            let entry_path = entry_dir.path().join("too-many-entries.cbz");
            let file = File::create(&entry_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            for index in 0..=MAX_ARCHIVE_ENTRIES {
                zip.start_file(format!("metadata-{index}.dat"), options)
                    .unwrap();
                zip.write_all(b"x").unwrap();
            }
            zip.finish().unwrap();
            assert_eq!(load_cbz(&entry_path).unwrap_err(), CbzError::TooManyEntries);
        }
    }

    #[test]
    fn later_pages_are_decoded_on_demand() {
        let (_dir, path) = archive(&[
            ("5.png", png(5, 2, 5)),
            ("1.png", png(1, 2, 1)),
            ("4.png", png(4, 2, 4)),
            ("2.png", png(2, 2, 2)),
            ("3.png", png(3, 2, 3)),
        ]);
        let loaded = load_cbz(&path).unwrap();
        assert_eq!(loaded.pages.len(), INITIAL_PAGES);
        let page = render_pages(&path, &loaded.entries, &[4]).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].index, 4);
        assert_eq!(page[0].width, 5);
        assert_eq!(&page[0].rgba[..4], &[5, 6, 7, 255]);
    }
}
