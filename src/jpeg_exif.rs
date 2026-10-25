//! Small EXIF APP1 writer for native JPEGs.

use std::io;

pub struct PhotoMetadata {
    pub width: u32,
    pub height: u32,
    pub model: String,
    pub exposure_us: Option<i32>,
    pub analogue_gain: Option<f32>,
}

struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    data: Vec<u8>,
}

fn ascii(tag: u16, value: &str) -> Entry {
    let mut data = value.as_bytes().to_vec();
    data.push(0);
    Entry {
        tag,
        kind: 2,
        count: data.len() as u32,
        data,
    }
}

fn short(tag: u16, value: u16) -> Entry {
    Entry {
        tag,
        kind: 3,
        count: 1,
        data: value.to_le_bytes().to_vec(),
    }
}

fn long(tag: u16, value: u32) -> Entry {
    Entry {
        tag,
        kind: 4,
        count: 1,
        data: value.to_le_bytes().to_vec(),
    }
}

fn append_ifd(tiff: &mut Vec<u8>, mut entries: Vec<Entry>) -> usize {
    entries.sort_by_key(|entry| entry.tag);
    let start = tiff.len();
    let table_end = start + 2 + entries.len() * 12 + 4;
    tiff.resize(table_end, 0);
    tiff[start..start + 2].copy_from_slice(&(entries.len() as u16).to_le_bytes());
    for (index, entry) in entries.into_iter().enumerate() {
        let slot = start + 2 + index * 12;
        tiff[slot..slot + 2].copy_from_slice(&entry.tag.to_le_bytes());
        tiff[slot + 2..slot + 4].copy_from_slice(&entry.kind.to_le_bytes());
        tiff[slot + 4..slot + 8].copy_from_slice(&entry.count.to_le_bytes());
        if entry.data.len() <= 4 {
            tiff[slot + 8..slot + 8 + entry.data.len()].copy_from_slice(&entry.data);
        } else {
            let data_offset = tiff.len() as u32;
            tiff[slot + 8..slot + 12].copy_from_slice(&data_offset.to_le_bytes());
            tiff.extend_from_slice(&entry.data);
            if tiff.len() % 2 != 0 {
                tiff.push(0);
            }
        }
    }
    start
}

pub(crate) fn local_datetime() -> io::Result<String> {
    let mut now: libc::time_t = 0;
    // SAFETY: time writes to valid time_t storage.
    unsafe { libc::time(&mut now) };
    // SAFETY: zero is a valid initial tm value; localtime_r fills it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&now, &mut tm) }.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut output = [0 as libc::c_char; 32];
    let format = b"%Y:%m:%d %H:%M:%S\0";
    // SAFETY: both buffers have valid lengths and the format is NUL terminated.
    let written = unsafe {
        libc::strftime(
            output.as_mut_ptr(),
            output.len(),
            format.as_ptr().cast(),
            &tm,
        )
    };
    if written == 0 {
        return Err(io::Error::other("could not format EXIF timestamp"));
    }
    // SAFETY: strftime wrote a NUL-terminated ASCII string.
    Ok(unsafe { std::ffi::CStr::from_ptr(output.as_ptr()) }
        .to_string_lossy()
        .into_owned())
}

pub fn add_exif(jpeg: &[u8], metadata: &PhotoMetadata) -> io::Result<Vec<u8>> {
    if !jpeg.starts_with(&[0xff, 0xd8]) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing JPEG SOI",
        ));
    }
    let datetime = local_datetime()?;
    let mut tiff = b"II\x2a\0\x08\0\0\0".to_vec();
    let ifd0 = append_ifd(
        &mut tiff,
        vec![
            ascii(0x010f, "Raspberry Pi"),
            ascii(0x0110, &metadata.model),
            short(0x0112, 1),
            ascii(0x0131, "Camera Square"),
            ascii(0x0132, &datetime),
            long(0x8769, 0),
        ],
    );
    let mut exif_entries = vec![
        ascii(0x9003, &datetime),
        long(0xa002, metadata.width),
        long(0xa003, metadata.height),
    ];
    if let Some(exposure_us) = metadata.exposure_us.filter(|value| *value > 0) {
        let mut data = Vec::with_capacity(8);
        data.extend_from_slice(&(exposure_us as u32).to_le_bytes());
        data.extend_from_slice(&1_000_000_u32.to_le_bytes());
        exif_entries.push(Entry {
            tag: 0x829a,
            kind: 5,
            count: 1,
            data,
        });
    }
    if let Some(gain) = metadata
        .analogue_gain
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        exif_entries.push(short(
            0x8827,
            (gain * 100.0).round().clamp(1.0, u16::MAX as f32) as u16,
        ));
    }
    let exif_ifd = append_ifd(&mut tiff, exif_entries);
    let pointer = ifd0 + 2 + 5 * 12 + 8;
    tiff[pointer..pointer + 4].copy_from_slice(&(exif_ifd as u32).to_le_bytes());
    let payload_len = 6 + tiff.len();
    let segment_len = payload_len + 2;
    if segment_len > u16::MAX as usize {
        return Err(io::Error::other("EXIF segment too large"));
    }
    let mut result = Vec::with_capacity(jpeg.len() + segment_len + 2);
    result.extend_from_slice(&jpeg[..2]);
    result.extend_from_slice(&[0xff, 0xe1]);
    result.extend_from_slice(&(segment_len as u16).to_be_bytes());
    result.extend_from_slice(b"Exif\0\0");
    result.extend_from_slice(&tiff);
    result.extend_from_slice(&jpeg[2..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_exif_before_jpeg_data() {
        let jpeg = [0xff, 0xd8, 0xff, 0xd9];
        let result = add_exif(
            &jpeg,
            &PhotoMetadata {
                width: 4056,
                height: 3040,
                model: "imx477".into(),
                exposure_us: Some(16_000),
                analogue_gain: Some(2.0),
            },
        )
        .unwrap();
        assert_eq!(&result[..4], &[0xff, 0xd8, 0xff, 0xe1]);
        assert_eq!(&result[6..12], b"Exif\0\0");
        assert_eq!(&result[result.len() - 2..], &[0xff, 0xd9]);
        assert!(result.windows(6).any(|part| part == b"imx477"));
    }
}
