//! Photo discovery and reduced-resolution JPEG decoding for the gallery.

use std::fs::{self, File};
use std::io::{self, BufReader};
use std::path::Path;

use jpeg_decoder::{Decoder, PixelFormat};

use crate::state::PhotoEntry;

const MAX_GALLERY_PHOTOS: usize = 512;
const MAX_DECODE_BYTES: usize = 64 * 1024 * 1024;

pub fn scan(directory: &Path) -> io::Result<Vec<PhotoEntry>> {
    let mut photos = Vec::new();
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
        photos.push(PhotoEntry { path, modified });
    }
    photos.sort_unstable_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| a.path.cmp(&b.path))
    });
    photos.truncate(MAX_GALLERY_PHOTOS);
    Ok(photos)
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
    // Match the C gallery's quarter-size decoding to avoid retaining full
    // sensor-resolution RGB images on the Pi Zero.
    decoder
        .scale(
            info.width.div_ceil(4).max(1),
            info.height.div_ceil(4).max(1),
        )
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
