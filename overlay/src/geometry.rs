//! Where Mochi sits: his spot per monitor, the drag, the flight home, the
//! input region, the peek at the top edge. Ports desktop.rs's `logic` module
//! (clamp_origin, default_spot, MARGIN) and desktop-logic.ts (isOverBody,
//! BODY_RADIUS_FRACTION). Coordinates are logical pixels, y down; a spot is
//! the surface's top-left corner relative to its monitor.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};

/// Kept between him and the monitor's edges (DesktopMochiLogic.clampMargin).
pub const MARGIN: f64 = 24.0;
/// Clickable body radius, as a fraction of the surface side.
pub const BODY_RADIUS_FRACTION: f64 = 0.24;
/// A press that moves farther than this is a drag, not a click.
pub const DRAG_THRESHOLD: f64 = 3.0;
/// Peek: dropped this close to the top edge, he docks there.
pub const DOCK_DISTANCE: f64 = 16.0;
/// Docked, this much of the side stays hidden above the edge.
pub const PEEK_HIDDEN: f64 = 0.45;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// Keeps a surface of side `side` inside a monitor of `size`, `margin` from each edge.
pub fn clamp_spot(spot: (f64, f64), side: f64, size: Size, margin: f64) -> (f64, f64) {
    let max_x = (size.w - side - margin).max(margin);
    let max_y = (size.h - side - margin).max(margin);
    (spot.0.max(margin).min(max_x), spot.1.max(margin).min(max_y))
}

/// Home: the bottom-right corner, like the Mac's first visit.
pub fn default_spot(side: f64, size: Size) -> (f64, f64) {
    clamp_spot((size.w - side - MARGIN, size.h - side - MARGIN), side, size, MARGIN)
}

/// Hit test of the round body inside the square surface (surface-local).
pub fn is_over_body(local: (f64, f64), side: f64) -> bool {
    let c = side / 2.0;
    let r = side * BODY_RADIUS_FRACTION;
    let (dx, dy) = (local.0 - c, local.1 - c);
    dx * dx + dy * dy <= r * r
}

/// The new spot while dragging: the global cursor, minus where he was
/// grabbed, minus the monitor's origin.
pub fn drag_spot(cursor: (f64, f64), grab: (f64, f64), monitor_origin: (f64, f64)) -> (f64, f64) {
    (cursor.0 - grab.0 - monitor_origin.0, cursor.1 - grab.1 - monitor_origin.1)
}

/// Where a drag ends: docked at the top edge (peek on and close to it), or
/// clamped inside the monitor.
pub fn drop_spot(spot: (f64, f64), side: f64, size: Size, peek: bool) -> ((f64, f64), bool) {
    if peek && spot.1 <= MARGIN + DOCK_DISTANCE {
        let x = spot.0.max(MARGIN).min((size.w - side - MARGIN).max(MARGIN));
        return ((x, 0.0), true);
    }
    (clamp_spot(spot, side, size, MARGIN), false)
}

/// The body disc as 2 px rows (a region is made of rectangles), shifted up by
/// `lift` px and cut at the surface's top: (x, y, w, h).
pub fn body_rows(side: f64, lift: f64) -> Vec<(i32, i32, i32, i32)> {
    let c = side / 2.0;
    let r = side * BODY_RADIUS_FRACTION;
    let cy = c - lift;
    let mut rows = Vec::new();
    let mut y = (cy - r).floor();
    while y < cy + r {
        let mid = y + 1.0 - cy;
        let half = (r * r - mid * mid).max(0.0).sqrt();
        let (top, bottom) = (y.max(0.0), y + 2.0);
        if bottom > top && half > 0.0 {
            rows.push(((c - half).floor() as i32, top as i32, (2.0 * half).ceil() as i32, (bottom - top) as i32));
        }
        y += 2.0;
    }
    rows
}

/// Smooth flight between two spots: ease-in-out, `p` from 0 to 1.
pub fn flight(from: (f64, f64), to: (f64, f64), p: f64) -> (f64, f64) {
    let p = p.clamp(0.0, 1.0);
    let e = if p < 0.5 { 2.0 * p * p } else { 1.0 - (-2.0 * p + 2.0).powi(2) / 2.0 };
    (from.0 + (to.0 - from.0) * e, from.1 + (to.1 - from.1) * e)
}

/// One saved spot: top-left on the monitor, and docked at the top edge.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    pub x: f64,
    pub y: f64,
    pub docked: bool,
}

/// ~/.local/state/coucou/overlay.json: { "spots": { "<monitor>": {x, y, docked} } }.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spots(pub BTreeMap<String, Spot>);

impl Spots {
    pub fn parse(text: &str) -> Spots {
        let mut out = BTreeMap::new();
        let Ok(v) = serde_json::from_str::<Value>(text) else { return Spots(out) };
        if let Some(map) = v.get("spots").and_then(Value::as_object) {
            for (name, s) in map {
                let x = s.get("x").and_then(Value::as_f64);
                let y = s.get("y").and_then(Value::as_f64);
                let docked = s.get("docked").and_then(Value::as_bool).unwrap_or(false);
                if let (Some(x), Some(y)) = (x, y) {
                    if x.is_finite() && y.is_finite() {
                        out.insert(name.clone(), Spot { x, y, docked });
                    }
                }
            }
        }
        Spots(out)
    }

    pub fn to_json(&self) -> String {
        let spots: serde_json::Map<String, Value> = self
            .0
            .iter()
            .map(|(k, s)| (k.clone(), json!({ "x": s.x.round(), "y": s.y.round(), "docked": s.docked })))
            .collect();
        format!("{}\n", json!({ "spots": spots }))
    }

    pub fn load(path: &Path) -> Spots {
        std::fs::read_to_string(path).map(|t| Spots::parse(&t)).unwrap_or_default()
    }

    /// Written through a temporary file, so a crash never leaves half a file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json())?;
        std::fs::rename(tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: Size = Size { w: 1920.0, h: 1080.0 };

    #[test]
    fn home_is_the_bottom_right_corner() {
        assert_eq!(default_spot(120.0, LAPTOP), (1920.0 - 144.0, 1080.0 - 144.0));
    }

    #[test]
    fn clamp_keeps_him_inside_with_a_margin() {
        assert_eq!(clamp_spot((-50.0, 5000.0), 120.0, LAPTOP, MARGIN), (24.0, 1080.0 - 144.0));
        assert_eq!(clamp_spot((300.0, 200.0), 120.0, LAPTOP, MARGIN), (300.0, 200.0));
        // A monitor smaller than him: pinned at the margin, never negative.
        assert_eq!(clamp_spot((10.0, 10.0), 120.0, Size { w: 100.0, h: 100.0 }, MARGIN), (24.0, 24.0));
    }

    #[test]
    fn body_hit_test() {
        assert!(is_over_body((60.0, 60.0), 120.0));
        assert!(is_over_body((60.0 + 28.0, 60.0), 120.0));
        assert!(!is_over_body((60.0 + 30.0, 60.0), 120.0));
        assert!(!is_over_body((5.0, 5.0), 120.0));
    }

    #[test]
    fn drag_follows_the_global_cursor() {
        // HDMI-A-1 sits at -2560,0: a cursor at -2000,300 grabbed at 60,60.
        assert_eq!(drag_spot((-2000.0, 300.0), (60.0, 60.0), (-2560.0, 0.0)), (500.0, 240.0));
    }

    #[test]
    fn drop_docks_at_the_top_edge_only_with_peek() {
        assert_eq!(drop_spot((400.0, 10.0), 120.0, LAPTOP, true), ((400.0, 0.0), true));
        assert_eq!(drop_spot((400.0, 10.0), 120.0, LAPTOP, false), ((400.0, 24.0), false));
        assert_eq!(drop_spot((400.0, 300.0), 120.0, LAPTOP, true), ((400.0, 300.0), false));
    }

    #[test]
    fn input_rows_cover_the_disc_only() {
        let rows = body_rows(120.0, 0.0);
        let r = 120.0 * BODY_RADIUS_FRACTION;
        let top = rows.iter().map(|r| r.1).min().unwrap();
        let bottom = rows.iter().map(|r| r.1 + r.3).max().unwrap();
        assert!((top as f64 - (60.0 - r)).abs() <= 2.0);
        assert!((bottom as f64 - (60.0 + r)).abs() <= 2.0);
        let widest = rows.iter().map(|r| r.2).max().unwrap();
        assert!((widest as f64 - 2.0 * r).abs() <= 2.0);
        assert!(rows.iter().all(|r| r.0 >= 0 && r.0 + r.2 <= 120));
        // Lifted for the peek: cut at the top edge.
        let lifted = body_rows(120.0, 54.0);
        assert!(lifted.iter().all(|r| r.1 >= 0));
        assert!(lifted.iter().map(|r| r.1 + r.3).max().unwrap() <= (60.0 - 54.0 + r + 2.0) as i32);
    }

    #[test]
    fn flight_eases_from_start_to_end() {
        assert_eq!(flight((0.0, 0.0), (100.0, 50.0), 0.0), (0.0, 0.0));
        assert_eq!(flight((0.0, 0.0), (100.0, 50.0), 1.0), (100.0, 50.0));
        assert_eq!(flight((0.0, 0.0), (100.0, 50.0), 0.5), (50.0, 25.0));
        assert!(flight((0.0, 0.0), (100.0, 0.0), 0.25).0 < 25.0);
    }

    #[test]
    fn spots_round_trip_and_ignore_garbage() {
        let mut s = Spots::default();
        s.0.insert("eDP-1".into(), Spot { x: 100.4, y: 200.6, docked: false });
        s.0.insert("HDMI-A-1".into(), Spot { x: 50.0, y: 0.0, docked: true });
        let back = Spots::parse(&s.to_json());
        assert_eq!(back.0["eDP-1"], Spot { x: 100.0, y: 201.0, docked: false });
        assert!(back.0["HDMI-A-1"].docked);
        assert_eq!(Spots::parse("nope"), Spots::default());
        assert!(Spots::parse(r#"{"spots":{"a":{"x":"1","y":2}}}"#).0.is_empty());
    }

    #[test]
    fn spots_save_and_load() {
        let dir = std::env::temp_dir().join(format!("coucou-mochi-test-{}", std::process::id()));
        let path = dir.join("overlay.json");
        let mut s = Spots::default();
        s.0.insert("eDP-1".into(), Spot { x: 10.0, y: 20.0, docked: false });
        s.save(&path).unwrap();
        assert_eq!(Spots::load(&path), s);
        let _ = std::fs::remove_dir_all(dir);
    }
}
