//! DNG output for an unpacked 12-bit Bayer stream from libcamera.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;

use tiff::encoder::{Rational, SRational, TiffEncoder};
use tiff::tags::Tag;

use crate::jpeg_exif::local_datetime;

pub struct RawCapture {
    pub data: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub cfa: [u8; 4],
    pub model: String,
    pub black_levels: Option<[i32; 4]>,
    pub colour_gains: Option<[f32; 2]>,
    pub colour_matrix: Option<[[f32; 3]; 3]>,
    pub exposure_us: Option<i32>,
    pub analogue_gain: Option<f32>,
}

fn multiply(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut result = [[0.0; 3]; 3];
    for row in 0..3 {
        for col in 0..3 {
            result[row][col] = (0..3).map(|k| a[row][k] * b[k][col]).sum();
        }
    }
    result
}

fn invert(m: [[f32; 3]; 3]) -> io::Result<[[f32; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if !det.is_finite() || det.abs() < 1e-6 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid DNG colour matrix",
        ));
    }
    let mut result = [[0.0; 3]; 3];
    for (row, values) in result.iter_mut().enumerate() {
        for (col, value) in values.iter_mut().enumerate() {
            let a = (col + 1) % 3;
            let b = (col + 2) % 3;
            let c = (row + 1) % 3;
            let d = (row + 2) % 3;
            *value = (m[a][c] * m[b][d] - m[a][d] * m[b][c]) / det;
        }
    }
    Ok(result)
}

fn colour_matrix(raw: &RawCapture) -> io::Result<[SRational; 9]> {
    let ccm = raw.colour_matrix.unwrap_or([
        [1.90255, -0.77478, -0.12777],
        [-0.31338, 1.88197, -0.56858],
        [-0.06001, -0.61785, 1.67786],
    ]);
    let gains = raw.colour_gains.unwrap_or([1.0, 1.0]);
    let rgb_to_xyz = [
        [0.4124564, 0.3575761, 0.1804375],
        [0.2126729, 0.7151522, 0.0721750],
        [0.0193339, 0.1191920, 0.9503041],
    ];
    let white_balance = [[gains[0], 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, gains[1]]];
    let matrix = invert(multiply(multiply(rgb_to_xyz, ccm), white_balance))?;
    Ok(std::array::from_fn(|index| {
        let value = matrix[index / 3][index % 3];
        SRational {
            n: (value * 100_000.0).round() as i32,
            d: 100_000,
        }
    }))
}

pub fn save_dng(path: &Path, raw: RawCapture) -> io::Result<()> {
    if raw.width == 0
        || raw.height == 0
        || raw.stride < raw.width * 2
        || raw.data.len() < raw.stride * raw.height
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid raw frame layout",
        ));
    }
    let mut pixels = Vec::with_capacity(raw.width * raw.height);
    for row in 0..raw.height {
        let start = row * raw.stride;
        for pair in raw.data[start..start + raw.width * 2].chunks_exact(2) {
            pixels.push(u16::from_le_bytes([pair[0], pair[1]]));
        }
    }
    let matrix = colour_matrix(&raw)?;
    let gains = raw.colour_gains.unwrap_or([1.0, 1.0]);
    let neutral = [
        Rational {
            n: 100_000,
            d: (gains[0] * 100_000.0).round().max(1.0) as u32,
        },
        Rational { n: 1, d: 1 },
        Rational {
            n: 100_000,
            d: (gains[1] * 100_000.0).round().max(1.0) as u32,
        },
    ];
    let mut black = [256_u16; 4];
    if let Some(levels) = raw.black_levels {
        // libcamera reports black levels in a normalized 16-bit range.
        let scaled = levels.map(|level| ((level.max(0) as u32 * 4096) / 65536) as u16);
        for (index, slot) in black.iter_mut().enumerate() {
            let colour = raw.cfa[index];
            let source = match colour {
                0 => 0,
                2 => 3,
                _ if raw.cfa[index ^ 1] == 0 => 1,
                _ => 2,
            };
            *slot = scaled[source];
        }
    }
    let result = (|| -> io::Result<()> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut encoder = TiffEncoder::new(file).map_err(io::Error::other)?;
        let datetime = local_datetime()?;
        let mut exif = encoder.extra_directory().map_err(io::Error::other)?;
        exif.write_tag(Tag::Unknown(0x9003), datetime.as_str())
            .map_err(io::Error::other)?;
        if let Some(exposure) = raw.exposure_us.filter(|value| *value > 0) {
            exif.write_tag(
                Tag::Unknown(0x829a),
                Rational {
                    n: exposure as u32,
                    d: 1_000_000,
                },
            )
            .map_err(io::Error::other)?;
        }
        if let Some(gain) = raw
            .analogue_gain
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            let iso = (gain * 100.0).round().clamp(1.0, u16::MAX as f32) as u16;
            exif.write_tag(Tag::Unknown(0x8827), iso)
                .map_err(io::Error::other)?;
        }
        let exif_offset = exif.finish_with_offsets().map_err(io::Error::other)?;
        let mut directory = encoder.image_directory().map_err(io::Error::other)?;
        let data_offset = directory
            .write_data(pixels.as_slice())
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::ImageWidth, raw.width as u32)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::ImageLength, raw.height as u32)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::BitsPerSample, 16_u16)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Compression, 1_u16)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::PhotometricInterpretation, 32803_u16)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::SamplesPerPixel, 1_u16)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::PlanarConfiguration, 1_u16)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::RowsPerStrip, raw.height as u32)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::StripOffsets, data_offset as u32)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::StripByteCounts, (pixels.len() * 2) as u32)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Orientation, 1_u16)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Make, "Raspberry Pi")
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Model, raw.model.as_str())
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Software, "Camera Square")
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::DateTime, datetime.as_str())
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::ExifDirectory, exif_offset.offset)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Unknown(50706), &[1_u8, 1, 0, 0][..])
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Unknown(50707), &[1_u8, 0, 0, 0][..])
            .map_err(io::Error::other)?;
        directory
            .write_tag(
                Tag::Unknown(50708),
                format!("Raspberry Pi {}", raw.model).as_str(),
            )
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Unknown(33421), &[2_u16, 2][..])
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Unknown(33422), &raw.cfa[..])
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Unknown(50713), &[2_u16, 2][..])
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Unknown(50714), &black[..])
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Unknown(50717), 4095_u16)
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Unknown(50721), &matrix[..])
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Unknown(50728), &neutral[..])
            .map_err(io::Error::other)?;
        directory
            .write_tag(Tag::Unknown(50778), 21_u16)
            .map_err(io::Error::other)?;
        directory.finish().map_err(io::Error::other)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverts_identity() {
        let matrix = invert([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]).unwrap();
        assert_eq!(matrix, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    }

    #[test]
    fn writes_bayer_dng_tags_and_pixels() {
        let path =
            std::env::temp_dir().join(format!("camera-square-dng-test-{}.dng", std::process::id()));
        let mut data = Vec::new();
        for value in 0_u16..16 {
            data.extend_from_slice(&value.to_le_bytes());
        }
        save_dng(
            &path,
            RawCapture {
                data,
                width: 4,
                height: 4,
                stride: 8,
                cfa: [2, 1, 1, 0],
                model: "imx477".into(),
                black_levels: None,
                colour_gains: None,
                colour_matrix: None,
                exposure_us: Some(16_000),
                analogue_gain: Some(2.0),
            },
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"II\x2a\0");
        let number =
            |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let ifd = number(4) as usize;
        let count = u16::from_le_bytes(bytes[ifd..ifd + 2].try_into().unwrap()) as usize;
        let entry = |tag: u16| {
            (0..count)
                .map(|index| ifd + 2 + index * 12)
                .find(|offset| {
                    u16::from_le_bytes(bytes[*offset..*offset + 2].try_into().unwrap()) == tag
                })
                .unwrap()
        };
        assert_eq!(number(entry(256) + 8), 4);
        assert_eq!(number(entry(257) + 8), 4);
        assert_eq!(
            u16::from_le_bytes(bytes[entry(262) + 8..entry(262) + 10].try_into().unwrap()),
            32803
        );
        assert_eq!(&bytes[entry(33422) + 8..entry(33422) + 12], &[2, 1, 1, 0]);
        assert_eq!(&bytes[entry(50706) + 8..entry(50706) + 12], &[1, 1, 0, 0]);
        let data_offset = number(entry(273) + 8) as usize;
        assert_eq!(&bytes[data_offset..data_offset + 4], &[0, 0, 1, 0]);
        std::fs::remove_file(path).unwrap();
    }
}
