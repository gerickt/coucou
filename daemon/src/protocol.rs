//! Wire formats, kept pure so they can be tested without sockets.
//!
//! * Relay socket (`coucou.sock`): `coucou-hook` writes one JSON object per
//!   line. Only for `PermissionRequest` do we answer, with one line: `allow`,
//!   `deny`, or `{"answers":{...}}`. No decision means no line at all.
//! * Control socket (`coucou-ctl.sock`): one command per connection, a single
//!   text line — `follow`, `status`, `ack <id>`, `decline <id>` or
//!   `decide <id> <allow|deny|answers-json>`, or `window <session_id>`
//!   (answered `ok <pid>,<pid>…` or `err …`).

use std::io::Read;

use serde_json::{json, Map, Value};

/// Largest relay line we read. coucou-hook keeps its lines well under this.
pub const MAX_LINE: usize = 1 << 20;
/// Longest string forwarded to the plugin.
/// Matches coucou-hook's own cap, so a question's text (the key its answer
/// goes back under) reaches the panel whole.
pub const FOLLOW_FIELD_LEN: usize = 2_000;

/// One command on the control socket.
#[derive(Debug, PartialEq)]
pub enum Control {
    Follow,
    Status,
    Ack(String),
    Decline(String),
    /// Request id and the normalised decision line.
    Decide(String, String),
    /// The processes that may own a session's window (`window <session_id>`).
    Window(String),
}

/// Parses one control line.
pub fn parse_control(line: &str) -> Result<Control, String> {
    let line = line.trim();
    let (verb, rest) = match line.split_once(char::is_whitespace) {
        Some((v, r)) => (v, r.trim()),
        None => (line, ""),
    };
    match verb {
        "follow" if rest.is_empty() => Ok(Control::Follow),
        "status" if rest.is_empty() => Ok(Control::Status),
        "ack" => valid_id(rest).map(Control::Ack),
        "decline" => valid_id(rest).map(Control::Decline),
        "decide" => {
            let (id, decision) = rest
                .split_once(char::is_whitespace)
                .ok_or_else(|| "decide needs <request_id> <decision>".to_string())?;
            let id = valid_id(id)?;
            Ok(Control::Decide(id, normalize_decision(decision)?))
        }
        "window" if crate::terminal::valid_session(rest) => Ok(Control::Window(rest.to_string())),
        "window" => Err("invalid session id".to_string()),
        _ => Err(format!("unknown command: {verb}")),
    }
}

/// A request id as the daemon issues them: `<pid>-<counter>`.
fn valid_id(id: &str) -> Result<String, String> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok {
        Ok(id.to_string())
    } else {
        Err("invalid request id".to_string())
    }
}

/// The line written back to the relay. `allow`/`always` become `allow`, `deny`
/// stays `deny`; a JSON object is an answer to AskUserQuestion, either already
/// `{"answers":{...}}` or the bare `{question: label}` map. Anything else is
/// refused: guessing could allow something nobody clicked.
pub fn normalize_decision(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    match raw {
        "allow" | "always" => return Ok("allow".into()),
        "deny" => return Ok("deny".into()),
        _ => {}
    }
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw) else {
        return Err("decision must be allow, deny or a JSON object of answers".into());
    };
    let answers: Map<String, Value> = match map.get("answers") {
        Some(Value::Object(a)) if map.len() == 1 => a.clone(),
        Some(_) => return Err("\"answers\" must be an object".into()),
        None => map,
    };
    // A single-select answer is a label, a multi-select one a non-empty list
    // of labels; coucou-hook then checks them against the questions asked.
    let fits = |v: &Value| match v {
        Value::String(_) => true,
        Value::Array(items) => !items.is_empty() && items.iter().all(Value::is_string),
        _ => false,
    };
    if answers.is_empty() || !answers.values().all(fits) {
        return Err("answers must map each question to a label or a list of labels".into());
    }
    // serde_json never emits a raw newline, so this stays one line.
    Ok(json!({ "answers": answers }).to_string())
}

/// What the log and the plugin may say about a decision: never the answers.
pub fn outcome_word(decision: &str) -> &str {
    if decision.starts_with('{') {
        "answered"
    } else {
        decision
    }
}

/// Reads up to the first newline (excluded), at most `max` bytes. `None` when
/// nothing usable arrived.
pub fn read_line(reader: &mut impl Read, max: usize) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') {
                    break;
                }
                if buf.len() > max {
                    return None;
                }
            }
            Err(_) => break,
        }
    }
    if let Some(i) = buf.iter().position(|b| *b == b'\n') {
        buf.truncate(i);
    }
    (!buf.is_empty() && buf.len() <= max).then_some(buf)
}

/// A relay line as a hook payload: a JSON object with its event name.
pub fn parse_hook(line: &[u8]) -> Option<(String, Value)> {
    let payload = serde_json::from_slice::<Value>(line).ok()?;
    let event = payload.as_object()?.get("hook_event_name")?.as_str()?.to_string();
    Some((event, payload))
}

/// A copy of `payload` with every string capped, for the plugin's stream.
pub fn slim(payload: &Value) -> Value {
    let mut v = payload.clone();
    cap_strings(&mut v, FOLLOW_FIELD_LEN);
    v
}

fn cap_strings(value: &mut Value, max: usize) {
    match value {
        Value::String(s) if s.len() > max => {
            let mut end = max;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
            s.push('…');
        }
        Value::Array(items) => items.iter_mut().for_each(|v| cap_strings(v, max)),
        Value::Object(map) => map.values_mut().for_each(|v| cap_strings(v, max)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_commands_parse() {
        assert_eq!(parse_control("follow\n"), Ok(Control::Follow));
        assert_eq!(parse_control("status"), Ok(Control::Status));
        assert_eq!(parse_control("ack 12-3"), Ok(Control::Ack("12-3".into())));
        assert_eq!(parse_control("decline 12-3"), Ok(Control::Decline("12-3".into())));
        assert_eq!(parse_control("decide 12-3 allow"), Ok(Control::Decide("12-3".into(), "allow".into())));
        assert_eq!(parse_control("decide 12-3 always"), Ok(Control::Decide("12-3".into(), "allow".into())));
        assert_eq!(parse_control("decide 12-3 deny"), Ok(Control::Decide("12-3".into(), "deny".into())));
        assert_eq!(parse_control("window ab-12_c"), Ok(Control::Window("ab-12_c".into())));
        assert!(parse_control("window ../x").is_err() && parse_control("window").is_err());
    }

    #[test]
    fn bad_control_commands_are_refused() {
        for bad in ["", "follow now", "ack", "ack ../x", "decide 1-1", "decide 1-1 yes", "decide 1-1 {\"a\":1}", "rm -rf"] {
            assert!(parse_control(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn answers_are_normalised_to_one_line() {
        let wrapped = normalize_decision(r#"{"answers":{"Which?":"A"}}"#).unwrap();
        let bare = normalize_decision(r#"{"Which?":"A"}"#).unwrap();
        assert_eq!(wrapped, bare);
        assert_eq!(wrapped, r#"{"answers":{"Which?":"A"}}"#);
        assert!(!wrapped.contains('\n'));
        assert!(normalize_decision(r#"{"answers":{}}"#).is_err());
        assert!(normalize_decision(r#"{"answers":"A"}"#).is_err());
        assert!(normalize_decision(r#"{"answers":{"q":"a"},"extra":1}"#).is_err());
        assert!(normalize_decision("[]").is_err());
    }

    #[test]
    fn multi_select_answers_are_lists_of_labels() {
        let d = normalize_decision(r#"{"Pick?":["A","B"],"One?":"C"}"#).unwrap();
        assert_eq!(d, r#"{"answers":{"One?":"C","Pick?":["A","B"]}}"#);
        assert!(normalize_decision(r#"{"Pick?":[]}"#).is_err());
        assert!(normalize_decision(r#"{"Pick?":[1]}"#).is_err());
    }

    #[test]
    fn outcome_word_hides_answers() {
        assert_eq!(outcome_word("allow"), "allow");
        assert_eq!(outcome_word(r#"{"answers":{"q":"secret"}}"#), "answered");
    }

    #[test]
    fn read_line_stops_at_newline_and_limit() {
        let mut ok: &[u8] = b"{\"a\":1}\nrest";
        assert_eq!(read_line(&mut ok, 100).unwrap(), b"{\"a\":1}");
        let mut no_newline: &[u8] = b"abc";
        assert_eq!(read_line(&mut no_newline, 100).unwrap(), b"abc");
        let mut empty: &[u8] = b"";
        assert!(read_line(&mut empty, 100).is_none());
        let big = vec![b'x'; 10_000];
        assert!(read_line(&mut big.as_slice(), 100).is_none());
    }

    #[test]
    fn hooks_need_an_object_with_an_event() {
        let (e, _) = parse_hook(br#"{"hook_event_name":"Stop"}"#).unwrap();
        assert_eq!(e, "Stop");
        assert!(parse_hook(b"[1]").is_none());
        assert!(parse_hook(br#"{"tool_name":"Bash"}"#).is_none());
        assert!(parse_hook(b"nope").is_none());
    }

    #[test]
    fn slim_caps_strings_on_char_boundaries() {
        let v = json!({ "tool_input": { "content": "é".repeat(2000) }, "n": 1 });
        let s = slim(&v);
        let c = s["tool_input"]["content"].as_str().unwrap();
        assert!(c.len() <= FOLLOW_FIELD_LEN + 4);
        assert!(c.ends_with('…'));
        assert_eq!(s["n"], 1);
    }
}
