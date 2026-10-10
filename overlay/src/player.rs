//! Plays Mochi's frame sequences: a port of mochi/sprite.luau's Player, with
//! the same rules (a loop, a one-shot over it, outfit fallback to "none" for
//! the lookup, frames held `hold / fps` seconds).

use std::collections::HashMap;
use std::sync::Arc;

use crate::manifest::Seq;

pub struct Player {
    seqs: Arc<HashMap<String, Arc<Seq>>>,
    pub outfit: String,
    key: Option<String>,
    seq: Option<Arc<Seq>>,
    /// Index in seq.frames.
    run: usize,
    /// Seconds left on the current run.
    left: f64,
    loop_name: String,
    once_name: Option<String>,
}

impl Player {
    pub fn new(seqs: Arc<HashMap<String, Arc<Seq>>>) -> Self {
        Player {
            seqs,
            outfit: "none".into(),
            key: None,
            seq: None,
            run: 0,
            left: 0.0,
            loop_name: "idle".into(),
            once_name: None,
        }
    }

    /// The outfit has this sequence of its own.
    pub fn has(&self, name: &str) -> bool {
        self.seqs.contains_key(&format!("{}.{name}", self.outfit))
    }

    fn lookup(&self, name: &str) -> Option<Arc<Seq>> {
        self.seqs
            .get(&format!("{}.{name}", self.outfit))
            .or_else(|| self.seqs.get(&format!("none.{name}")))
            .cloned()
    }

    fn start(&mut self, name: &str) -> bool {
        let Some(seq) = self.lookup(name) else { return false };
        self.key = Some(format!("{}.{name}", self.outfit));
        self.left = seq.frames[0].1 as f64 / seq.fps;
        self.run = 0;
        self.seq = Some(seq);
        true
    }

    #[cfg(test)]
    pub fn once_name(&self) -> Option<&str> {
        self.once_name.as_deref()
    }

    /// The loop to show whenever no one-shot is playing.
    pub fn set_loop(&mut self, name: &str) {
        let current = self.key.as_deref() == Some(&format!("{}.{name}", self.outfit));
        if name == self.loop_name && (self.once_name.is_some() || current) {
            return;
        }
        self.loop_name = name.to_string();
        if self.once_name.is_none() {
            self.start(name);
        }
    }

    /// Like set_loop, for a variant of the loop playing now (the gaze
    /// directions of a state): same run and time when the two line up, so the
    /// eyes move without restarting the blink cycle.
    pub fn set_loop_keep_phase(&mut self, name: &str) {
        if name == self.loop_name {
            return self.set_loop(name);
        }
        let playing_loop = self.once_name.is_none();
        let (run, left, before) = (self.run, self.left, self.seq.clone());
        self.set_loop(name);
        if !playing_loop {
            return;
        }
        if let (Some(a), Some(b)) = (before, self.seq.as_ref()) {
            let same_shape = a.fps == b.fps
                && a.frames.len() == b.frames.len()
                && a.frames.iter().zip(&b.frames).all(|(x, y)| x.1 == y.1);
            if same_shape {
                self.run = run;
                self.left = left;
            }
        }
    }

    /// A one-shot; false when the outfit has no such sequence.
    pub fn once(&mut self, name: &str) -> bool {
        if !self.has(name) {
            return false;
        }
        self.once_name = Some(name.to_string());
        self.start(name);
        true
    }

    /// Drops the one-shot in flight (its body no longer matches), back to the loop.
    pub fn cancel(&mut self) {
        if self.once_name.take().is_none() {
            return;
        }
        let name = self.loop_name.clone();
        self.start(&name);
    }

    pub fn set_outfit(&mut self, outfit: &str) {
        if outfit == self.outfit {
            return;
        }
        self.outfit = outfit.to_string();
        self.once_name = None;
        let name = self.loop_name.clone();
        self.start(&name);
    }

    /// The file shown now, relative to the plugin directory.
    pub fn frame(&mut self) -> Option<&str> {
        if self.seq.is_none() {
            let name = self.loop_name.clone();
            self.start(&name);
        }
        let seq = self.seq.as_ref()?;
        Some(seq.frames[self.run].0.as_str())
    }

    /// Seconds until the picture changes (infinity for a still).
    pub fn due(&self) -> f64 {
        let Some(seq) = &self.seq else { return f64::INFINITY };
        if seq.frames.len() == 1 && seq.looped {
            return f64::INFINITY;
        }
        self.left.max(0.0)
    }

    /// Advances by `secs`; true when the frame changed.
    pub fn step(&mut self, secs: f64) -> bool {
        let Some(seq) = self.seq.clone() else {
            let name = self.loop_name.clone();
            return self.start(&name);
        };
        let before_file = seq.frames[self.run].0.clone();
        let before_key = self.key.clone();
        self.left -= secs;
        let mut guard = 0;
        let mut seq = seq;
        while self.left <= 0.0 && guard < 500 {
            guard += 1;
            if self.run + 1 < seq.frames.len() {
                self.run += 1;
                self.left += seq.frames[self.run].1 as f64 / seq.fps;
            } else if seq.looped {
                self.run = 0;
                self.left += seq.frames[0].1 as f64 / seq.fps;
                if seq.frames.len() == 1 {
                    self.left = self.left.max(0.001);
                    break;
                }
            } else {
                // One-shot finished: back to the loop.
                self.once_name = None;
                let name = self.loop_name.clone();
                self.start(&name);
                self.left = self.left.max(0.001);
                break;
            }
            if let Some(s) = &self.seq {
                seq = s.clone();
            }
        }
        let now_file = self.seq.as_ref().map(|s| s.frames[self.run].0.clone());
        self.key != before_key || now_file.as_deref() != Some(before_file.as_str())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn seq(fps: f64, looped: bool, frames: &[(&str, u32)]) -> Arc<Seq> {
        Arc::new(Seq { fps, looped, frames: frames.iter().map(|(f, h)| (f.to_string(), *h)).collect() })
    }

    pub(crate) fn sample() -> Arc<HashMap<String, Arc<Seq>>> {
        let mut m = HashMap::new();
        m.insert("none.idle".into(), seq(10.0, true, &[("i1", 2), ("i2", 1)]));
        m.insert("none.idle.look.e".into(), seq(10.0, true, &[("e1", 2), ("e2", 1)]));
        m.insert("none.working".into(), seq(10.0, true, &[("w1", 1), ("w2", 1)]));
        m.insert("none.sleeping".into(), seq(4.0, true, &[("z1", 1)]));
        m.insert("none.slap".into(), seq(10.0, false, &[("s1", 1), ("s2", 1)]));
        m.insert("none.working.slap".into(), seq(10.0, false, &[("ws1", 1)]));
        m.insert("crown.idle".into(), seq(10.0, true, &[("c1", 1)]));
        m.insert("crown.slap".into(), seq(10.0, false, &[("cs1", 1)]));
        Arc::new(m)
    }

    #[test]
    fn loop_holds_and_wraps() {
        let mut p = Player::new(sample());
        assert_eq!(p.frame(), Some("i1"));
        assert!((p.due() - 0.2).abs() < 1e-9);
        assert!(!p.step(0.1));
        assert!(p.step(0.1));
        assert_eq!(p.frame(), Some("i2"));
        assert!(p.step(0.1));
        assert_eq!(p.frame(), Some("i1"));
    }

    #[test]
    fn one_shot_then_back_to_loop() {
        let mut p = Player::new(sample());
        p.frame();
        assert!(p.once("slap"));
        assert_eq!(p.frame(), Some("s1"));
        p.step(0.1);
        assert_eq!(p.frame(), Some("s2"));
        p.step(0.1);
        assert_eq!(p.frame(), Some("i1"));
        assert_eq!(p.once_name(), None);
    }

    #[test]
    fn set_loop_during_one_shot_waits_for_it() {
        let mut p = Player::new(sample());
        p.frame();
        p.once("slap");
        p.set_loop("working");
        assert_eq!(p.frame(), Some("s1"));
        p.step(0.2);
        assert_eq!(p.frame(), Some("w1"));
    }

    #[test]
    fn cancel_returns_to_loop() {
        let mut p = Player::new(sample());
        p.frame();
        p.once("slap");
        p.cancel();
        assert_eq!(p.frame(), Some("i1"));
    }

    #[test]
    fn outfit_falls_back_to_none_for_loops_but_once_needs_its_own() {
        let mut p = Player::new(sample());
        p.set_outfit("crown");
        assert_eq!(p.frame(), Some("c1"));
        p.set_loop("working");
        assert_eq!(p.frame(), Some("w1"));
        assert!(!p.once("working.slap"));
        assert!(p.once("slap"));
        assert_eq!(p.frame(), Some("cs1"));
    }

    #[test]
    fn still_loop_never_due() {
        let mut p = Player::new(sample());
        p.set_loop("sleeping");
        p.frame();
        assert_eq!(p.due(), f64::INFINITY);
        assert!(!p.step(10.0));
    }

    #[test]
    fn gaze_variant_keeps_phase() {
        let mut p = Player::new(sample());
        p.frame();
        p.step(0.25);
        assert_eq!(p.frame(), Some("i2"));
        p.set_loop_keep_phase("idle.look.e");
        assert_eq!(p.frame(), Some("e2"));
        assert!((p.due() - 0.05).abs() < 1e-9);
        // A different shape restarts.
        p.set_loop_keep_phase("working");
        assert_eq!(p.frame(), Some("w1"));
    }
}
