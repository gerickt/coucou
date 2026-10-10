//! Hyprland: the event socket (socket2) for the focused monitor, and the
//! request socket for the global cursor position. Both are plain Unix sockets
//! under $XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE; no hyprctl
//! process is spawned. Without Hyprland the overlay stays on its first
//! monitor and the eyes only follow the pointer over him.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The focused monitor changed (by name).
    FocusedMonitor(String),
    MonitorRemoved(String),
    MonitorAdded(String),
}

/// One socket2 line: "EVENT>>DATA".
pub fn parse_event(line: &str) -> Option<Event> {
    let (name, data) = line.trim_end().split_once(">>")?;
    let first = |s: &str| s.split(',').next().unwrap_or("").to_string();
    let ev = match name {
        // focusedmon>>MONNAME,WORKSPACENAME ; focusedmonv2>>MONNAME,WORKSPACEID
        "focusedmon" | "focusedmonv2" => Event::FocusedMonitor(first(data)),
        "monitorremoved" => Event::MonitorRemoved(data.to_string()),
        // monitorremovedv2>>ID,NAME,DESCRIPTION
        "monitorremovedv2" => Event::MonitorRemoved(data.split(',').nth(1)?.to_string()),
        "monitoradded" => Event::MonitorAdded(data.to_string()),
        "monitoraddedv2" => Event::MonitorAdded(data.split(',').nth(1)?.to_string()),
        _ => return None,
    };
    match &ev {
        Event::FocusedMonitor(n) | Event::MonitorRemoved(n) | Event::MonitorAdded(n) if n.is_empty() => None,
        _ => Some(ev),
    }
}

/// `cursorpos` answers "X, Y" in layout (logical) coordinates.
pub fn parse_cursorpos(text: &str) -> Option<(f64, f64)> {
    let (x, y) = text.trim().split_once(',')?;
    let (x, y) = (x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?);
    (x.is_finite() && y.is_finite()).then_some((x, y))
}

/// The focused monitor's name in `j/monitors`.
pub fn focused_in_monitors_json(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    v.as_array()?
        .iter()
        .find(|m| m.get("focused").and_then(|f| f.as_bool()) == Some(true))
        .and_then(|m| m.get("name")?.as_str().map(str::to_string))
}

/// The instance directory, when Hyprland runs this session.
pub fn socket_dir() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok().filter(|s| !s.is_empty() && !s.contains('/'))?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok().filter(|d| d.starts_with('/'))?;
    let dir = PathBuf::from(runtime).join("hypr").join(sig);
    dir.join(".socket.sock").exists().then_some(dir)
}

/// One request on .socket.sock; the answer is read until Hyprland closes it.
pub fn request(dir: &std::path::Path, cmd: &str) -> std::io::Result<String> {
    let mut s = UnixStream::connect(dir.join(".socket.sock"))?;
    s.set_read_timeout(Some(Duration::from_millis(300)))?;
    s.set_write_timeout(Some(Duration::from_millis(300)))?;
    s.write_all(cmd.as_bytes())?;
    let mut out = String::new();
    s.read_to_string(&mut out)?;
    Ok(out)
}

pub fn cursor(dir: &std::path::Path) -> Option<(f64, f64)> {
    request(dir, "cursorpos").ok().and_then(|t| parse_cursorpos(&t))
}

pub fn focused_monitor(dir: &std::path::Path) -> Option<String> {
    request(dir, "j/monitors").ok().and_then(|t| focused_in_monitors_json(&t))
}

/// Splits what socket2 sent into complete lines, keeping a partial tail.
pub fn take_lines(buf: &mut String) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(i) = buf.find('\n') {
        out.push(buf[..i].to_string());
        buf.drain(..=i);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focused_monitor_events() {
        assert_eq!(parse_event("focusedmon>>HDMI-A-1,3"), Some(Event::FocusedMonitor("HDMI-A-1".into())));
        assert_eq!(parse_event("focusedmonv2>>eDP-1,1\n"), Some(Event::FocusedMonitor("eDP-1".into())));
        // Workspace names may hold commas: only the first field is the monitor.
        assert_eq!(parse_event("focusedmon>>eDP-1,name:a,b"), Some(Event::FocusedMonitor("eDP-1".into())));
    }

    #[test]
    fn monitor_hotplug_events() {
        assert_eq!(parse_event("monitorremoved>>HDMI-A-1"), Some(Event::MonitorRemoved("HDMI-A-1".into())));
        assert_eq!(parse_event("monitorremovedv2>>1,HDMI-A-1,Dell"), Some(Event::MonitorRemoved("HDMI-A-1".into())));
        assert_eq!(parse_event("monitoraddedv2>>2,DP-2,LG"), Some(Event::MonitorAdded("DP-2".into())));
    }

    #[test]
    fn other_lines_are_ignored() {
        assert_eq!(parse_event("workspace>>3"), None);
        assert_eq!(parse_event("activewindow>>kitty,~"), None);
        assert_eq!(parse_event("garbage"), None);
        assert_eq!(parse_event("focusedmon>>"), None);
    }

    #[test]
    fn cursorpos_answer() {
        assert_eq!(parse_cursorpos("1210, 433\n"), Some((1210.0, 433.0)));
        assert_eq!(parse_cursorpos("-2000, 300"), Some((-2000.0, 300.0)));
        assert_eq!(parse_cursorpos("unknown request"), None);
        assert_eq!(parse_cursorpos("1, nan"), None);
    }

    #[test]
    fn focused_from_monitors_json() {
        let j = r#"[{"name":"HDMI-A-1","focused":false},{"name":"eDP-1","focused":true}]"#;
        assert_eq!(focused_in_monitors_json(j), Some("eDP-1".into()));
        assert_eq!(focused_in_monitors_json("[]"), None);
    }

    #[test]
    fn lines_split_with_partial_tail() {
        let mut b = String::from("focusedmon>>a,1\nworkspace>>2\nfocusedm");
        assert_eq!(take_lines(&mut b), ["focusedmon>>a,1", "workspace>>2"]);
        assert_eq!(b, "focusedm");
    }
}
