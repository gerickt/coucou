//! coucou-daemon — the bridge between coucou-hook and the Noctalia plugin.
//!
//! ```text
//! coucou-daemon                       serve both sockets (default)
//! coucou-daemon --follow              print one JSON line per event (plugin stream)
//! coucou-daemon --status-json         print the status snapshot
//! coucou-daemon ack <id>              the card is on screen
//! coucou-daemon decline <id>          nobody can act on it, hand back to the terminal
//! coucou-daemon decide <id> <allow|deny|answers-json>
//! ```
//!
//! `COUCOU_ACK_TIMEOUT_MS` and `COUCOU_DECISION_TIMEOUT_MS` override the
//! 800 ms / 108 s waits (for testing).

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

mod hub;
mod paths;
mod protocol;
mod server;

const USAGE: &str = "usage: coucou-daemon [--follow | --status-json | ack <id> | decline <id> | decide <id> <allow|deny|answers-json>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = paths::runtime_dir();
    let control = paths::control_socket(&dir);
    match args.first().map(String::as_str) {
        None | Some("serve") => serve(&dir),
        Some("--follow") => follow(&control),
        Some("--status-json") => {
            println!("{}", server::status_or_offline(&control));
            ExitCode::SUCCESS
        }
        Some(verb @ ("ack" | "decline" | "decide")) => send(&control, &format!("{verb} {}", args[1..].join(" "))),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("unknown argument: {other}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn env_ms(key: &str) -> Option<Duration> {
    std::env::var(key).ok()?.parse::<u64>().ok().map(Duration::from_millis)
}

fn serve(dir: &Path) -> ExitCode {
    if !paths::is_private_dir(dir) {
        eprintln!("coucou-daemon: {} is not a private runtime directory (0700, owned by you)", dir.display());
        return ExitCode::FAILURE;
    }
    let defaults = hub::Timeouts::default();
    let timeouts = hub::Timeouts {
        ack: env_ms("COUCOU_ACK_TIMEOUT_MS").unwrap_or(defaults.ack),
        decision: env_ms("COUCOU_DECISION_TIMEOUT_MS").unwrap_or(defaults.decision),
    };
    let relay = paths::relay_socket(dir);
    let control = paths::control_socket(dir);
    if let Err(err) = server::start(&relay, &control, timeouts) {
        eprintln!("coucou-daemon: {err}");
        return ExitCode::FAILURE;
    }
    eprintln!("coucou-daemon: listening on {} and {}", relay.display(), control.display());
    loop {
        std::thread::park();
    }
}

/// The plugin's stream. Never exits on its own while stdout is open: while the
/// daemon is away it says so once and retries, so the plugin keeps one
/// long-lived process.
fn follow(control: &Path) -> ExitCode {
    let mut out = std::io::stdout();
    let mut announced = false;
    loop {
        match server::follow(control, &mut out) {
            Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => return ExitCode::SUCCESS,
            Ok(()) => announced = false,
            Err(_) => {}
        }
        if !announced {
            use std::io::Write;
            if writeln!(out, r#"{{"type":"daemon","running":false}}"#).and_then(|_| out.flush()).is_err() {
                return ExitCode::SUCCESS;
            }
            announced = true;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn send(control: &Path, command: &str) -> ExitCode {
    match server::request(control, command) {
        Ok(answer) if answer == "ok" => ExitCode::SUCCESS,
        Ok(answer) => {
            eprintln!("coucou-daemon: {answer}");
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("coucou-daemon: daemon not reachable: {err}");
            ExitCode::FAILURE
        }
    }
}
