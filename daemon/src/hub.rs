//! Shared daemon state: the pending permission requests, the plugin streams
//! following us, and a small status snapshot.
//!
//! The waiting rules are the original app's (pipe.rs):
//! * a request is only held for a human once the plugin *acknowledged* it is
//!   on screen, within `ack` (800 ms); otherwise the terminal takes over;
//! * after that, at most `decision` (108 s, under the relay's 110 s);
//! * one card at a time: a second request while one is pending is declined
//!   back to the terminal at once;
//! * no decision is silence, never a guess.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::protocol;

/// What the plugin can say about a pending request.
#[derive(Debug, PartialEq)]
pub enum Reply {
    Ack,
    Decision(String),
    Decline,
}

/// How a pending request ended.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Outcome {
    Answered,
    Declined,
    Unacked,
    TimedOut,
    /// The relay hung up first (the terminal answered, or the agent quit).
    Gone,
}

impl Outcome {
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Answered => "answered",
            Outcome::Declined => "declined",
            Outcome::Unacked => "unacked",
            Outcome::TimedOut => "timeout",
            Outcome::Gone => "gone",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub ack: Duration,
    pub decision: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self { ack: Duration::from_millis(800), decision: Duration::from_secs(108) }
    }
}

/// How often the decision wait checks that the relay is still there.
const ALIVE_POLL: Duration = Duration::from_millis(250);

/// Waits for the plugin: first the ack, then a human. `alive` tells whether
/// the relay is still connected; a relay that left needs no answer.
pub fn wait_for_decision(
    rx: &Receiver<Reply>,
    t: Timeouts,
    alive: &dyn Fn() -> bool,
) -> (Option<String>, Outcome) {
    match rx.recv_timeout(t.ack) {
        Ok(Reply::Ack) => {}
        // A click that beats the ack is still a click.
        Ok(Reply::Decision(d)) => return (Some(d), Outcome::Answered),
        Ok(Reply::Decline) => return (None, Outcome::Declined),
        Err(_) => return (None, Outcome::Unacked),
    }
    let deadline = Instant::now() + t.decision;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return (None, Outcome::TimedOut);
        }
        match rx.recv_timeout(left.min(ALIVE_POLL)) {
            Ok(Reply::Decision(d)) => return (Some(d), Outcome::Answered),
            Ok(Reply::Decline) => return (None, Outcome::Declined),
            // A second ack changes nothing.
            Ok(Reply::Ack) => {}
            Err(RecvTimeoutError::Timeout) => {
                if !alive() {
                    return (None, Outcome::Gone);
                }
            }
            Err(RecvTimeoutError::Disconnected) => return (None, Outcome::TimedOut),
        }
    }
}

/// Result of registering a permission request.
pub enum Registration {
    Pending(String, Receiver<Reply>),
    /// Another request holds the card.
    Busy,
}

#[derive(Default)]
struct Status {
    last_event: String,
    agent: String,
    tool: String,
    session_id: String,
    updated_at: u64,
}

#[derive(Default)]
pub struct Hub {
    pending: Mutex<HashMap<String, (Sender<Reply>, Value)>>,
    followers: Mutex<Vec<Sender<String>>>,
    status: Mutex<Status>,
    counter: AtomicU64,
    /// Where each session's terminal is (terminal.rs).
    pub windows: crate::terminal::Sessions,
}

impl Hub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends one JSON line to every follower; followers that left are dropped.
    pub fn broadcast(&self, msg: &Value) {
        let line = msg.to_string();
        self.followers.lock().unwrap().retain(|tx| tx.send(line.clone()).is_ok());
    }

    /// True when at least one plugin stream is attached. A follower that left
    /// is only noticed on the next broadcast, so this may be optimistic; the
    /// ack timeout covers that case.
    pub fn has_followers(&self) -> bool {
        !self.followers.lock().unwrap().is_empty()
    }

    /// A new follower: its channel, and the hello line it starts with.
    pub fn follow(&self) -> (Receiver<String>, String) {
        let (tx, rx) = mpsc::channel();
        let pending: Vec<Value> = self.pending.lock().unwrap().values().map(|(_, p)| p.clone()).collect();
        let hello = json!({ "type": "hello", "pending": pending, "status": self.status_json() });
        self.followers.lock().unwrap().push(tx);
        (rx, hello.to_string())
    }

    /// Records a hook event in the status snapshot.
    pub fn note(&self, event: &str, payload: &Value) {
        let text = |k: &str| payload.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        let mut s = self.status.lock().unwrap();
        s.last_event = event.to_string();
        s.agent = text("coucou_agent");
        s.tool = text("tool_name");
        s.session_id = text("session_id");
        s.updated_at = now_secs();
    }

    pub fn status_json(&self) -> Value {
        let pending = self.pending.lock().unwrap().len();
        let followers = self.followers.lock().unwrap().len();
        let s = self.status.lock().unwrap();
        json!({
            "running": true,
            "last_event": s.last_event,
            "agent": if s.agent.is_empty() { "claude" } else { s.agent.as_str() },
            "tool": s.tool,
            "session_id": s.session_id,
            "updated_at": s.updated_at,
            "pending": pending,
            "followers": followers,
        })
    }

    /// Registers a permission request, unless one is already pending.
    /// On success the payload gains its `request_id`.
    pub fn register(&self, payload: &mut Value) -> Registration {
        let mut pending = self.pending.lock().unwrap();
        if !pending.is_empty() {
            return Registration::Busy;
        }
        let id = format!("{}-{}", std::process::id(), self.counter.fetch_add(1, Ordering::Relaxed) + 1);
        payload["request_id"] = json!(id);
        let (tx, rx) = mpsc::channel();
        pending.insert(id.clone(), (tx, protocol::slim(payload)));
        Registration::Pending(id, rx)
    }

    /// Forgets a request once its connection is done.
    pub fn finish(&self, id: &str) {
        self.pending.lock().unwrap().remove(id);
    }

    /// Forwards the plugin's reply. False when the request is unknown.
    pub fn reply(&self, id: &str, reply: Reply) -> bool {
        let pending = self.pending.lock().unwrap();
        match pending.get(id) {
            Some((tx, _)) => tx.send(reply).is_ok(),
            None => false,
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn short() -> Timeouts {
        Timeouts { ack: Duration::from_millis(60), decision: Duration::from_millis(200) }
    }

    #[test]
    fn no_ack_means_silence_quickly() {
        let (_tx, rx) = mpsc::channel();
        let start = Instant::now();
        assert_eq!(wait_for_decision(&rx, short(), &|| true), (None, Outcome::Unacked));
        assert!(start.elapsed() < Duration::from_millis(180));
    }

    #[test]
    fn ack_then_decision_is_returned() {
        let (tx, rx) = mpsc::channel();
        tx.send(Reply::Ack).unwrap();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            tx.send(Reply::Decision("allow".into())).unwrap();
        });
        assert_eq!(wait_for_decision(&rx, short(), &|| true), (Some("allow".into()), Outcome::Answered));
    }

    #[test]
    fn a_click_that_beats_the_ack_still_counts() {
        let (tx, rx) = mpsc::channel();
        tx.send(Reply::Decision("deny".into())).unwrap();
        assert_eq!(wait_for_decision(&rx, short(), &|| true), (Some("deny".into()), Outcome::Answered));
    }

    #[test]
    fn ack_without_decision_times_out_silently() {
        let (tx, rx) = mpsc::channel();
        tx.send(Reply::Ack).unwrap();
        let start = Instant::now();
        assert_eq!(wait_for_decision(&rx, short(), &|| true), (None, Outcome::TimedOut));
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(190) && took < Duration::from_millis(600), "{took:?}");
        drop(tx);
    }

    #[test]
    fn decline_releases_at_once() {
        let (tx, rx) = mpsc::channel();
        tx.send(Reply::Decline).unwrap();
        assert_eq!(wait_for_decision(&rx, short(), &|| true), (None, Outcome::Declined));
        let (tx, rx) = mpsc::channel();
        tx.send(Reply::Ack).unwrap();
        tx.send(Reply::Decline).unwrap();
        assert_eq!(wait_for_decision(&rx, short(), &|| true), (None, Outcome::Declined));
    }

    #[test]
    fn a_relay_that_left_ends_the_wait() {
        let (tx, rx) = mpsc::channel();
        tx.send(Reply::Ack).unwrap();
        let t = Timeouts { ack: Duration::from_millis(60), decision: Duration::from_secs(5) };
        let start = Instant::now();
        assert_eq!(wait_for_decision(&rx, t, &|| false), (None, Outcome::Gone));
        assert!(start.elapsed() < Duration::from_secs(1));
        drop(tx);
    }

    #[test]
    fn one_card_at_a_time() {
        let hub = Hub::new();
        let mut first = json!({ "hook_event_name": "PermissionRequest" });
        let Registration::Pending(id, _rx) = hub.register(&mut first) else { panic!("first must register") };
        assert_eq!(first["request_id"], json!(id));
        let mut second = json!({ "hook_event_name": "PermissionRequest" });
        assert!(matches!(hub.register(&mut second), Registration::Busy));
        assert!(second.get("request_id").is_none());
        hub.finish(&id);
        let mut third = json!({ "hook_event_name": "PermissionRequest" });
        let Registration::Pending(id3, _) = hub.register(&mut third) else { panic!("free again") };
        assert_ne!(id, id3);
    }

    #[test]
    fn replies_reach_only_known_requests() {
        let hub = Hub::new();
        let mut p = json!({ "hook_event_name": "PermissionRequest" });
        let Registration::Pending(id, rx) = hub.register(&mut p) else { panic!() };
        assert!(hub.reply(&id, Reply::Ack));
        assert!(hub.reply(&id, Reply::Decision("allow".into())));
        assert_eq!(rx.recv().unwrap(), Reply::Ack);
        assert_eq!(rx.recv().unwrap(), Reply::Decision("allow".into()));
        assert!(!hub.reply("nope-1", Reply::Ack));
    }

    #[test]
    fn followers_get_hello_with_pending_and_broadcasts() {
        let hub = Hub::new();
        let mut p = json!({ "hook_event_name": "PermissionRequest", "tool_name": "Bash" });
        let Registration::Pending(id, _rx) = hub.register(&mut p) else { panic!() };
        let (rx, hello) = hub.follow();
        let hello: Value = serde_json::from_str(&hello).unwrap();
        assert_eq!(hello["type"], "hello");
        assert_eq!(hello["pending"][0]["request_id"], json!(id));
        hub.broadcast(&json!({ "type": "hook" }));
        assert_eq!(rx.recv().unwrap(), r#"{"type":"hook"}"#);
        drop(rx);
        hub.broadcast(&json!({ "type": "hook" }));
        assert!(hub.followers.lock().unwrap().is_empty());
    }

    #[test]
    fn status_tracks_the_last_event() {
        let hub = Hub::new();
        hub.note("PreToolUse", &json!({ "tool_name": "Bash", "coucou_agent": "codex", "session_id": "s1" }));
        let s = hub.status_json();
        assert_eq!(s["running"], true);
        assert_eq!(s["last_event"], "PreToolUse");
        assert_eq!(s["tool"], "Bash");
        assert_eq!(s["agent"], "codex");
        assert_eq!(s["pending"], 0);
        hub.note("Stop", &json!({}));
        assert_eq!(hub.status_json()["agent"], "claude");
    }
}
