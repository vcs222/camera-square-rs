//! Dependency-free RGB565 raster drawing used by the framebuffer UI.

pub fn rgb(r: u8, g: u8, b: u8) -> u16 {
    ((u16::from(r) >> 3) << 11) | ((u16::from(g) >> 2) << 5) | (u16::from(b) >> 3)
}

pub struct Canvas<'a> {
    pub pixels: &'a mut [u16],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
}

impl Canvas<'_> {
    fn bounds(&self) -> (i64, i64) {
        (
            self.width.min(self.stride).min(i32::MAX as usize) as i64,
            self.height.min(i32::MAX as usize) as i64,
        )
    }

    pub fn pixel(&mut self, x: i32, y: i32, color: u16) {
        if x < 0
            || y < 0
            || x as usize >= self.width
            || x as usize >= self.stride
            || y as usize >= self.height
        {
            return;
        }
        if let Some(index) = (y as usize)
            .checked_mul(self.stride)
            .and_then(|row| row.checked_add(x as usize))
        {
            if let Some(pixel) = self.pixels.get_mut(index) {
                *pixel = color;
            }
        }
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u16) {
        if w <= 0 || h <= 0 {
            return;
        }
        let (width, height) = self.bounds();
        let left = i64::from(x).max(0).min(width) as usize;
        let right = (i64::from(x) + i64::from(w)).max(0).min(width) as usize;
        let top = i64::from(y).max(0).min(height) as usize;
        let bottom = (i64::from(y) + i64::from(h)).max(0).min(height) as usize;
        if left >= right || top >= bottom {
            return;
        }
        for row in top..bottom {
            let Some(start) = row
                .checked_mul(self.stride)
                .and_then(|offset| offset.checked_add(left))
            else {
                break;
            };
            if start >= self.pixels.len() {
                break;
            }
            let end = start.saturating_add(right - left).min(self.pixels.len());
            self.pixels[start..end].fill(color);
        }
    }

    /// Filled circle, matching the original C drawing primitive.
    pub fn circle(&mut self, cx: i32, cy: i32, radius: i32, color: u16) {
        if radius < 0 {
            return;
        }
        let (width, height) = self.bounds();
        let (cx, cy, r) = (i64::from(cx), i64::from(cy), i64::from(radius));
        let top = (cy - r).max(0);
        let bottom = (cy + r).min(height - 1);
        let left = (cx - r).max(0);
        let right = (cx + r).min(width - 1);
        if top > bottom || left > right {
            return;
        }
        for y in top..=bottom {
            for x in left..=right {
                let dx = x - cx;
                let dy = y - cy;
                if dx * dx + dy * dy <= r * r {
                    self.pixel(x as i32, y as i32, color);
                }
            }
        }
    }

    /// Clips before rasterizing so offscreen endpoints do not cause long loops.
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u16) {
        let (width, height) = self.bounds();
        if width == 0 || height == 0 {
            return;
        }
        let (x, y) = (f64::from(x0), f64::from(y0));
        let (dx, dy) = (f64::from(x1) - x, f64::from(y1) - y);
        let (mut enter, mut exit) = (0.0_f64, 1.0_f64);
        for (p, q) in [
            (-dx, x),
            (dx, (width - 1) as f64 - x),
            (-dy, y),
            (dy, (height - 1) as f64 - y),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return;
                }
            } else if p < 0.0 {
                enter = enter.max(q / p);
            } else {
                exit = exit.min(q / p);
            }
            if enter > exit {
                return;
            }
        }
        let mut ax = (x + enter * dx).round().clamp(0.0, (width - 1) as f64) as i64;
        let mut ay = (y + enter * dy).round().clamp(0.0, (height - 1) as f64) as i64;
        let bx = (x + exit * dx).round().clamp(0.0, (width - 1) as f64) as i64;
        let by = (y + exit * dy).round().clamp(0.0, (height - 1) as f64) as i64;
        let dx = (bx - ax).abs();
        let dy = -(by - ay).abs();
        let sx = if ax < bx { 1 } else { -1 };
        let sy = if ay < by { 1 } else { -1 };
        let mut error = dx + dy;
        loop {
            self.pixel(ax as i32, ay as i32, color);
            if ax == bx && ay == by {
                break;
            }
            let twice = error * 2;
            if twice >= dy {
                error += dy;
                ax += sx;
            }
            if twice <= dx {
                error += dx;
                ay += sy;
            }
        }
    }

    pub fn text(&mut self, x: i32, y: i32, text: &str, scale: i32, color: u16) {
        if scale <= 0 {
            return;
        }
        let mut cursor = i64::from(x);
        let (width, _) = self.bounds();
        for character in text.chars() {
            if cursor >= width {
                break;
            }
            let glyph = glyph(character);
            for row in 0..7 {
                for col in 0..5 {
                    if glyph[row * 6 + col] != b'1' {
                        continue;
                    }
                    let gx = cursor + col as i64 * i64::from(scale);
                    let gy = i64::from(y) + row as i64 * i64::from(scale);
                    if gx > i64::from(i32::MAX) || gy > i64::from(i32::MAX) {
                        continue;
                    }
                    self.fill_rect(gx as i32, gy as i32, scale, scale, color);
                }
            }
            cursor = cursor.saturating_add(6 * i64::from(scale));
        }
    }
}

fn glyph(character: char) -> &'static [u8; 41] {
    match character {
        'A' => b"01110/10001/10001/11111/10001/10001/10001",
        'B' => b"11110/10001/10001/11110/10001/10001/11110",
        'C' => b"01111/10000/10000/10000/10000/10000/01111",
        'D' => b"11110/10001/10001/10001/10001/10001/11110",
        'E' => b"11111/10000/10000/11110/10000/10000/11111",
        'F' => b"11111/10000/10000/11110/10000/10000/10000",
        'G' => b"01111/10000/10000/10111/10001/10001/01111",
        'H' => b"10001/10001/10001/11111/10001/10001/10001",
        'I' => b"11111/00100/00100/00100/00100/00100/11111",
        'J' => b"00111/00010/00010/00010/10010/10010/01100",
        'K' => b"10001/10010/10100/11000/10100/10010/10001",
        'L' => b"10000/10000/10000/10000/10000/10000/11111",
        'M' => b"10001/11011/10101/10101/10001/10001/10001",
        'N' => b"10001/11001/11001/10101/10011/10011/10001",
        'O' => b"01110/10001/10001/10001/10001/10001/01110",
        'P' => b"11110/10001/10001/11110/10000/10000/10000",
        'Q' => b"01110/10001/10001/10001/10101/10010/01101",
        'R' => b"11110/10001/10001/11110/10100/10010/10001",
        'S' => b"01111/10000/10000/01110/00001/00001/11110",
        'T' => b"11111/00100/00100/00100/00100/00100/00100",
        'U' => b"10001/10001/10001/10001/10001/10001/01110",
        'V' => b"10001/10001/10001/10001/10001/01010/00100",
        'W' => b"10001/10001/10001/10101/10101/10101/01010",
        'X' => b"10001/10001/01010/00100/01010/10001/10001",
        'Y' => b"10001/10001/01010/00100/00100/00100/00100",
        'Z' => b"11111/00001/00010/00100/01000/10000/11111",
        '0' => b"01110/10001/10011/10101/11001/10001/01110",
        '1' => b"00100/01100/00100/00100/00100/00100/01110",
        '2' => b"01110/10001/00001/00010/00100/01000/11111",
        '3' => b"11110/00001/00001/01110/00001/00001/11110",
        '4' => b"00010/00110/01010/10010/11111/00010/00010",
        '5' => b"11111/10000/10000/11110/00001/00001/11110",
        '6' => b"01110/10000/10000/11110/10001/10001/01110",
        '7' => b"11111/00001/00010/00100/01000/01000/01000",
        '8' => b"01110/10001/10001/01110/10001/10001/01110",
        '9' => b"01110/10001/10001/01111/00001/00001/01110",
        ':' => b"00000/00100/00100/00000/00100/00100/00000",
        '+' => b"00000/00100/00100/11111/00100/00100/00000",
        '-' => b"00000/00000/00000/11111/00000/00000/00000",
        '/' => b"00001/00010/00100/01000/10000/00000/00000",
        '.' => b"00000/00000/00000/00000/00000/00110/00110",
        _ => b"00000/00000/00000/00000/00000/00000/00000",
    }
}
