//! Photo discovery and reduced-resolution JPEG decoding for the gallery.

use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::Path;

use jpeg_decoder::{Decoder, PixelFormat};

use crate::state::{PhotoEntry, PhotoFormat};

const MAX_GALLERY_PHOTOS: usize = 512;
const MAX_DECODE_BYTES: usize = 64 * 1024 * 1024;

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
        let raw = if dng_preview {
            directory
                .parent()
                .unwrap_or(directory)
                .join(path.file_name().unwrap_or_default())
                .with_extension("dng")
        } else {
            path.with_extension("dng")
        };
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
}
