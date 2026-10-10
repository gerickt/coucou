//! Mochi's behaviour on the overlay: a port of mochi/desktop.luau (itself the
//! original desktop window's rules, desktop/main.ts + desktop-logic.ts +
//! island.ts), minus the drawing. Pure: time only moves through step(), and
//! what must happen outside (a sound, a panel, flying home) comes out as
//! effects.
//!
//! - each agent state has its loop; error and finished first play the state
//!   change's own tween; an alert (approval, question) makes him surprised;
//! - a click is a poke (slap, annoyed) once it is clear it is not a double
//!   click (desktop/main.ts DOUBLE_CLICK_MS); three pokes within 1.7 s make
//!   him dizzy for 3.3 s (island.ts handleDizzy); a double click flies home;
//! - hovering 1.9 s = love, at most every 6 s (island.ts scheduleLove);
//! - a click while a request waits opens the approval panel;
//! - he greets at launch; he falls asleep after 120 s without agent activity
//!   once the pointer is away (shouldSleep: 150 px with a cursor poll, else
//!   4 s of pointer memory); paused, he sleeps;
//! - he dances while music plays in the dance states; dancing keeps him awake;
//! - a new outfit choice makes him proud with a pop;
//! - his eyes follow the pointer in the idle and working loops (gaze frames).

use crate::gaze::Dir;
use crate::player::Player;

pub const SLEEP_AFTER: f64 = 120.0;
pub const POINTER_MEMORY: f64 = 4.0;
pub const SLEEP_MOUSE_DISTANCE: f64 = 150.0;
pub const DOUBLE_CLICK: f64 = 0.3;
pub const DIZZY_WINDOW: f64 = 1.7;
pub const DIZZY_SECS: f64 = 3.3;
pub const LOVE_AFTER: f64 = 1.9;
pub const LOVE_EVERY: f64 = 6.0;

const STATES: [&str; 10] =
    ["idle", "working", "thinking", "searching", "approval", "question", "error", "finished", "ratelimit", "sleeping"];
const DANCE_STATES: [&str; 5] = ["idle", "working", "thinking", "searching", "finished"];
/// States with gaze frames (tools/render-mochi sequences.mjs GAZE_STATES).
pub const GAZE_STATES: [&str; 2] = ["idle", "working"];

fn enter_of(state: &str) -> Option<&'static str> {
    match state {
        "approval" => Some("approval.alert"),
        "question" => Some("question.alert"),
        "error" => Some("error.enter"),
        "finished" => Some("finished.enter"),
        _ => None,
    }
}

/// What the plugin's service publishes (see state.rs).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub state: String,
    pub paused: bool,
    pub music: bool,
    /// A one-shot the service asks for (the gulp of a file): name, revision.
    pub reaction: Option<(String, i64)>,
    /// What he wears today (the service resolves "auto").
    pub outfit: String,
    /// The outfit setting as chosen.
    pub outfit_choice: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Sound(&'static str),
    OpenApproval,
    Home,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Timer {
    DizzyEnd,
    Annoyed,
    Poke,
}

pub struct Mochi {
    pub player: Player,
    snap: Snapshot,
    t: f64,
    shown: Option<String>,
    dizzy: bool,
    slap_times: Vec<f64>,
    hover_since: Option<f64>,
    loved_at: f64,
    timers: Vec<(f64, Timer)>,
    outfit_choice: Option<String>,
    last_agent_active: f64,
    last_pointer: f64,
    asleep: bool,
    reaction_rev: Option<i64>,
    /// Pointer distance to his centre from a cursor poll; None without one.
    cursor_distance: Option<f64>,
    look: Option<Dir>,
    last_click: Option<f64>,
    dragging: bool,
    /// The desktop widget also shows him ("both"): it plays the outfit pop.
    pub quiet_outfit_pop: bool,
    pub effects: Vec<Effect>,
    dirty: bool,
}

impl Mochi {
    pub fn new(player: Player, snap: Snapshot, greet_sound: bool) -> Self {
        let mut m = Mochi {
            player,
            snap,
            t: 0.0,
            shown: None,
            dizzy: false,
            slap_times: Vec::new(),
            hover_since: None,
            loved_at: -100.0,
            timers: Vec::new(),
            outfit_choice: None,
            last_agent_active: 0.0,
            last_pointer: -100.0,
            asleep: false,
            reaction_rev: None,
            cursor_distance: None,
            look: None,
            last_click: None,
            dragging: false,
            quiet_outfit_pop: false,
            effects: Vec::new(),
            dirty: true,
        };
        m.read_reaction();
        m.sync();
        if !m.snap.paused && m.player.once("greet") && greet_sound {
            m.effects.push(Effect::Sound("greeting"));
        }
        m
    }

    #[cfg(test)]
    pub fn asleep(&self) -> bool {
        self.asleep
    }

    #[cfg(test)]
    pub fn dizzy(&self) -> bool {
        self.dizzy
    }

    fn agent_state(&self) -> &str {
        let s = self.snap.state.as_str();
        if STATES.contains(&s) {
            s
        } else {
            "idle"
        }
    }

    fn dancing(&self) -> bool {
        self.snap.music && !self.snap.paused && DANCE_STATES.contains(&self.agent_state())
    }

    fn base_state(&self) -> String {
        if self.snap.paused || self.asleep {
            "sleeping".into()
        } else {
            self.agent_state().to_string()
        }
    }

    /// The shown state (dizzy included), for the overlay's own choices.
    pub fn shown_state(&self) -> &str {
        if self.dizzy {
            "dizzy"
        } else {
            self.shown.as_deref().unwrap_or("idle")
        }
    }

    /// The eyes may follow the pointer now (worth polling the cursor).
    pub fn wants_gaze(&self) -> bool {
        !self.dizzy && !self.asleep && !self.dancing() && GAZE_STATES.contains(&self.shown_state())
    }

    fn loop_for(&self, s: &str) -> String {
        if s == "idle" && self.dancing() && self.player.has("dance") {
            return "dance".into();
        }
        if let Some(d) = self.look {
            let name = format!("{s}.look.{}", d.name());
            if GAZE_STATES.contains(&s) && self.player.has(&name) {
                return name;
            }
        }
        s.to_string()
    }

    fn near(&self) -> bool {
        let remembered = self.t - self.last_pointer < POINTER_MEMORY;
        match self.cursor_distance {
            Some(d) => d < SLEEP_MOUSE_DISTANCE || remembered,
            None => remembered,
        }
    }

    fn sync(&mut self) {
        let outfit = if self.snap.outfit.is_empty() { "none".to_string() } else { self.snap.outfit.clone() };
        if outfit != self.player.outfit {
            self.player.set_outfit(&outfit);
            self.dirty = true;
        }
        let choice = self.snap.outfit_choice.clone();
        if let Some(before) = &self.outfit_choice {
            // A new choice in the settings: pop, and he is proud of it.
            if *before != choice && !self.dizzy {
                if !self.quiet_outfit_pop {
                    self.effects.push(Effect::Sound("pop"));
                }
                self.player.once("proud");
                self.dirty = true;
            }
        }
        self.outfit_choice = Some(choice);

        let s = self.agent_state();
        if (s != "idle" && s != "sleeping") || self.dancing() {
            self.last_agent_active = self.t;
        }
        let sleep = !self.dizzy
            && !self.dragging
            && self.t - self.last_agent_active > SLEEP_AFTER
            && !self.near();
        if sleep != self.asleep {
            self.asleep = sleep;
            self.dirty = true;
        }
        if self.dizzy {
            return;
        }
        let s = self.base_state();
        if self.shown.as_deref() != Some(s.as_str()) {
            let first = self.shown.is_none();
            self.shown = Some(s.clone());
            // A reaction drawn over the previous state's body would now be wrong.
            if !first {
                self.player.cancel();
            }
            let name = self.loop_for(&s);
            self.player.set_loop(&name);
            if let (Some(enter), false) = (enter_of(&s), first) {
                self.player.once(enter);
            }
            self.dirty = true;
        } else {
            let name = self.loop_for(&s);
            self.player.set_loop_keep_phase(&name);
        }
    }

    fn note_pointer(&mut self) {
        self.last_pointer = self.t;
        if self.asleep {
            self.asleep = false;
            self.dirty = true;
        }
    }

    /// A reaction over the current state's body when the frames have it.
    fn react(&mut self, name: &str) -> bool {
        let s = self.shown.clone().unwrap_or_else(|| "idle".into());
        if s != "idle" && self.player.once(&format!("{s}.{name}")) {
            return true;
        }
        self.player.once(name)
    }

    fn read_reaction(&mut self) {
        let rev = self.snap.reaction.as_ref().map(|r| r.1);
        if let (Some(before), Some(now)) = (self.reaction_rev, rev) {
            if now != before && !self.dizzy {
                let name = self.snap.reaction.as_ref().map(|r| r.0.clone()).unwrap_or_default();
                self.note_pointer();
                self.react(&name);
                self.dirty = true;
            }
        }
        self.reaction_rev = rev.or(self.reaction_rev).or(Some(0));
    }

    // ── Inputs ────────────────────────────────────────────────────────────────

    pub fn set_snapshot(&mut self, snap: Snapshot) {
        if snap == self.snap {
            return;
        }
        self.snap = snap;
        self.read_reaction();
        self.sync();
        self.dirty = true;
    }

    /// A click (press and release without a drag) on his body.
    pub fn click(&mut self) {
        self.note_pointer();
        if let Some(at) = self.last_click {
            if self.t - at <= DOUBLE_CLICK {
                self.last_click = None;
                self.timers.retain(|(_, k)| *k != Timer::Poke);
                self.effects.push(Effect::Home);
                return;
            }
        }
        self.last_click = Some(self.t);
        self.timers.retain(|(_, k)| *k != Timer::Poke);
        self.later(DOUBLE_CLICK, Timer::Poke);
    }

    fn poke(&mut self) {
        self.note_pointer();
        self.sync();
        let s = self.base_state();
        if !self.dizzy && (s == "approval" || s == "question") {
            self.effects.push(Effect::OpenApproval);
            return;
        }
        if self.dizzy {
            return;
        }
        let t = self.t;
        self.slap_times.push(t);
        self.slap_times.retain(|at| t - at < DIZZY_WINDOW);
        if self.slap_times.len() >= 3 {
            self.begin_dizzy();
        } else {
            self.effects.push(Effect::Sound("slap"));
            self.later(0.06, Timer::Annoyed);
            self.react("slap");
            self.dirty = true;
        }
    }

    fn begin_dizzy(&mut self) {
        self.dizzy = true;
        self.slap_times.clear();
        self.hover_since = None;
        self.effects.push(Effect::Sound("dizzy"));
        self.player.set_loop("dizzy");
        self.player.once("dizzy.enter");
        self.dirty = true;
        self.later(DIZZY_SECS, Timer::DizzyEnd);
    }

    fn end_dizzy(&mut self) {
        self.dizzy = false;
        let s = self.base_state();
        self.shown = Some(s.clone());
        let name = self.loop_for(&s);
        self.player.set_loop(&name);
        self.player.once("dizzy.exit");
        self.dirty = true;
    }

    pub fn hover(&mut self, on: bool) {
        self.note_pointer();
        if on && self.hover_since.is_none() {
            if self.t - self.loved_at >= LOVE_EVERY {
                self.effects.push(Effect::Sound("hover"));
            }
            self.hover_since = Some(self.t);
        } else if !on {
            self.hover_since = None;
        }
    }

    /// The pointer moved over him.
    pub fn pointer_moved(&mut self) {
        self.note_pointer();
    }

    /// From the cursor poll: distance to his centre and where he looks.
    pub fn set_cursor(&mut self, distance: Option<f64>, look: Option<Dir>) {
        self.cursor_distance = distance;
        if look != self.look {
            self.look = look;
            self.dirty = true;
            if !self.dizzy {
                if let Some(s) = self.shown.clone() {
                    let name = self.loop_for(&s);
                    self.player.set_loop_keep_phase(&name);
                }
            }
        }
    }

    pub fn set_dragging(&mut self, on: bool) {
        self.dragging = on;
        if on {
            self.timers.retain(|(_, k)| *k != Timer::Poke);
            self.last_click = None;
            self.note_pointer();
        }
    }

    fn later(&mut self, secs: f64, timer: Timer) {
        self.timers.push((self.t + secs, timer));
    }

    // ── Clock ─────────────────────────────────────────────────────────────────

    /// Advances by `secs`; true when the picture must be drawn again.
    pub fn step(&mut self, secs: f64) -> bool {
        let secs = secs.max(0.0);
        // The picture first: a one-shot started below begins at its first
        // frame instead of being advanced by the time already gone.
        let changed = self.player.step(secs);
        self.t += secs;
        loop {
            let t = self.t;
            let Some(i) = self.timers.iter().position(|(at, _)| *at <= t) else { break };
            let (_, timer) = self.timers.remove(i);
            match timer {
                Timer::DizzyEnd => self.end_dizzy(),
                Timer::Annoyed => self.effects.push(Effect::Sound("annoyed")),
                Timer::Poke => {
                    self.last_click = None;
                    self.poke()
                }
            }
        }
        self.sync();
        if let Some(since) = self.hover_since {
            if !self.dizzy && self.t - since >= LOVE_AFTER && self.t - self.loved_at >= LOVE_EVERY {
                self.loved_at = self.t;
                self.hover_since = None;
                if self.react("love") {
                    self.effects.push(Effect::Sound("love"));
                }
                self.dirty = true;
            }
        }
        let out = changed || self.dirty;
        self.dirty = false;
        out
    }

    /// Seconds until something changes (infinity when nothing will).
    pub fn due(&self) -> f64 {
        let mut due = self.player.due();
        for (at, _) in &self.timers {
            due = due.min(at - self.t);
        }
        if !self.asleep && !self.dragging {
            let wake_until = (self.last_agent_active + SLEEP_AFTER).max(self.last_pointer + POINTER_MEMORY);
            due = due.min((wake_until - self.t + 0.01).max(0.2));
        }
        if let Some(since) = self.hover_since {
            if self.t - self.loved_at >= LOVE_EVERY {
                due = due.min(since + LOVE_AFTER - self.t);
            }
        }
        due.max(0.0)
    }

    pub fn frame(&mut self) -> Option<String> {
        self.player.frame().map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Seq;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn seqs() -> Arc<HashMap<String, Arc<Seq>>> {
        let mut m = HashMap::new();
        let mut add = |name: &str, looped: bool, n: usize| {
            let frames = (0..n).map(|i| (format!("{name}#{i}"), 1)).collect();
            m.insert(format!("none.{name}"), Arc::new(Seq { fps: 10.0, looped, frames }));
        };
        for s in STATES {
            add(s, true, 2);
        }
        add("dizzy", true, 2);
        for d in Dir::ALL {
            add(&format!("idle.look.{}", d.name()), true, 2);
        }
        for once in ["greet", "slap", "working.slap", "love", "dizzy.enter", "dizzy.exit", "proud", "approval.alert", "finished.enter", "gulp"] {
            add(once, false, 3);
        }
        add("dance", true, 4);
        Arc::new(m)
    }

    fn snap(state: &str) -> Snapshot {
        Snapshot { state: state.into(), outfit: "none".into(), outfit_choice: "auto".into(), ..Default::default() }
    }

    fn mochi(state: &str) -> Mochi {
        Mochi::new(Player::new(seqs()), snap(state), true)
    }

    fn sounds(m: &mut Mochi) -> Vec<&'static str> {
        m.effects
            .drain(..)
            .filter_map(|e| if let Effect::Sound(s) = e { Some(s) } else { None })
            .collect()
    }

    fn frame(m: &mut Mochi) -> String {
        m.frame().unwrap()
    }

    #[test]
    fn greets_at_launch_then_idles() {
        let mut m = mochi("idle");
        assert_eq!(sounds(&mut m), ["greeting"]);
        assert!(frame(&mut m).starts_with("greet#"));
        m.step(0.35);
        assert!(frame(&mut m).starts_with("idle#"));
    }

    #[test]
    fn no_greeting_when_paused() {
        let mut m = Mochi::new(Player::new(seqs()), Snapshot { paused: true, ..snap("idle") }, true);
        assert!(sounds(&mut m).is_empty());
        assert!(frame(&mut m).starts_with("sleeping#"));
    }

    #[test]
    fn click_is_a_slap_after_the_double_click_window() {
        let mut m = mochi("idle");
        m.step(1.0);
        sounds(&mut m);
        m.click();
        m.step(0.1);
        assert!(sounds(&mut m).is_empty());
        m.step(0.25);
        assert_eq!(sounds(&mut m), ["slap"]);
        assert!(frame(&mut m).starts_with("slap#"));
        m.step(0.06);
        assert_eq!(sounds(&mut m), ["annoyed"]);
    }

    #[test]
    fn slap_drawn_over_the_working_body() {
        let mut m = mochi("working");
        m.step(1.0);
        m.click();
        m.step(0.31);
        assert!(frame(&mut m).starts_with("working.slap#"));
    }

    #[test]
    fn double_click_flies_home_without_slap() {
        let mut m = mochi("idle");
        m.step(1.0);
        m.effects.clear();
        m.click();
        m.step(0.15);
        m.click();
        assert_eq!(m.effects, [Effect::Home]);
        m.effects.clear();
        m.step(1.0);
        assert!(m.effects.is_empty());
    }

    #[test]
    fn three_pokes_within_1_7_s_make_him_dizzy_for_3_3_s() {
        let mut m = mochi("idle");
        m.step(1.0);
        sounds(&mut m);
        for _ in 0..3 {
            m.click();
            m.step(0.5);
        }
        let s = sounds(&mut m);
        assert_eq!(s.iter().filter(|x| **x == "slap").count(), 2);
        assert!(s.contains(&"dizzy"));
        assert!(m.dizzy());
        assert!(frame(&mut m).starts_with("dizzy"));
        // Clicks while dizzy do nothing.
        m.click();
        m.step(0.4);
        assert!(sounds(&mut m).is_empty());
        m.step(3.3);
        assert!(!m.dizzy());
        assert!(frame(&mut m).starts_with("dizzy.exit#"));
    }

    #[test]
    fn pokes_spread_out_do_not_make_him_dizzy() {
        let mut m = mochi("idle");
        m.step(1.0);
        for _ in 0..3 {
            m.click();
            m.step(0.9);
        }
        assert!(!m.dizzy());
    }

    #[test]
    fn hover_1_9_s_is_love_at_most_every_6_s() {
        let mut m = mochi("idle");
        m.step(1.0);
        sounds(&mut m);
        m.hover(true);
        assert_eq!(sounds(&mut m), ["hover"]);
        m.step(1.8);
        assert!(sounds(&mut m).is_empty());
        m.step(0.15);
        assert_eq!(sounds(&mut m), ["love"]);
        assert!(frame(&mut m).starts_with("love#"));
        // Out and back in soon: no hover sound, no love before 6 s.
        m.hover(false);
        m.hover(true);
        m.step(2.0);
        assert!(sounds(&mut m).is_empty());
        m.step(4.1);
        assert_eq!(sounds(&mut m), ["love"]);
    }

    #[test]
    fn due_points_at_the_love_moment() {
        let mut m = mochi("sleeping");
        m.step(1.0);
        m.hover(true);
        assert!(m.due() <= LOVE_AFTER + 1e-9);
    }

    #[test]
    fn click_during_approval_opens_the_panel() {
        let mut m = mochi("approval");
        m.step(1.0);
        m.effects.clear();
        m.click();
        m.step(0.35);
        assert_eq!(m.effects, [Effect::OpenApproval]);
    }

    #[test]
    fn alert_makes_him_surprised_then_loops() {
        let mut m = mochi("idle");
        m.step(1.0);
        m.set_snapshot(snap("approval"));
        assert!(frame(&mut m).starts_with("approval.alert#"));
        m.step(0.35);
        assert!(frame(&mut m).starts_with("approval#"));
    }

    #[test]
    fn sleeps_after_120_s_without_agent_activity_when_the_pointer_is_away() {
        let mut m = mochi("idle");
        m.set_cursor(Some(500.0), None);
        m.step(119.0);
        assert!(!m.asleep());
        m.step(1.5);
        assert!(m.asleep());
        assert!(frame(&mut m).starts_with("sleeping#"));
        // The pointer comes over him: awake.
        m.pointer_moved();
        m.step(0.01);
        assert!(!m.asleep());
    }

    #[test]
    fn stays_awake_with_the_pointer_near_or_work_going_on() {
        let mut m = mochi("idle");
        m.set_cursor(Some(100.0), None);
        m.step(200.0);
        assert!(!m.asleep());
        let mut w = mochi("working");
        w.set_cursor(Some(900.0), None);
        w.step(200.0);
        assert!(!w.asleep());
        // Work stops: 120 s later he dozes off.
        w.set_snapshot(snap("idle"));
        w.step(119.0);
        assert!(!w.asleep());
        w.step(2.0);
        assert!(w.asleep());
    }

    #[test]
    fn dances_while_music_plays_and_stays_awake() {
        let mut m = mochi("idle");
        m.step(3.0);
        m.set_snapshot(Snapshot { music: true, ..snap("idle") });
        m.step(0.01);
        assert!(frame(&mut m).starts_with("dance#"));
        m.set_cursor(Some(900.0), None);
        m.step(300.0);
        assert!(!m.asleep());
        assert!(!m.wants_gaze());
    }

    #[test]
    fn new_outfit_choice_is_proud_with_a_pop() {
        let mut m = mochi("idle");
        m.step(3.0);
        sounds(&mut m);
        m.set_snapshot(Snapshot { outfit_choice: "crown".into(), ..snap("idle") });
        assert_eq!(sounds(&mut m), ["pop"]);
        assert!(frame(&mut m).starts_with("proud#"));
        m.quiet_outfit_pop = true;
        m.step(1.0);
        m.set_snapshot(Snapshot { outfit_choice: "bow".into(), ..snap("idle") });
        assert!(sounds(&mut m).is_empty());
    }

    #[test]
    fn reaction_from_the_service_plays_once_not_at_start() {
        let first = Snapshot { reaction: Some(("gulp".into(), 4)), ..snap("idle") };
        let mut m = Mochi::new(Player::new(seqs()), first.clone(), false);
        m.step(3.0);
        assert!(frame(&mut m).starts_with("idle#"));
        m.set_snapshot(Snapshot { reaction: Some(("gulp".into(), 5)), ..first });
        assert!(frame(&mut m).starts_with("gulp#"));
    }

    #[test]
    fn eyes_follow_the_cursor_in_idle_only() {
        let mut m = mochi("idle");
        m.step(3.0);
        assert!(m.wants_gaze());
        m.set_cursor(Some(300.0), Some(Dir::NE));
        m.step(0.01);
        assert!(frame(&mut m).starts_with("idle.look.ne#"));
        m.set_cursor(Some(30.0), None);
        m.step(0.01);
        assert!(frame(&mut m).starts_with("idle#"));
        m.set_snapshot(snap("thinking"));
        m.step(0.01);
        assert!(!m.wants_gaze());
    }

    #[test]
    fn dragging_keeps_him_awake_and_cancels_a_pending_poke() {
        let mut m = mochi("idle");
        m.set_cursor(Some(900.0), None);
        m.step(1.0);
        sounds(&mut m);
        m.click();
        m.set_dragging(true);
        m.step(200.0);
        assert!(sounds(&mut m).is_empty());
        assert!(!m.asleep());
    }
}
