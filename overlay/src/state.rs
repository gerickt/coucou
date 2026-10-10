//! What the plugin's service publishes for the overlay:
//! $XDG_RUNTIME_DIR/coucou-mochi.json, written by mochi/service.luau whenever
//! Mochi's state or one of the overlay's settings changes (plus a heartbeat).
//! The service owns the state machine (hooks, approvals, chat, music); the
//! overlay only draws and reacts, like the desktop widget does.

use serde_json::Value;

use crate::mochi::Snapshot;

pub const FILE_NAME: &str = "coucou-mochi.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Overlay,
    DesktopWidget,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerChoice {
    Top,
    Overlay,
}

/// The plugin settings the overlay follows (mochi/plugin.toml).
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub sound: bool,
    pub surface: Surface,
    pub size: u32,
    pub layer: LayerChoice,
    pub peek: bool,
    pub open_on_hover: bool,
    pub follow_cursor: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sound: true,
            surface: Surface::Overlay,
            size: 120,
            layer: LayerChoice::Top,
            peek: false,
            open_on_hover: false,
            follow_cursor: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Published {
    pub snap: Snapshot,
    pub settings: Settings,
    /// The daemon answers (false: "offline").
    pub daemon: bool,
    /// Service clock (ms) at the last write.
    pub ts: f64,
    /// The line under him while the agent works or something waits ("Bash",
    /// "Approval needed", "Paused"); empty when there is nothing to say.
    pub label: String,
    /// "Coucou, <name>!" once the service knows the first name (else empty).
    pub greeting: String,
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn bool_of(v: &Value, k: &str, default: bool) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(default)
}

/// One short line: control characters out, at most 80 characters.
fn clean(raw: Option<&str>) -> String {
    raw.unwrap_or_default().chars().filter(|c| !c.is_control()).take(80).collect::<String>().trim().to_string()
}

pub fn parse(text: &str) -> Option<Published> {
    let v: Value = serde_json::from_str(text).ok()?;
    if !v.is_object() {
        return None;
    }
    let reaction = v.get("reaction").and_then(|r| {
        let name = str_of(r, "name")?.to_string();
        let rev = r.get("rev").and_then(Value::as_f64)? as i64;
        Some((name, rev))
    });
    let snap = Snapshot {
        state: str_of(&v, "state").unwrap_or("idle").to_string(),
        paused: bool_of(&v, "paused", false),
        music: bool_of(&v, "music", false),
        reaction,
        outfit: str_of(&v, "outfit").unwrap_or("none").to_string(),
        outfit_choice: str_of(&v, "outfit_choice").unwrap_or("auto").to_string(),
    };
    let d = Settings::default();
    let s = v.get("settings").cloned().unwrap_or(Value::Null);
    let settings = Settings {
        sound: bool_of(&s, "sound_enabled", d.sound),
        surface: match str_of(&s, "mochi_surface") {
            Some("desktop_widget") => Surface::DesktopWidget,
            Some("both") => Surface::Both,
            _ => Surface::Overlay,
        },
        size: s.get("overlay_size").and_then(Value::as_f64).map(|n| n.clamp(64.0, 256.0) as u32).unwrap_or(d.size),
        layer: match str_of(&s, "overlay_layer") {
            Some("overlay") => LayerChoice::Overlay,
            _ => LayerChoice::Top,
        },
        peek: bool_of(&s, "overlay_peek", d.peek),
        open_on_hover: bool_of(&s, "overlay_open_on_hover", d.open_on_hover),
        follow_cursor: bool_of(&s, "overlay_follow_cursor", d.follow_cursor),
    };
    Some(Published {
        snap,
        settings,
        daemon: bool_of(&v, "daemon", false),
        ts: v.get("ts").and_then(Value::as_f64).unwrap_or(0.0),
        label: clean(str_of(&v, "label")),
        greeting: clean(str_of(&v, "greeting")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_file() {
        let p = parse(
            r#"{"state":"working","paused":false,"daemon":true,"music":true,
            "reaction":{"name":"gulp","rev":3},"outfit":"crown","outfit_choice":"crown",
            "settings":{"sound_enabled":false,"mochi_surface":"both","overlay_size":300,
            "overlay_layer":"overlay","overlay_peek":true,"overlay_open_on_hover":true,
            "overlay_follow_cursor":false},"ts":1234}"#,
        )
        .unwrap();
        assert_eq!(p.snap.state, "working");
        assert!(p.snap.music);
        assert_eq!(p.snap.reaction, Some(("gulp".into(), 3)));
        assert_eq!(p.snap.outfit, "crown");
        assert!(p.daemon);
        assert_eq!(p.ts, 1234.0);
        assert_eq!((p.label.as_str(), p.greeting.as_str()), ("", ""));
        let s = p.settings;
        assert!(!s.sound);
        assert_eq!(s.surface, Surface::Both);
        assert_eq!(s.size, 256);
        assert_eq!(s.layer, LayerChoice::Overlay);
        assert!(s.peek && s.open_on_hover && !s.follow_cursor);
    }

    #[test]
    fn label_and_greeting_are_one_clean_line() {
        let p = parse(r#"{"label":"Ba\nsh\u0007 ","greeting":"Coucou, Théodore!"}"#).unwrap();
        assert_eq!(p.label, "Bash");
        assert_eq!(p.greeting, "Coucou, Théodore!");
        let long = format!(r#"{{"label":"{}"}}"#, "x".repeat(200));
        assert_eq!(parse(&long).unwrap().label.chars().count(), 80);
    }

    #[test]
    fn defaults_when_fields_are_missing() {
        let p = parse("{}").unwrap();
        assert_eq!(p.snap.state, "idle");
        assert_eq!(p.snap.outfit, "none");
        assert_eq!(p.settings, Settings::default());
        assert!(!p.daemon);
    }

    #[test]
    fn half_written_or_odd_files_are_refused() {
        assert_eq!(parse(r#"{"state":"work"#), None);
        assert_eq!(parse("[1,2]"), None);
        assert_eq!(parse(""), None);
    }
}
