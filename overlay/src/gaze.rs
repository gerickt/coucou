//! Mochi's eyes follow the pointer: desktop-logic.ts gaze() (BotCanvasView
//! lookX / lookY), reduced to the eight rendered directions.

/// The directions tools/render-mochi renders ("<state>.look.<dir>").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    NW,
}

impl Dir {
    #[cfg(test)]
    pub const ALL: [Dir; 8] = [Dir::N, Dir::NE, Dir::E, Dir::SE, Dir::S, Dir::SW, Dir::W, Dir::NW];

    pub fn name(self) -> &'static str {
        match self {
            Dir::N => "n",
            Dir::NE => "ne",
            Dir::E => "e",
            Dir::SE => "se",
            Dir::S => "s",
            Dir::SW => "sw",
            Dir::W => "w",
            Dir::NW => "nw",
        }
    }
}

/// desktop-logic.ts gaze(): x right +, y up + (screen y grows downwards).
pub fn gaze(bot: (f64, f64), mouse: (f64, f64)) -> (f64, f64) {
    (((mouse.0 - bot.0) / 260.0).tanh(), -((mouse.1 - bot.1) / 200.0).tanh())
}

/// Below this the eyes stay centred (the plain loop).
pub const DEAD_ZONE: f64 = 0.3;

/// The rendered direction nearest to a look, or None near the centre.
pub fn direction(look: (f64, f64)) -> Option<Dir> {
    let (x, y) = look;
    if x.hypot(y) < DEAD_ZONE {
        return None;
    }
    // Eight 45° sectors, east = 0, counter-clockwise (y is up).
    let angle = y.atan2(x).to_degrees();
    let sector = (((angle + 360.0 + 22.5) % 360.0) / 45.0) as usize % 8;
    Some([Dir::E, Dir::NE, Dir::N, Dir::NW, Dir::W, Dir::SW, Dir::S, Dir::SE][sector])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaze_matches_desktop_logic() {
        let (x, y) = gaze((100.0, 100.0), (360.0, 300.0));
        assert!((x - 1f64.tanh()).abs() < 1e-12);
        assert!((y + 1f64.tanh()).abs() < 1e-12);
        assert_eq!(gaze((0.0, 0.0), (0.0, 0.0)), (0.0, -0.0));
    }

    #[test]
    fn nearest_direction() {
        let bot = (500.0, 500.0);
        let at = |dx: f64, dy: f64| direction(gaze(bot, (bot.0 + dx, bot.1 + dy)));
        // Pointer on him or close: eyes centred.
        assert_eq!(at(0.0, 0.0), None);
        assert_eq!(at(40.0, 20.0), None);
        // Screen y grows downwards: a pointer above him is north.
        assert_eq!(at(0.0, -400.0), Some(Dir::N));
        assert_eq!(at(0.0, 400.0), Some(Dir::S));
        assert_eq!(at(600.0, 0.0), Some(Dir::E));
        assert_eq!(at(-600.0, 0.0), Some(Dir::W));
        assert_eq!(at(400.0, -300.0), Some(Dir::NE));
        assert_eq!(at(-400.0, 300.0), Some(Dir::SW));
        assert_eq!(at(-400.0, -300.0), Some(Dir::NW));
        assert_eq!(at(400.0, 300.0), Some(Dir::SE));
    }

    #[test]
    fn names_are_the_rendered_ones() {
        let names: Vec<_> = Dir::ALL.iter().map(|d| d.name()).collect();
        assert_eq!(names, ["n", "ne", "e", "se", "s", "sw", "w", "nw"]);
    }
}
