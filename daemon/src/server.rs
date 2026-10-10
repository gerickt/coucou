//! The two Unix sockets: the relay socket coucou-hook connects to, and the
//! control socket the Noctalia plugin follows and answers through.
//!
//! Claude Code is never blocked by us: a PermissionRequest is only held when a
//! plugin stream is attached and acknowledges the card within the ack timeout;
//! every other event is read and the connection dropped at once.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::hub::{wait_for_decision, Hub, Outcome, Registration, Reply, Timeouts};
use crate::protocol::{self, Control};

/// How long any peer gets to send its single line.
const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// How often followers hear from us when nothing happens.
pub const HEARTBEAT: Duration = Duration::from_secs(15);
/// How long a follower may stall a write before it is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_millis(500);

/// The process on the other end, as the kernel saw it.
fn peer_cred(stream: &UnixStream) -> Option<libc::ucred> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    (rc == 0 && len as usize == std::mem::size_of::<libc::ucred>()).then_some(cred)
}

pub fn same_user(stream: &UnixStream) -> bool {
    peer_cred(stream).map(|c| c.uid) == Some(crate::paths::uid())
}

/// The relay's pid (SO_PEERCRED), to find its terminal (terminal.rs).
fn peer_pid(stream: &UnixStream) -> Option<u32> {
    peer_cred(stream).and_then(|c| u32::try_from(c.pid).ok()).filter(|pid| *pid > 1)
}

/// False once the peer hung up (or the socket broke).
fn peer_alive(fd: i32) -> bool {
    let mut pfd = libc::pollfd { fd, events: libc::POLLIN | libc::POLLRDHUP, revents: 0 };
    let rc = unsafe { libc::poll(&mut pfd, 1, 0) };
    if rc < 0 {
        return false;
    }
    pfd.revents & (libc::POLLRDHUP | libc::POLLHUP | libc::POLLERR) == 0
}

/// Binds `path`, refusing to serve on top of a live socket. A stale socket
/// left by a crash answers nothing and is removed.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if path.exists() {
        if UnixStream::connect(path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("another coucou-daemon already serves {}", path.display()),
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Starts both accept loops in background threads.
pub fn start(relay: &Path, control: &Path, timeouts: Timeouts) -> io::Result<Arc<Hub>> {
    let relay_listener = bind(relay)?;
    let control_listener = bind(control)?;
    let hub = Arc::new(Hub::new());

    let h = hub.clone();
    thread::spawn(move || accept_loop(relay_listener, move |s| handle_relay(&h, s, timeouts)));
    let h = hub.clone();
    thread::spawn(move || accept_loop(control_listener, move |s| handle_control(&h, s)));
    // A heartbeat, so a follower can tell a quiet daemon from a dead stream,
    // and followers that left are dropped even when nothing happens.
    let h = hub.clone();
    thread::spawn(move || loop {
        thread::sleep(HEARTBEAT);
        h.broadcast(&json!({ "type": "ping" }));
    });
    Ok(hub)
}

fn accept_loop(listener: UnixListener, handler: impl Fn(UnixStream) + Send + Sync + Clone + 'static) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            thread::sleep(Duration::from_millis(200));
            continue;
        };
        // Only our own user may drive Mochi or answer for the agent.
        if !same_user(&stream) {
            eprintln!("coucou-daemon: refused a connection from another user");
            continue;
        }
        let handler = handler.clone();
        thread::spawn(move || handler(stream));
    }
}

/// One relay connection: one hook event.
fn handle_relay(hub: &Hub, mut stream: UnixStream, timeouts: Timeouts) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    // The relay's ancestors lead to the session's terminal window ("Open
    // terminal"). Walked before reading: a relay with nothing to wait for
    // exits right after its line, and then /proc no longer has it.
    let ancestors = peer_pid(&stream).map(crate::terminal::process_ancestors).unwrap_or_default();
    let Some(line) = protocol::read_line(&mut stream, protocol::MAX_LINE) else { return };
    let Some((event, mut payload)) = protocol::parse_hook(&line) else { return };
    hub.note(&event, &payload);
    hub.windows.note(&payload, &event, ancestors);

    if event != "PermissionRequest" {
        hub.broadcast(&json!({ "type": "hook", "payload": protocol::slim(&payload) }));
        return;
    }
    // Nobody to show the card to: the terminal takes over at once.
    if !hub.has_followers() {
        return;
    }
    let (id, rx) = match hub.register(&mut payload) {
        Registration::Pending(id, rx) => (id, rx),
        Registration::Busy => {
            payload["coucou_declined"] = json!(true);
            hub.broadcast(&json!({ "type": "hook", "payload": protocol::slim(&payload) }));
            return;
        }
    };
    hub.broadcast(&json!({ "type": "hook", "payload": protocol::slim(&payload) }));

    let fd = stream.as_raw_fd();
    let (decision, outcome) = wait_for_decision(&rx, timeouts, &|| peer_alive(fd));
    hub.finish(&id);
    let word = match (&decision, outcome) {
        (Some(d), Outcome::Answered) => protocol::outcome_word(d).to_string(),
        _ => outcome.word().to_string(),
    };
    hub.broadcast(&json!({ "type": "resolved", "request_id": id, "outcome": word }));
    eprintln!("coucou-daemon: request {id} {word}");

    // No decision: say nothing at all, and coucou-hook stays silent too.
    if let Some(d) = decision {
        let _ = stream.write_all(format!("{d}\n").as_bytes());
        let _ = stream.flush();
    }
}

/// One control connection: one command.
fn handle_control(hub: &Hub, mut stream: UnixStream) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let Some(line) = protocol::read_line(&mut stream, 64 * 1024) else { return };
    let cmd = match protocol::parse_control(&String::from_utf8_lossy(&line)) {
        Ok(cmd) => cmd,
        Err(err) => {
            let _ = writeln!(stream, "err {err}");
            return;
        }
    };
    let delivered = match cmd {
        Control::Follow => return follow_loop(hub, stream),
        Control::Status => {
            let _ = writeln!(stream, "{}", hub.status_json());
            return;
        }
        Control::Ack(id) => hub.reply(&id, Reply::Ack),
        Control::Decline(id) => hub.reply(&id, Reply::Decline),
        Control::Decide(id, d) => hub.reply(&id, Reply::Decision(d)),
        Control::Window(session) => {
            let answer = match hub.windows.lookup(&session) {
                Some(pids) => format!("ok {}", crate::terminal::encode_pids(&pids)),
                None => "err unknown session".to_string(),
            };
            let _ = writeln!(stream, "{answer}");
            return;
        }
    };
    let _ = writeln!(stream, "{}", if delivered { "ok" } else { "err unknown request" });
}

fn follow_loop(hub: &Hub, mut stream: UnixStream) {
    let (rx, hello) = hub.follow();
    if writeln!(stream, "{hello}").is_err() {
        return;
    }
    for line in rx {
        if writeln!(stream, "{line}").and_then(|_| stream.flush()).is_err() {
            return;
        }
    }
}

// ── Client side ───────────────────────────────────────────────────────────────

fn connect(control: &Path) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(control)?;
    if !same_user(&stream) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "control socket served by another user"));
    }
    Ok(stream)
}

/// Sends one command and returns the daemon's one-line answer.
pub fn request(control: &Path, command: &str) -> io::Result<String> {
    let mut stream = connect(control)?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    writeln!(stream, "{command}")?;
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer)?;
    Ok(answer.trim().to_string())
}

/// Copies the event stream to `out` until the daemon goes away.
pub fn follow(control: &Path, out: &mut impl Write) -> io::Result<()> {
    let mut stream = connect(control)?;
    writeln!(stream, "follow")?;
    for line in BufReader::new(stream).lines() {
        writeln!(out, "{}", line?)?;
        out.flush()?;
    }
    Ok(())
}

pub fn status_or_offline(control: &Path) -> Value {
    request(control, "status")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({ "running": false }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::mpsc;
    use std::path::PathBuf;
    use std::time::Instant;

    type Sockets = (PathBuf, PathBuf);

    fn temp_sockets(name: &str) -> Sockets {
        let dir = std::env::temp_dir().join(format!("coucou-srv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        (dir.join("coucou.sock"), dir.join("coucou-ctl.sock"))
    }

    fn short() -> Timeouts {
        Timeouts { ack: Duration::from_millis(150), decision: Duration::from_millis(400) }
    }

    /// Follows the daemon in a thread; lines arrive on the returned channel.
    fn follower(control: PathBuf) -> mpsc::Receiver<Value> {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let stream = connect(&control).unwrap();
            let mut w = stream.try_clone().unwrap();
            writeln!(w, "follow").unwrap();
            for line in BufReader::new(stream).lines() {
                let Ok(line) = line else { return };
                if tx.send(serde_json::from_str::<Value>(&line).unwrap()).is_err() {
                    return;
                }
            }
        });
        let hello = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(hello["type"], "hello");
        rx
    }

    /// Plays coucou-hook: sends a line and returns what the daemon answered.
    fn relay(path: PathBuf, line: &'static str) -> thread::JoinHandle<String> {
        thread::spawn(move || {
            let mut s = UnixStream::connect(&path).unwrap();
            s.write_all(line.as_bytes()).unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
            out
        })
    }

    const PERMISSION: &str = "{\"hook_event_name\":\"PermissionRequest\",\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"ls\"}}\n";

    fn next_of(rx: &mpsc::Receiver<Value>, kind: &str) -> Value {
        loop {
            let v = rx.recv_timeout(Duration::from_secs(3)).expect("event");
            if v["type"] == kind {
                return v;
            }
        }
    }

    #[test]
    fn an_acknowledged_click_goes_back_to_the_relay() {
        let (r, c) = temp_sockets("allow");
        let _hub = start(&r, &c, short()).unwrap();
        let events = follower(c.clone());
        let answer = relay(r, PERMISSION);
        let hook = next_of(&events, "hook");
        let id = hook["payload"]["request_id"].as_str().unwrap().to_string();
        assert_eq!(hook["payload"]["tool_name"], "Bash");
        assert_eq!(request(&c, &format!("ack {id}")).unwrap(), "ok");
        assert_eq!(request(&c, &format!("decide {id} allow")).unwrap(), "ok");
        assert_eq!(answer.join().unwrap(), "allow\n");
        let resolved = next_of(&events, "resolved");
        assert_eq!(resolved["outcome"], "allow");
        assert_eq!(request(&c, &format!("decide {id} allow")).unwrap(), "err unknown request");
    }

    #[test]
    fn no_ack_or_no_decision_is_silence() {
        let (r, c) = temp_sockets("silence");
        let _hub = start(&r, &c, short()).unwrap();
        let events = follower(c.clone());

        let start_at = Instant::now();
        assert_eq!(relay(r.clone(), PERMISSION).join().unwrap(), "");
        assert!(start_at.elapsed() < Duration::from_millis(400));
        assert_eq!(next_of(&events, "resolved")["outcome"], "unacked");

        let answer = relay(r, PERMISSION);
        let id = next_of(&events, "hook")["payload"]["request_id"].as_str().unwrap().to_string();
        request(&c, &format!("ack {id}")).unwrap();
        assert_eq!(answer.join().unwrap(), "");
        assert_eq!(next_of(&events, "resolved")["outcome"], "timeout");
    }

    #[test]
    fn without_a_plugin_the_terminal_takes_over_at_once() {
        let (r, c) = temp_sockets("alone");
        let _hub = start(&r, &c, Timeouts { ack: Duration::from_secs(5), decision: Duration::from_secs(5) }).unwrap();
        let start_at = Instant::now();
        assert_eq!(relay(r, PERMISSION).join().unwrap(), "");
        assert!(start_at.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_second_request_is_declined_while_one_is_pending() {
        let (r, c) = temp_sockets("busy");
        let _hub = start(&r, &c, short()).unwrap();
        let events = follower(c.clone());
        let first = relay(r.clone(), PERMISSION);
        let id = next_of(&events, "hook")["payload"]["request_id"].as_str().unwrap().to_string();
        request(&c, &format!("ack {id}")).unwrap();
        assert_eq!(relay(r, PERMISSION).join().unwrap(), "");
        let declined = next_of(&events, "hook");
        assert_eq!(declined["payload"]["coucou_declined"], true);
        request(&c, &format!("decide {id} {}", r#"{"Which?":"A"}"#)).unwrap();
        assert_eq!(first.join().unwrap(), "{\"answers\":{\"Which?\":\"A\"}}\n");
    }

    #[test]
    fn other_events_are_broadcast_and_status_follows() {
        let (r, c) = temp_sockets("events");
        let _hub = start(&r, &c, short()).unwrap();
        let events = follower(c.clone());
        let out = relay(r, "{\"hook_event_name\":\"PreToolUse\",\"tool_name\":\"Edit\",\"coucou_agent\":\"gemini\"}\n");
        assert_eq!(out.join().unwrap(), "");
        let hook = next_of(&events, "hook");
        assert_eq!(hook["payload"]["hook_event_name"], "PreToolUse");
        let status = status_or_offline(&c);
        assert_eq!(status["running"], true);
        assert_eq!(status["agent"], "gemini");
        assert_eq!(status["tool"], "Edit");
        assert_eq!(request(&c, "bogus").unwrap(), "err unknown command: bogus");
    }

    #[test]
    fn a_session_remembers_where_its_relay_ran() {
        let (r, c) = temp_sockets("window");
        let _hub = start(&r, &c, short()).unwrap();
        // The relay here is a thread of this test: SO_PEERCRED gives our pid.
        let out = relay(r.clone(), "{\"hook_event_name\":\"PreToolUse\",\"session_id\":\"test-win-1\"}\n");
        assert_eq!(out.join().unwrap(), "");
        let expected = crate::terminal::process_ancestors(std::process::id());
        let answer = request(&c, "window test-win-1").unwrap();
        if expected.is_empty() {
            assert_eq!(answer, "err unknown session");
        } else {
            assert_eq!(answer, format!("ok {}", crate::terminal::encode_pids(&expected)));
        }
        assert_eq!(request(&c, "window test-never").unwrap(), "err unknown session");
        relay(r, "{\"hook_event_name\":\"SessionEnd\",\"session_id\":\"test-win-1\"}\n").join().unwrap();
        assert_eq!(request(&c, "window test-win-1").unwrap(), "err unknown session");
    }

    #[test]
    fn a_live_socket_is_never_taken_over_and_a_stale_one_is() {
        let (r, c) = temp_sockets("bind");
        let _hub = start(&r, &c, short()).unwrap();
        assert_eq!(bind(&r).unwrap_err().kind(), io::ErrorKind::AddrInUse);
        let stale = r.with_file_name("stale.sock");
        drop(UnixListener::bind(&stale).unwrap());
        assert!(bind(&stale).is_ok());
        assert_eq!(status_or_offline(&r.with_file_name("none.sock"))["running"], false);
    }
}
