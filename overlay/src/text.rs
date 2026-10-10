//! A line of text under Mochi: the greeting ("Coucou, <name>!") and the tool
//! or state label the desktop widget writes under him.
//!
//! Rasterised here with `ab_glyph` (pure Rust, no C font stack) and a system
//! font found once with `fc-match`, then composited onto the frame in the
//! shm buffer's premultiplied ARGB8888, on a rounded dark pill so it reads
//! over any wallpaper. No font found: no text, nothing else changes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};

/// Where a bold sans usually lives when `fc-match` is not there.
const FALLBACKS: &[&str] = &[
    "/usr/share/fonts/noto/NotoSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/liberation/LiberationSans-Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
];

/// The UI font: `fc-match`'s bold sans-serif, else a known path.
fn find_font_path() -> Option<PathBuf> {
    let out = Command::new("fc-match")
        .args(["-f", "%{file}", "sans-serif:weight=bold"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok());
    if let Some(p) = out.map(PathBuf::from).filter(|p| p.is_absolute() && p.is_file()) {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        // ab_glyph reads TrueType/OpenType outlines, not bitmap or Type 1 fonts.
        if ext == "ttf" || ext == "otf" {
            return Some(p);
        }
    }
    FALLBACKS.iter().map(Path::new).find(|p| p.is_file()).map(Path::to_path_buf)
}

pub struct Label {
    font: Option<FontVec>,
}

impl Label {
    /// Looks the font up and loads it.
    pub fn new() -> Self {
        let font = find_font_path().and_then(|p| std::fs::read(p).ok()).and_then(|d| FontVec::try_from_vec(d).ok());
        Label { font }
    }

    pub fn available(&self) -> bool {
        self.font.is_some()
    }

    /// Width in pixels of `text` at `px`.
    pub fn width(&self, text: &str, px: f32) -> f32 {
        let Some(font) = &self.font else { return 0.0 };
        let scaled = font.as_scaled(PxScale::from(px));
        let mut w = 0.0;
        let mut prev = None;
        for c in text.chars() {
            let id = scaled.glyph_id(c);
            if let Some(p) = prev {
                w += scaled.kern(p, id);
            }
            w += scaled.h_advance(id);
            prev = Some(id);
        }
        w
    }

    /// `text` cut with an ellipsis so it is at most `max` pixels wide.
    pub fn fit(&self, text: &str, px: f32, max: f32) -> String {
        if self.width(text, px) <= max {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        for n in (0..chars.len()).rev() {
            let cut: String = chars[..n].iter().collect::<String>().trim_end().to_string() + "…";
            if self.width(&cut, px) <= max {
                return cut;
            }
        }
        String::new()
    }

    /// Draws `text` on a pill centred at the bottom of a `side` x `side`
    /// premultiplied ARGB8888 canvas. `px` is the font size in pixels.
    pub fn draw_bottom(&self, canvas: &mut [u8], side: u32, text: &str, px: f32) {
        let Some(font) = &self.font else { return };
        let pad = px * 0.55;
        let max = side as f32 - 2.0 * pad - 4.0;
        let text = self.fit(text, px, max);
        if text.is_empty() {
            return;
        }
        let w = self.width(&text, px);
        let scaled = font.as_scaled(PxScale::from(px));
        let h = scaled.ascent() - scaled.descent() + px * 0.3;
        let (pw, ph) = (w + 2.0 * pad, h);
        let x0 = (side as f32 - pw) / 2.0;
        let y0 = side as f32 - ph - 1.0;
        fill_pill(canvas, side, x0, y0, pw, ph, [0, 0, 0, 175]);
        let baseline = y0 + px * 0.15 + scaled.ascent();
        let mut x = x0 + pad;
        let mut prev = None;
        for c in text.chars() {
            let id = scaled.glyph_id(c);
            if let Some(p) = prev {
                x += scaled.kern(p, id);
            }
            let glyph = id.with_scale_and_position(PxScale::from(px), point(x, baseline));
            x += scaled.h_advance(id);
            prev = Some(id);
            if let Some(outlined) = font.outline_glyph(glyph) {
                let b = outlined.px_bounds();
                outlined.draw(|gx, gy, cov| {
                    let (cx, cy) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                    over(canvas, side, cx, cy, [244, 244, 244], cov);
                });
            }
        }
    }
}

/// Composites `rgb` at coverage `a` (0..=1) over a premultiplied ARGB pixel.
fn over(canvas: &mut [u8], side: u32, x: i32, y: i32, rgb: [u8; 3], a: f32) {
    if x < 0 || y < 0 || x >= side as i32 || y >= side as i32 || a <= 0.0 {
        return;
    }
    let a = a.min(1.0);
    let i = (y as usize * side as usize + x as usize) * 4;
    let px = &mut canvas[i..i + 4];
    let keep = 1.0 - a;
    // Bytes are B, G, R, A, premultiplied.
    px[0] = (rgb[2] as f32 * a + px[0] as f32 * keep).round().min(255.0) as u8;
    px[1] = (rgb[1] as f32 * a + px[1] as f32 * keep).round().min(255.0) as u8;
    px[2] = (rgb[0] as f32 * a + px[2] as f32 * keep).round().min(255.0) as u8;
    px[3] = (255.0 * a + px[3] as f32 * keep).round().min(255.0) as u8;
}

/// A rounded rectangle with soft edges, `rgba` straight (not premultiplied).
fn fill_pill(canvas: &mut [u8], side: u32, x: f32, y: f32, w: f32, h: f32, rgba: [u8; 4]) {
    let r = h / 2.0;
    let (x1, y1) = ((x + w).ceil() as i32, (y + h).ceil() as i32);
    for py in (y.floor() as i32)..y1 {
        for pxl in (x.floor() as i32)..x1 {
            let (cx, cy) = (pxl as f32 + 0.5, py as f32 + 0.5);
            // Distance to the pill's core segment, minus its radius.
            let nx = cx.clamp(x + r, x + w - r);
            let d = ((cx - nx).powi(2) + (cy - (y + r)).powi(2)).sqrt() - r;
            let cover = (0.5 - d).clamp(0.0, 1.0);
            if cover > 0.0 {
                over(canvas, side, pxl, py, [rgba[0], rgba[1], rgba[2]], cover * rgba[3] as f32 / 255.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font() -> Option<Label> {
        let l = Label::new();
        l.available().then_some(l)
    }

    #[test]
    fn over_is_premultiplied_and_clipped() {
        let mut c = vec![0u8; 2 * 2 * 4];
        over(&mut c, 2, 0, 0, [255, 0, 0], 0.5);
        assert_eq!(&c[..4], &[0, 0, 128, 128], "half red over nothing: B G R A");
        over(&mut c, 2, 9, 9, [255, 0, 0], 1.0);
        over(&mut c, 2, -1, 0, [255, 0, 0], 1.0);
        assert!(c[4..].iter().all(|b| *b == 0), "outside pixels are left alone");
        over(&mut c, 2, 0, 0, [255, 0, 0], 1.0);
        assert_eq!(&c[..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn pill_is_drawn_inside_its_box_with_soft_corners() {
        let mut c = vec![0u8; 40 * 40 * 4];
        fill_pill(&mut c, 40, 10.0, 10.0, 20.0, 10.0, [0, 0, 0, 200]);
        let a = |x: usize, y: usize| c[(y * 40 + x) * 4 + 3];
        assert_eq!(a(20, 15), 200, "middle is solid");
        assert_eq!(a(5, 15), 0, "outside is clear");
        assert!(a(10, 10) < 100, "the corner is rounded off");
    }

    #[test]
    fn no_font_means_no_drawing() {
        let l = Label { font: None };
        let mut c = vec![0u8; 16 * 16 * 4];
        l.draw_bottom(&mut c, 16, "Hello", 8.0);
        assert!(c.iter().all(|b| *b == 0));
        assert_eq!(l.width("Hello", 8.0), 0.0);
    }

    #[test]
    fn text_is_measured_cut_and_drawn_with_the_system_font() {
        let Some(l) = font() else { return eprintln!("no system font: skipped") };
        let w = l.width("Coucou, Gerick!", 10.0);
        assert!(w > 40.0 && w < 120.0, "plausible width {w}");
        assert_eq!(l.fit("Bash", 10.0, 200.0), "Bash");
        let cut = l.fit("A very long running tool name indeed", 10.0, 60.0);
        assert!(cut.ends_with('…') && l.width(&cut, 10.0) <= 60.0, "{cut}");
        assert_eq!(l.fit("Wide", 10.0, 1.0), "");
        let mut c = vec![0u8; 120 * 120 * 4];
        l.draw_bottom(&mut c, 120, "Coucou, Gerick!", 10.0);
        let rows: Vec<usize> = (0..120).filter(|y| c[(y * 120) * 4..((y + 1) * 120) * 4].chunks(4).any(|p| p[3] > 0)).collect();
        assert!(rows.first().is_some_and(|y| *y > 90) && *rows.last().unwrap() < 120, "drawn in the bottom band: {rows:?}");
        let lit = c.chunks(4).filter(|p| p[2] > 200).count();
        assert!(lit > 25, "the letters are bright: {lit}");
        // Idempotent in size: a longer text never leaves the canvas.
        l.draw_bottom(&mut c, 120, &"W".repeat(60), 10.0);
    }
}
