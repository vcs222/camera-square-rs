//! Validate and copy libcamera YUV420 planes before a camera request is reused.

use std::io;

#[derive(Clone, Copy)]
pub struct Yuv420Layout {
    pub width: usize,
    pub height: usize,
    pub y_stride: usize,
    pub uv_stride: usize,
}

pub struct OwnedYuv420 {
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

pub fn copy_yuv420(planes: &[&[u8]], layout: Yuv420Layout) -> io::Result<OwnedYuv420> {
    let Yuv420Layout {
        width,
        height,
        y_stride,
        uv_stride,
    } = layout;
    if width == 0
        || height == 0
        || width % 2 != 0
        || height % 2 != 0
        || width > y_stride
        || width / 2 > uv_stride
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid YUV420 layout",
        ));
    }
    let y_len = y_stride
        .checked_mul(height)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "YUV420 size overflow"))?;
    let uv_len = uv_stride
        .checked_mul(height / 2)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "YUV420 size overflow"))?;
    let (y, u, v) = match planes {
        [combined] => {
            let needed = y_len
                .checked_add(uv_len)
                .and_then(|n| n.checked_add(uv_len))
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "YUV420 size overflow")
                })?;
            if combined.len() < needed {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short YUV420 frame",
                ));
            }
            (
                &combined[..y_len],
                &combined[y_len..y_len + uv_len],
                &combined[y_len + uv_len..needed],
            )
        }
        [y, u, v] if y.len() >= y_len && u.len() >= uv_len && v.len() >= uv_len => {
            (&y[..y_len], &u[..uv_len], &v[..uv_len])
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "YUV420 requires one contiguous or three planar buffers",
            ));
        }
    };
    Ok(OwnedYuv420 {
        y: y.to_vec(),
        u: u.to_vec(),
        v: v.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Yuv420Layout, copy_yuv420};

    #[test]
    fn copies_contiguous_frame_with_padding() {
        let layout = Yuv420Layout {
            width: 2,
            height: 2,
            y_stride: 4,
            uv_stride: 2,
        };
        let frame = [10, 20, 0, 0, 30, 40, 0, 0, 50, 0, 60, 0];
        let copy = copy_yuv420(&[&frame], layout).unwrap();
        assert_eq!(copy.y, [10, 20, 0, 0, 30, 40, 0, 0]);
        assert_eq!(copy.u, [50, 0]);
        assert_eq!(copy.v, [60, 0]);
    }

    #[test]
    fn copies_three_planes() {
        let layout = Yuv420Layout {
            width: 2,
            height: 2,
            y_stride: 2,
            uv_stride: 1,
        };
        let copy = copy_yuv420(&[&[1, 2, 3, 4], &[5], &[6]], layout).unwrap();
        assert_eq!(copy.y, [1, 2, 3, 4]);
        assert_eq!(copy.u, [5]);
        assert_eq!(copy.v, [6]);
    }

    #[test]
    fn rejects_short_frame_and_odd_dimensions() {
        let layout = Yuv420Layout {
            width: 2,
            height: 2,
            y_stride: 2,
            uv_stride: 1,
        };
        assert!(copy_yuv420(&[&[1, 2, 3]], layout).is_err());
        assert!(copy_yuv420(&[&[1, 2, 3, 4]], Yuv420Layout { width: 1, ..layout }).is_err());
    }
}
