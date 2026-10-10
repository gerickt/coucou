//! Frames: PNG files from mochi/frames, decoded once, scaled to the surface's
//! pixel size (area average, premultiplied) and kept as wl_shm ARGB8888
//! (little endian: B, G, R, A, premultiplied).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A frame ready to copy into a buffer: `side` x `side` pixels.
pub struct Frame {
    pub side: u32,
    pub argb: Vec<u8>,
}

/// Straight RGBA8 of a PNG (palette and transparency expanded).
pub fn decode(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
    let size = reader.output_buffer_size().ok_or("png: frame too large")?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
    let (w, h) = (info.width, info.height);
    let px = (w * h) as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..px * 4].to_vec(),
        png::ColorType::Rgb => buf[..px * 3].as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..px * 2].as_chunks::<2>().0.iter().flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..px].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("png: palette not expanded".into()),
    };
    Ok((w, h, rgba))
}

/// Area-average resize of straight RGBA to `side` x `side`, premultiplied,
/// as ARGB8888 little endian bytes.
pub fn to_argb(w: u32, h: u32, rgba: &[u8], side: u32) -> Vec<u8> {
    let (sw, sh) = (w as usize, h as usize);
    let n = side as usize;
    let (fx, fy) = (sw as f64 / n as f64, sh as f64 / n as f64);
    let mut out = vec![0u8; n * n * 4];
    for dy in 0..n {
        let (y0, y1) = (dy as f64 * fy, (dy + 1) as f64 * fy);
        for dx in 0..n {
            let (x0, x1) = (dx as f64 * fx, (dx + 1) as f64 * fx);
            let mut acc = [0f64; 4];
            let mut area = 0.0;
            let mut sy = y0.floor() as usize;
            while (sy as f64) < y1 && sy < sh {
                let wy = (y1.min(sy as f64 + 1.0) - y0.max(sy as f64)).max(0.0);
                let mut sx = x0.floor() as usize;
                while (sx as f64) < x1 && sx < sw {
                    let wx = (x1.min(sx as f64 + 1.0) - x0.max(sx as f64)).max(0.0);
                    let k = wx * wy;
                    let p = &rgba[(sy * sw + sx) * 4..][..4];
                    let a = p[3] as f64 / 255.0;
                    acc[0] += p[0] as f64 * a * k;
                    acc[1] += p[1] as f64 * a * k;
                    acc[2] += p[2] as f64 * a * k;
                    acc[3] += p[3] as f64 * k;
                    area += k;
                    sx += 1;
                }
                sy += 1;
            }
            if area <= 0.0 {
                continue;
            }
            let o = &mut out[(dy * n + dx) * 4..][..4];
            let c = |v: f64| (v / area).round().clamp(0.0, 255.0) as u8;
            o[0] = c(acc[2]);
            o[1] = c(acc[1]);
            o[2] = c(acc[0]);
            o[3] = c(acc[3]);
        }
    }
    out
}

/// Decoded frames by file, for one pixel size. Bounded: past `limit` entries
/// it starts over (sequences reuse few files; a full reload costs little).
pub struct Cache {
    dir: PathBuf,
    side: u32,
    limit: usize,
    frames: HashMap<String, Frame>,
}

impl Cache {
    pub fn new(dir: PathBuf, side: u32) -> Self {
        Cache { dir, side, limit: 400, frames: HashMap::new() }
    }

    /// A new pixel size empties the cache.
    pub fn set_side(&mut self, side: u32) {
        if side != self.side {
            self.side = side;
            self.frames.clear();
        }
    }

    pub fn get(&mut self, file: &str) -> Option<&Frame> {
        if !self.frames.contains_key(file) {
            if self.frames.len() >= self.limit {
                self.frames.clear();
            }
            match decode(&self.dir.join(file)) {
                Ok((w, h, rgba)) => {
                    let argb = to_argb(w, h, &rgba, self.side);
                    self.frames.insert(file.to_string(), Frame { side: self.side, argb });
                }
                Err(e) => {
                    eprintln!("coucou-mochi: {e}");
                    return None;
                }
            }
        }
        self.frames.get(file)
    }
}

/// Copies `frame` into a `side` x `side` canvas, `lift` px higher (the peek),
/// clearing what it does not cover.
pub fn blit(canvas: &mut [u8], side: u32, frame: &Frame, lift: u32) {
    canvas.fill(0);
    let n = side.min(frame.side) as usize;
    let lift = lift as usize;
    for y in 0..n {
        let src_y = y + lift;
        if src_y >= frame.side as usize {
            break;
        }
        let src = &frame.argb[src_y * frame.side as usize * 4..][..n * 4];
        canvas[y * side as usize * 4..][..n * 4].copy_from_slice(src);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_premultiplies_and_averages() {
        // 2x2 -> 1x1: one opaque red pixel, three transparent ones.
        let rgba = [255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let out = to_argb(2, 2, &rgba, 1);
        // B, G, R, A premultiplied: a quarter of red at a quarter alpha.
        assert_eq!(out, [0, 0, 64, 64]);
    }

    #[test]
    fn same_size_is_a_premultiplied_copy() {
        let rgba = [200, 100, 50, 128];
        assert_eq!(to_argb(1, 1, &rgba, 1), [25, 50, 100, 128]);
    }

    #[test]
    fn blit_with_lift_shifts_up() {
        let frame = Frame { side: 2, argb: vec![1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4] };
        let mut canvas = vec![9u8; 16];
        blit(&mut canvas, 2, &frame, 1);
        assert_eq!(canvas, [3, 3, 3, 3, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn decodes_a_shipped_frame() {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../mochi"));
        let Ok(text) = std::fs::read_to_string(dir.join("frames/manifest.json")) else { return };
        let m = crate::manifest::parse(&text).unwrap();
        let file = m.seqs["none.idle"].frames[0].0.clone();
        let (w, h, rgba) = decode(&dir.join(&file)).unwrap();
        assert_eq!((w, h), (m.px, m.px));
        // Transparent corners, an opaque body in the middle.
        assert_eq!(rgba[3], 0);
        let mid = ((h / 2 * w + w / 2) * 4 + 3) as usize;
        assert_eq!(rgba[mid], 255);
        let mut cache = Cache::new(dir, 120);
        assert_eq!(cache.get(&file).unwrap().argb.len(), 120 * 120 * 4);
    }
}
