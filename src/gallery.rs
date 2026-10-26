//! Photo discovery and reduced-resolution JPEG decoding for the gallery.

use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::Path;
use std::time::UNIX_EPOCH;

use jpeg_decoder::{Decoder, PixelFormat};

use crate::state::{PhotoDetails, PhotoEntry, PhotoFormat};

const MAX_GALLERY_PHOTOS: usize = 512;
const MAX_DECODE_BYTES: usize = 64 * 1024 * 1024;
const MAX_EXIF_SCAN_BYTES: u64 = 256 * 1024;

pub fn scan(directory: &Path) -> io::Result<Vec<PhotoEntry>> {
    let mut photos = Vec::new();
    scan_jpegs(directory, false, &mut photos)?;
    let previews = directory.join(".gallery");
    if fs::symlink_metadata(&previews).is_ok_and(|metadata| metadata.is_dir()) {
        scan_jpegs(&previews, true, &mut photos)?;
    }
    photos.sort_unstable_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.path.cmp(&a.path))
    });
    photos.truncate(MAX_GALLERY_PHOTOS);
    photos.reverse();
    Ok(photos)
}

fn scan_jpegs(directory: &Path, dng_preview: bool, photos: &mut Vec<PhotoEntry>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|extension| extension.as_encoded_bytes().eq_ignore_ascii_case(b"jpg"))
        {
            continue;
        }
        // Do not let gallery entries redirect image reads outside the photo directory.
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let raw = raw_path(&path, dng_preview);
        let has_raw = fs::symlink_metadata(&raw).is_ok_and(|metadata| metadata.is_file());
        if dng_preview && !has_raw {
            continue;
        }
        let format = if dng_preview {
            PhotoFormat::Dng
        } else if has_raw {
            PhotoFormat::JpegAndDng
        } else {
            PhotoFormat::Jpeg
        };
        photos.push(PhotoEntry {
            path,
            modified,
            format,
        });
    }
    Ok(())
}

fn raw_path(path: &Path, dng_preview: bool) -> std::path::PathBuf {
    if dng_preview {
        path.parent()
            .and_then(Path::parent)
            .unwrap_or_else(|| path.parent().unwrap_or(path))
            .join(path.file_name().unwrap_or_default())
            .with_extension("dng")
    } else {
        path.with_extension("dng")
    }
}

pub fn read_details(photo: &PhotoEntry) -> io::Result<PhotoDetails> {
    let mut decoder = Decoder::new(BufReader::new(File::open(&photo.path)?));
    decoder.read_info().map_err(invalid_data)?;
    let info = decoder
        .info()
        .ok_or_else(|| invalid_data("JPEG has no image metadata"))?;
    let jpeg_bytes = fs::metadata(&photo.path)?.len();
    let raw_bytes = if photo.format == PhotoFormat::Jpeg {
        None
    } else {
        fs::metadata(raw_path(&photo.path, photo.format == PhotoFormat::Dng))
            .ok()
            .map(|metadata| metadata.len())
    };
    let exif = jpeg_exif_summary(&photo.path).unwrap_or_default();
    Ok(PhotoDetails {
        width: u32::from(info.width),
        height: u32::from(info.height),
        jpeg_bytes,
        raw_bytes,
        exposure_us: exif.exposure_us,
        iso: exif.iso,
        captured_at: exif.captured_at.or_else(|| local_timestamp(photo.modified)),
    })
}

fn local_timestamp(modified: std::time::SystemTime) -> Option<String> {
    let seconds = modified.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let seconds: libc::time_t = seconds.try_into().ok()?;
    // SAFETY: zero is a valid initial tm value and localtime_r fills it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers refer to initialized local variables.
    if unsafe { libc::localtime_r(&seconds, &mut tm) }.is_null() {
        return None;
    }
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    ))
}

#[derive(Default)]
struct ExifSummary {
    exposure_us: Option<u32>,
    iso: Option<u32>,
    captured_at: Option<String>,
}

fn jpeg_exif_summary(path: &Path) -> Option<ExifSummary> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_EXIF_SCAN_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut at = 2;
    while at + 4 <= bytes.len() && bytes[at] == 0xff {
        let marker = bytes[at + 1];
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        let segment_len = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        if segment_len < 2 || at + 2 + segment_len > bytes.len() {
            break;
        }
        let payload = &bytes[at + 4..at + 2 + segment_len];
        if marker == 0xe1 && payload.starts_with(b"Exif\0\0") {
            return parse_exif(&payload[6..]);
        }
        at += 2 + segment_len;
    }
    None
}

fn exif_u16(bytes: &[u8], offset: usize, little: bool) -> Option<u16> {
    let value: [u8; 2] = bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?;
    Some(if little {
        u16::from_le_bytes(value)
    } else {
        u16::from_be_bytes(value)
    })
}

fn exif_u32(bytes: &[u8], offset: usize, little: bool) -> Option<u32> {
    let value: [u8; 4] = bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?;
    Some(if little {
        u32::from_le_bytes(value)
    } else {
        u32::from_be_bytes(value)
    })
}

fn exif_field(bytes: &[u8], ifd: usize, tag: u16, little: bool) -> Option<(u16, &[u8])> {
    let count = usize::from(exif_u16(bytes, ifd, little)?).min(256);
    for index in 0..count {
        let entry = ifd.checked_add(2)?.checked_add(index.checked_mul(12)?)?;
        if exif_u16(bytes, entry, little)? != tag {
            continue;
        }
        let kind = exif_u16(bytes, entry + 2, little)?;
        let unit = match kind {
            2 => 1,
            3 => 2,
            4 => 4,
            5 => 8,
            _ => return None,
        };
        let length = usize::try_from(exif_u32(bytes, entry + 4, little)?)
            .ok()?
            .checked_mul(unit)?;
        if length == 0 || length > 256 {
            return None;
        }
        let start = if length <= 4 {
            entry + 8
        } else {
            usize::try_from(exif_u32(bytes, entry + 8, little)?).ok()?
        };
        return Some((kind, bytes.get(start..start.checked_add(length)?)?));
    }
    None
}

fn parse_exif(bytes: &[u8]) -> Option<ExifSummary> {
    let little = match bytes.get(..4)? {
        b"II\x2a\0" => true,
        b"MM\0\x2a" => false,
        _ => return None,
    };
    let ifd0 = usize::try_from(exif_u32(bytes, 4, little)?).ok()?;
    let (_, pointer) = exif_field(bytes, ifd0, 0x8769, little)?;
    let exif_ifd = usize::try_from(exif_u32(pointer, 0, little)?).ok()?;
    let exposure_us = exif_field(bytes, exif_ifd, 0x829a, little).and_then(|(kind, value)| {
        if kind != 5 {
            return None;
        }
        let numerator = f64::from(exif_u32(value, 0, little)?);
        let denominator = f64::from(exif_u32(value, 4, little)?);
        if denominator <= 0.0 {
            return None;
        }
        let us = numerator * 1_000_000.0 / denominator;
        (us.is_finite() && us > 0.0 && us <= u32::MAX as f64).then_some(us.round() as u32)
    });
    let iso = exif_field(bytes, exif_ifd, 0x8827, little).and_then(|(kind, value)| match kind {
        3 => exif_u16(value, 0, little).map(u32::from),
        4 => exif_u32(value, 0, little),
        _ => None,
    });
    let captured_at = exif_field(bytes, exif_ifd, 0x9003, little).and_then(|(kind, value)| {
        if kind != 2 {
            return None;
        }
        let text = std::str::from_utf8(value.split(|&byte| byte == 0).next()?).ok()?;
        let raw = text.as_bytes();
        if raw.len() < 16 || !raw.is_ascii() {
            return None;
        }
        Some(format!(
            "{}-{}-{} {}:{}",
            &text[0..4],
            &text[5..7],
            &text[8..10],
            &text[11..13],
            &text[14..16]
        ))
    });
    Some(ExifSummary {
        exposure_us,
        iso,
        captured_at,
    })
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

pub fn decode_rgb(path: &Path) -> io::Result<(Vec<u8>, usize, usize)> {
    let mut decoder = Decoder::new(BufReader::new(File::open(path)?));
    decoder.set_max_decoding_buffer_size(MAX_DECODE_BYTES);
    decoder.read_info().map_err(invalid_data)?;
    let info = decoder
        .info()
        .ok_or_else(|| invalid_data("JPEG has no image metadata"))?;
    // Keep compact DNG gallery previews at display resolution; quarter-size
    // decode is still needed for full sensor-resolution JPEGs on the Pi Zero.
    let (target_width, target_height) =
        if u32::from(info.width) * u32::from(info.height) <= 1_000_000 {
            (info.width, info.height)
        } else {
            (
                info.width.div_ceil(4).max(1),
                info.height.div_ceil(4).max(1),
            )
        };
    decoder
        .scale(target_width, target_height)
        .map_err(invalid_data)?;
    let mut pixels = decoder.decode().map_err(invalid_data)?;
    let info = decoder
        .info()
        .ok_or_else(|| invalid_data("JPEG has no decoded image metadata"))?;
    let width = usize::from(info.width);
    let height = usize::from(info.height);
    let count = width
        .checked_mul(height)
        .filter(|&count| count > 0)
        .ok_or_else(|| invalid_data("Invalid JPEG dimensions"))?;
    let expected = count
        .checked_mul(info.pixel_format.pixel_bytes())
        .ok_or_else(|| invalid_data("JPEG dimensions overflow"))?;
    if pixels.len() != expected {
        return Err(invalid_data("JPEG pixel buffer has an invalid length"));
    }
    match info.pixel_format {
        PixelFormat::RGB24 => {}
        PixelFormat::L8 => {
            let rgb_len = count
                .checked_mul(3)
                .ok_or_else(|| invalid_data("JPEG RGB dimensions overflow"))?;
            if rgb_len > MAX_DECODE_BYTES {
                return Err(invalid_data("JPEG exceeds gallery memory limit"));
            }
            pixels
                .try_reserve_exact(rgb_len - count)
                .map_err(io::Error::other)?;
            pixels.resize(rgb_len, 0);
            for index in (0..count).rev() {
                let value = pixels[index];
                pixels[index * 3..index * 3 + 3].fill(value);
            }
        }
        PixelFormat::CMYK32 => {
            // The decoder returns conventional CMYK. Compact in place so
            // conversion does not require a second image-sized allocation.
            for index in 0..count {
                let base = index * 4;
                let c = u16::from(pixels[base]);
                let m = u16::from(pixels[base + 1]);
                let y = u16::from(pixels[base + 2]);
                let k = u16::from(pixels[base + 3]);
                pixels[index * 3] = ((255 - c) * (255 - k) / 255) as u8;
                pixels[index * 3 + 1] = ((255 - m) * (255 - k) / 255) as u8;
                pixels[index * 3 + 2] = ((255 - y) * (255 - k) / 255) as u8;
            }
            pixels.truncate(count * 3);
        }
        PixelFormat::L16 => return Err(invalid_data("16-bit JPEG is unsupported")),
    }
    Ok((pixels, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jpeg_exif::{PhotoMetadata, add_exif};

    #[test]
    fn includes_dng_gallery_previews() {
        let directory =
            std::env::temp_dir().join(format!("camera-square-gallery-test-{}", std::process::id()));
        let previews = directory.join(".gallery");
        fs::create_dir_all(&previews).unwrap();
        fs::write(directory.join("regular.jpg"), []).unwrap();
        fs::write(directory.join("raw.dng"), []).unwrap();
        fs::write(previews.join("raw.jpg"), []).unwrap();
        let photos = scan(&directory).unwrap();
        assert_eq!(photos.len(), 2);
        assert!(photos.iter().any(
            |entry| entry.path == previews.join("raw.jpg") && entry.format == PhotoFormat::Dng
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn reads_capture_details_from_native_exif() {
        let path = std::env::temp_dir().join(format!(
            "camera-square-exif-test-{}.jpg",
            std::process::id()
        ));
        let jpeg = add_exif(
            &[0xff, 0xd8, 0xff, 0xd9],
            &PhotoMetadata {
                width: 4056,
                height: 3040,
                model: "imx477".into(),
                exposure_us: Some(125),
                analogue_gain: Some(4.0),
            },
        )
        .unwrap();
        fs::write(&path, jpeg).unwrap();
        let exif = jpeg_exif_summary(&path).unwrap();
        assert_eq!(exif.exposure_us, Some(125));
        assert_eq!(exif.iso, Some(400));
        assert!(exif.captured_at.is_some());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn dng_details_use_raw_file_size_and_preview_metadata() {
        let directory = std::env::temp_dir().join(format!(
            "camera-square-dng-details-test-{}",
            std::process::id()
        ));
        let previews = directory.join(".gallery");
        fs::create_dir_all(&previews).unwrap();
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, 90)
            .encode(&[100, 120, 140], 1, 1, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        let jpeg = add_exif(
            &jpeg,
            &PhotoMetadata {
                width: 1,
                height: 1,
                model: "imx477".into(),
                exposure_us: Some(2_000_000),
                analogue_gain: Some(2.0),
            },
        )
        .unwrap();
        fs::write(previews.join("raw.jpg"), jpeg).unwrap();
        fs::write(directory.join("raw.dng"), [1_u8, 2, 3, 4]).unwrap();
        let photo = scan(&directory).unwrap().pop().unwrap();
        let details = read_details(&photo).unwrap();
        assert_eq!(photo.format, PhotoFormat::Dng);
        assert_eq!((details.width, details.height), (1, 1));
        assert_eq!(details.raw_bytes, Some(4));
        assert_eq!(details.exposure_us, Some(2_000_000));
        fs::remove_dir_all(directory).unwrap();
    }
}
